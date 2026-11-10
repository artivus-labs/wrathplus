// The goal runner belongs to the harness. While the example agent plays, `Goals.on` stays true:
// a planner that switches it off (to "wait" with nothing running) finds it on again at the next
// check, and is told once why. It is also what turns the runner back on after the client was
// restarted (a fresh client starts with it off). WRATHPLUS_AGENT_GOALS=0 never starts this.
//
// Kept on from here rather than locked in the client's Lua: wp.goals stays a plain module for a
// manual MCP session (off until someone turns it on), and one check covers both a planner's
// `Goals.on = false` and a restarted client.

import type { Lua } from "./goals.ts";

export const KEEP_ON_LUA = `if not Goals then return "unavailable" end
if Goals.on then return "on" end
Goals.on = true
return "reasserted"`;

export const RUNNER_OFF_LUA = "if Goals then Goals.on = false end return true";

export const RUNNER_NOTICE =
  "The goal runner was off (Goals.on = false: set by your Lua, or lost when the client restarted) and is on again. While you play, the runner is always on: the harness turns it back on. Shape what it does with goals (push_goal, drop_goal), actions (Goals.action) and holds (hold: a bounded pause for something in the game). Switching it off only leaves your character standing there.";

export type RunnerState = "on" | "reasserted" | "unavailable";

export class RunnerLock {
  #started = false;
  #told = false;
  reasserted = 0;

  constructor(
    private readonly lua: Lua,
    /** Tell the planner (once per run). */
    private readonly tell: (text: string) => void,
    private readonly log: (line: string) => void = () => {},
  ) {}

  /** Turn the runner on if it is off. The first time is the harness starting it. */
  async check(): Promise<RunnerState> {
    let state: RunnerState = "unavailable";
    try {
      const result = await this.lua(KEEP_ON_LUA);
      const value = result.ok ? result.values?.[0] : undefined;
      if (value === "on" || value === "reasserted") state = value;
    } catch {
      // The client is loading or restarting: the next check finds it.
    }
    if (!this.#started) {
      if (state !== "unavailable") this.#started = true;
      return state;
    }
    if (state === "reasserted") {
      this.reasserted++;
      this.log("Goal runner: was off, turned back on");
      if (!this.#told) {
        this.#told = true;
        this.tell(RUNNER_NOTICE);
      }
    }
    return state;
  }

  /** Check every `everyMs` until aborted, then turn the runner off (the harness is leaving). */
  async run(signal: AbortSignal, everyMs = 15_000): Promise<void> {
    const state = await this.check();
    this.log(state === "unavailable" ? "Goal runner: unavailable (this client has no wp.goals, or it is still loading)" : "Goal runner: on");
    const timer = setInterval(() => void this.check(), everyMs);
    signal.addEventListener("abort", () => {
      clearInterval(timer);
      void this.lua(RUNNER_OFF_LUA).catch(() => undefined);
    });
  }
}
