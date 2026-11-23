// The scorecard: a character scored on many goals at once, sampled from the client through the
// control port and kept as a time series on disk, one JSONL file per character. Measurements are
// fixed here; what the character does about them is the agent's. Every number is read through the
// stock API (never the agent's own Lua), and the functions it reads are watched for replacement.

import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { gaps, type NeedGap, type NeedsTable, NEEDS } from "./needs.ts";

/** What the sampler needs of the control port (AgentClient has it). */
export interface SamplePort {
  status(): Promise<unknown>;
  lua(code: string, budget?: number): Promise<unknown>;
  events(query: { since?: number; names?: string[]; limit?: number }): Promise<unknown>;
}

/** The stock functions a sample reads: replaced by Lua between samples, the sample is tampered. */
export const WATCHED = [
  "UnitName", "GetRealmName", "UnitLevel", "UnitXP", "UnitXPMax", "GetMoney", "UnitIsDeadOrGhost",
  "UnitAffectingCombat", "UnitHealth", "UnitHealthMax", "UnitPower", "UnitPowerMax", "UnitPowerType",
  "GetUnitSpeed", "UnitCastingInfo", "UnitChannelInfo", "GetRealZoneText", "GetSubZoneText",
  "GetInventoryItemLink", "GetItemInfo", "GetNumSkillLines", "GetSkillLineInfo", "GetNumSpellTabs",
  "GetSpellTabInfo", "GetNumQuestLogEntries", "GetQuestLogTitle",
] as const;

/** Professions on the skill lines (3.3.5 English names). */
const PROFESSIONS = [
  "Alchemy", "Blacksmithing", "Enchanting", "Engineering", "Herbalism", "Inscription", "Jewelcrafting",
  "Leatherworking", "Mining", "Skinning", "Tailoring", "Cooking", "First Aid", "Fishing",
];

/** One read of the character, as Lua tables. Equipped slots: 1-18 without the shirt (4). */
export const SAMPLE_LUA = `
local out = { fns = {} }
for _, n in ipairs({ ${WATCHED.map((n) => JSON.stringify(n)).join(", ")} }) do out.fns[n] = tostring(rawget(_G, n)) end
out.name, out.realm = UnitName("player"), GetRealmName and GetRealmName() or ""
out.level, out.xp, out.xp_max = UnitLevel("player") or 0, UnitXP("player") or 0, UnitXPMax("player") or 0
out.money = GetMoney() or 0
out.dead = UnitIsDeadOrGhost("player") and true or false
out.combat = UnitAffectingCombat("player") and true or false
out.zone, out.subzone = GetRealZoneText() or "", GetSubZoneText() or ""
local mana = (UnitPowerType("player") or 0) == 0
local full = (UnitHealth("player") or 0) >= (UnitHealthMax("player") or 0)
  and (not mana or (UnitPower("player") or 0) >= (UnitPowerMax("player") or 0))
local casting = UnitCastingInfo("player") or UnitChannelInfo("player")
out.idle = not out.dead and not out.combat and full and not casting and (GetUnitSpeed("player") or 0) == 0
local sum, n, empty, uncached = 0, 0, 0, 0
for slot = 1, 18 do
  if slot ~= 4 then
    local link = GetInventoryItemLink("player", slot)
    if link then
      local ilvl = select(4, GetItemInfo(link))
      if ilvl then sum, n = sum + ilvl, n + 1 else uncached = uncached + 1 end
    else
      empty = empty + 1
    end
  end
end
out.gear = { average = n > 0 and sum / n or 0, items = n, empty = empty, uncached = uncached }
local known = {}
for _, p in ipairs({ ${PROFESSIONS.map((n) => JSON.stringify(n)).join(", ")} }) do known[p] = true end
out.professions = {}
for i = 1, (GetNumSkillLines() or 0) do
  local name, header, _, rank, _, _, max = GetSkillLineInfo(i)
  if name and not header and known[name] then out.professions[name] = { rank = rank or 0, max = max or 0 } end
end
out.spells = 0
for i = 1, (GetNumSpellTabs() or 0) do out.spells = out.spells + (select(4, GetSpellTabInfo(i)) or 0) end
out.quests = {}
for i = 1, (GetNumQuestLogEntries() or 0) do
  local title, _, _, _, header, _, complete, _, id = GetQuestLogTitle(i)
  if title and not header and id then out.quests[tostring(id)] = complete == 1 end
end
return out`;

export interface Sample {
  /** Wall clock, ms since the epoch. */
  at: number;
  /** The client's own clock (status.time, seconds since its port opened): a drop is a restart. */
  client_time: number;
  name: string;
  realm: string;
  level: number;
  xp: number;
  xp_max: number;
  money: number;
  dead: boolean;
  combat: boolean;
  idle: boolean;
  zone: string;
  subzone: string;
  gear: { average: number; items: number; empty: number; uncached: number };
  professions: Record<string, { rank: number; max: number }>;
  spells: number;
  /** Quest id → complete, for the quests in the log. */
  quests: Record<string, boolean>;
  /** PLAYER_DEAD events since the previous sample. */
  deaths: number;
  /** Watched stock functions replaced since the previous sample of this UI session. */
  tampered: string[];
  /** The UI's Lua was rebuilt since the previous sample (a relog or ReloadUI). */
  ui_reloaded: boolean;
}

/** What a stretch of samples scores: counts, rates per hour, and why it may not count. */
export interface WindowScore {
  from: number;
  to: number;
  /** In-world time covered by consecutive samples, hours. */
  hours: number;
  samples: number;
  xp: number;
  money: number;
  quests_done: number;
  quests_accepted: number;
  deaths: number;
  new_zones: number;
  new_subzones: number;
  spells_learned: number;
  skill_ups: number;
  levels: number;
  idle_share: number | null;
  /** Normalised needs gap at the start and end (`needs.ts`). */
  needs_start: number | null;
  needs_end: number | null;
  rates: Rates;
  /** Things that make the stretch untrustworthy: restarts, gaps, tampering, UI reloads. */
  breaks: string[];
}

/** The objectives a trial compares, per hour where they are rates. */
export interface Rates {
  xp_per_hour: number | null;
  gold_per_hour: number | null;
  quests_per_hour: number | null;
  exploration_per_hour: number | null;
  skill_per_hour: number | null;
  deaths_per_hour: number | null;
  idle_share: number | null;
  needs_gap_per_hour: number | null;
}

const HOUR = 3_600_000;

/** Experience gained from `a` to `b`, across level-ups (each level's max is known at its start). */
export function xpGained(a: Sample, b: Sample): number {
  if (b.level === a.level) return Math.max(0, b.xp - a.xp);
  if (b.level < a.level) return 0;
  // The levels between are not seen: count the rest of a's level and b's progress.
  return Math.max(0, a.xp_max - a.xp) + b.xp;
}

/** A filename-safe key for a character. */
export function characterKey(name: string, realm: string): string {
  const safe = (s: string) => s.replace(/[^A-Za-z0-9_-]/g, "_") || "_";
  return `${safe(realm)}-${safe(name)}`;
}

/** The samples, per character, appended to `<dir>/<realm>-<name>/samples.jsonl`. */
export class SampleStore {
  #cache = new Map<string, Sample[]>();
  constructor(readonly dir: string) {}

  #file(key: string): string {
    return join(this.dir, key, "samples.jsonl");
  }

  add(sample: Sample): void {
    const key = characterKey(sample.name, sample.realm);
    const all = this.all(key);
    mkdirSync(join(this.dir, key), { recursive: true });
    appendFileSync(this.#file(key), `${JSON.stringify(sample)}\n`);
    all.push(sample);
  }

  all(key: string): Sample[] {
    let cached = this.#cache.get(key);
    if (!cached) {
      const file = this.#file(key);
      cached = existsSync(file)
        ? readFileSync(file, "utf8").split("\n").filter(Boolean).flatMap((line) => {
            try {
              return [JSON.parse(line) as Sample];
            } catch {
              return [];
            }
          })
        : [];
      this.#cache.set(key, cached);
    }
    return cached;
  }

  between(key: string, from: number, to: number): Sample[] {
    return this.all(key).filter((s) => s.at >= from && s.at <= to);
  }
}

export interface ScoreOptions {
  /** A gap between samples longer than this is not counted (ms): the client was away. */
  maxGapMs: number;
  needs?: NeedsTable;
}

/** Score samples `[from, to]`, with `before` (everything earlier) for what counts as new. */
export function score(samples: Sample[], before: Sample[], options: ScoreOptions): WindowScore {
  const needs = options.needs ?? NEEDS;
  const breaks: string[] = [];
  const seenZones = new Set(before.map((s) => s.zone));
  const seenSub = new Set(before.map((s) => `${s.zone}/${s.subzone}`));
  const s0 = samples[0];
  const out: WindowScore = {
    from: s0?.at ?? 0,
    to: samples.at(-1)?.at ?? 0,
    hours: 0,
    samples: samples.length,
    xp: 0,
    money: 0,
    quests_done: 0,
    quests_accepted: 0,
    deaths: 0,
    new_zones: 0,
    new_subzones: 0,
    spells_learned: 0,
    skill_ups: 0,
    levels: 0,
    idle_share: null,
    needs_start: s0 ? needGap(s0, needs) : null,
    needs_end: samples.length ? needGap(samples.at(-1)!, needs) : null,
    rates: emptyRates(),
    breaks,
  };
  let idle = 0;
  for (const [i, b] of samples.entries()) {
    if (b.idle) idle++;
    if (b.tampered.length) breaks.push(`stock functions replaced: ${b.tampered.join(", ")}`);
    // Exploring is arriving somewhere new during the stretch, not where it starts.
    if (i > 0 && !seenZones.has(b.zone) && b.zone) out.new_zones++;
    if (i > 0 && !seenSub.has(`${b.zone}/${b.subzone}`) && b.subzone) out.new_subzones++;
    seenZones.add(b.zone);
    seenSub.add(`${b.zone}/${b.subzone}`);
    // A sample's deaths happened since the one before, which may lie outside the window.
    if (i > 0) out.deaths += b.deaths;
    const a = samples[i - 1];
    if (!a) continue;
    if (b.ui_reloaded) breaks.push("the UI reloaded");
    if (b.client_time < a.client_time) {
      breaks.push("the client restarted");
      continue;
    }
    if (b.at - a.at > options.maxGapMs) {
      breaks.push(`no sample for ${Math.round((b.at - a.at) / 1000)}s`);
      continue;
    }
    if (characterKey(a.name, a.realm) !== characterKey(b.name, b.realm)) {
      breaks.push("another character");
      continue;
    }
    out.hours += (b.at - a.at) / HOUR;
    out.xp += xpGained(a, b);
    out.money += b.money - a.money;
    out.levels += Math.max(0, b.level - a.level);
    out.spells_learned += Math.max(0, b.spells - a.spells);
    for (const [name, p] of Object.entries(b.professions)) {
      out.skill_ups += Math.max(0, p.rank - (a.professions[name]?.rank ?? p.rank));
    }
    for (const [id, complete] of Object.entries(a.quests)) {
      // Gone from the log: handed in if it was complete, else abandoned (not counted).
      if (!(id in b.quests) && complete) out.quests_done++;
    }
    for (const id of Object.keys(b.quests)) if (!(id in a.quests)) out.quests_accepted++;
  }
  out.idle_share = samples.length ? idle / samples.length : null;
  out.rates = rates(out);
  return out;
}

function emptyRates(): Rates {
  return {
    xp_per_hour: null,
    gold_per_hour: null,
    quests_per_hour: null,
    exploration_per_hour: null,
    skill_per_hour: null,
    deaths_per_hour: null,
    idle_share: null,
    needs_gap_per_hour: null,
  };
}

function rates(w: WindowScore): Rates {
  if (w.hours <= 0) return { ...emptyRates(), idle_share: w.idle_share };
  const per = (n: number) => n / w.hours;
  return {
    xp_per_hour: per(w.xp),
    gold_per_hour: per(w.money),
    quests_per_hour: per(w.quests_done),
    exploration_per_hour: per(w.new_zones + w.new_subzones),
    skill_per_hour: per(w.skill_ups),
    deaths_per_hour: per(w.deaths),
    idle_share: w.idle_share,
    needs_gap_per_hour:
      w.needs_start !== null && w.needs_end !== null ? per(w.needs_end - w.needs_start) : null,
  };
}

/** The needs as one number: the sum of each gap as a share of its target (0 = all met). */
export function needGap(s: Sample, needs: NeedsTable = NEEDS): number {
  return gaps(s, needs).reduce((sum, g) => sum + g.share, 0);
}

export interface SamplerOptions {
  store: SampleStore;
  now?: () => number;
}

/** Reads the character, when it is in the world, on each tick. */
export class Sampler {
  #cursor: number | undefined;
  #fns: Record<string, string> | undefined;
  #busy = false;
  last: Sample | undefined;
  /** Why the last attempt stored nothing, if it did not. */
  lastSkip: string | undefined;

  constructor(
    readonly port: SamplePort,
    readonly options: SamplerOptions,
  ) {}

  /** One sample: stored and returned, or undefined out of the world (lastSkip says why). */
  async tick(): Promise<Sample | undefined> {
    if (this.#busy) return undefined;
    this.#busy = true;
    try {
      const sample = await this.#read();
      if (sample) {
        this.options.store.add(sample);
        this.last = sample;
        this.lastSkip = undefined;
      }
      return sample;
    } catch (error) {
      this.lastSkip = error instanceof Error ? error.message : String(error);
      return undefined;
    } finally {
      this.#busy = false;
    }
  }

  async #deaths(): Promise<number> {
    const answer = (await this.port.events({ since: this.#cursor, names: ["PLAYER_DEAD"], limit: 1000 })) as {
      events?: unknown[];
      next?: number;
    };
    const first = this.#cursor === undefined;
    if (typeof answer.next === "number") this.#cursor = answer.next;
    return first ? 0 : (answer.events?.length ?? 0);
  }

  async #read(): Promise<Sample | undefined> {
    const status = (await this.port.status()) as { in_world?: boolean; time?: number; player?: unknown };
    if (!status.in_world || !status.player) {
      this.lastSkip = "not in the world";
      return undefined;
    }
    const deaths = await this.#deaths();
    const answer = (await this.port.lua(SAMPLE_LUA)) as { ok?: boolean; values?: unknown[]; error?: string };
    if (!answer.ok) throw new Error(`sample failed: ${answer.error ?? "no answer"}`);
    const raw = answer.values?.[0] as (Omit<Sample, "at" | "client_time" | "deaths" | "tampered" | "ui_reloaded"> & { fns: Record<string, string> }) | undefined;
    if (!raw || typeof raw !== "object" || !raw.name) {
      this.lastSkip = "the character is not readable yet";
      return undefined;
    }
    const { fns, ...rest } = raw;
    const before = this.#fns;
    this.#fns = fns;
    const changed = before ? WATCHED.filter((n) => before[n] !== fns[n]) : [];
    // Every watched function new at once is a new Lua VM (relog, ReloadUI), not tampering.
    const reloaded = changed.length === WATCHED.length;
    return {
      ...rest,
      // Lua tables with no entries arrive as empty arrays.
      professions: Array.isArray(rest.professions) ? {} : rest.professions,
      quests: Array.isArray(rest.quests) ? {} : rest.quests,
      at: (this.options.now ?? Date.now)(),
      client_time: Number(status.time ?? 0),
      deaths,
      tampered: reloaded ? [] : changed,
      ui_reloaded: reloaded,
    };
  }
}

export interface Scorecard {
  character: string;
  sampled_at: string | null;
  /** Why no sample was taken lately, if none was. */
  note?: string;
  current: Record<string, unknown> | null;
  needs: NeedGap[];
  trends: Record<string, WindowScore>;
}

/** The scorecard tool's answer: the latest values, the needs and the trends over a few spans. */
export function scorecard(
  store: SampleStore,
  key: string,
  now: number,
  options: ScoreOptions & { spansMinutes?: number[] },
): Scorecard {
  const all = store.all(key);
  const last = all.at(-1);
  const trends: Record<string, WindowScore> = {};
  for (const minutes of options.spansMinutes ?? [15, 60, 240]) {
    const from = now - minutes * 60_000;
    trends[`last_${minutes}m`] = score(all.filter((s) => s.at >= from), all.filter((s) => s.at < from), options);
  }
  trends.all = score(all, [], options);
  const whole = trends.all;
  return {
    character: key,
    sampled_at: last ? new Date(last.at).toISOString() : null,
    current: last
      ? {
          level: last.level,
          xp: last.xp,
          xp_max: last.xp_max,
          money: last.money,
          gear: last.gear,
          professions: last.professions,
          spells: last.spells,
          zone: last.zone,
          subzone: last.subzone,
          zones_visited: new Set(all.map((s) => s.zone).filter(Boolean)).size,
          subzones_visited: new Set(all.filter((s) => s.subzone).map((s) => `${s.zone}/${s.subzone}`)).size,
          quests_done: whole.quests_done,
          quests_accepted: whole.quests_accepted,
          deaths: whole.deaths,
          dead: last.dead,
        }
      : null,
    needs: last ? gaps(last, options.needs ?? NEEDS) : [],
    trends,
  };
}
