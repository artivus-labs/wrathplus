// How the planner sleeps: it ends its turn (a reply with no tool call) and is woken by news, or
// by a wake-up it asked for (wake_me: after a while, or when given events fire). Every wake comes
// with a one-line status of the character, so the planner need not spend calls re-checking.

import { createSdkMcpServer, tool, type HookCallback } from "@anthropic-ai/claude-agent-sdk";
import { z } from "zod";
import type { GameEvent } from "./watch.ts";

/** One line on the character: health, place, what the Lua is doing, quest progress. */
export const STATUS_LUA = `
local out = {}
local hp, max = UnitHealth("player"), UnitHealthMax("player")
out[#out + 1] = string.format("level %d, health %d/%d%s%s", UnitLevel("player") or 0, hp or 0, max or 0,
  UnitIsGhost("player") and ", a ghost" or (UnitIsDeadOrGhost("player") and ", dead" or ""), UnitAffectingCombat("player") and ", in combat" or "")
local p = AgentPosition and AgentPosition()
local zone, sub = GetZoneText() or "", GetSubZoneText() or ""
out[#out + 1] = string.format("in %s%s%s", zone, sub ~= "" and (" / " .. sub) or "",
  p and string.format(" at (%.0f, %.0f, %.0f)", p.x, p.y, p.z) or "")
if Revive and Revive.state then out[#out + 1] = "revive: " .. Revive.state end
if Move and Move.mode then out[#out + 1] = "move: " .. (Move.mode == "follow" and "following" or Move.mode) .. (Move.state and (", " .. Move.state) or "") end
if Combat and Combat.state then out[#out + 1] = "combat: " .. Combat.state end
if Agent and Agent.steps and Agent.steps.at > 0 then
  local s = Agent.steps.list[Agent.steps.at]
  out[#out + 1] = string.format("steps: %d of %d, %s", Agent.steps.at, #Agent.steps.list, s and s.name or "?")
end
if Upkeep and #Upkeep.log > 0 then out[#out + 1] = "upkeep lately: " .. table.concat(Upkeep.log, "; ") end
local asks = {}
for i = 1, 4 do
  local f = _G["StaticPopup" .. i]
  if f and f:IsShown() and f.which then asks[#asks + 1] = f.which end
end
if #asks > 0 then out[#out + 1] = "waiting on you: " .. table.concat(asks, ", ") end
local a = Agent and Agent.activity and Agent.activity()
out[#out + 1] = a and string.format("activity %s for %ds", a.name, a.seconds) or "no activity"
local w = AgentWalk and AgentWalk()
if w and w.status ~= "idle" then out[#out + 1] = "walk " .. w.status end
local quests = {}
for i = 1, (GetNumQuestLogEntries() or 0) do
  local title, _, _, _, header, _, complete = GetQuestLogTitle(i)
  if title and not header then
    local goals = {}
    for j = 1, (GetNumQuestLeaderBoards(i) or 0) do
      local text = GetQuestLogLeaderBoard(j, i)
      if text then goals[#goals + 1] = text end
    end
    local state = complete == 1 and "complete" or (complete == -1 and "failed" or table.concat(goals, ", "))
    quests[#quests + 1] = title .. (state ~= "" and (" (" .. state .. ")") or "")
  end
end
out[#out + 1] = #quests > 0 and ("quests: " .. table.concat(quests, "; ")) or "no quests"
return table.concat(out, "; ")
`;

/** The status line from the client, or null when it does not answer (loading, logged out). */
export async function statusLine(url: string, token: string): Promise<string | null> {
  try {
    const response = await fetch(`${url}/lua`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
      body: STATUS_LUA,
      signal: AbortSignal.timeout(2000),
    });
    const answer = (await response.json()) as { ok?: boolean; values?: unknown[] };
    return answer.ok && typeof answer.values?.[0] === "string" ? answer.values[0] : null;
  } catch {
    return null;
  }
}

/** One pending wake-up: after some seconds, or on the first of some events, whichever is first. */
export class Waker {
  #timer: ReturnType<typeof setTimeout> | undefined;
  #events = new Set<string>();
  #reason = "";
  /** Asked to be woken this turn: the game tools are refused until the turn ends. */
  #asleep = false;

  constructor(private readonly onWake: (text: string) => void) {}

  get asleep(): boolean {
    return this.#asleep;
  }

  /** The planner's turn ended. */
  turnEnded(): void {
    this.#asleep = false;
  }

  get pending(): boolean {
    return this.#timer !== undefined || this.#events.size > 0;
  }

  /** Replace the pending wake-up. */
  set(seconds: number | undefined, events: string[], reason: string): string {
    this.cancel();
    this.#reason = reason;
    this.#events = new Set(events);
    this.#asleep = true;
    if (seconds) this.#timer = setTimeout(() => this.#fire(`${seconds}s passed`), seconds * 1000);
    const when = [seconds ? `in ${seconds}s` : "", events.length ? `on ${events.join(", ")}` : ""].filter(Boolean);
    return `I will wake you ${when.join(" or ")}. End your turn now.`;
  }

  cancel(): void {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    this.#events.clear();
  }

  /** Every batch of game events: fire on the first one asked for. */
  notice(events: GameEvent[]): void {
    const hit = events.find((e) => this.#events.has(e.event));
    if (hit) this.#fire(`${hit.event}${hit.args.length ? ` ${JSON.stringify(hit.args).slice(0, 200)}` : ""}`);
  }

  #fire(why: string): void {
    const reason = this.#reason;
    this.cancel();
    this.onWake(`Wake-up you asked for (${reason}): ${why}.`);
  }
}

export const WAKE_TOOLS = ["mcp__wake__wake_me"];

/** A PreToolUse hook: once the planner has asked to be woken, it may still write its notes, but
 *  the game tools are refused until it ends the turn (a reply with no tool call). */
export function sleepGuard(waker: Waker): HookCallback {
  return async (input) => {
    if (!waker.asleep || input.hook_event_name !== "PreToolUse" || !input.tool_name.startsWith("mcp__wrathplus__")) {
      return {};
    }
    return {
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason:
          "You asked to be woken, so this turn is over: reply without calling a tool. The wake-up (or any news) starts your next turn.",
      },
    };
  };
}

/** The planner's alarm clock. */
export function wakeServer(waker: Waker) {
  return createSdkMcpServer({
    name: "wake",
    version: "0.1.0",
    tools: [
      tool(
        "wake_me",
        "Ask to be woken later; your turn ends with it (the game tools are refused until you reply without a tool call). Wakes you after `seconds`, or when one of `events` fires (game events such as AGENT_WALK_DONE, QUEST_LOG_UPDATE, PLAYER_REGEN_ENABLED), whichever is first. Replaces any earlier wake-up. You are woken anyway by news (the fast loop, deaths, whispers, stuck walks); use this only when you are driving something yourself and want to look again.",
        {
          seconds: z.number().int().min(1).optional(),
          events: z.array(z.string().regex(/^[A-Z][A-Z0-9_]*$/)).optional(),
          reason: z.string().describe("What to check when you wake."),
        },
        async ({ seconds, events, reason }) => ({
          content: [
            {
              type: "text" as const,
              text:
                !seconds && !events?.length
                  ? "Give seconds, events or both."
                  : waker.set(seconds, events ?? [], reason),
            },
          ],
        }),
      ),
    ],
  });
}
