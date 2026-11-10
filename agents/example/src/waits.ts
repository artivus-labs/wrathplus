// Nobody is coming. A planner trained as an assistant expects a user in the loop, and when it runs
// out of safe ideas it asks "the operator" and waits. This run has no operator, so the harness
// refuses the waits that can only end with a person answering:
//   - a hold whose reason is a person, an answer, an instruction or a permission (`HoldGate`);
//   - the same hold again after the fast loop ended it (`HoldGate.stopped`);
//   - a long sleep with nothing running in the game (`idleWakeRefusal`).
// Each refusal says the same thing: nobody will answer, decide yourself, here is what you have.

import { choice, type Jev } from "./jev.ts";
import type { Lua } from "./goals.ts";

export const NOBODY =
  "There is no operator and no user: nobody will answer a question, approve a plan, give an instruction or log in. Every decision is yours: decide now with what you know, note why in MEMORY.md, and act.";

/** Words that make a wait a wait for a person. Used when no fast tier can read the reason (and when
 *  Jev fails): it refuses the obvious cases. With a fast tier Jev reads the reason instead, since
 *  "answer" can be an honest game word ("Pally's answer to my invite"). */
export const PERSON_WAIT =
  /\b(operators?|users?|humans?|developers?|admins?|supervisors?|maintainers?|owners?|answers?|repl(?:y|ies)|decisions?|instructions?|permissions?|approvals?|approve[sd]?|confirmations?|go[- ]ahead|no[- ]go|guidance|feedback)\b/i;

/** What the client says about itself in one call: the goals and their plans, the facts, the zones
 *  that fit, the party, and whether anything is running. */
export const BRIEF_LUA = `if not Goals then return false end
local goals = {}
for _, g in ipairs(Goals.list()) do
  goals[#goals + 1] = { name = g.name, why = g.why, priority = g.priority, status = g.status, plan = g.plan, note = g.note }
end
local zones = {}
for _, z in ipairs(Goals.zones and Goals.zones() or {}) do
  if #zones < 6 and (z.fit == "ok" or z.current) then
    zones[#zones + 1] = { name = z.name, min = z.min, max = z.max, fit = z.fit, distance = z.distance, current = z.current or nil }
  end
end
local party = {}
for i = 1, (GetNumPartyMembers and GetNumPartyMembers() or 0) do
  local name = UnitName("party" .. i)
  if name then party[#party + 1] = { name = name, online = not UnitIsConnected or UnitIsConnected("party" .. i) and true or false } end
end
local reason, waited, remaining
if Goals.holding then reason, waited, remaining = Goals.holding() end
local walk = AgentWalk and AgentWalk()
return { on = Goals.on and true or false, state = Goals.state, goals = goals, facts = Goals.facts(), zones = zones,
  party = party, hold = reason and { reason = reason, waited = waited, remaining = remaining } or nil,
  activity = Agent and Agent.activity and Agent.activity() or nil,
  walking = walk and walk.status == "walking" or false,
  quests_busy = Quests and Quests.busy and Quests.busy() or false,
  combat = UnitAffectingCombat("player") and true or false,
  dead = UnitIsDeadOrGhost("player") and true or false, level = UnitLevel("player") }`;

interface BriefGoal {
  name: string;
  why?: string;
  priority?: number;
  status?: string;
  plan?: unknown;
  note?: string;
}

export interface Brief {
  on: boolean;
  state?: string;
  goals: BriefGoal[];
  facts: Record<string, unknown>;
  zones: { name: string; min: number; max: number; fit: string; distance?: number; current?: boolean }[];
  party: { name: string; online: boolean }[];
  hold?: { reason: string; waited: number; remaining: number };
  activity?: { name: string; seconds: number } | null;
  walking: boolean;
  quests_busy: boolean;
  combat: boolean;
  dead: boolean;
  level?: number;
}

// An empty Lua table arrives as {} rather than []: read lists defensively.
const list = <T>(value: unknown): T[] => (Array.isArray(value) ? (value as T[]) : []);

/** The client's brief, or null when it has no goal runner or does not answer. */
export async function readBrief(lua: Lua): Promise<Brief | null> {
  try {
    const result = await lua(BRIEF_LUA);
    const value = result.ok ? result.values?.[0] : undefined;
    if (!value || typeof value !== "object") return null;
    const raw = value as Record<string, unknown>;
    return {
      ...(raw as unknown as Brief),
      goals: list<BriefGoal>(raw.goals),
      zones: list<Brief["zones"][number]>(raw.zones),
      party: list<Brief["party"][number]>(raw.party),
      facts: raw.facts && typeof raw.facts === "object" ? (raw.facts as Record<string, unknown>) : {},
    };
  } catch {
    return null;
  }
}

/** Each goal with the next actions the runner has planned, or why none reaches it. */
export function goalLines(goals: BriefGoal[]): string[] {
  return goals.map((g) => {
    const plan = list<string>(g.plan);
    const what =
      g.status === "blocked" || (g.status === "active" && !plan.length)
        ? `blocked: ${g.note ?? "no plan reaches it now"}`
        : plan.length
          ? `${g.status ?? "active"}; next: ${plan.join(" → ")}`
          : (g.status ?? "?");
    return `- ${g.name}${g.priority === undefined ? "" : ` (priority ${g.priority})`}: ${what}${g.why ? `. Why: ${g.why}` : ""}`;
  });
}

/** What the planner has to decide with: the goals and their next actions, the facts, the zones. */
export function describeBrief(brief: Brief | null): string {
  if (!brief) return "The goals tool shows your goals, their plans and the facts.";
  const out: string[] = [];
  out.push(`Your goals and what the runner would do next (the runner is ${brief.on ? "on" : "off"}${brief.state ? `: ${brief.state}` : ""}):`);
  out.push(...(brief.goals.length ? goalLines(brief.goals) : ["- none"]));
  const facts = Object.entries(brief.facts)
    .filter(([, v]) => v !== false)
    .map(([k, v]) => (v === true ? k : `${k} = ${String(v)}`));
  out.push(`Facts that hold now: ${facts.join(", ") || "none"}.`);
  if (brief.zones.length) {
    const zones = brief.zones.map((z) => `${z.name} (levels ${z.min}-${z.max}${z.current ? ", here" : z.distance !== undefined ? `, ${z.distance} yd` : ""}${z.fit !== "ok" ? `, ${z.fit === "low" ? "below you" : "above you"}` : ""})`);
    out.push(`Zones for level ${brief.level ?? "?"} (Goals.zones()): ${zones.join("; ")}.`);
  }
  return out.join("\n");
}

/** Nothing is running and nothing will start by itself: no activity, walk, fight or quest visit,
 *  and the goal runner is off or has no plan it can act on. */
export function gameIdle(brief: Brief): boolean {
  if (brief.activity || brief.walking || brief.quests_busy || brief.combat || brief.dead) return false;
  if (!brief.on) return true;
  return !brief.goals.some((g) => g.status === "active" && list<string>(g.plan).length > 0);
}

/** The longest sleep the planner may ask for while the game is idle (s). Long enough to look
 *  again at something slow, short enough that an idle character is noticed within minutes. */
export const IDLE_WAKE_SECONDS = 300;

/** A wake-up that would leave an idle character standing: one with no time limit, or one longer
 *  than `IDLE_WAKE_SECONDS`. Returns the refusal, or null when the wake-up is fine. */
export async function idleWakeRefusal(lua: Lua, seconds: number | undefined, limit = IDLE_WAKE_SECONDS): Promise<string | null> {
  if (seconds !== undefined && seconds <= limit) return null;
  const brief = await readBrief(lua);
  if (!brief || !gameIdle(brief)) return null;
  const asked = seconds === undefined ? "with no time limit" : `of ${seconds}s`;
  return [
    `Not set: nothing is running in the game (no activity, walk or fight, and the goal runner ${brief.on ? "has nothing it can do" : "is off"}), so a wake-up ${asked} would leave your character standing there. Sleeping changes nothing: nothing is in progress that could finish, and nobody will act for you meanwhile.`,
    NOBODY,
    "Start something now, then sleep: push a goal the runner can act on, teach the action a blocked goal is missing, travel to a zone that fits, or run an activity. Your turn has not ended.",
    describeBrief(brief),
    `To look again soon instead, ask for at most ${limit}s.`,
  ].join("\n");
}

const STOP_WORDS = new Set(
  "the a an and or but for from with without until till about into onto over still yet not no none any again another more also just now then while when what where which who whom whose why how this that these those here there have has had was were are been being will would should could may might must can cannot wait waits waiting waited await awaits awaiting hold holds holding held pause paused pending check checks checking checked recheck keep keeps keeping stay staying".split(" "),
);

/** The words that carry a hold's reason: lower case, no digits or punctuation, no filler. */
export function reasonWords(reason: string): Set<string> {
  const words = reason
    .toLowerCase()
    .replace(/[^a-z\s]+/g, " ")
    .split(/\s+/)
    .filter((w) => w.length > 2 && !STOP_WORDS.has(w))
    // A crude stem, so "decisions" matches "decision" and "answered" "answer".
    .map((w) => w.replace(/(?:ing|ed|s)$/, ""));
  return new Set(words);
}

/** The same wait in other words: half the distinct words shared, or one reason's words all inside
 *  the other's ("Still no operator zone decision" / "No operator zone decision yet (3rd check)"). */
export function similarReasons(a: string, b: string): boolean {
  const x = reasonWords(a);
  const y = reasonWords(b);
  if (!x.size || !y.size) return a.trim().toLowerCase() === b.trim().toLowerCase();
  let shared = 0;
  for (const w of x) if (y.has(w)) shared++;
  if (shared === Math.min(x.size, y.size)) return true;
  return shared / (x.size + y.size - shared) >= 0.5;
}

export function waitQuestion() {
  return choice(
    "A game-playing agent wants to pause its goals and wait, for the reason in state.reason. Nobody outside the game is there: no operator, user or developer will ever answer it, decide for it or give it permission. Its party is in state.party (name, online). What is it waiting for?",
    {
      game: "Something in the game that will happen without anyone outside deciding it: a party member who is online (to arrive, catch up, finish, reply in chat), a respawn, a cooldown, a buff or debuff running out, health or mana coming back, a boat, a scheduled event, a group forming.",
      person: "A person outside the game, or something only one could give: an operator, a user, a developer, an administrator, 'you', an answer to a question it asked, a decision it wants made for it, an instruction, a permission, an approval, a confirmation; or a party member who is offline.",
    },
  );
}

export interface HoldGateOptions {
  lua: Lua;
  /** The fast tier, when there is one: it reads the reason. Without it, `PERSON_WAIT` does. */
  jev?: Jev;
  now?: () => number;
  /** After the fast loop ends a wait, the same wait is refused for this long (ms, default 30 min). */
  refuseMs?: number;
}

/** What stands between the planner's `hold` tool and `Goals.hold`. */
export class HoldGate {
  #ended: { reason: string; at: number }[] = [];
  #open: { at: number; ms: number } | undefined;
  #closedMs = 0;
  calls = 0;

  constructor(readonly options: HoldGateOptions) {}

  #now(): number {
    return this.options.now?.() ?? Date.now();
  }

  /** Seconds held this session: every hold placed (the planner's and the fast loop's extensions),
   *  each counted until it ran out, was replaced or was released. */
  get totalSeconds(): number {
    const open = this.#open ? Math.min(this.#open.ms, this.#now() - this.#open.at) : 0;
    return Math.round((this.#closedMs + Math.max(0, open)) / 1000);
  }

  #close(): void {
    if (this.#open) this.#closedMs += Math.max(0, Math.min(this.#open.ms, this.#now() - this.#open.at));
    this.#open = undefined;
  }

  /** A hold was placed (by the planner, or kept going by the fast loop). */
  placed(seconds: number): void {
    this.#close();
    this.#open = { at: this.#now(), ms: seconds * 1000 };
  }

  released(): void {
    this.#close();
  }

  /** The fast loop judged this wait over (stop_waiting). */
  stopped(reason: string): void {
    this.#close();
    this.#ended.push({ reason, at: this.#now() });
    if (this.#ended.length > 20) this.#ended.shift();
  }

  /** Why this hold may not be placed, or null when it may. */
  async check(reason: string): Promise<string | null> {
    const now = this.#now();
    const refuseMs = this.options.refuseMs ?? 1_800_000;
    const ended = this.#ended.findLast((e) => now - e.at < refuseMs && similarReasons(e.reason, reason));
    if (ended) {
      const ago = Math.max(1, Math.round((now - ended.at) / 60_000));
      const left = Math.max(1, Math.round((refuseMs - (now - ended.at)) / 60_000));
      return this.#refusal(
        `Hold refused: this is the wait the fast loop ended ${ago} min ago ("${ended.reason}": stop waiting, nothing it waited for is coming). The same wait is refused for another ${left} min.`,
      );
    }
    if (await this.#forAPerson(reason)) {
      return this.#refusal(`Hold refused: "${reason}" waits for a person, an answer, an instruction or a permission, and none will come.`);
    }
    return null;
  }

  async #forAPerson(reason: string): Promise<boolean> {
    const jev = this.options.jev;
    if (!jev) return PERSON_WAIT.test(reason);
    try {
      const brief = await readBrief(this.options.lua);
      const response = await jev.ask({ reason, party: brief?.party ?? [] }, { waits_for: waitQuestion() });
      this.calls++;
      const answer = response.answers.waits_for;
      if (answer?.type === "choice") return answer.choice === "person" && answer.confidence >= 0.5;
    } catch {
      // Jev did not answer: the pattern is the fallback.
    }
    return PERSON_WAIT.test(reason);
  }

  async #refusal(first: string): Promise<string> {
    return [
      first,
      NOBODY,
      `Hold only for something in the game that will happen by itself (a party member who is online, a respawn, a cooldown, a scheduled event). You have held for ${duration(this.totalSeconds)} in all this session.`,
      describeBrief(await readBrief(this.options.lua)),
    ].join("\n");
  }
}

export function duration(seconds: number): string {
  if (seconds < 90) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  return minutes < 90 ? `${minutes} min` : `${(minutes / 60).toFixed(1)} h`;
}
