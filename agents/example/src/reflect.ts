// The reflection loop: now and then the planner stops playing and reviews how it has been doing,
// then works on its own harness (its Lua modules, goals and actions) like a coding agent on its own
// code. A review is triggered by a death, a level-up, actions that keep failing, a stall (no
// progress for a while), or, when nothing else has, a periodic review at a quiet moment. It gets a
// digest of what happened since the last one, built from what the harness already sees (the game
// events, fight reviews, goal and action signals, the fast loop's decisions and holds, idle time,
// Lua errors) and a scoreboard to judge changes by.

import type { Change, ModuleVersions } from "./modules.ts";
import { line as changeLine } from "./modules.ts";
import type { Decision, HoldDecision, TimeShares } from "./policy.ts";
import { shares } from "./policy.ts";
import type { GameEvent } from "./watch.ts";

/** The character's experience at a moment, read from the client (`XP_LUA`). */
export interface XpSample {
  level: number;
  xp: number;
  max: number;
}

export const XP_LUA = `return { level = UnitLevel("player") or 0, xp = UnitXP("player") or 0, max = UnitXPMax("player") or 0 }`;

/** What the fast tier has done, for the digest. */
export interface FastTier {
  decisions: Decision[];
  holds: HoldDecision[];
  time: TimeShares;
}

export interface ReflectOptions {
  /** Deliver a reflection turn to the planner. */
  onReflect: (text: string, score: Score) => void;
  now?: () => number;
  /** Let a trigger settle (a death's fight review, the corpse run) before reviewing (ms). */
  settleMs?: number;
  /** At least this long between reviews (ms). */
  minGapMs?: number;
  /** A review at a quiet moment this long after the last one, if nothing else triggered one (ms). */
  periodMs?: number;
  /** No progress (experience, a kill, a quest handed in, a goal done) for this long is a stall (ms). */
  stallMs?: number;
  /** This many failed actions within `failureWindowMs` trigger a review. */
  failures?: number;
  failureWindowMs?: number;
  /** Whether now is a good moment (not in the middle of a planner turn). Combat is tracked here. */
  quiet?: () => boolean;
  fast?: () => FastTier | undefined;
  versions?: ModuleVersions;
}

/** The scoreboard: this window (since the last review) and the session. */
export interface Score {
  window_minutes: number;
  xp_per_hour: number | null;
  session_xp_per_hour: number | null;
  level: number | null;
  deaths: number;
  session_deaths: number;
  quests_done: number;
  session_quests_done: number;
  kills: number;
  fights_lost: number;
  action_failures: number;
  idle_pct: number | null;
  hold_pct: number | null;
}

interface Totals {
  since: number;
  xp: number;
  deaths: number;
  quests: number;
}

interface Window {
  since: number;
  deaths: { at: number; cause?: string }[];
  levels: number[];
  reviews: string[];
  kills: number;
  lost: number;
  goalsDone: { name: string; ok: boolean }[];
  blocked: Map<string, string>;
  failures: { at: number; what: string; why: string }[];
  holdsExpired: number;
  luaErrors: Map<string, number>;
  gameErrors: Map<string, number>;
  quests: number;
  pickedUp: number;
  xp: number;
  time: TimeShares;
}

const zeroTime = (): TimeShares => ({ busy: 0, idle: 0, hold: 0, other: 0 });

export class Reflection {
  #window: Window;
  #session: Totals;
  #xp: XpSample | undefined;
  #pending = new Set<string>();
  #dueAt = 0;
  #lastReview: number;
  #lastProgress: number;
  #stallFlagged = false;
  #failuresFlagged = false;
  #combat = false;
  #timeAtReview: TimeShares = zeroTime();
  reviews = 0;

  constructor(readonly options: ReflectOptions) {
    const now = this.#now();
    this.#lastReview = now;
    this.#lastProgress = now;
    this.#session = { since: now, xp: 0, deaths: 0, quests: 0 };
    this.#window = this.#fresh(now);
  }

  #now(): number {
    return this.options.now?.() ?? Date.now();
  }

  #fresh(now: number): Window {
    return {
      since: now,
      deaths: [],
      levels: [],
      reviews: [],
      kills: 0,
      lost: 0,
      goalsDone: [],
      blocked: new Map(),
      failures: [],
      holdsExpired: 0,
      luaErrors: new Map(),
      gameErrors: new Map(),
      quests: 0,
      pickedUp: 0,
      xp: 0,
      time: zeroTime(),
    };
  }

  /** A trigger: the review comes once it has settled, the gap since the last has passed, and it is quiet. */
  trigger(reason: string, settleMs = this.options.settleMs ?? 20_000): void {
    const now = this.#now();
    if (!this.#pending.size) this.#dueAt = now + settleMs;
    else this.#dueAt = Math.max(this.#dueAt, now + settleMs);
    this.#pending.add(reason);
  }

  get pending(): string[] {
    return [...this.#pending];
  }

  #progress(): void {
    this.#lastProgress = this.#now();
    this.#stallFlagged = false;
  }

  /** Every batch of game events. */
  notice(events: GameEvent[]): void {
    const w = this.#window;
    const now = this.#now();
    for (const e of events) {
      const a = e.args;
      switch (e.event) {
        case "PLAYER_DEAD":
          w.deaths.push({ at: now });
          this.#session.deaths++;
          this.trigger("you died");
          break;
        case "PLAYER_LEVEL_UP":
          w.levels.push(Number(a[0]));
          this.#progress();
          this.trigger(`you reached level ${a[0]}`);
          break;
        case "AGENT_COMBAT_REVIEW": {
          const text = String(a[0] ?? "");
          w.reviews.push(text);
          // A death's review names what killed it: the cause, for the death without one.
          const death = w.deaths.findLast((d) => d.cause === undefined);
          if (death && text.startsWith("You died")) death.cause = text.split(". ")[0];
          break;
        }
        case "AGENT_COMBAT_DONE":
          if (a[1] === "killed") {
            w.kills++;
            this.#progress();
          } else if (a[1] === "lost") w.lost++;
          break;
        case "PLAYER_REGEN_DISABLED":
          this.#combat = true;
          break;
        case "PLAYER_REGEN_ENABLED":
        case "PLAYER_ALIVE":
          this.#combat = false;
          break;
        case "AGENT_QUESTS_DONE":
          if (Number(a[2]) > 0) {
            w.quests += Number(a[2]);
            this.#session.quests += Number(a[2]);
            this.#progress();
          }
          w.pickedUp += Number(a[1]) || 0;
          break;
        case "AGENT_GOAL_DONE":
          w.goalsDone.push({ name: String(a[0]), ok: a[1] !== false });
          if (a[1] !== false) this.#progress();
          break;
        case "AGENT_GOAL_BLOCKED":
          w.blocked.set(String(a[0]), String(a[1] ?? ""));
          break;
        case "AGENT_ACTION_FAILED":
          this.#failed(now, `${String(a[1])} (for ${String(a[0])})`, String(a[2] ?? ""));
          break;
        case "AGENT_INTENT_DONE":
          // Goal actions are counted by AGENT_ACTION_FAILED; a replaced activity did not fail.
          if (a[1] === false && !String(a[0]).startsWith("goal:") && !String(a[2] ?? "").startsWith("replaced")) {
            this.#failed(now, String(a[0]), String(a[2] ?? ""));
          }
          break;
        case "AGENT_HOLD_EXPIRED":
          w.holdsExpired++;
          break;
        case "AGENT_LUA":
          if (a[1] === false) bump(w.luaErrors, `${String(a[2]).slice(0, 160)} (in: ${String(a[0]).slice(0, 60)})`);
          break;
        case "UI_ERROR_MESSAGE":
          if (typeof a[0] === "string") bump(w.gameErrors, a[0]);
          break;
      }
    }
  }

  #failed(now: number, what: string, why: string): void {
    const w = this.#window;
    w.failures.push({ at: now, what, why });
    const windowMs = this.options.failureWindowMs ?? 600_000;
    const recent = w.failures.filter((f) => now - f.at < windowMs).length;
    if (recent >= (this.options.failures ?? 3) && !this.#failuresFlagged) {
      this.#failuresFlagged = true;
      this.trigger(`${recent} actions failed in ${Math.round(windowMs / 60_000)} min`, 0);
    }
  }

  /** The character's experience now (sampled every minute or so). */
  sample(xp: XpSample): void {
    const last = this.#xp;
    this.#xp = xp;
    if (!last || !(xp.level > 0)) return;
    let gained = 0;
    if (xp.level === last.level) gained = xp.xp - last.xp;
    // A level gained: the rest of the old level, then this one's (a skipped level counts its max).
    else if (xp.level > last.level) gained = last.max - last.xp + xp.xp + (xp.level - last.level - 1) * xp.max;
    if (gained > 0) {
      this.#window.xp += gained;
      this.#session.xp += gained;
      this.#progress();
    }
  }

  /** Call now and then (every 15 s or so): fires a review when one is due. */
  check(): string | null {
    const now = this.#now();
    if (!this.#stallFlagged && now - this.#lastProgress >= (this.options.stallMs ?? 900_000)) {
      this.#stallFlagged = true;
      this.trigger(`no progress for ${Math.round((now - this.#lastProgress) / 60_000)} min`, 0);
    }
    const quiet = !this.#combat && (this.options.quiet?.() ?? true);
    if (!this.#pending.size && quiet && now - this.#lastReview >= (this.options.periodMs ?? 1_800_000)) {
      this.#pending.add("a regular review");
      this.#dueAt = now;
    }
    if (!this.#pending.size || now < this.#dueAt || !quiet) return null;
    if (now - this.#lastReview < (this.options.minGapMs ?? 300_000)) return null;
    return this.reflect();
  }

  /** The scoreboard so far. */
  score(): Score {
    const now = this.#now();
    const w = this.#window;
    const hours = (now - w.since) / 3_600_000;
    const sessionHours = (now - this.#session.since) / 3_600_000;
    const time = this.#windowTime();
    const share = time ? shares(time) : null;
    const perHour = (xp: number, h: number) => (h > 0 && this.#xp ? Math.round(xp / h) : null);
    return {
      window_minutes: Math.round((now - w.since) / 60_000),
      xp_per_hour: perHour(w.xp, hours),
      session_xp_per_hour: perHour(this.#session.xp, sessionHours),
      level: this.#xp?.level ?? null,
      deaths: w.deaths.length,
      session_deaths: this.#session.deaths,
      quests_done: w.quests,
      session_quests_done: this.#session.quests,
      kills: w.kills,
      fights_lost: w.lost,
      action_failures: w.failures.length,
      idle_pct: share?.idle ?? null,
      hold_pct: share?.hold ?? null,
    };
  }

  /** The fast tier's time since the last review. */
  #windowTime(): TimeShares | null {
    const time = this.options.fast?.()?.time;
    if (!time) return null;
    const base = this.#timeAtReview;
    return { busy: time.busy - base.busy, idle: time.idle - base.idle, hold: time.hold - base.hold, other: time.other - base.other };
  }

  /** Build the digest, hand it to the planner, and start a new window. */
  reflect(): string {
    const now = this.#now();
    const reasons = this.#pending.size ? [...this.#pending] : ["asked for"];
    const score = this.score();
    const text = digest(reasons, this.#window, score, this.options.fast?.(), this.options.versions?.changelog(undefined, 200) ?? []);
    this.reviews++;
    this.#pending.clear();
    this.#failuresFlagged = false;
    this.#lastReview = now;
    const time = this.options.fast?.()?.time;
    if (time) this.#timeAtReview = { ...time };
    this.#window = this.#fresh(now);
    this.options.onReflect(text, score);
    return text;
  }
}

function bump(map: Map<string, number>, key: string): void {
  map.set(key, (map.get(key) ?? 0) + 1);
}

const top = (map: Map<string, number>, n = 5) =>
  [...map.entries()].sort((a, b) => b[1] - a[1]).slice(0, n).map(([k, v]) => `"${k}" ×${v}`);

const clock = (ms: number) => new Date(ms).toISOString().slice(11, 16);

/** The reflection turn's text. */
export function digest(reasons: string[], w: Window, score: Score, fast: FastTier | undefined, changes: Change[]): string {
  const out: string[] = [];
  out.push(`Reflection (${reasons.join("; ")}). Since your last review, ${score.window_minutes} min ago:`);
  const rate = (v: number | null) => (v === null ? "n/a" : v.toLocaleString("en"));
  out.push(
    [
      "Scoreboard (since the last review | this session):",
      `- XP/hour: ${rate(score.xp_per_hour)} | ${rate(score.session_xp_per_hour)}${score.level ? ` (level ${score.level})` : ""}`,
      `- Deaths: ${score.deaths} | ${score.session_deaths}`,
      `- Quests handed in: ${score.quests_done} | ${score.session_quests_done}${w.pickedUp ? ` (${w.pickedUp} picked up)` : ""}`,
      `- Fights: ${score.kills} won, ${score.fights_lost} lost`,
      `- Idle: ${score.idle_pct === null ? "n/a" : `${score.idle_pct}%`}, holding: ${score.hold_pct === null ? "n/a" : `${score.hold_pct}%`}`,
      `- Failed actions: ${score.action_failures}`,
    ].join("\n"),
  );
  if (w.deaths.length) out.push(`Deaths:\n${w.deaths.map((d) => `- ${clock(d.at)} ${d.cause ?? "cause not reported (no fight review)"}`).join("\n")}`);
  if (w.levels.length) out.push(`Levels reached: ${w.levels.join(", ")}.`);
  if (w.reviews.length) out.push(`Fight reviews:\n${w.reviews.slice(-3).map((r) => `- ${r}`).join("\n")}`);
  const goals: string[] = [];
  if (w.goalsDone.length) goals.push(`- Done: ${w.goalsDone.map((g) => `${g.name}${g.ok ? "" : " (failed)"}`).join(", ")}`);
  if (w.blocked.size) goals.push(`- Blocked: ${[...w.blocked].map(([n, why]) => `${n}${why ? ` (${why})` : ""}`).join(", ")}`);
  if (w.failures.length) {
    const byWhat = new Map<string, number>();
    for (const f of w.failures) bump(byWhat, `${f.what}: ${f.why}`);
    goals.push(`- Failed actions: ${top(byWhat).join(", ")}`);
  }
  if (goals.length) out.push(`Goals and actions:\n${goals.join("\n")}`);
  if (fast) {
    const since = w.since;
    const decisions = fast.decisions.filter((d) => d.at >= since);
    const holds = fast.holds.filter((h) => h.at >= since);
    const lines: string[] = [];
    if (decisions.length) {
      const counts = new Map<string, number>();
      for (const d of decisions) bump(counts, d.choice);
      const unsure = decisions.filter((d) => d.confidence < 0.5).length;
      const errors = decisions.filter((d) => d.result?.startsWith("error")).length;
      lines.push(`- ${decisions.length} choices: ${top(counts, 8).map((s) => s.replace(/"/g, "")).join(", ")}${unsure ? `; ${unsure} unsure (< 0.5)` : ""}${errors ? `; ${errors} failed to start` : ""}`);
    }
    for (const h of holds.slice(-5)) {
      lines.push(`- Hold "${h.reason}" ran out after ${h.minutes_waited} min (here: ${h.here.join(", ") || "nobody"}): ${h.choice}${h.seconds ? ` for ${h.seconds}s` : ""}`);
    }
    if (lines.length) out.push(`Fast loop (Jev):\n${lines.join("\n")}`);
  }
  if (w.holdsExpired && !fast) out.push(`Holds that ran out: ${w.holdsExpired}.`);
  if (w.luaErrors.size) out.push(`Lua errors:\n${top(w.luaErrors).map((s) => `- ${s}`).join("\n")}`);
  if (w.gameErrors.size) out.push(`Game errors: ${top(w.gameErrors).join(", ")}.`);
  const recent = changes.filter((c) => Date.parse(c.at) >= w.since);
  const earlier = changes.filter((c) => Date.parse(c.at) < w.since && c.kind !== "baseline").slice(-5);
  if (recent.length || earlier.length) {
    out.push(
      `Your module changes (module_versions has them all):\n${[...earlier, ...recent].map((c) => `- ${changeLine(c)}`).join("\n")}\nJudge each change by what it said to watch: if the numbers got worse since, roll it back (rollback_module).`,
    );
  }
  out.push(GUIDANCE);
  return out.join("\n\n");
}

export const GUIDANCE = `Work on your harness as a coding agent works on its own code:
1. Pick the one or two things that cost you the most above (deaths, idle time, an action that keeps failing, a goal that stays blocked, a slow fight). Leave the rest.
2. Read before you change: the module (modules), its history (module_versions), the goal and its plan (goals).
3. Make a small change: one behaviour at a time (a fact, an action's pre or done, a Lua handler for a fight, a rotation entry), not a rewrite.
4. Test it in the client first: run the new code with lua on the real case and read the result back.
5. Deploy: save_module, AgentReload it, then note_change with why and what to watch on this scoreboard.
6. At the next review, check that number. Worse: rollback_module. Better: keep it, and write the lesson in MEMORY.md.
If nothing needs changing, say so in a line, and end your turn.`;
