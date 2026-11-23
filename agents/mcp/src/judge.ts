// The judge: deterministic, and out of the agent's reach. A planner proposes a module change as a
// candidate with a claim; the judge runs the champion (the module as it stands) and the candidate
// in alternating time windows, scores each window from the scorecard, and keeps the candidate only
// if it improves at least one objective and regresses none beyond that objective's tolerance
// (Pareto-style). Every attempt goes in an append-only ledger. Its files live in its own folder,
// which neither Lua nor a module path reaches; no tool writes its config.
//
//   <dir>/judge.json            optional overrides of DEFAULT_CONFIG (read at each trial's start)
//   <dir>/ledger.jsonl          every proposal, window and verdict, appended
//   <dir>/modules/<name>/vNNNN.lua   the versions the judge has seen (baseline, candidates)

import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Rates, Sample, SampleStore, WindowScore } from "./scorecard.ts";
import { characterKey, score } from "./scorecard.ts";

export type ObjectiveName = keyof Rates;

export interface Objective {
  /** Which way is better. */
  better: "higher" | "lower";
  /** The tolerance: the larger of `abs` and `rel` times the champion's mean. A change smaller than
   *  it is neither a gain nor a loss. */
  abs: number;
  rel: number;
}

export interface JudgeConfig {
  /** Each window's length, and the settling time at its start that is not scored. */
  window_minutes: number;
  settle_seconds: number;
  /** Champion/candidate pairs to score; void pairs are replaced, up to `extra_pairs` more. */
  pairs: number;
  extra_pairs: number;
  /** Fewer valid pairs than this: inconclusive, and the champion stays. */
  min_valid_pairs: number;
  /** A gain or loss also needs |mean delta| / standard error at least this (with 2+ pairs). */
  min_t: number;
  /** A death in a window voids it for the rates; its deaths still count against its arm. */
  void_on_death: boolean;
  /** Deaths are rare: a gain or loss in deaths/h also needs this many more or fewer deaths. */
  deaths_min_count: number;
  objectives: Record<ObjectiveName, Objective>;
}

export const DEFAULT_CONFIG: JudgeConfig = {
  window_minutes: 12,
  settle_seconds: 30,
  pairs: 3,
  extra_pairs: 2,
  min_valid_pairs: 2,
  min_t: 1,
  void_on_death: true,
  deaths_min_count: 2,
  objectives: {
    xp_per_hour: { better: "higher", abs: 0, rel: 0.1 },
    gold_per_hour: { better: "higher", abs: 1000, rel: 0.15 },
    quests_per_hour: { better: "higher", abs: 1, rel: 0.15 },
    exploration_per_hour: { better: "higher", abs: 2, rel: 0.2 },
    skill_per_hour: { better: "higher", abs: 2, rel: 0.15 },
    deaths_per_hour: { better: "lower", abs: 1, rel: 0 },
    idle_share: { better: "lower", abs: 0.05, rel: 0 },
    needs_gap_per_hour: { better: "lower", abs: 0.05, rel: 0 },
  },
};

/** The config for a trial: DEFAULT_CONFIG with `<dir>/judge.json` laid over it. */
export function loadConfig(dir: string): JudgeConfig {
  const path = join(dir, "judge.json");
  if (!existsSync(path)) return structuredClone(DEFAULT_CONFIG);
  const over = JSON.parse(readFileSync(path, "utf8")) as Partial<JudgeConfig>;
  const objectives = { ...DEFAULT_CONFIG.objectives };
  for (const [name, o] of Object.entries(over.objectives ?? {})) {
    const base = objectives[name as ObjectiveName];
    if (base) objectives[name as ObjectiveName] = { ...base, ...o };
  }
  return { ...structuredClone(DEFAULT_CONFIG), ...over, objectives };
}

export type Arm = "champion" | "candidate";

/** One scored window, as the ledger keeps it. */
export interface WindowRecord {
  pair: number;
  arm: Arm;
  start: number;
  end: number;
  /** Why it does not count for the rates, if it does not. */
  void?: string;
  /** Voided by a death: its deaths and hours still count in the deaths objective. */
  died?: boolean;
  hours: number;
  deaths: number;
  rates: Rates;
  counts: Omit<WindowScore, "rates" | "breaks" | "from" | "to">;
}

export interface ObjectiveResult {
  objective: ObjectiveName;
  outcome: "improved" | "regressed" | "neutral" | "no data";
  champion: number | null;
  candidate: number | null;
  /** Mean of the candidate-minus-champion differences, in the objective's own units. */
  delta: number | null;
  /** delta over the champion's mean. */
  relative: number | null;
  tolerance: number;
  /** Pairs compared, the differences' standard deviation, and the effect size: t = delta /
   *  standard error, d = delta / sd (null with one pair, or when every pair differs alike). */
  n: number;
  sd: number | null;
  t: number | null;
  d: number | null;
  deltas?: number[];
}

export type Outcome = "promoted" | "rolled_back" | "inconclusive";

export interface Verdict {
  verdict: Outcome;
  valid_pairs: number;
  objectives: ObjectiveResult[];
  reasons: string[];
}

const mean = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / xs.length;
const round = (x: number | null) => (x === null || !Number.isFinite(x) ? null : Math.round(x * 1000) / 1000);

function stats(deltas: number[]): { m: number; sd: number; t: number | null; d: number | null } {
  const m = mean(deltas);
  if (deltas.length < 2) return { m, sd: 0, t: null, d: null };
  const sd = Math.sqrt(deltas.reduce((s, x) => s + (x - m) ** 2, 0) / (deltas.length - 1));
  const se = sd / Math.sqrt(deltas.length);
  const inf = m === 0 ? 0 : Math.sign(m) * Infinity;
  return { m, sd, t: se > 0 ? m / se : inf, d: sd > 0 ? m / sd : inf };
}

/** The valid champion/candidate pairs, in order. */
export function validPairs(windows: WindowRecord[]): { champion: WindowRecord; candidate: WindowRecord }[] {
  const byPair = new Map<number, Partial<Record<Arm, WindowRecord>>>();
  for (const w of windows) byPair.set(w.pair, { ...byPair.get(w.pair), [w.arm]: w });
  return [...byPair.entries()]
    .sort(([a], [b]) => a - b)
    .flatMap(([, p]) => (p.champion && p.candidate && !p.champion.void && !p.candidate.void ? [{ champion: p.champion, candidate: p.candidate }] : []));
}

/** Paired comparison of one rate objective. */
function paired(name: ObjectiveName, o: Objective, pairs: ReturnType<typeof validPairs>, config: JudgeConfig): ObjectiveResult {
  const sign = o.better === "higher" ? 1 : -1;
  const both = pairs.flatMap(({ champion, candidate }) => {
    const a = champion.rates[name];
    const b = candidate.rates[name];
    return a === null || b === null ? [] : [{ a, b }];
  });
  if (!both.length) {
    return { objective: name, outcome: "no data", champion: null, candidate: null, delta: null, relative: null, tolerance: o.abs, n: 0, sd: null, t: null, d: null };
  }
  const champ = mean(both.map((p) => p.a));
  const cand = mean(both.map((p) => p.b));
  const deltas = both.map((p) => p.b - p.a);
  const { m, sd, t, d } = stats(deltas);
  const tolerance = Math.max(o.abs, o.rel * Math.abs(champ));
  const better = sign * m;
  const sure = t === null || sign * t >= config.min_t;
  const outcome = better > tolerance && sure ? "improved" : better < -tolerance && (t === null || -sign * t >= config.min_t) ? "regressed" : "neutral";
  return {
    objective: name,
    outcome,
    champion: round(champ),
    candidate: round(cand),
    delta: round(m),
    relative: champ !== 0 ? round(m / Math.abs(champ)) : null,
    tolerance: round(tolerance)!,
    n: both.length,
    sd: deltas.length > 1 ? round(sd) : null,
    t: round(t),
    d: round(d),
    deltas: deltas.map((x) => round(x)!),
  };
}

/** Deaths per hour over each arm's every scored window, death-voided ones included: a candidate
 *  that dies more cannot hide its deaths in void windows. */
function deaths(o: Objective, windows: WindowRecord[], config: JudgeConfig): ObjectiveResult {
  const arm = (a: Arm) => {
    const counted = windows.filter((w) => w.arm === a && (!w.void || w.died));
    const hours = counted.reduce((s, w) => s + w.hours, 0);
    const count = counted.reduce((s, w) => s + w.deaths, 0);
    return { hours, count, rate: hours > 0 ? count / hours : null };
  };
  const champ = arm("champion");
  const cand = arm("candidate");
  const base = { objective: "deaths_per_hour" as const, tolerance: o.abs, n: 0, sd: null, t: null, d: null };
  if (champ.rate === null || cand.rate === null) {
    return { ...base, outcome: "no data", champion: champ.rate, candidate: cand.rate, delta: null, relative: null };
  }
  const delta = cand.rate - champ.rate;
  const fewer = champ.count - cand.count;
  const outcome =
    delta < -o.abs && fewer >= config.deaths_min_count
      ? "improved"
      : delta > o.abs && -fewer >= config.deaths_min_count
        ? "regressed"
        : "neutral";
  return {
    ...base,
    outcome,
    champion: round(champ.rate),
    candidate: round(cand.rate),
    delta: round(delta),
    relative: champ.rate > 0 ? round(delta / champ.rate) : null,
    deltas: [champ.count, cand.count],
  };
}

/** The verdict on a trial's windows: promoted only with a gain and no loss beyond tolerance. */
export function decide(windows: WindowRecord[], config: JudgeConfig): Verdict {
  const pairs = validPairs(windows);
  const objectives = (Object.entries(config.objectives) as [ObjectiveName, Objective][]).map(([name, o]) =>
    name === "deaths_per_hour" ? deaths(o, windows, config) : paired(name, o, pairs, config),
  );
  const describe = (r: ObjectiveResult) =>
    `${r.objective} ${r.champion} -> ${r.candidate} (delta ${r.delta}, tolerance ${r.tolerance}${r.t !== null ? `, t ${r.t}` : r.sd === 0 ? ", every pair alike" : ""})`;
  const improved = objectives.filter((r) => r.outcome === "improved");
  const regressed = objectives.filter((r) => r.outcome === "regressed");
  const reasons = [
    ...improved.map((r) => `improved: ${describe(r)}`),
    ...regressed.map((r) => `regressed: ${describe(r)}`),
  ];
  if (pairs.length < config.min_valid_pairs) {
    return {
      verdict: "inconclusive",
      valid_pairs: pairs.length,
      objectives,
      reasons: [`only ${pairs.length} valid pair(s) of ${config.min_valid_pairs} needed`, ...reasons],
    };
  }
  if (!improved.length) reasons.push("no objective improved beyond its tolerance");
  return { verdict: improved.length && !regressed.length ? "promoted" : "rolled_back", valid_pairs: pairs.length, objectives, reasons };
}

// ---------------------------------------------------------------------------------------------
// Ledger and versions

export type LedgerEntry =
  | { kind: "baseline"; at: string; module: string; version: number }
  | { kind: "proposed"; at: string; trial: number; module: string; version: number; claim: string }
  | { kind: "started"; at: string; trial: number; module: string; champion: number; candidate: number; character: string; config: JudgeConfig }
  | ({ kind: "window"; at: string; trial: number } & WindowRecord)
  | ({ kind: "verdict"; at: string; trial: number; module: string; champion: number; candidate: number } & Verdict)
  | { kind: "ended"; at: string; trial: number; module: string; outcome: "aborted" | "abandoned"; reason: string };

type WithoutAt<T> = T extends unknown ? Omit<T, "at"> : never;
export type NewEntry = WithoutAt<LedgerEntry>;

const MODULE_NAME = /^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)*$/;

export class Ledger {
  readonly path: string;
  constructor(dir: string) {
    mkdirSync(dir, { recursive: true });
    this.path = join(dir, "ledger.jsonl");
  }

  append(entry: NewEntry): LedgerEntry {
    const full = { at: new Date().toISOString(), ...entry } as LedgerEntry;
    appendFileSync(this.path, `${JSON.stringify(full)}\n`);
    return full;
  }

  entries(): LedgerEntry[] {
    if (!existsSync(this.path)) return [];
    return readFileSync(this.path, "utf8")
      .split("\n")
      .filter(Boolean)
      .flatMap((line) => {
        try {
          return [JSON.parse(line) as LedgerEntry];
        } catch {
          return [];
        }
      });
  }

  /** The champion version of a module (0: no module), or undefined when none is recorded. */
  champion(module: string): number | undefined {
    let out: number | undefined;
    for (const e of this.entries()) {
      if (e.kind === "baseline" && e.module === module) out = e.version;
      if (e.kind === "verdict" && e.module === module && e.verdict === "promoted") out = e.candidate;
    }
    return out;
  }
}

/** The module versions the judge has seen, kept in its own folder (0 is "no module"). */
export class Versions {
  readonly dir: string;
  constructor(dir: string) {
    this.dir = join(dir, "modules");
  }

  #path(name: string, version: number): string {
    if (!MODULE_NAME.test(name)) throw new Error(`not a module name: ${name}`);
    return join(this.dir, name, `v${String(version).padStart(4, "0")}.lua`);
  }

  latest(name: string): number {
    const dir = join(this.dir, name);
    if (!MODULE_NAME.test(name) || !existsSync(dir)) return 0;
    return Math.max(0, ...readdirSync(dir).map((f) => Number(/^v(\d+)\.lua$/.exec(f)?.[1] ?? 0)));
  }

  add(name: string, source: string): number {
    const version = this.latest(name) + 1;
    const path = this.#path(name, version);
    mkdirSync(join(this.dir, name), { recursive: true });
    writeFileSync(path, source);
    return version;
  }

  /** A version's source; undefined for 0 (no module). */
  source(name: string, version: number): string | undefined {
    if (version === 0) return undefined;
    return readFileSync(this.#path(name, version), "utf8");
  }
}

// ---------------------------------------------------------------------------------------------
// Trials

/** What the judge needs of the control port (AgentClient has it). */
export interface JudgePort {
  lua(code: string, budget?: number): Promise<unknown>;
  /** The module's source, or a rejection (404) when there is none. */
  readModule(name: string): Promise<string>;
  saveModule(name: string, source: string): Promise<unknown>;
  deleteModule(name: string): Promise<unknown>;
}

interface OpenWindow {
  pair: number;
  arm: Arm;
  start: number;
  measureFrom: number;
  end: number;
}

interface Trial {
  id: number;
  module: string;
  claim: string;
  candidate: number;
  champion: number;
  character: string;
  config: JudgeConfig;
  windows: WindowRecord[];
  next: number;
  open?: OpenWindow;
}

export interface JudgeOptions {
  dir: string;
  port: JudgePort;
  store: SampleStore;
  /** The latest sample, to know the character is in the world now. */
  latest: () => Sample | undefined;
  maxGapMs: number;
  now?: () => number;
}

/** stop() on the loaded module, if it has one, before it is swapped out. */
const stopLua = (name: string) =>
  `local m = __wp_loaded and __wp_loaded[${JSON.stringify(name)}] if type(m) == "table" and type(m.stop) == "function" then pcall(m.stop) end return true`;
const reloadLua = (name: string) => `AgentReload(${JSON.stringify(name)}) return true`;
const forgetLua = (name: string) => `if __wp_loaded then __wp_loaded[${JSON.stringify(name)}] = nil end return true`;

/** Which arm runs in window `w`: pairs alternate their order (ABBA), so drift cancels. */
export function armOf(w: number): { pair: number; arm: Arm } {
  const pair = Math.floor(w / 2);
  const first: Arm = pair % 2 === 0 ? "champion" : "candidate";
  const arm = w % 2 === 0 ? first : first === "champion" ? "candidate" : "champion";
  return { pair, arm };
}

export class Judge {
  readonly ledger: Ledger;
  readonly versions: Versions;
  #queue: number[] = [];
  #proposals = new Map<number, { module: string; version: number; claim: string }>();
  #trial: Trial | undefined;
  #busy = false;

  constructor(readonly options: JudgeOptions) {
    this.ledger = new Ledger(options.dir);
    this.versions = new Versions(options.dir);
  }

  #now(): number {
    return this.options.now?.() ?? Date.now();
  }

  /** A candidate: kept as a new version and queued for a trial, not loaded yet. */
  propose(module: string, source: string, claim: string): { trial: number; version: number; ahead: number } {
    if (!MODULE_NAME.test(module)) throw new Error(`module names are dotted words: ${module}`);
    const version = this.versions.add(module, source);
    const trial = 1 + this.ledger.entries().filter((e) => e.kind === "proposed").length;
    this.ledger.append({ kind: "proposed", trial, module, version, claim });
    this.#proposals.set(trial, { module, version, claim });
    this.#queue.push(trial);
    return { trial, version, ahead: this.#queue.length - 1 + (this.#trial ? 1 : 0) };
  }

  /** After a restart of this server: a trial that was running is ended (its champion put back),
   *  and proposals never started are queued again. */
  async recover(): Promise<void> {
    const entries = this.ledger.entries();
    const done = new Set(entries.flatMap((e) => (e.kind === "verdict" || e.kind === "ended" ? [e.trial] : [])));
    const started = new Map(entries.flatMap((e) => (e.kind === "started" ? [[e.trial, e] as const] : [])));
    for (const e of entries) {
      if (e.kind !== "proposed" || done.has(e.trial)) continue;
      const s = started.get(e.trial);
      if (!s) {
        this.#proposals.set(e.trial, { module: e.module, version: e.version, claim: e.claim });
        this.#queue.push(e.trial);
        continue;
      }
      let reason = "the judge restarted mid-trial; the champion is back";
      try {
        await this.#load(e.module, s.champion);
      } catch (error) {
        reason += ` (putting it back failed: ${error instanceof Error ? error.message : String(error)})`;
      }
      this.ledger.append({ kind: "ended", trial: e.trial, module: e.module, outcome: "abandoned", reason });
    }
  }

  /** Put a version on the client and (re)load it; version 0 removes the module. */
  async #load(module: string, version: number): Promise<void> {
    const ok = (answer: unknown) => {
      const a = answer as { ok?: boolean; error?: string };
      if (!a?.ok) throw new Error(`loading ${module} v${version}: ${a?.error ?? "no answer"}`);
    };
    ok(await this.options.port.lua(stopLua(module)));
    const source = this.versions.source(module, version);
    if (source === undefined) {
      await this.options.port.deleteModule(module).catch(() => undefined);
      ok(await this.options.port.lua(forgetLua(module)));
      return;
    }
    await this.options.port.saveModule(module, source);
    ok(await this.options.port.lua(reloadLua(module)));
  }

  /** The client's copy of the module: undefined when there is none. */
  async #onClient(module: string): Promise<string | undefined> {
    try {
      return await this.options.port.readModule(module);
    } catch (error) {
      if ((error as { status?: number }).status === 404) return undefined;
      throw error;
    }
  }

  async #start(id: number, sample: Sample): Promise<void> {
    const p = this.#proposals.get(id)!;
    this.#proposals.delete(id);
    let champion = this.ledger.champion(p.module);
    if (champion === undefined) {
      // First trial of this module: what the client holds now is the champion.
      const current = await this.#onClient(p.module);
      champion = current === undefined ? 0 : this.versions.add(p.module, current);
      this.ledger.append({ kind: "baseline", module: p.module, version: champion });
    }
    const config = loadConfig(this.options.dir);
    const character = characterKey(sample.name, sample.realm);
    this.ledger.append({ kind: "started", trial: id, module: p.module, champion, candidate: p.version, character, config });
    this.#trial = { id, module: p.module, claim: p.claim, candidate: p.version, champion, character, config, windows: [], next: 0 };
  }

  /** Advance: start the next trial, open or close a window, or give the verdict. Call it after
   *  every sample (it does nothing while the character is out of the world). */
  async tick(): Promise<void> {
    if (this.#busy) return;
    this.#busy = true;
    try {
      await this.#step();
    } finally {
      this.#busy = false;
    }
  }

  async #step(): Promise<void> {
    const now = this.#now();
    const last = this.options.latest();
    const inWorld = !!last && now - last.at <= this.options.maxGapMs;
    if (!this.#trial) {
      const id = this.#queue[0];
      if (id === undefined || !inWorld) return;
      this.#queue.shift();
      await this.#start(id, last!);
    }
    const t = this.#trial!;
    if (!t.open) {
      if (!inWorld) return;
      const { pair, arm } = armOf(t.next);
      const version = arm === "champion" ? t.champion : t.candidate;
      try {
        await this.#load(t.module, version);
      } catch (error) {
        const why = error instanceof Error ? error.message : String(error);
        if (arm === "candidate") return this.#finish(t, { verdict: "rolled_back", valid_pairs: 0, objectives: [], reasons: [`the candidate failed to load: ${why}`] });
        return this.#end(t, "aborted", `the champion failed to load: ${why}`);
      }
      const minutes = t.config.window_minutes * 60_000;
      t.open = { pair, arm, start: now, measureFrom: now + t.config.settle_seconds * 1000, end: now + minutes };
      return;
    }
    if (now < t.open.end) return;
    // The module must still be what this window loaded: a planner's save mid-trial ends it.
    const expected = this.versions.source(t.module, t.open.arm === "champion" ? t.champion : t.candidate);
    const actual = await this.#onClient(t.module);
    if (actual !== expected) {
      return this.#end(t, "aborted", `${t.module} was changed on the client during the ${t.open.arm} window; the trial stops and the module is left as it is`);
    }
    const record = this.#score(t, t.open);
    t.windows.push(record);
    this.ledger.append({ kind: "window", trial: t.id, ...record });
    t.open = undefined;
    t.next++;
    const valid = validPairs(t.windows).length;
    const pairsRun = Math.floor(t.next / 2);
    if (t.next % 2 === 0 && (valid >= t.config.pairs || pairsRun >= t.config.pairs + t.config.extra_pairs)) {
      await this.#finish(t, decide(t.windows, t.config));
    }
  }

  #score(t: Trial, w: OpenWindow): WindowRecord {
    const all = this.options.store.all(t.character);
    const samples = all.filter((s) => s.at >= w.measureFrom && s.at <= w.end);
    const s = score(samples, all.filter((x) => x.at < w.measureFrom), { maxGapMs: this.options.maxGapMs });
    const { rates, breaks, from: _from, to: _to, ...counts } = s;
    const died = s.deaths > 0 || samples.some((x) => x.dead);
    let reason: string | undefined;
    if (samples.length < 2) reason = `${samples.length} sample(s) in the window`;
    else if (breaks.length) reason = [...new Set(breaks)].join("; ");
    else if (died && t.config.void_on_death) reason = "the character died";
    return {
      pair: w.pair,
      arm: w.arm,
      start: w.start,
      end: w.end,
      ...(reason ? { void: reason } : {}),
      ...(reason && died && !breaks.length && samples.length >= 2 ? { died: true } : {}),
      hours: s.hours,
      deaths: s.deaths,
      rates,
      counts,
    };
  }

  async #finish(t: Trial, verdict: Verdict): Promise<void> {
    const keep = verdict.verdict === "promoted" ? t.candidate : t.champion;
    try {
      await this.#load(t.module, keep);
    } catch (error) {
      verdict.reasons.push(`loading the kept version failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    this.ledger.append({ kind: "verdict", trial: t.id, module: t.module, champion: t.champion, candidate: t.candidate, ...verdict });
    this.#trial = undefined;
  }

  #end(t: Trial, outcome: "aborted" | "abandoned", reason: string): void {
    this.ledger.append({ kind: "ended", trial: t.id, module: t.module, outcome, reason });
    this.#trial = undefined;
  }

  /** The running trial and the queue, for the ledger tool. */
  status(): { running: Record<string, unknown> | null; queued: number[] } {
    const t = this.#trial;
    return {
      running: t
        ? {
            trial: t.id,
            module: t.module,
            claim: t.claim,
            window: t.open ? { ...t.open, start: iso(t.open.start), measureFrom: iso(t.open.measureFrom), end: iso(t.open.end) } : null,
            windows_scored: t.windows.length,
            valid_pairs: validPairs(t.windows).length,
          }
        : null,
      queued: [...this.#queue],
    };
  }
}

const iso = (ms: number) => new Date(ms).toISOString();

/** The ledger as trials, newest last: claim, status, windows and verdict. */
export function trials(entries: LedgerEntry[], module?: string): Record<string, unknown>[] {
  const out = new Map<number, Record<string, unknown>>();
  for (const e of entries) {
    if (e.kind === "baseline") continue;
    const t = out.get(e.trial) ?? { trial: e.trial, status: "queued", windows: [] as unknown[] };
    out.set(e.trial, t);
    switch (e.kind) {
      case "proposed":
        Object.assign(t, { module: e.module, candidate: e.version, claim: e.claim, proposed: e.at });
        break;
      case "started":
        Object.assign(t, { status: "running", champion: e.champion, character: e.character, started: e.at });
        break;
      case "window": {
        const { kind: _k, trial: _t, at: _a, counts: _c, ...w } = e;
        (t.windows as unknown[]).push({ ...w, start: iso(w.start), end: iso(w.end) });
        break;
      }
      case "verdict":
        Object.assign(t, { status: e.verdict, valid_pairs: e.valid_pairs, objectives: e.objectives, reasons: e.reasons, decided: e.at });
        break;
      case "ended":
        Object.assign(t, { status: e.outcome, reasons: [e.reason], decided: e.at });
        break;
    }
  }
  return [...out.values()].filter((t) => !module || t.module === module);
}
