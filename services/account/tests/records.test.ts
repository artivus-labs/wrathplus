import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { EventPump, NSID, oauthPublisher, parseEvents, recordsFor, type Publisher, type RecordWrite } from "../src/records.ts";
import { ALICE_DID, FakeRealm } from "./fakes.ts";

const REALM = "wrathplus.artivus.com.au";
const ev = (id: number, kind: string, value: number, extra = "") => `EV ${id} 1762646400 ${ALICE_DID} 42 ${kind} ${value} 12 1 2 0 Thrall${extra}`;

describe("parseEvents", () => {
  test("parses lines, keeps titles with spaces, skips junk", () => {
    const out = parseEvents([ev(1, "quest", 783, " A Threat Within"), "garbage", "EV x", ev(2, "level", 12), `EV 3 1 not-a-did 42 level 2 2 1 1 0 X`, ev(4, "bogus", 1)].join("\n"));
    expect(out.map((e) => e.id)).toEqual([1, 2]);
    expect(out[0]).toMatchObject({ did: ALICE_DID, guid: 42, kind: "quest", value: 783, name: "Thrall", title: "A Threat Within", race: 1, class: 2 });
  });
});

describe("recordsFor", () => {
  const [login, level, quest, achievement] = parseEvents([ev(1, "login", 0), ev(2, "level", 12), ev(3, "quest", 783, " A Threat Within"), ev(4, "achievement", 6, " Level 10")].join("\n"));
  test("login -> character record keyed by realm and guid", () => {
    const w = recordsFor(login!, { realm: REALM, publishQuests: false });
    expect(w).toHaveLength(1);
    expect(w[0]).toMatchObject({ collection: NSID.character, rkey: `${REALM}:42` });
    expect(w[0]!.record).toMatchObject({ $type: NSID.character, name: "Thrall", level: 12, updatedAt: "2025-11-09T00:00:00.000Z" });
  });
  test("level -> character + milestone with a content key (idempotent)", () => {
    const w = recordsFor(level!, { realm: REALM, publishQuests: false });
    expect(w.map((x) => x.collection)).toEqual([NSID.character, NSID.milestone]);
    expect(w[1]!.rkey).toBe(`${REALM}:42:level:12`);
  });
  test("quests only when enabled, keyed with time (dailies repeat)", () => {
    expect(recordsFor(quest!, { realm: REALM, publishQuests: false })).toEqual([]);
    const w = recordsFor(quest!, { realm: REALM, publishQuests: true });
    expect(w[0]!.rkey).toBe(`${REALM}:42:quest:783:1762646400`);
    expect(w[0]!.record.title).toBe("A Threat Within");
  });
  test("achievement -> milestone", () => {
    const w = recordsFor(achievement!, { realm: REALM, publishQuests: false });
    expect(w[0]).toMatchObject({ collection: NSID.milestone, rkey: `${REALM}:42:achievement:6` });
  });
  test("record keys are valid ATProto record keys", () => {
    for (const e of [login, level, quest, achievement]) {
      for (const w of recordsFor(e!, { realm: REALM, publishQuests: true })) expect(w.rkey).toMatch(/^[A-Za-z0-9._:~-]{1,512}$/);
    }
  });
});

describe("EventPump", () => {
  test("publishes in order and acks the last handled event", async () => {
    const realm = new FakeRealm();
    realm.events = [ev(1, "login", 0), ev(2, "level", 2), ev(3, "achievement", 6)];
    const seen: string[] = [];
    const publish: Publisher = async (_did, writes) => {
      seen.push(...writes.map((w) => w.rkey));
      return "published";
    };
    const r = await new EventPump({ soap: realm.run, publish, realm: REALM, publishQuests: false }).tick();
    expect(r).toEqual({ handled: 3, acked: 3 });
    expect(realm.acked).toEqual([3]);
    expect(seen.length).toBe(4);
  });

  test("stops at a transient failure and retries it next time; gives up after maxAttempts", async () => {
    const realm = new FakeRealm();
    realm.events = [ev(1, "login", 0), ev(2, "level", 2), ev(3, "level", 3)];
    const publish: Publisher = async (_d, writes: RecordWrite[]) => (writes.some((w) => w.rkey.endsWith(":level:2")) ? "retry" : "published");
    const pump = new EventPump({ soap: realm.run, publish, realm: REALM, publishQuests: false, maxAttempts: 3, log: () => {} });
    expect(await pump.tick()).toEqual({ handled: 1, acked: 1 });
    expect(await pump.tick()).toEqual({ handled: 0, acked: undefined });
    expect(await pump.tick()).toEqual({ handled: 2, acked: 3 });
    expect(realm.events).toEqual([]);
  });

  test("events for DIDs without a session are acknowledged and dropped", async () => {
    const realm = new FakeRealm();
    realm.events = [ev(1, "level", 2)];
    const publish = oauthPublisher(async () => {
      throw new Error("unused");
    }, () => false);
    expect(await new EventPump({ soap: realm.run, publish, realm: REALM, publishQuests: false }).tick()).toEqual({ handled: 1, acked: 1 });
  });

  test("oauthPublisher writes putRecord with the session and maps statuses", async () => {
    const calls: Array<{ path: string; body: Record<string, unknown> }> = [];
    let status = 200;
    const session = {
      fetchHandler: async (path: string, init?: RequestInit) => {
        calls.push({ path, body: JSON.parse(String(init?.body)) });
        return new Response("{}", { status });
      },
    };
    const publish = oauthPublisher(async () => session, () => true, () => {});
    const writes = recordsFor(parseEvents(ev(2, "level", 12))[0]!, { realm: REALM, publishQuests: false });
    expect(await publish(ALICE_DID, writes)).toBe("published");
    expect(calls[0]).toMatchObject({ path: "/xrpc/com.atproto.repo.putRecord", body: { repo: ALICE_DID, collection: NSID.character } });
    status = 503;
    expect(await publish(ALICE_DID, writes)).toBe("retry");
    status = 403;
    expect(await publish(ALICE_DID, writes)).toBe("rejected");
  });
});

describe("lexicons", () => {
  const dir = join(import.meta.dir, "..", "lexicons");
  const files = readdirSync(dir).filter((f) => f.endsWith(".json"));
  test("one file per NSID we write, named after it", () => {
    expect(files.sort()).toEqual([`${NSID.character}.json`, `${NSID.milestone}.json`]);
  });
  test("records we build carry every required field of their lexicon", () => {
    const records = recordsFor(parseEvents(ev(2, "level", 12, " x"))[0]!, { realm: REALM, publishQuests: true });
    for (const w of records) {
      const lex = JSON.parse(readFileSync(join(dir, `${w.collection}.json`), "utf8"));
      expect(lex.id).toBe(w.collection);
      const main = lex.defs.main;
      expect(main.type).toBe("record");
      expect(main.key).toBe("any");
      for (const field of main.record.required as string[]) expect(w.record[field]).toBeDefined();
      for (const field of Object.keys(w.record).filter((k) => k !== "$type")) expect(main.record.properties[field]).toBeDefined();
    }
  });
});
