// The fast tier: Jev (TypeSafe's System One model) chooses which intent the agent acts on next,
// and answers the decisions its realtime Lua hands up. It is event-driven: it sleeps while an
// activity runs and wakes when something happens that needs a choice (the activity ended, the
// planner sent a new plan, a decision opened, combat started, the character died...). A slow
// heartbeat is the safety net. The realtime tier (the agent's own Lua) does the work between
// choices; the slow tier (the planner) sets the plan and hears when it stalls, looks complete, or
// an intent keeps failing.

import { choice, chosen, Jev, noul, yes, type Question } from "./jev.ts";
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
  PLAYER_ALIVE: { reason: "alive again", interrupt: true },
  PLAYER_UNGHOST: { reason: "alive again", interrupt: true },
  LOOT_OPENED: { reason: "loot window", interrupt: false },
  GOSSIP_SHOW: { reason: "gossip window", interrupt: false },
  QUEST_DETAIL: { reason: "quest window", interrupt: false },
  QUEST_PROGRESS: { reason: "quest window", interrupt: false },
  QUEST_COMPLETE: { reason: "quest turn-in window", interrupt: false },
  PLAYER_LEVEL_UP: { reason: "level up", interrupt: false },
  PARTY_INVITE_REQUEST: { reason: "party invite", interrupt: false },
};

/** A compact state for Jev, read through the client's Lua in one call. */
export const STATE_LUA = `local V = require("wp.values")
local near = {}
for i, o in ipairs(AgentNearby(60)) do
  if #near >= 15 then break end
  if o.kind ~= "gameobject" or o.name then
    near[#near + 1] = { guid = o.guid, name = o.name, kind = o.kind, yards = o.distance,
      reaction = o.reaction, dead = o.dead or nil, quest = o.quest, npc = o.npc,
      lootable = o.lootable or nil, level = o.level,
      health = o.health_max and o.health_max > 0 and math.floor(100 * o.health / o.health_max) or nil }
  end
end
return { level = UnitLevel("player"), health = V.health("player"), power = V.power("player"),
  combat = UnitAffectingCombat("player") and true or false,
  dead = UnitIsDeadOrGhost("player") and true or false,
  casting = UnitCastingInfo("player") and true or false,
  target = UnitName("target"), target_health = V.health("target"),
  zone = GetRealZoneText(), subzone = GetSubZoneText(), position = AgentPosition(),
  quest_log = GetNumQuestLogEntries(), free_bag_slots = V.free_bag_slots(),
  gossip_open = GossipFrame and GossipFrame:IsShown() and true or false,
  quest_window_open = QuestFrame and QuestFrame:IsShown() and true or false,
  loot_open = LootFrame and LootFrame:IsShown() and true or false,
  activity = Agent and Agent.activity() or nil, walk = AgentWalk and AgentWalk().status or nil,
  registered = Agent and Agent.available() or {}, decisions = Agent and Agent.questions() or {},
  nearby = near }`;

/** Lua source as a long-bracket string literal that cannot be closed by its own contents. */
export function luaString(text: string): string {
  let level = 0;
  while (text.includes(`]${"=".repeat(level)}]`)) level++;
  const eq = "=".repeat(level);
  // A newline right after the opening bracket is dropped by Lua, so one is added on purpose.
  return `[${eq}[\n${text}]${eq}]`;
}

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
    if (ok) {
      this.#failures.delete(name);
      return;
    }
    const count = (this.#failures.get(name) ?? 0) + 1;
    this.#failures.set(name, count);
    if (count === 3) {
      this.options.onNotify(`The intent "${name}" has failed 3 times in a row (last: ${String(why)}). Fix its Lua or change the plan.`);
    }
  }

  questions(registered: Registered[], decisions: OpenDecision[], choose: boolean) {
    const questions: Record<string, Question> = {};
    if (choose) {
      const criteria: Record<string, string> = {};
      for (const intent of registered) criteria[intent.name] = intent.when;
      criteria[WAIT.name] = WAIT.when;
      questions.next = choice(
        "A game-playing agent is following the plan in state.plan. Given the game state and what just happened (state.woke_because), which intent should it start now?",
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
    };
    const registered = Array.isArray(game.registered) ? game.registered : [];
    const decisions = Array.isArray(game.decisions) ? game.decisions : [];
    // While an activity runs, only an interrupting event makes Jev reconsider it.
    const choose = registered.length > 0 && (!game.activity || interrupt);
    if (!choose && !decisions.length) return null;
    const response = await this.options.jev.ask(
      {
        plan: this.plan || "No plan from the planner yet: follow the agent's own registered intents.",
        woke_because: reasons,
        game: game as never,
        recent_events: this.events,
        recent_decisions: this.decisions.slice(-5).map((d) => d.choice),
      },
      this.questions(registered, decisions, choose),
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
    if (!choose) return null;
    const next = chosen(response.answers.next);
    const decision: Decision = {
      at: Date.now(),
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
