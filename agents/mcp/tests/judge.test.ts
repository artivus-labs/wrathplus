import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { armOf, DEFAULT_CONFIG, decide, Judge, type JudgeConfig, loadConfig, trials, type WindowRecord } from "../src/judge.ts";
import { judgeDir } from "../src/monitor.ts";
import { characterKey, type Rates, type Sample, SampleStore } from "../src/scorecard.ts";
import { FakePort, sample, scratch } from "./fixtures.ts";

const STEP = 30_000;
const MODULE = "grind.loop";
const CHAMPION = "-- pull everything\nreturn {}";
const CANDIDATE = "-- pull one at a time\nreturn {}";

/** Short windows for tests: 2 minutes, no settling, 2 pairs (+1 spare), 2 needed. */
const FAST: Partial<JudgeConfig> = { window_minutes: 2, settle_seconds: 0, pairs: 2, extra_pairs: 1, min_valid_pairs: 2 };

/** A world that plays whichever version of the module the client holds: `play` says what one
 *  30-second step does to the character under each. Samples go in the store, then the judge ticks. */
class World {
  dir = scratch();
  store = new SampleStore(join(this.dir, "characters"));
  port = new FakePort();
  now = 1_000_000_000;
  clientTime = 100;
  last: Sample | undefined;
  state = sample(0);
  judge: Judge;
  /** Changes per step, by which version is loaded (the module's source, or undefined for none). */
  play: (source: string | undefined, step: number) => Partial<Sample> = () => ({});
  steps = 0;

  constructor(config: Partial<JudgeConfig> = FAST) {
    writeFileSync(join(this.dir, "judge.json"), JSON.stringify(config));
    this.judge = new Judge({ dir: this.dir, port: this.port, store: this.store, latest: () => this.last, maxGapMs: 90_000, now: () => this.now });
  }

  async step(): Promise<void> {
    this.now += STEP;
    this.clientTime += STEP / 1000;
    this.steps++;
    const change = this.play(this.port.files.get(MODULE), this.steps);
    this.state = { ...this.state, ...change, at: this.now, client_time: change.client_time ?? this.clientTime };
    this.clientTime = this.state.client_time;
    this.store.add(this.state);
    this.last = this.state;
    await this.judge.tick();
  }

  async run(steps: number): Promise<void> {
    for (let i = 0; i < steps; i++) await this.step();
  }

  ledger() {
    return trials(this.judge.ledger.entries());
  }
}

/** XP per step: 100 under the champion, `fast` under the candidate. */
const xpRate = (fast: number) => (w: World) => (source: string | undefined) => ({
  xp: w.state.xp + (source === CANDIDATE ? fast : 100),
});

describe("trials", () => {
  test("windows alternate champion and candidate, ABBA", () => {
    expect([0, 1, 2, 3, 4, 5].map((w) => armOf(w))).toEqual([
      { pair: 0, arm: "champion" },
      { pair: 0, arm: "candidate" },
      { pair: 1, arm: "candidate" },
      { pair: 1, arm: "champion" },
      { pair: 2, arm: "champion" },
      { pair: 2, arm: "candidate" },
    ]);
  });

  test("a candidate that levels faster and costs nothing is promoted and left loaded", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = xpRate(200)(w);
    const proposed = w.judge.propose(MODULE, CANDIDATE, "pull one at a time: same safety, more xp");
    expect(proposed).toEqual({ trial: 1, version: 1, ahead: 0 });
    // Proposing loads nothing.
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
    await w.run(4 * 5);
    const [trial] = w.ledger();
    expect(trial).toMatchObject({ trial: 1, module: MODULE, status: "promoted", claim: "pull one at a time: same safety, more xp", champion: 2, candidate: 1, valid_pairs: 2 });
    expect((trial!.windows as { arm: string }[]).map((x) => x.arm)).toEqual(["champion", "candidate", "candidate", "champion"]);
    const xp = (trial!.objectives as { objective: string; outcome: string; delta: number; relative: number }[]).find((o) => o.objective === "xp_per_hour")!;
    expect(xp.outcome).toBe("improved");
    expect(xp.relative).toBeCloseTo(1, 1);
    expect(w.port.files.get(MODULE)).toBe(CANDIDATE);
    expect(w.judge.ledger.champion(MODULE)).toBe(1);
    // Each swap stopped the loaded module and reloaded the new one.
    expect(w.port.lua_calls.filter((c) => c.includes("AgentReload")).length).toBe(5);
    expect(w.port.lua_calls.filter((c) => c.includes("m.stop")).length).toBe(5);
  });

  test("a gain bought with a loss beyond tolerance is rolled back", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = (source) => ({ xp: w.state.xp + (source === CANDIDATE ? 200 : 100), idle: source === CANDIDATE });
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(20);
    const [trial] = w.ledger();
    expect(trial!.status).toBe("rolled_back");
    expect(trial!.reasons).toEqual(expect.arrayContaining([expect.stringMatching(/^improved: xp_per_hour/), expect.stringMatching(/^regressed: idle_share/)]));
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
    expect(w.judge.ledger.champion(MODULE)).toBe(2);
  });

  test("a gain inside the tolerance is no gain", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = xpRate(105)(w);
    w.judge.propose(MODULE, CANDIDATE, "slightly better");
    await w.run(20);
    const [trial] = w.ledger();
    expect(trial!.status).toBe("rolled_back");
    expect(trial!.reasons).toContain("no objective improved beyond its tolerance");
  });

  test("a restart voids its window, a spare pair replaces it, and the void is in the ledger", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    const xp = xpRate(200)(w);
    // The client restarts during the second window (the candidate's, pair 0).
    w.play = (source, step) => ({ ...xp(source), ...(step === 7 ? { client_time: 1 } : {}) });
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(30);
    const [trial] = w.ledger();
    const windows = trial!.windows as { pair: number; arm: string; void?: string }[];
    expect(windows.map((x) => [x.pair, x.arm, x.void ?? null])).toEqual([
      [0, "champion", null],
      [0, "candidate", "the client restarted"],
      [1, "candidate", null],
      [1, "champion", null],
      [2, "champion", null],
      [2, "candidate", null],
    ]);
    expect(trial).toMatchObject({ status: "promoted", valid_pairs: 2 });
  });

  test("a death is scored in its window: deaths/h regress and the candidate is not kept", async () => {
    const w = new World({ ...FAST, deaths_min_count: 1, extra_pairs: 0 });
    w.port.files.set(MODULE, CHAMPION);
    const xp = xpRate(200)(w);
    // The candidate dies once in each of its windows (steps 7-10 and 12-15).
    w.play = (source, step) => ({ ...xp(source), deaths: step === 8 || step === 13 ? 1 : 0 });
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(20);
    const [trial] = w.ledger();
    const windows = trial!.windows as { arm: string; void?: string }[];
    expect(windows.every((x) => x.void === undefined)).toBe(true);
    expect(trial!.valid_pairs).toBe(2);
    const deaths = (trial!.objectives as { objective: string; outcome: string }[]).find((o) => o.objective === "deaths_per_hour")!;
    expect(deaths.outcome).toBe("regressed");
    expect(trial!.status).not.toBe("promoted");
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
  });

  test("void_on_death: a death voids its window for the rates but still counts against its arm", async () => {
    const w = new World({ ...FAST, void_on_death: true, deaths_min_count: 1, extra_pairs: 0 });
    w.port.files.set(MODULE, CHAMPION);
    const xp = xpRate(200)(w);
    // The candidate dies once in each of its windows (steps 7-10 and 12-15).
    w.play = (source, step) => ({ ...xp(source), deaths: step === 8 || step === 13 ? 1 : 0 });
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(20);
    const [trial] = w.ledger();
    const windows = trial!.windows as { arm: string; void?: string; died?: boolean }[];
    expect(windows.filter((x) => x.arm === "candidate").map((x) => [x.void, x.died])).toEqual([
      ["the character died", true],
      ["the character died", true],
    ]);
    expect(trial!.status).toBe("inconclusive");
    const deaths = (trial!.objectives as { objective: string; outcome: string; deltas: number[] }[]).find((o) => o.objective === "deaths_per_hour")!;
    expect(deaths.outcome).toBe("regressed");
    expect(deaths.deltas).toEqual([0, 2]);
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
  });

  test("a module saved by hand mid-trial stops the trial and is left alone", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = (source, step) => {
      if (step === 8) w.port.files.set(MODULE, "-- the planner's own edit");
      return xpRate(200)(w)(source);
    };
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(12);
    const [trial] = w.ledger();
    expect(trial!.status).toBe("aborted");
    expect((trial!.reasons as string[])[0]).toContain("was changed on the client during the candidate window");
    expect(w.port.files.get(MODULE)).toBe("-- the planner's own edit");
  });

  test("a candidate that fails to load loses at once", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.port.failLoad.add(CANDIDATE);
    w.judge.propose(MODULE, CANDIDATE, "broken");
    await w.run(10);
    const [trial] = w.ledger();
    expect(trial!.status).toBe("rolled_back");
    expect((trial!.reasons as string[])[0]).toMatch(/^the candidate failed to load: .*syntax error/);
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
  });

  test("a new module's champion is its absence: removed in champion windows", async () => {
    const w = new World();
    w.play = xpRate(200)(w);
    w.judge.propose(MODULE, CANDIDATE, "a new helper");
    await w.step();
    expect(w.port.files.has(MODULE)).toBe(false);
    expect(w.port.lua_calls.at(-1)).toContain("__wp_loaded[\"grind.loop\"] = nil");
    await w.run(19);
    expect(w.ledger()[0]).toMatchObject({ status: "promoted", champion: 0 });
    expect(w.port.files.get(MODULE)).toBe(CANDIDATE);
  });

  test("nothing starts while the character is out of the world", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.judge.propose(MODULE, CANDIDATE, "x");
    await w.judge.tick();
    expect(w.ledger()[0]!.status).toBe("queued");
    expect(w.port.lua_calls).toEqual([]);
  });

  test("after a judge restart a running trial is abandoned with its champion back, and queued ones resume", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = xpRate(200)(w);
    w.judge.propose(MODULE, CANDIDATE, "first");
    w.judge.propose("other", "return 1", "second");
    await w.run(6);
    expect(w.port.files.get(MODULE)).toBe(CANDIDATE);
    const again = new Judge({ dir: w.dir, port: w.port, store: w.store, latest: () => w.last, maxGapMs: 90_000, now: () => w.now });
    await again.recover();
    expect(w.port.files.get(MODULE)).toBe(CHAMPION);
    const ledger = trials(again.ledger.entries());
    expect(ledger.map((t) => [t.trial, t.status])).toEqual([
      [1, "abandoned"],
      [2, "queued"],
    ]);
    expect(again.status().queued).toEqual([2]);
  });

  test("the ledger is append-only: every entry stays, in order", async () => {
    const w = new World();
    w.port.files.set(MODULE, CHAMPION);
    w.play = xpRate(200)(w);
    w.judge.propose(MODULE, CANDIDATE, "more xp");
    await w.run(20);
    expect(w.judge.ledger.entries().map((e) => e.kind)).toEqual(["proposed", "baseline", "started", "window", "window", "window", "window", "verdict"]);
    expect(w.judge.ledger.entries()[2]).toMatchObject({ character: characterKey("Agentsix", "Beta") });
  });
});

const rates = (over: Partial<Rates>): Rates => ({
  xp_per_hour: 1000,
  gold_per_hour: 0,
  quests_per_hour: 0,
  exploration_per_hour: 0,
  skill_per_hour: 0,
  deaths_per_hour: 0,
  idle_share: 0.1,
  needs_gap_per_hour: 0,
  ...over,
});

function windows(pairs: [Partial<Rates>, Partial<Rates>][]): WindowRecord[] {
  const counts = {} as WindowRecord["counts"];
  return pairs.flatMap(([a, b], pair) => [
    { pair, arm: "champion" as const, start: 0, end: 1, hours: 0.2, deaths: 0, rates: rates(a), counts },
    { pair, arm: "candidate" as const, start: 1, end: 2, hours: 0.2, deaths: 0, rates: rates(b), counts },
  ]);
}

describe("the Pareto rule", () => {
  const config = DEFAULT_CONFIG;

  test("a steady gain is promoted, with the effect size", () => {
    const v = decide(windows([[{}, { xp_per_hour: 1300 }], [{}, { xp_per_hour: 1250 }], [{}, { xp_per_hour: 1350 }]]), config);
    expect(v.verdict).toBe("promoted");
    const xp = v.objectives.find((o) => o.objective === "xp_per_hour")!;
    expect(xp).toMatchObject({ outcome: "improved", delta: 300, relative: 0.3, tolerance: 100, n: 3, deltas: [300, 250, 350] });
    expect(xp.t).toBeCloseTo(300 / (50 / Math.sqrt(3)), 2);
  });

  test("a mean gain buried in noise is neutral", () => {
    const v = decide(windows([[{}, { xp_per_hour: 2500 }], [{}, { xp_per_hour: 400 }], [{}, { xp_per_hour: 900 }]]), config);
    expect(v.objectives.find((o) => o.objective === "xp_per_hour")!.outcome).toBe("neutral");
    expect(v.verdict).toBe("rolled_back");
  });

  test("lower is better where it should be", () => {
    const v = decide(windows([[{}, { idle_share: 0.01 }], [{}, { idle_share: 0.0 }]]), config);
    expect(v.objectives.find((o) => o.objective === "idle_share")!.outcome).toBe("improved");
    expect(v.verdict).toBe("promoted");
  });

  test("too few valid pairs is inconclusive", () => {
    const w = windows([[{}, { xp_per_hour: 2000 }], [{}, { xp_per_hour: 2000 }]]);
    w[3]!.void = "the client restarted";
    const v = decide(w, config);
    expect(v.verdict).toBe("inconclusive");
    expect(v.reasons[0]).toBe("only 1 valid pair(s) of 2 needed");
  });

  test("judge.json overrides the defaults objective by objective", () => {
    const dir = scratch();
    writeFileSync(join(dir, "judge.json"), JSON.stringify({ window_minutes: 15, objectives: { xp_per_hour: { rel: 0.2 } } }));
    const c = loadConfig(dir);
    expect(c.window_minutes).toBe(15);
    expect(c.pairs).toBe(DEFAULT_CONFIG.pairs);
    expect(c.objectives.xp_per_hour).toEqual({ better: "higher", abs: 0, rel: 0.2 });
    expect(c.objectives.gold_per_hour).toEqual(DEFAULT_CONFIG.objectives.gold_per_hour);
  });
});

describe("isolation", () => {
  test("the judge's folder defaults beside the modules and may not be inside them", () => {
    expect(judgeDir("/home/wp/Agent/scripts")).toBe("/home/wp/Agent/judge");
    expect(judgeDir("/home/wp/Agent/scripts", "/srv/judge")).toBe("/srv/judge");
    expect(() => judgeDir("/home/wp/Agent/scripts", "/home/wp/Agent/scripts/judge")).toThrow(/inside the agent's modules folder/);
    expect(() => judgeDir("/home/wp/Agent/scripts", "/home/wp/Agent/scripts")).toThrow();
  });
});
