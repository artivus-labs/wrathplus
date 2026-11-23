import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { gaps, NEEDS } from "../src/needs.ts";
import { characterKey, needGap, type SamplePort, SampleStore, Sampler, score, scorecard, WATCHED, xpGained } from "../src/scorecard.ts";
import { sample, scratch } from "./fixtures.ts";

const MIN = 60_000;
const opts = { maxGapMs: 90_000 };

/** A stubbed control port: in the world, the Lua answer and PLAYER_DEAD events set by the test. */
class StubPort implements SamplePort {
  inWorld = true;
  time = 100;
  raw: Record<string, unknown> = {};
  fns: Record<string, string> = Object.fromEntries(WATCHED.map((n) => [n, `function: builtin: ${n}`]));
  dead = 0;
  seq = 0;
  calls: string[] = [];

  async status() {
    this.calls.push("status");
    return { in_world: this.inWorld, time: this.time, player: this.inWorld ? { name: "Agentsix" } : null };
  }
  async lua(code: string) {
    this.calls.push(code.includes("GetSkillLineInfo") ? "sample" : "lua");
    return { ok: true, values: [{ ...this.raw, fns: { ...this.fns } }] };
  }
  async events(query: { since?: number; names?: string[] }) {
    this.calls.push(`events since=${query.since} names=${query.names?.join(",")}`);
    const events = query.since === undefined ? [] : Array.from({ length: this.dead }, () => ({ event: "PLAYER_DEAD" }));
    this.dead = 0;
    this.seq += events.length + 1;
    return { events, next: this.seq, missed: 0 };
  }
}

const RAW = {
  name: "Agentsix",
  realm: "Beta",
  level: 10,
  xp: 500,
  xp_max: 10_000,
  money: 1234,
  dead: false,
  combat: false,
  idle: true,
  zone: "Elwynn Forest",
  subzone: "Goldshire",
  gear: { average: 7.5, items: 10, empty: 7, uncached: 0 },
  professions: { Cooking: { rank: 20, max: 75 } },
  spells: 22,
  quests: { "7": true, "33": false },
};

describe("sampler", () => {
  test("samples through the stock API, stores per character and counts deaths from events", async () => {
    const dir = scratch();
    const store = new SampleStore(dir);
    const port = new StubPort();
    port.raw = RAW;
    let now = 1_000_000;
    const sampler = new Sampler(port, { store, now: () => now });
    const first = await sampler.tick();
    expect(first).toMatchObject({ level: 10, money: 1234, deaths: 0, tampered: [], ui_reloaded: false, client_time: 100 });
    expect(port.calls).toEqual(["status", "events since=undefined names=PLAYER_DEAD", "sample"]);
    port.dead = 1;
    now += 30_000;
    const second = await sampler.tick();
    expect(second?.deaths).toBe(1);
    const key = characterKey("Agentsix", "Beta");
    const lines = readFileSync(join(dir, key, "samples.jsonl"), "utf8").trim().split("\n");
    expect(lines).toHaveLength(2);
    expect(new SampleStore(dir).all(key).map((s) => s.deaths)).toEqual([0, 1]);
  });

  test("out of the world nothing is stored and the reason is kept", async () => {
    const store = new SampleStore(scratch());
    const port = new StubPort();
    port.inWorld = false;
    const sampler = new Sampler(port, { store });
    expect(await sampler.tick()).toBeUndefined();
    expect(sampler.lastSkip).toBe("not in the world");
    expect(port.calls).toEqual(["status"]);
  });

  test("a stock function replaced between samples marks the sample tampered; all of them is a new UI", async () => {
    const port = new StubPort();
    port.raw = RAW;
    const sampler = new Sampler(port, { store: new SampleStore(scratch()) });
    await sampler.tick();
    port.fns.UnitXP = "function: 0xbad";
    expect((await sampler.tick())?.tampered).toEqual(["UnitXP"]);
    port.fns = Object.fromEntries(WATCHED.map((n) => [n, `function: builtin2: ${n}`]));
    const reloaded = await sampler.tick();
    expect(reloaded?.tampered).toEqual([]);
    expect(reloaded?.ui_reloaded).toBe(true);
  });

  test("empty Lua tables arriving as arrays become empty maps", async () => {
    const port = new StubPort();
    port.raw = { ...RAW, professions: [], quests: [] };
    const s = await new Sampler(port, { store: new SampleStore(scratch()) }).tick();
    expect(s?.professions).toEqual({});
    expect(s?.quests).toEqual({});
  });
});

describe("score", () => {
  test("experience counts across a level-up", () => {
    expect(xpGained(sample(0, { xp: 9000 }), sample(1, { level: 11, xp: 300, xp_max: 11_000 }))).toBe(1300);
    expect(xpGained(sample(0, { xp: 100 }), sample(1, { xp: 400 }))).toBe(300);
  });

  test("counts quests, exploration, skill, spells, money and idle share as rates per hour", () => {
    const before = [sample(-MIN, { subzone: "Goldshire" })];
    const w = [
      sample(0, { xp: 0, money: 100, quests: { "1": true, "2": false }, professions: { Cooking: { rank: 10, max: 75 } } }),
      sample(MIN, { xp: 500, money: 200, quests: { "2": false, "3": false }, professions: { Cooking: { rank: 11, max: 75 } }, subzone: "Northshire", idle: true }),
      sample(2 * MIN, { xp: 1000, money: 400, quests: { "3": false }, professions: { Cooking: { rank: 13, max: 75 } }, spells: 21, subzone: "Northshire" }),
    ];
    const s = score(w, before, opts);
    expect(s.hours).toBeCloseTo(2 / 60);
    expect(s).toMatchObject({ xp: 1000, money: 300, quests_done: 1, quests_accepted: 1, new_subzones: 1, new_zones: 0, skill_ups: 3, spells_learned: 1 });
    expect(s.idle_share).toBeCloseTo(1 / 3);
    expect(s.rates.xp_per_hour).toBeCloseTo(30_000);
    expect(s.rates.quests_per_hour).toBeCloseTo(30);
    expect(s.breaks).toEqual([]);
  });

  test("a restart, a long gap and tampering are breaks, and their stretches are not counted", () => {
    const w = [
      sample(0, { xp: 0 }),
      sample(MIN / 2, { xp: 100 }),
      sample(MIN, { xp: 200, client_time: 1 }),
      sample(5 * MIN, { xp: 900, client_time: 300 }),
      sample(5.5 * MIN, { xp: 1000, client_time: 330, tampered: ["UnitXP"] }),
    ];
    const s = score(w, [], opts);
    expect(s.breaks).toEqual(["the client restarted", "no sample for 240s", "stock functions replaced: UnitXP"]);
    expect(s.xp).toBe(200);
    expect(s.hours).toBeCloseTo(1 / 60);
  });

  test("deaths come from each sample after the first", () => {
    const s = score([sample(0, { deaths: 3 }), sample(MIN / 2, { deaths: 1 }), sample(MIN, { deaths: 0 })], [], opts);
    expect(s.deaths).toBe(1);
  });
});

describe("needs", () => {
  test("gaps rise with level and are zero when met", () => {
    const at10 = gaps({ level: 10, money: 1000, gear: { average: 3, items: 5 }, professions: { Cooking: { rank: 10, max: 75 } } });
    expect(at10).toEqual([
      { need: "profession:Cooking", target: 50, have: 10, gap: 15, share: 0.3, unit: "skill" },
      { need: "gear", target: 10, have: 3, gap: 2, share: 0.2, unit: "item level" },
      { need: "gold", target: 2500, have: 1000, gap: 1500, share: 0.6, unit: "copper" },
    ]);
    const met = gaps({ level: 10, money: 5000, gear: { average: 12, items: 5 }, professions: { Cooking: { rank: 30, max: 75 } } });
    expect(met.map((g) => g.gap)).toEqual([0, 0, 0]);
    // Ten levels on with nothing done, the same character falls behind on every need.
    const at20 = gaps({ level: 20, money: 5000, gear: { average: 12, items: 5 }, professions: { Cooking: { rank: 30, max: 75 } } });
    expect(at20.map((g) => g.gap)).toEqual([45, 3, 5000]);
    expect(NEEDS.profession.target(90)).toBe(450);
  });

  test("the scorecard sums the shares into one needs number", () => {
    expect(needGap(sample(0, { level: 10, money: 1000, gear: { average: 3, items: 5, empty: 0, uncached: 0 } }))).toBeCloseTo(0.8);
  });
});

describe("scorecard", () => {
  test("reports current values, needs and trends over spans", () => {
    const store = new SampleStore(scratch());
    const now = 300 * MIN;
    for (let i = 0; i <= 120; i++) {
      store.add(sample(now - (120 - i) * MIN / 2, { xp: i * 50, money: i * 10, subzone: i < 100 ? "Goldshire" : "Northshire" }));
    }
    const card = scorecard(store, characterKey("Agentsix", "Beta"), now, opts);
    expect(card.current).toMatchObject({ level: 10, xp: 6000, money: 1200, subzones_visited: 2, zones_visited: 1 });
    expect(Object.keys(card.trends)).toEqual(["last_15m", "last_60m", "last_240m", "all"]);
    expect(card.trends.last_60m!.rates.xp_per_hour).toBeCloseTo(6000);
    expect(card.trends.last_15m!.new_subzones).toBe(1);
    expect(card.needs.map((n) => n.need)).toEqual(["gear", "gold"]);
  });
});
