// The fast tier: Jev (TypeSafe's System One model) picks, every second or so, which of the
// planner's intents to act on now, given the plan and the current game state, and runs its Lua.
// The realtime tier (the agent's own wp.engine modules) keeps running underneath; the slow tier
// (the planner) sets the plan and the intents and is told when Jev thinks the plan has stalled.

import { choice, chosen, Jev, noul, yes } from "./jev.ts";

/** One thing the fast loop may do: a name, when it applies (Jev reads this), and its Lua. */
export interface Intent {
  name: string;
  when: string;
  lua: string;
  /** Seconds before the same intent may run again (default 3). */
  every?: number;
}

export interface Decision {
  at: number;
  choice: string;
  confidence: number;
  ran: boolean;
  result?: string;
  stuck: number;
}

/** The intent that is always there: let the character's own behaviour carry on. */
export const WAIT: Intent = {
  name: "wait",
  when: "Nothing needs a decision right now: the character's own behaviour (its Lua) is handling things, it is busy (casting, walking, fighting) or the plan says to wait.",
  lua: "",
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
  nearby = near }`;

export interface PolicyOptions {
  url: string;
  token: string;
  jev: Jev;
  intervalMs?: number;
  /** Told when Jev keeps judging that the plan has stalled. */
  onStalled: (summary: string) => void;
  log?: (line: string) => void;
}

export class Policy {
  plan = "";
  intents: Intent[] = [];
  decisions: Decision[] = [];
  events: string[] = [];
  #lastRun = new Map<string, number>();
  #stuckStreak = 0;
  #busy = false;

  constructor(readonly options: PolicyOptions) {}

  /** Set by the planner: the plan and the intents Jev chooses among (wait is always added). */
  update(plan: string, intents: Intent[]): void {
    this.plan = plan;
    this.intents = intents.filter((i) => i.name !== WAIT.name);
    this.#stuckStreak = 0;
  }

  /** Lines from the event watcher, kept for the state Jev sees. */
  note(line: string): void {
    this.events.push(line);
    if (this.events.length > 10) this.events.shift();
  }

  /** The question Jev answers each tick. */
  questions() {
    const criteria: Record<string, string> = {};
    for (const intent of [...this.intents, WAIT]) criteria[intent.name] = intent.when;
    return {
      next: choice(
        "A game-playing agent is following the plan in state.plan. Given the game state, which intent should it act on right now?",
        criteria,
      ),
      stuck: noul(
        "Has the plan stalled: is the character failing to make progress on it, or does the situation no longer fit the plan?",
      ),
    };
  }

  async tick(): Promise<Decision | null> {
    if (!this.plan || this.#busy) return null;
    this.#busy = true;
    try {
      const state = await this.#lua(STATE_LUA);
      if (!state.ok) return null;
      const response = await this.options.jev.ask(
        {
          plan: this.plan,
          // Plain JSON from the client's Lua.
          game: state.values[0] as never,
          recent_events: this.events,
          recent_decisions: this.decisions.slice(-5).map((d) => d.choice),
        },
        this.questions(),
      );
      const next = chosen(response.answers.next);
      const name = next.choice;
      const decision: Decision = {
        at: Date.now(),
        choice: name,
        confidence: next.confidence,
        ran: false,
        stuck: yes(response.answers.stuck),
      };
      const intent = this.intents.find((i) => i.name === name);
      const last = this.#lastRun.get(name) ?? 0;
      if (intent && Date.now() - last >= (intent.every ?? 3) * 1000) {
        const result = await this.#lua(intent.lua);
        decision.ran = true;
        decision.result = result.ok ? JSON.stringify(result.values ?? []) : `error: ${result.error}`;
        this.#lastRun.set(name, Date.now());
      }
      this.decisions.push(decision);
      if (this.decisions.length > 50) this.decisions.shift();
      this.options.log?.(
        `[jev] ${name} (${decision.confidence.toFixed(2)})${decision.ran ? ` → ${decision.result}` : ""}`,
      );
      this.#stuckStreak = decision.stuck >= 0.8 ? this.#stuckStreak + 1 : 0;
      if (this.#stuckStreak === 5) {
        this.options.onStalled(
          `The fast loop judges that the plan has stalled (5 checks in a row). Its recent choices: ${this.decisions
            .slice(-5)
            .map((d) => d.choice)
            .join(", ")}. Review the game state and update the plan or intents.`,
        );
      }
      return decision;
    } finally {
      this.#busy = false;
    }
  }

  /** Tick on a timer until aborted; errors are logged and the loop goes on. */
  async run(signal: AbortSignal): Promise<void> {
    const interval = this.options.intervalMs ?? 1500;
    while (!signal.aborted) {
      const started = Date.now();
      try {
        await this.tick();
      } catch (error) {
        this.options.log?.(`[jev] error: ${error instanceof Error ? error.message : String(error)}`);
      }
      await Bun.sleep(Math.max(0, interval - (Date.now() - started)));
    }
  }

  /** A summary for the planner. */
  describe(): string {
    return JSON.stringify(
      {
        plan: this.plan,
        intents: this.intents.map((i) => i.name),
        recent: this.decisions.slice(-10).map((d) => ({
          choice: d.choice,
          confidence: Number(d.confidence.toFixed(2)),
          ran: d.ran,
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
