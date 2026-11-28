// Self-imposed rules: the constraints an agent sets on itself (a road it will not take, a zone it
// skips, a mob family it refuses, a quest it abandons), each with its reason and when it was set,
// and what playing under it has cost since. This is perception for the planner, nothing more: no
// rule is enforced, judged, expired or removed here. Keeping, revising or dropping one is the
// agent's call; the harness only puts the numbers next to it, and asks for a decision when an
// objective has been flat for a while.
//
// A rule is written where the agent already keeps such things:
// - in a Lua module's header comment, the module that enforces it:
//     -- rule: skip Redridge Mountains | why: 5 deaths in 2 h to Black Dragon Whelps | set: 2026-11-27 16:25
// - or a line of its notes, in the same form: `rule: ... | why: ... | set: ...`.
// `set:` is when the rule was made, not when the line was written: a rule recorded a day after it
// was made gets the day it was made, so its cost covers all the time it has stood.
// Optional fields: `recorded: <date>` (when the line was written, if later), `applies: <what it
// covers>`, `kept: <date> because <reason>` (the last time the agent looked at its cost and chose
// to keep it).
//
// A rule's cost counts from `set` when known, else from when it was recorded (agents/example's
// rule_set without `since`): the cost line then says "since recorded", so a rule made before
// its record does not look cheaper than it was.
//
// Both example harnesses use this: agents/example (its rule tools, and module headers) and
// agents/omp-example (module headers and notes/rules.md).

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { score, type Sample } from "./scorecard.ts";

export interface Rule {
  /** A harness's own id for the rule, when it keeps them in a store (agents/example's rules.json). */
  id?: string;
  rule: string;
  why?: string;
  /** When it was made, as written. */
  set?: string;
  /** `set`, parsed (ms since the epoch), when it parses. */
  setAt?: number;
  /** When it was written down, as written (when that is known and may differ from `set`). */
  recorded?: string;
  /** `recorded`, parsed. */
  recordedAt?: number;
  /** What it covers (an area, a mob family, a quest), when the agent said. */
  applies?: string;
  /** Where it lives: the module that enforces it, or the notes it is written in. */
  where?: string;
  /** The last reasoned keep: when, and why. */
  kept?: { at?: string; because: string };
}

/** `2026-11-27 16:25`, `2026-11-27T16:25Z`, `2026-11-27`, `2026-11-27T16:25:10.223+10:00`: ms
 *  since the epoch, or undefined. A date and time with no zone is the machine's local time (how an
 *  agent writes the time it sees). */
export function parseWhen(text: string | undefined): number | undefined {
  const t = text?.trim();
  if (!t) return undefined;
  const m = /^(\d{4}-\d{2}-\d{2})(?:[ T](\d{1,2}:\d{2}(?::\d{2}(?:\.\d+)?)?))?\s*(Z|UTC|[+-]\d{2}:?\d{2})?$/i.exec(t);
  if (!m) return undefined;
  const zone = !m[3] ? "" : /^[+-]/.test(m[3]) ? m[3].replace(/^([+-]\d{2}):?(\d{2})$/, "$1:$2") : "Z";
  const iso = `${m[1]}T${m[2] ? (/^\d:/.test(m[2]) ? `0${m[2]}` : m[2]) : "00:00"}${zone}`;
  const at = Date.parse(iso);
  return Number.isFinite(at) ? at : undefined;
}

const RULE_LINE = /^\s*(?:--+\s*|[-*]\s+)?rule:\s*(.+)$/i;

/** One `rule: ... | why: ... | set: ...` line, or undefined when it is not one. */
export function parseRuleLine(line: string, where?: string): Rule | undefined {
  const m = RULE_LINE.exec(line);
  if (!m?.[1]) return undefined;
  const [head = "", ...fields] = m[1].split(/\s+\|\s+/);
  const rule: Rule = { rule: head.trim() };
  if (!rule.rule) return undefined;
  for (const field of fields) {
    const f = /^\s*(why|set|recorded|applies(?: to)?|kept)\s*:\s*(.*)$/i.exec(field);
    if (!f?.[1]) continue;
    const key = f[1].toLowerCase();
    const value = (f[2] ?? "").trim();
    if (key === "why") rule.why = value;
    else if (key === "set") rule.set = value;
    else if (key === "recorded") rule.recorded = value;
    else if (key.startsWith("applies")) rule.applies = value;
    else if (key === "kept") {
      const k = /^(.*?)\s*\bbecause\b:?\s*(.*)$/i.exec(value);
      rule.kept = k ? { ...(k[1] ? { at: k[1] } : {}), because: k[2] ?? "" } : { because: value };
    }
  }
  const at = parseWhen(rule.set);
  if (at !== undefined) rule.setAt = at;
  const recordedAt = parseWhen(rule.recorded);
  if (recordedAt !== undefined) rule.recordedAt = recordedAt;
  if (where) rule.where = where;
  return rule;
}

/** When a rule's cost counts from: when it was made, else when it was recorded. */
export function costStart(rule: Rule): number | undefined {
  return rule.setAt ?? rule.recordedAt;
}

/** True when a rule's cost counts only from its record (no `set` date: it may be older). */
export function fromRecordOnly(rule: Rule): boolean {
  return rule.setAt === undefined && rule.recordedAt !== undefined;
}

/** The rule lines of a Lua module's header: the comment block at its top. A header line counts
 *  only with a `why:` or `set:` field: modules also describe their own logic as
 *  "-- Rule: out of combat, hp < 90% -> bandage", which is no limit the agent set itself. */
export function rulesInModule(source: string, module: string): Rule[] {
  const out: Rule[] = [];
  for (const line of source.split("\n")) {
    const t = line.trim();
    if (t === "") continue;
    if (!t.startsWith("--")) break;
    const rule = parseRuleLine(t, module);
    if (rule && (rule.why !== undefined || rule.set !== undefined || rule.recorded !== undefined)) out.push(rule);
  }
  return out;
}

/** The rule lines anywhere in a notes file. */
export function rulesInNotes(text: string, where: string): Rule[] {
  return text.split("\n").flatMap((line) => parseRuleLine(line, where) ?? []);
}

/** Every module's header rules under the client's modules folder (`<folder>/a/b.lua` is `a.b`). */
export function rulesInFolder(folder: string): Rule[] {
  if (!existsSync(folder)) return [];
  const out: Rule[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith(".lua")) {
        const module = relative(folder, path).replace(/\.lua$/, "").split(/[\\/]/).join(".");
        try {
          out.push(...rulesInModule(readFileSync(path, "utf8"), module));
        } catch {
          // A file being written: next time.
        }
      }
    }
  };
  walk(folder);
  return out;
}

// -- What a rule has cost --------------------------------------------------------------------------

/** What happened while a rule stood: in-world hours, experience, quests handed in, deaths, and new
 *  zones and subzones (when the source knows them). */
export interface Cost {
  hours: number;
  xp: number;
  quests: number;
  deaths: number;
  places?: number;
}

/** The cost since `from`, from scorecard samples (agents/mcp's scorecard), or undefined without any. */
export function costFromSamples(samples: Sample[], from: number, maxGapMs: number): Cost | undefined {
  const since = samples.filter((s) => s.at >= from);
  if (since.length < 2) return undefined;
  const w = score(since, samples.filter((s) => s.at < from), { maxGapMs });
  return { hours: w.hours, xp: w.xp, quests: w.quests_done, deaths: w.deaths, places: w.new_zones + w.new_subzones };
}

/** The judge's folder, as the MCP server picks it (agents/mcp/src/monitor.ts): WRATHPLUS_JUDGE_DIR,
 *  else `judge` beside the modules folder. */
export function judgeFolder(modulesFolder: string, configured?: string): string {
  return resolve(configured?.trim() || join(modulesFolder, "..", "judge"));
}

/** The samples file of the character sampled most recently in a judge folder (the one playing). */
export function latestSamplesFile(judgeDir: string): string | undefined {
  const dir = join(judgeDir, "characters");
  if (!existsSync(dir)) return undefined;
  let best: { path: string; at: number } | undefined;
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry, "samples.jsonl");
    try {
      const at = statSync(path).mtimeMs;
      if (!best || at > best.at) best = { path, at };
    } catch {
      // No samples for this character.
    }
  }
  return best?.path;
}

/** A samples file, read fresh (the MCP server appends to it). */
export function readSamples(path: string): Sample[] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter(Boolean)
    .flatMap((line) => {
      try {
        return [JSON.parse(line) as Sample];
      } catch {
        return [];
      }
    });
}

const span = (hours: number): string => {
  const minutes = Math.round(hours * 60);
  if (minutes < 60) return `${minutes} min`;
  return `${(minutes / 60).toFixed(1)} h`;
};

const clip = (s: string, n: number): string => (s.length > n ? `${s.slice(0, n - 1)}…` : s);
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** What happened since, in a few words: "0 XP over 9.0 h, 0 quests, 0 deaths, 0 new places". */
export function costText(cost: Cost): string {
  const parts = [
    `${Math.round(cost.xp).toLocaleString("en")} XP over ${span(cost.hours)}`,
    plural(cost.quests, "quest"),
    plural(cost.deaths, "death"),
  ];
  if (cost.places !== undefined) parts.push(plural(cost.places, "new place"));
  return parts.join(", ");
}

/** One line for a rule and its cost (`cost`: measured from `costStart(rule)`):
 *  `rule "no walk south of y -1000" (set 2026-11-26 14:10, why: 2 deaths on the Rockjaw road; in paladin.zonehold) · since: 0 XP over 9.0 h, 0 quests, 0 deaths`.
 *  With no set date but a record date it reads `(recorded 2026-11-28 01:22Z; ...) · since recorded: ...`. */
export function ruleLine(rule: Rule, cost: Cost | undefined): string {
  const label = rule.id ? `#${rule.id}` : "";
  const recordOnly = fromRecordOnly(rule);
  const about = [
    rule.set ? `set ${rule.set}` : "",
    !rule.set && rule.recorded ? `recorded ${rule.recorded}` : "",
    rule.why ? `why: ${clip(rule.why, 140)}` : "",
    rule.applies ? `applies to ${clip(rule.applies, 80)}` : "",
    rule.where ? `in ${rule.where}` : "",
  ].filter(Boolean);
  const head = `${label ? `${label} ` : ""}rule "${clip(rule.rule, 140)}"${about.length ? ` (${about.join("; ")})` : ""}`;
  const sinceWord = recordOnly ? "since recorded" : "since";
  const since =
    costStart(rule) === undefined
      ? "since: not known (no set date)"
      : cost
        ? `${sinceWord}: ${costText(cost)}`
        : `${sinceWord}: no measurements`;
  const kept = rule.kept ? ` · kept${rule.kept.at ? ` ${rule.kept.at}` : ""} because ${clip(rule.kept.because, 140)}` : "";
  return `${head} · ${since}${kept}`;
}

// -- Stalls ----------------------------------------------------------------------------------------

/** One reflection or check-in window, as the harness measured it. */
export interface StallWindow {
  minutes: number;
  xp: number;
  quests: number;
  /** False when the window does not count (the client was out of the world for most of it). */
  counted?: boolean;
}

export interface StallOptions {
  /** This many counted windows in a row with no experience is a stall. */
  xpWindows: number;
  /** This many hours in a row of counted windows with no quest handed in is a stall. */
  questHours: number;
  /** A shorter window (a review a death triggered) adds its minutes but is not a window of its
   *  own for `xpWindows`. */
  minWindowMinutes: number;
}

export const STALL_DEFAULTS: StallOptions = { xpWindows: 3, questHours: 4, minWindowMinutes: 10 };

/** What has been flat, and for how long. */
export interface Stall {
  /** Windows in a row with no experience, and their minutes. */
  xpWindows: number;
  xpMinutes: number;
  /** Minutes in a row with no quest handed in. */
  questMinutes: number;
  /** Which objectives crossed their threshold. */
  xp: boolean;
  quests: boolean;
}

/** Follows the windows and says when an objective has been flat long enough to ask for a decision. */
export class StallWatch {
  readonly options: StallOptions;
  #xpWindows = 0;
  #xpMinutes = 0;
  #questMinutes = 0;

  constructor(options: Partial<StallOptions> = {}) {
    this.options = { ...STALL_DEFAULTS, ...options };
  }

  /** Add a window; the stall it makes, if any. */
  add(w: StallWindow): Stall | undefined {
    if (w.counted === false) return this.current;
    if (w.xp > 0) {
      this.#xpWindows = 0;
      this.#xpMinutes = 0;
    } else {
      this.#xpMinutes += w.minutes;
      if (w.minutes >= this.options.minWindowMinutes) this.#xpWindows++;
    }
    if (w.quests > 0) this.#questMinutes = 0;
    else this.#questMinutes += w.minutes;
    return this.current;
  }

  get current(): Stall | undefined {
    const xp = this.options.xpWindows > 0 && this.#xpWindows >= this.options.xpWindows;
    const quests = this.options.questHours > 0 && this.#questMinutes >= this.options.questHours * 60;
    if (!xp && !quests) return undefined;
    return { xpWindows: this.#xpWindows, xpMinutes: this.#xpMinutes, questMinutes: this.#questMinutes, xp, quests };
  }
}

/** The stall, plainly: "STALL: no experience in your last 18 reviews (9.0 h); no quest handed in
 *  for 9.0 h." `unit` names the windows ("reviews", "check-ins"). */
export function stallText(stall: Stall, unit = "reviews"): string {
  const parts: string[] = [];
  if (stall.xp) parts.push(`no experience in your last ${stall.xpWindows} ${unit} (${span(stall.xpMinutes / 60)})`);
  if (stall.quests) parts.push(`no quest handed in for ${span(stall.questMinutes / 60)}`);
  return `STALL: ${parts.join("; ")}.`;
}

/** The decision a stall asks for. `keep` says how this harness records a reasoned keep. Which
 *  change to make is never suggested: that is the planner's. */
export function decisionRequest(stall: Stall, keep: string, rules: number, unit = "reviews"): string {
  const own = rules > 0 ? " That change may be to revise or drop one of your own rules listed above." : "";
  return `${stallText(stall, unit)} This is a decision point. Either make one concrete change that could end the stall, and name it.${own} Or decide to keep things as they are and record why: ${keep}. "No change" without a reason is not an answer.`;
}
