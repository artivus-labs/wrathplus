// The fast tier: Jev (TypeSafe's System One model) chooses which intent the agent acts on next,
// and answers the decisions its realtime Lua hands up. It is event-driven: it sleeps while an
// activity runs and wakes when something happens that needs a choice (the activity ended, the
// planner sent a new plan, a decision opened, combat started, the character died...). A slow
// heartbeat is the safety net. The realtime tier (the agent's own Lua) does the work between
// choices; the slow tier (the planner) sets the plan and hears when it stalls, looks complete, or
// an intent keeps failing.

import { choice, chosen, Jev, noul, yes, type Answer, type Question } from "./jev.ts";
import { holdLua, luaString } from "./goals.ts";
import { goalLines, NOBODY } from "./waits.ts";

export { luaString };
import type { GameEvent } from "./watch.ts";

/** A planner intent: when it applies (Jev reads this), the Lua that starts it, and optionally the
 *  Lua that says it has finished (until) and how long it may take. */
export interface Intent {
  name: string;
  when: string;
  lua: string;
  until?: string;
  timeout?: number;
}

/** One of the planner's steps: run in order by the client (`Agent.set_steps`). */
export interface Step {
  name: string;
  lua: string;
  until?: string;
  timeout?: number;
}

/** An intent as `Agent.available()` lists it. */
export interface Registered {
  name: string;
  when: string;
}

/** A choice the realtime tier is waiting on (`Agent.decide`), as `Agent.questions()` lists it. */
export interface OpenDecision {
  id: number;
  name: string;
  question: string;
  options: Record<string, string>;
  context?: unknown;
}

export interface Decision {
  at: number;
  reasons: string[];
  choice: string;
  confidence: number;
  ran: boolean;
  result?: string;
  stuck: number;
}

export const WAIT = {
  name: "wait",
  when: "Nothing needs a new choice: the current activity or the character's own behaviour is handling things, or the plan says to wait.",
};

/** Events that wake the fast tier, and whether they should make it reconsider a running activity. */
const WAKE: Record<string, { reason: string; interrupt: boolean }> = {
  AGENT_INTENT_DONE: { reason: "activity ended", interrupt: true },
  AGENT_DECISION: { reason: "decision opened", interrupt: false },
  AGENT_WALK_DONE: { reason: "walk ended", interrupt: false },
  AGENT_COMBAT_DONE: { reason: "fight ended", interrupt: false },
  AGENT_PARTY_QUEST: { reason: "a party member's quests changed", interrupt: true },
  PLAYER_REGEN_DISABLED: { reason: "combat started", interrupt: true },
  PLAYER_REGEN_ENABLED: { reason: "combat ended", interrupt: false },
  PLAYER_DEAD: { reason: "died", interrupt: true },
  // Fired on releasing to a ghost as well as on a spirit healer's resurrection.
  PLAYER_ALIVE: { reason: "released or resurrected", interrupt: true },
  PLAYER_UNGHOST: { reason: "alive again", interrupt: true },
  LOOT_OPENED: { reason: "loot window", interrupt: false },
  GOSSIP_SHOW: { reason: "gossip window", interrupt: false },
  QUEST_DETAIL: { reason: "quest window", interrupt: false },
  QUEST_PROGRESS: { reason: "quest window", interrupt: false },
  QUEST_COMPLETE: { reason: "quest turn-in window", interrupt: false },
  PLAYER_LEVEL_UP: { reason: "level up", interrupt: false },
  PARTY_INVITE_REQUEST: { reason: "party invite", interrupt: false },
  AGENT_HOLD_EXPIRED: { reason: "a hold ran out", interrupt: true },
  AGENT_GOAL_DONE: { reason: "goal finished", interrupt: false },
  AGENT_GOAL_BLOCKED: { reason: "goal blocked", interrupt: false },
  AGENT_ACTION_FAILED: { reason: "goal action failed", interrupt: false },
};

/** What `wait` means depends on what is going on: only a running activity or a bounded hold makes
 *  it a real choice. With nothing running it is offered for a grace period (the goal runner starts
 *  the next action by itself), then not at all. */
export const WAIT_WHEN = {
  busy: "Nothing needs a new choice: the current activity or the character's own behaviour is handling things.",
  hold: "A hold is active (state.game.hold: why, and the seconds left): keep it unless something needs doing now.",
  idle: "Nothing is running and there is no hold: the goal runner is about to start the top goal's next action by itself. Choose this only if that is the right thing; the character must not stand idle.",
};

/** The choices on a hold that has run out (AGENT_HOLD_EXPIRED). */
export const HOLD_CHOICES = {
  keep_waiting: "Keep waiting: what it waits for is really coming (whoever it waits for is online and on the way, or said to wait), and waiting longer is worth more than getting on.",
  stop_waiting: "Stop waiting and get on with its goals: whoever it waits for is offline, not here and not coming, or it has waited long enough; or it waits for someone outside the game (an operator, a user, an answer, an instruction, a permission), and nobody outside the game will ever answer.",
};

/** How much longer a kept hold lasts, as choices for Jev. */
export const HOLD_LENGTHS: Record<string, { seconds: number; when: string }> = {
  minute: { seconds: 60, when: "About a minute: it is nearly here." },
  few_minutes: { seconds: 180, when: "A few minutes." },
  ten_minutes: { seconds: 600, when: "Up to ten minutes: they said they would be a while." },
};

/** A hold that ran out, as the client signalled it. */
export interface HoldExpiry {
  reason: string;
  /** Seconds this hold lasted. */
  waited: number;
}

interface PartyMember {
  name: string;
  online?: boolean;
  yards?: number;
}

export interface Goal {
  name: string;
  why?: string;
  priority?: number;
  status?: string;
  plan?: string[];
  note?: string;
}

/** What Jev reads to decide on an expired hold: the reason, how long in all, who is actually here. */
export interface HoldContext {
  reason: string;
  minutes_waited: number;
  party: { here: string[]; online_elsewhere: string[]; offline: string[] };
  goals: Goal[];
}

export function holdContext(expiry: HoldExpiry, totalSeconds: number, game: { party?: unknown; goals?: unknown }): HoldContext {
  const party = Array.isArray(game.party) ? (game.party as PartyMember[]) : [];
  const goals = Array.isArray(game.goals) ? (game.goals as Goal[]) : [];
  return {
    reason: expiry.reason,
    minutes_waited: Math.round(totalSeconds / 6) / 10,
    party: {
      // In sight (within 100 yards) and online: actually here.
      here: party.filter((p) => p.online !== false && p.yards !== undefined).map((p) => p.name),
      online_elsewhere: party.filter((p) => p.online !== false && p.yards === undefined).map((p) => p.name),
      offline: party.filter((p) => p.online === false).map((p) => p.name),
    },
    goals: goals.map((g) => ({ name: g.name, priority: g.priority, status: g.status, ...(g.why ? { why: g.why } : {}) })),
  };
}

export function holdQuestions(): Record<string, Question> {
  const lengths: Record<string, string> = {};
  for (const [name, l] of Object.entries(HOLD_LENGTHS)) lengths[name] = l.when;
  return {
    hold: choice(
      "The agent paused its goals to wait, for the reason in state.hold_expired.reason, and that wait has run out. It has waited state.hold_expired.minutes_waited minutes in all. Who it might wait for is in state.hold_expired.party: here (online and in sight), online_elsewhere, offline. Should it keep waiting, or stop and get on with its goals (state.hold_expired.goals)? An agent must never wait indefinitely.",
      HOLD_CHOICES,
    ),
    hold_for: choice("If it keeps waiting, for how much longer?", lengths),
  };
}

/** A hold decision, kept for fast_loop_status and the reflection digest. */
export interface HoldDecision {
  at: number;
  reason: string;
  minutes_waited: number;
  here: string[];
  choice: keyof typeof HOLD_CHOICES;
  seconds?: number;
  confidence: number;
  result?: string;
}

/** Where the time goes, from the state read at each wake (ms). */
export interface TimeShares {
  busy: number;
  idle: number;
  hold: number;
  /** Dead (or a ghost): neither busy nor idle. */
  other: number;
}

/** A compact state for Jev, read through the client's Lua in one call. */
export const STATE_LUA = `local V = require("wp.values")
local near, seen = {}, {}
for _, o in ipairs(AgentNearby(100)) do if o.name then seen[o.name] = o end end
local quests = {}
for _, q in ipairs(Quests and Quests.list and Quests.list() or {}) do
  local goals = {}
  for _, o in ipairs(q.objectives) do goals[#goals + 1] = o.text end
  quests[#quests + 1] = { title = q.title, level = q.level, complete = q.complete or nil, objectives = goals }
end
local party = {}
for i = 1, (GetNumPartyMembers() or 0) do
  local unit = "party" .. i
  local name = UnitName(unit)
  if name then
    local o = seen[name]
    party[#party + 1] = { name = name, online = not UnitIsConnected or UnitIsConnected(unit) and true or false,
      yards = o and math.floor(o.distance) or nil, health = V.health(unit),
      combat = UnitAffectingCombat(unit) and true or nil }
  end
end
for i, o in ipairs(AgentNearby(60)) do
  if #near >= 15 then break end
  if o.kind ~= "gameobject" or o.name then
    near[#near + 1] = { guid = o.guid, name = o.name, kind = o.kind, yards = o.distance,
      reaction = o.reaction, dead = o.dead or nil, quest = o.quest, npc = o.npc,
      lootable = o.lootable or nil, level = o.level,
      health = o.health_max and o.health_max > 0 and math.floor(100 * o.health / o.health_max) or nil }
  end
end
local goals, hold
if Goals then
  goals = {}
  for _, g in ipairs(Goals.list()) do
    goals[#goals + 1] = { name = g.name, why = g.why, priority = g.priority, status = g.status,
      plan = g.plan, note = g.note }
  end
  local reason, waited, remaining
  if Goals.holding then reason, waited, remaining = Goals.holding() end
  if reason then hold = { reason = reason, waited = waited, remaining = remaining } end
end
return { level = UnitLevel("player"), health = V.health("player"), power = V.power("player"),
  combat = UnitAffectingCombat("player") and true or false,
  dead = UnitIsDeadOrGhost("player") and true or false,
  casting = UnitCastingInfo("player") and true or false,
  target = UnitName("target"), target_health = V.health("target"),
  zone = GetRealZoneText(), subzone = GetSubZoneText(), position = AgentPosition(),
  quests = quests, party = party, free_bag_slots = V.free_bag_slots(),
  gossip_open = GossipFrame and GossipFrame:IsShown() and true or false,
  quest_window_open = QuestFrame and QuestFrame:IsShown() and true or false,
  loot_open = LootFrame and LootFrame:IsShown() and true or false,
  activity = Agent and Agent.activity() or nil, walk = AgentWalk and AgentWalk().status or nil,
  registered = Agent and Agent.available() or {}, decisions = Agent and Agent.questions() or {},
  goals = goals, hold = hold, nearby = near }`;

/** The Lua that registers the planner's intents (and removes ones it no longer lists). */
export function registerLua(intents: Intent[], steps: Step[] = []): string {
  // Planner intents are tagged, so a new plan (or a new harness on the same client) replaces them
  // all, while intents the agent's own modules registered stay.
  const lines = ["for name, spec in pairs(Agent.intents) do if spec.plan then Agent.remove(name) end end"];
  for (const i of intents) {
    const done = i.until ? `, done = assert(loadstring(${luaString(i.until)}))` : "";
    const timeout = i.timeout ? `, timeout = ${Number(i.timeout)}` : "";
    lines.push(
      `Agent.intent(${JSON.stringify(i.name)}, { when = ${luaString(i.when)}, run = assert(loadstring(${luaString(i.lua)}))${done}${timeout}, plan = true })`,
    );
  }
  // The steps run as one activity, started at once unless something else is running.
  const list = steps.map((s) => {
    const done = s.until ? `, done = assert(loadstring(${luaString(s.until)}))` : "";
    const timeout = s.timeout ? `, timeout = ${Number(s.timeout)}` : "";
    return `{ name = ${luaString(s.name)}, run = assert(loadstring(${luaString(s.lua)}))${done}${timeout} }`;
  });
  lines.push(`Agent.set_steps({ ${list.join(", ")} })`);
  if (steps.length) lines.push(`if not Agent.current then Agent.run("follow_steps") end`);
  return lines.join("\n") + "\nreturn true";
}

export interface PolicyOptions {
  url: string;
  token: string;
  jev: Jev;
  /** With nothing happening, check anyway this often (ms). */
  heartbeatMs?: number;
  /** Idle (no activity, no hold) for this long, `wait` is no longer offered (ms, default 60 s). */
  idleGraceMs?: number;
  /** Idle for this long, the planner is told (ms, default 3 min). */
  idleNotifyMs?: number;
  /** Still idle, it is told again this often, more plainly (ms, default 10 min). */
  idleRepeatMs?: number;
  /** A hold decision: the wait was ended (stop_waiting) or kept going for `seconds`. */
  onHold?: (decision: HoldDecision) => void;
  /** A test clock. */
  now?: () => number;
  /** The run's goal, which Jev reads beside the planner's plan. */
  goal?: string;
  /** Told when the plan stalls, looks complete, or an intent keeps failing. */
  onNotify: (summary: string) => void;
  log?: (line: string) => void;
}

export class Policy {
  plan = "";
  intents: Intent[] = [];
  decisions: Decision[] = [];
  events: string[] = [];
  calls = 0;
  #reasons = new Set<string>();
  #interrupt = false;
  #wake: (() => void) | undefined;
  #stuckStreak = 0;
  #doneStreak = 0;
  #failures = new Map<string, number>();
  #told = new Set<string>();
  holds: HoldDecision[] = [];
  #expired: HoldExpiry[] = [];
  /** Seconds waited so far on a hold kept going, by reason. */
  #holdTotals = new Map<string, number>();
  #blockedTold = new Map<string, number>();
  #idleSince: number | undefined;
  #idleToldAt: number | undefined;
  #idleTellings = 0;
  #last: { at: number; kind: keyof TimeShares } | undefined;
  time: TimeShares = { busy: 0, idle: 0, hold: 0, other: 0 };

  constructor(readonly options: PolicyOptions) {}

  /** The planner's plan and intents: registered in the client's Lua as activities. */
  async update(plan: string, intents: Intent[], steps: Step[] = []): Promise<string> {
    const result = await this.#lua(registerLua(intents, steps));
    if (!result.ok) return `Could not register the intents: ${result.error}`;
    this.plan = plan;
    this.intents = intents;
    this.#stuckStreak = 0;
    this.#doneStreak = 0;
    this.#told.clear();
    this.wake("new plan", true);
    return `Plan set with ${intents.length} intents (+ wait)${steps.length ? ` and ${steps.length} steps, started` : ""}.`;
  }

  /** Something happened that may need a choice. */
  wake(reason: string, interrupt = false): void {
    this.#reasons.add(reason);
    this.#interrupt ||= interrupt;
    this.#wake?.();
  }

  /** Raw game events from the watcher: notes for Jev's state, and the ones that wake it. */
  notice(events: GameEvent[]): void {
    for (const e of events) {
      const wake = WAKE[e.event];
      if (e.event === "AGENT_INTENT_DONE") this.#activityEnded(String(e.args[0]), e.args[1] === true, e.args[2]);
      if (e.event === "AGENT_HOLD_EXPIRED") this.#expired.push({ reason: String(e.args[0] ?? "?"), waited: Number(e.args[1] ?? 0) || 0 });
      if (e.event === "AGENT_GOAL_BLOCKED") this.#goalBlocked(String(e.args[0] ?? "?"), e.args[1]);
      if (wake) {
        this.wake(wake.reason, wake.interrupt);
        this.note(`${wake.reason}${e.args.length ? ` (${e.args.filter((a) => a !== null).join(", ")})` : ""}`);
      }
    }
  }

  note(line: string): void {
    this.events.push(line);
    if (this.events.length > 10) this.events.shift();
  }

  #activityEnded(name: string, ok: boolean, why: unknown): void {
    // The planner's steps ending is its business: all done, or one that failed (not a fight
    // interrupting them, which Jev resumes).
    if (name === "follow_steps" && !String(why ?? "").startsWith("replaced")) {
      this.options.onNotify(ok ? "Your steps are all done." : `Your steps stopped: ${String(why)}. Fix that step or set a new plan.`);
      return;
    }
    // Replaced by another activity (a fight, a request) is not a failure.
    if (ok || String(why ?? "").startsWith("replaced")) {
      this.#failures.delete(name);
      return;
    }
    const count = (this.#failures.get(name) ?? 0) + 1;
    this.#failures.set(name, count);
    if (count === 3) {
      this.options.onNotify(
        name.startsWith("goal:")
          ? `The goal action "${name.slice(5)}" has failed 3 times in a row (last: ${String(why)}). Fix the action (Goals.action: its pre, run or done), teach a better one, or change the goal.`
          : `The intent "${name}" has failed 3 times in a row (last: ${String(why)}). Fix its Lua or change the plan.`,
      );
    }
  }

  #goalBlocked(name: string, why: unknown): void {
    const now = this.#now();
    if (now - (this.#blockedTold.get(name) ?? -Infinity) < 600_000) return;
    this.#blockedTold.set(name, now);
    this.options.onNotify(
      `Your goal "${name}" is blocked${why ? `: ${String(why)}` : ""}. The runner has moved on to the next goal. Teach an action that gets there (Goals.action, or a Goals.fact it needs), or drop or change the goal.`,
    );
  }

  #now(): number {
    return this.options.now?.() ?? Date.now();
  }

  /** Charge the time since the last state read to what the character was doing then. */
  #account(kind: keyof TimeShares, now: number): void {
    if (this.#last) this.time[this.#last.kind] += Math.max(0, now - this.#last.at);
    this.#last = { at: now, kind };
  }

  /** How long the character has been idle (no activity, no hold), in ms; 0 when it is not. */
  get idleMs(): number {
    return this.#idleSince === undefined ? 0 : this.#now() - this.#idleSince;
  }

  /** `wait`'s description now, or null when it is not offered. */
  #wait(game: { activity?: unknown; hold?: unknown }): string | null {
    if (game.activity) return WAIT_WHEN.busy;
    if (game.hold) return WAIT_WHEN.hold;
    return this.idleMs < (this.options.idleGraceMs ?? 60_000) ? WAIT_WHEN.idle : null;
  }

  questions(registered: Registered[], decisions: OpenDecision[], choose: boolean, wait: string | null = WAIT.when, hold = false) {
    const questions: Record<string, Question> = {};
    if (choose) {
      const criteria: Record<string, string> = {};
      for (const intent of registered) criteria[intent.name] = intent.when;
      if (wait !== null) criteria[WAIT.name] = wait;
      questions.next = choice(
        "A game-playing agent is working toward its goals (state.game.goals, highest first) and following the plan in state.plan. Given the game state and what just happened (state.woke_because), which intent should it start now?",
        criteria,
      );
      questions.stuck = noul(
        "Has the plan stalled: is the character failing to make progress on it, or does the situation no longer fit the plan?",
      );
      questions.done = noul("Has the plan's goal been fully achieved?");
    }
    for (const d of decisions) {
      questions[`decision_${d.id}`] = choice(
        `${d.question} (A decision the agent's own logic is waiting on; its context is in state.game.decisions, id ${d.id}.)`,
        d.options,
      );
    }
    if (hold) Object.assign(questions, holdQuestions());
    return questions;
  }

  /** One wake: read the state, ask Jev what is needed, carry it out. */
  async tick(reasons: string[], interrupt: boolean): Promise<Decision | null> {
    const state = await this.#lua(STATE_LUA);
    if (!state.ok) return null;
    const game = (state.values[0] ?? {}) as {
      registered?: Registered[];
      decisions?: OpenDecision[];
      activity?: { name: string; seconds: number } | null;
      hold?: { reason: string; waited: number; remaining: number } | null;
      goals?: Goal[];
      party?: unknown;
      dead?: boolean;
      combat?: boolean;
    };
    const registered = Array.isArray(game.registered) ? game.registered : [];
    const decisions = Array.isArray(game.decisions) ? game.decisions : [];
    const now = this.#now();
    // Where the time goes, and how long the character has stood idle.
    const kind: keyof TimeShares = game.dead ? "other" : game.activity || game.combat ? "busy" : game.hold ? "hold" : "idle";
    this.#account(kind, now);
    if (kind === "idle") this.#idleSince ??= now;
    else {
      this.#idleSince = undefined;
      this.#idleToldAt = undefined;
      this.#idleTellings = 0;
    }
    // Told once the stretch is long enough, then again while it lasts: an idle character is never
    // something the planner may leave be.
    const idleDue =
      this.#idleToldAt === undefined
        ? this.idleMs >= (this.options.idleNotifyMs ?? 180_000)
        : now - this.#idleToldAt >= (this.options.idleRepeatMs ?? 600_000);
    if (kind === "idle" && idleDue) {
      this.#idleToldAt = now;
      this.options.onNotify(idleNotice(Math.round(this.idleMs / 1000), Array.isArray(game.goals) ? game.goals : [], this.#idleTellings++));
    }
    // A hold that ran out: Jev decides whether to keep waiting (the latest one; older ones are moot).
    const expiry = this.#expired.at(-1);
    this.#expired = [];
    // While an activity runs, only an interrupting event makes Jev reconsider it.
    const choose = registered.length > 0 && (!game.activity || interrupt);
    if (!choose && !decisions.length && !expiry) return null;
    const total = expiry ? (this.#holdTotals.get(expiry.reason) ?? 0) + expiry.waited : 0;
    const context = expiry ? holdContext(expiry, total, game) : undefined;
    const response = await this.options.jev.ask(
      {
        goal: this.options.goal,
        plan: this.plan || "No plan from the planner: work toward the top goal in state.game.goals, and never stand idle.",
        woke_because: reasons,
        game: game as never,
        recent_events: this.events,
        recent_decisions: this.decisions.slice(-5).map((d) => d.choice),
        ...(context ? { hold_expired: context } : {}),
      },
      this.questions(registered, decisions, choose, this.#wait(game), expiry !== undefined),
    );
    this.calls++;
    for (const d of decisions) {
      const answer = response.answers[`decision_${d.id}`];
      if (answer?.type !== "choice") continue;
      const result = await this.#lua(`return Agent.resolve(${d.id}, ${JSON.stringify(answer.choice)})`);
      this.options.log?.(
        `[jev] decision ${d.name}: ${answer.choice} (${answer.confidence.toFixed(2)})${result.ok ? "" : ` → error: ${result.error}`}`,
      );
    }
    if (expiry && context) await this.#decideHold(expiry, total, context, response.answers.hold, response.answers.hold_for);
    if (!choose) return null;
    const next = chosen(response.answers.next);
    const decision: Decision = {
      at: now,
      reasons,
      choice: next.choice,
      confidence: next.confidence,
      ran: false,
      stuck: yes(response.answers.stuck),
    };
    // Choosing what is already running, or wait, leaves it running.
    if (next.choice !== WAIT.name && next.choice !== game.activity?.name) {
      const result = await this.#lua(`return Agent.run(${JSON.stringify(next.choice)})`);
      decision.ran = true;
      decision.result = result.ok ? JSON.stringify(result.values ?? []) : `error: ${result.error}`;
    }
    this.decisions.push(decision);
    if (this.decisions.length > 50) this.decisions.shift();
    this.options.log?.(
      `[jev] ${reasons.join(", ")} → ${next.choice} (${next.confidence.toFixed(2)})${decision.ran ? ` started${decision.result?.startsWith("error") ? `: ${decision.result}` : ""}` : ""}`,
    );
    this.#stuckStreak = decision.stuck >= 0.8 ? this.#stuckStreak + 1 : 0;
    this.#doneStreak = yes(response.answers.done) >= 0.9 ? this.#doneStreak + 1 : 0;
    if (this.#stuckStreak === 3) this.#tell("stalled", `The fast loop judges the plan has stalled (3 choices in a row). Recent: ${this.#recent()}. Review the game and update the plan or your Lua.`);
    if (this.#doneStreak === 2) this.#tell("done", `The fast loop judges the plan's goal achieved. Recent: ${this.#recent()}. Confirm it and set the next plan, or say you are done.`);
    return decision;
  }

  /** Keep waiting (a new bounded hold) or stop waiting (the runner carries on), as Jev chose. */
  async #decideHold(expiry: HoldExpiry, total: number, context: HoldContext, answer: Answer | undefined, length: Answer | undefined): Promise<void> {
    // No usable answer: stop waiting. The agent must never be left waiting on a silence.
    const keep = answer?.type === "choice" && answer.choice === "keep_waiting";
    const decision: HoldDecision = {
      at: this.#now(),
      reason: expiry.reason,
      minutes_waited: context.minutes_waited,
      here: context.party.here,
      choice: keep ? "keep_waiting" : "stop_waiting",
      confidence: answer?.type === "choice" ? answer.confidence : 0,
    };
    if (keep) {
      const name = length?.type === "choice" && HOLD_LENGTHS[length.choice] ? length.choice : "minute";
      decision.seconds = HOLD_LENGTHS[name]?.seconds ?? 60;
      const result = await this.#lua(holdLua(expiry.reason, decision.seconds));
      decision.result = result.ok ? "held" : `error: ${result.error}`;
      this.#holdTotals.set(expiry.reason, total);
    } else {
      this.#holdTotals.delete(expiry.reason);
      decision.result = "released";
    }
    this.holds.push(decision);
    this.options.onHold?.(decision);
    if (this.holds.length > 50) this.holds.shift();
    this.note(`hold "${expiry.reason}" ran out after ${context.minutes_waited} min: ${decision.choice}${decision.seconds ? ` ${decision.seconds}s` : ""}`);
    this.options.log?.(
      `[jev] hold "${expiry.reason}" (${context.minutes_waited} min, here: ${context.party.here.join(", ") || "nobody"}) → ${decision.choice}${decision.seconds ? ` for ${decision.seconds}s` : ""} (${decision.confidence.toFixed(2)})${decision.result?.startsWith("error") ? `: ${decision.result}` : ""}`,
    );
  }

  #recent(): string {
    return this.decisions.slice(-5).map((d) => d.choice).join(", ");
  }

  #tell(kind: string, text: string): void {
    if (this.#told.has(kind)) return;
    this.#told.add(kind);
    this.options.onNotify(text);
  }

  /** Wake on events (batched over 250 ms), or on the heartbeat; until aborted. */
  async run(signal: AbortSignal): Promise<void> {
    // A previous harness's plan intents may still be registered in the client: start clean.
    await this.#lua("for name, spec in pairs(Agent.intents) do if spec.plan then Agent.remove(name) end end return true").catch(() => undefined);
    const heartbeat = this.options.heartbeatMs ?? 20_000;
    while (!signal.aborted) {
      if (!this.#reasons.size) {
        await new Promise<void>((resolve) => {
          const timer = setTimeout(() => {
            this.#reasons.add("heartbeat");
            resolve();
          }, heartbeat);
          this.#wake = () => {
            clearTimeout(timer);
            resolve();
          };
        });
        this.#wake = undefined;
        await Bun.sleep(250);
      }
      const reasons = [...this.#reasons];
      const interrupt = this.#interrupt;
      this.#reasons.clear();
      this.#interrupt = false;
      try {
        await this.tick(reasons, interrupt);
      } catch (error) {
        this.options.log?.(`[jev] error: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
  }

  /** A summary for the planner. */
  describe(): string {
    return JSON.stringify(
      {
        plan: this.plan,
        intents: this.intents.map((i) => i.name),
        jev_calls: this.calls,
        recent: this.decisions.slice(-10).map((d) => ({
          woke_because: d.reasons,
          choice: d.choice,
          confidence: Number(d.confidence.toFixed(2)),
          started: d.ran,
          result: d.result,
          stuck: Number(d.stuck.toFixed(2)),
        })),
        holds: this.holds.slice(-5).map((h) => ({ reason: h.reason, minutes_waited: h.minutes_waited, here: h.here, choice: h.choice, seconds: h.seconds })),
        idle_seconds: Math.round(this.idleMs / 1000),
        time_share: shares(this.time),
      },
      null,
      2,
    );
  }

  async #lua(code: string): Promise<{ ok: boolean; values: unknown[]; error?: string }> {
    const response = await fetch(`${this.options.url}/lua`, {
      method: "POST",
      headers: { Authorization: `Bearer ${this.options.token}` },
      body: code,
    });
    if (!response.ok) return { ok: false, values: [], error: `HTTP ${response.status}` };
    return (await response.json()) as { ok: boolean; values: unknown[]; error?: string };
  }
}

/** What the planner is told about an idle character: the goals with the runner's next actions or
 *  why each is blocked and, from the second telling on, that nobody else will move it. */
export function idleNotice(seconds: number, goals: Goal[], tellings = 0): string {
  const lines = goalLines(goals).join("\n") || "- none (is wp.goals loaded?)";
  const first = `The character has stood idle for ${seconds}s: no activity, no hold, and nothing started.`;
  if (tellings === 0) {
    return `${first}\nGoals:\n${lines}\nFind out why the runner has nothing to do (the goals tool shows each goal's plan and the facts), then push a goal it can act on or teach the missing action.`;
  }
  return `${first} You were told ${tellings === 1 ? "once" : `${tellings} times`} already and it is still standing there. ${NOBODY}\nGoals, with what the runner would do next or why it cannot:\n${lines}\nAct in this turn: unblock the top goal (teach the action or the fact it is missing), push a goal the runner can act on (a zone that fits your level: Goals.zones()), or start an activity yourself. Do not end the turn with the character idle.`;
}

/** Time shares as whole percentages, or null before anything has been measured. */
export function shares(time: TimeShares): Record<keyof TimeShares, number> | null {
  const total = time.busy + time.idle + time.hold + time.other;
  if (total <= 0) return null;
  const pct = (ms: number) => Math.round((100 * ms) / total);
  return { busy: pct(time.busy), idle: pct(time.idle), hold: pct(time.hold), other: pct(time.other) };
}
