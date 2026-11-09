// Realm progress -> ATProto records in the player's own repository.
//
// mod-atproto-identity keeps an append-only queue of progress events for linked accounts and
// hands them out over SOAP (`.atproto events N`, one `EV …` line each); we publish them and then
// acknowledge (`.atproto ack ID`), which deletes them on the realm. Record keys are derived from
// the event's content, never from its queue id, so publishing an event twice (a crash between
// publish and ack) overwrites the same record instead of duplicating it.

import { commands, type SoapRun } from "./soap.ts";
import { isDid } from "./identity.ts";

export const NSID = {
  character: "au.com.artivus.wrathplus.character",
  milestone: "au.com.artivus.wrathplus.milestone",
} as const;

export const EVENT_KINDS = ["login", "level", "quest", "achievement"] as const;
export type EventKind = (typeof EVENT_KINDS)[number];

export interface RealmEvent {
  id: number;
  at: number;
  did: string;
  guid: number;
  kind: EventKind;
  value: number;
  level: number;
  race: number;
  class: number;
  gender: number;
  name: string;
  title: string;
}

const UINT = /^[0-9]{1,19}$/;

/** `EV id unix did guid kind value level race class gender name [title…]`, one per line. */
export function parseEvents(output: string): RealmEvent[] {
  const out: RealmEvent[] = [];
  for (const line of output.split("\n")) {
    const parts = line.trim().split(" ");
    if (parts[0] !== "EV" || parts.length < 12) continue;
    const [, id, at, did, guid, kind, value, level, race, cls, gender, name] = parts as [string, ...string[]];
    const numbers = [id, at, guid, value, level, race, cls, gender];
    if (!numbers.every((n) => n !== undefined && UINT.test(n))) continue;
    if (!did || !isDid(did) || !EVENT_KINDS.includes(kind as EventKind) || !name) continue;
    out.push({
      id: Number(id),
      at: Number(at),
      did,
      guid: Number(guid),
      kind: kind as EventKind,
      value: Number(value),
      level: Number(level),
      race: Number(race),
      class: Number(cls),
      gender: Number(gender),
      name,
      title: parts.slice(12).join(" ").trim(),
    });
  }
  return out;
}

export interface RecordWrite {
  collection: string;
  rkey: string;
  record: Record<string, unknown>;
}

const RKEY_UNSAFE = /[^A-Za-z0-9._:~-]/g;

export function characterRkey(realm: string, guid: number): string {
  return `${realm.replace(RKEY_UNSAFE, "-")}:${guid}`;
}

export interface RecordOptions {
  realm: string;
  publishQuests: boolean;
}

/** The writes one event turns into (possibly none). */
export function recordsFor(event: RealmEvent, options: RecordOptions): RecordWrite[] {
  const at = new Date(event.at * 1000).toISOString();
  const charKey = characterRkey(options.realm, event.guid);
  const writes: RecordWrite[] = [];
  if (event.kind === "login" || event.kind === "level") {
    writes.push({
      collection: NSID.character,
      rkey: charKey,
      record: {
        $type: NSID.character,
        realm: options.realm,
        guid: event.guid,
        name: event.name,
        race: event.race,
        class: event.class,
        gender: event.gender,
        level: event.level,
        updatedAt: at,
      },
    });
  }
  if (event.kind === "quest" && !options.publishQuests) return writes;
  if (event.kind !== "login") {
    // Quests repeat (dailies), levels and achievements do not.
    const suffix = event.kind === "quest" ? `:${event.at}` : "";
    const record: Record<string, unknown> = {
      $type: NSID.milestone,
      realm: options.realm,
      guid: event.guid,
      characterName: event.name,
      kind: event.kind,
      value: event.value,
      level: event.level,
      achievedAt: at,
    };
    if (event.title) record.title = event.title.slice(0, 256);
    writes.push({ collection: NSID.milestone, rkey: `${charKey}:${event.kind}:${event.value}${suffix}`, record });
  }
  return writes;
}

export type PublishResult = "published" | "no_session" | "rejected" | "retry";

/** Writes records to `did`'s repository with its stored OAuth session. */
export type Publisher = (did: string, writes: RecordWrite[]) => Promise<PublishResult>;

export interface PumpOptions extends RecordOptions {
  soap: SoapRun;
  publish: Publisher;
  batch?: number;
  maxAttempts?: number;
  log?: (message: string) => void;
}

/** One poll of the realm's queue. Stops at the first event that should be retried. */
export class EventPump {
  private readonly attempts = new Map<number, number>();

  constructor(private readonly options: PumpOptions) {}

  async tick(): Promise<{ handled: number; acked?: number }> {
    const { soap, publish, batch = 50, maxAttempts = 5, log = console.log } = this.options;
    const events = parseEvents(await soap(commands.events(batch)));
    let acked: number | undefined;
    let handled = 0;
    for (const event of events) {
      const writes = recordsFor(event, this.options);
      const result = writes.length ? await publish(event.did, writes).catch(() => "retry" as const) : "published";
      if (result === "retry") {
        const n = (this.attempts.get(event.id) ?? 0) + 1;
        this.attempts.set(event.id, n);
        if (n < maxAttempts) break;
        log(`records: giving up on event ${event.id} for ${event.did} after ${n} attempts`);
      }
      this.attempts.delete(event.id);
      acked = event.id;
      handled++;
    }
    if (acked !== undefined) await soap(commands.ack(acked));
    return { handled, acked };
  }
}

/** A Publisher over the OAuth client's `restore(did)` session. */
export function oauthPublisher(
  restore: (did: string) => Promise<{ fetchHandler(path: string, init?: RequestInit): Promise<Response> }>,
  hasSession: (did: string) => boolean,
  log: (message: string) => void = console.log,
): Publisher {
  return async (did, writes) => {
    if (!hasSession(did)) return "no_session";
    let session;
    try {
      session = await restore(did);
    } catch (e) {
      log(`records: no usable session for ${did}: ${(e as Error).message}`);
      return "no_session";
    }
    for (const w of writes) {
      const res = await session.fetchHandler("/xrpc/com.atproto.repo.putRecord", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ repo: did, collection: w.collection, rkey: w.rkey, record: w.record }),
      });
      if (res.status === 429 || res.status >= 500) return "retry";
      if (!res.ok) {
        log(`records: ${did} ${w.collection}/${w.rkey} rejected (${res.status}): ${(await res.text()).slice(0, 200)}`);
        return "rejected";
      }
    }
    return "published";
  };
}
