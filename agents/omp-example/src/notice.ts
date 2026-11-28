// What the character passes (wp.notice's AGENT_NOTICED), for the planner: notices are batched into
// one wake per window rather than one each, and each check-in carries a digest of every notice
// since the last one, muted or not (read from the client's Notice.passed), so reflection can see
// what was walked past.

import type { GameEvent, Inbox } from "./wake.ts";

export const NOTICED = "AGENT_NOTICED";

/** One AGENT_NOTICED(kind, name, guid, distance, x, y, extra). */
export interface Notice {
  kind: string;
  name: string;
  guid?: string;
  distance?: number;
  extra?: string;
}

const str = (v: unknown): string => (v === undefined || v === null ? "" : String(v));

export function parseNotice(e: GameEvent): Notice {
  const [kind, name, guid, distance, , , extra] = e.args;
  const d = Number(distance);
  return {
    kind: str(kind),
    name: str(name) || "?",
    guid: str(guid) || undefined,
    distance: distance === null || distance === undefined || !Number.isFinite(d) ? undefined : d,
    extra: str(extra) || undefined,
  };
}

/** How a kind reads in a sentence. */
const KIND_WORDS: Record<string, string> = {
  quest_available: "quest giver",
  quest_turnin: "quest to hand in",
  trainer: "trainer",
  vendor: "vendor",
  repair: "repairs",
  flightmaster: "flight master",
  innkeeper: "innkeeper",
  banker: "banker",
  mailbox: "mailbox",
  auctioneer: "auctioneer",
  rare: "rare",
  elite: "elite",
  node: "node",
  chest: "chest",
  player: "player",
  subzone: "subzone",
  portal: "portal to",
};

/** A kind's word, and what the notice adds about it: quest giver "!", flight master "path not
 *  learnt", vendor "Weaponsmith". */
function kindText(n: Notice): { word: string; detail?: string } {
  const word = KIND_WORDS[n.kind] ?? n.kind;
  switch (n.kind) {
    case "quest_available":
      return { word: "quest giver", detail: n.extra === "trivial" ? "grey !" : "!" };
    case "quest_turnin":
      return { word: "quest giver", detail: "?" };
    case "flightmaster":
      return { word, detail: n.extra === "unknown" ? "path not learnt" : undefined };
    default:
      return { word, detail: n.extra };
  }
}

/** AGENT_NOTICED events collected for `windowMs` from the first, then given to the planner as one
 *  line, nearest first. A window of 0 hands each over at once. They bypass the repeat suppression:
 *  every notice is new by construction (the client signals each thing once). */
export class NoticeBatch {
  #items: Notice[] = [];
  #opened: number | undefined;

  constructor(
    readonly windowMs: number,
    /** Most things listed in one line; the rest are counted. */
    readonly listed = 12,
  ) {}

  add(e: GameEvent, now: number): void {
    if (this.#opened === undefined) this.#opened = now;
    this.#items.push(parseNotice(e));
  }

  get size(): number {
    return this.#items.length;
  }

  /** Whether the window has run out (and there is something to say). */
  due(now: number): boolean {
    return this.#opened !== undefined && now - this.#opened >= this.windowMs;
  }

  /** The line, and start a new window; null when nothing was noticed. */
  take(): string | null {
    if (!this.#items.length) return null;
    // One entry per thing: an NPC with several services is one entry with them all.
    const things = new Map<string, { n: Notice; words: string[]; details: string[] }>();
    for (const n of this.#items) {
      const key = n.guid ? `guid ${n.guid}` : `${n.kind} ${n.name}`;
      const thing = things.get(key) ?? { n, words: [], details: [] };
      things.set(key, thing);
      const { word, detail } = kindText(n);
      if (!thing.words.includes(word)) thing.words.push(word);
      if (detail && !thing.details.includes(detail)) thing.details.push(detail);
    }
    const sorted = [...things.values()].sort((a, b) => (a.n.distance ?? 0) - (b.n.distance ?? 0));
    const parts = sorted.slice(0, this.listed).map(({ n, words, details }) => {
      const detail = details.length ? ` (${details.join(", ")})` : "";
      const far = n.kind === "subzone" || n.distance === undefined ? "" : ` ${Math.round(n.distance)} yd`;
      return `${words.join(", ")} ${n.name}${detail}${far}`;
    });
    const more = sorted.length > this.listed ? `; and ${sorted.length - this.listed} more` : "";
    this.#items = [];
    this.#opened = undefined;
    return `Noticed: ${parts.join("; ")}${more}. (Notice.recent has them; Notice.mute(kind[, filter]) stops a kind waking you.)`;
  }
}

/** The Lua the launcher runs for the digest: Notice.passed after its cursor. */
export function passedLua(since: number): string {
  return `return Notice and Notice.passed(${Math.max(0, Math.floor(since))})`;
}

interface PassedKind {
  kind: string;
  count: number;
  muted: number;
  names: string[];
}

/** A `/lua` answer of Notice.passed, as the check-in's evidence lines, and the cursor to read
 *  from the next time (undefined when the answer was not one). */
export function digestLines(answer: unknown): { lines: string[]; next?: number } {
  const a = answer as { ok?: boolean; values?: unknown[] } | undefined;
  const passed = (a?.ok ? a.values?.[0] : undefined) as { next?: number; dropped?: number; kinds?: PassedKind[] } | undefined;
  if (!passed || typeof passed !== "object" || typeof passed.next !== "number") return { lines: [] };
  // An empty Lua table reads as an object, not a list.
  const kinds = Array.isArray(passed.kinds) ? passed.kinds : [];
  if (!kinds.length) return { lines: [], next: passed.next };
  const total = kinds.reduce((s, k) => s + (k.count || 0), 0);
  const parts = kinds.map((k) => {
    const names = Array.isArray(k.names) && k.names.length ? `: ${k.names.map(str).join(", ")}${k.count > k.names.length ? ", …" : ""}` : "";
    const muted = k.muted ? `, ${k.muted} muted` : "";
    return `${k.count} ${KIND_WORDS[k.kind] ?? k.kind}${muted}${names}`;
  });
  const dropped = passed.dropped ? `, ${passed.dropped} more no longer kept` : "";
  return { lines: [`passed since the last check-in (${total} noticed${dropped}): ${parts.join("; ")}`], next: passed.next };
}

/** Hand the batch to the inbox as one notable line once its window is over, or sooner when the
 *  planner is being woken for something else anyway (the notices ride along). Urgent events never
 *  wait for it: they reach the inbox on their own. Returns how many notices went. */
export function flushNotices(batch: NoticeBatch, inbox: Inbox, now: number): number {
  if (!batch.size || (!batch.due(now) && !inbox.size)) return 0;
  const count = batch.size;
  const line = batch.take();
  if (line) inbox.note(line, "notable");
  return count;
}
