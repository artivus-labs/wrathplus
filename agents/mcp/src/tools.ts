// The MCP tools an agent plays through: Lua, screenshots, events and status.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { AgentError, type AgentClient } from "./client.ts";

const LUA_DESCRIPTION = `Run a Lua chunk in the WrathPlus game client's UI Lua VM: the stock World of Warcraft 3.3.5a (Wrath of the Lich King, build 12340) addon API, as an addon would see it.
Use it to read game state (UnitName("player"), UnitHealth("target"), GetPlayerMapPosition("player"), GetQuestLogTitle(i), GetContainerItemInfo(bag, slot), ...) and to act (TargetUnit, CastSpellByName, UseContainerItem, SendChatMessage, InteractUnit, ...).
Movement functions work from here (MoveForwardStart/MoveForwardStop, TurnLeftStart/TurnLeftStop, StrafeLeftStart, JumpOrAscendStart, ToggleAutoRun, ...): start a motion, wait, then stop it in a later call.
A bare expression (e.g. \`UnitLevel("player")\`) returns its values; otherwise use \`return a, b\`. The answer is JSON: {ok, values, output} where output holds anything print()ed, or {ok:false, error, output} on a Lua error.
Each chunk runs to completion inside one game frame, so do not loop waiting for the world to change: return and call again (or use the events tool).
For logic that should run on its own every frame (rotations, following, looting), save it as a module with save_module and load it with require("name") (AgentReload("name") after changes); a module named autoexec runs whenever the client enters the world. Frames work as in addons: CreateFrame("Frame"):SetScript("OnUpdate", fn) ticks, :RegisterEvent(...) reacts.
Beyond the stock API: AgentNearby([yards]) lists every unit, player, game object and corpse the client knows (guid, name, kind, distance, x/y/z, level, health, dead, reaction, quest marker, npc services, lootable); AgentPosition(); AgentTarget(guid); AgentInteract(guid) (talk to, loot or use it; straight-line walk); AgentMoveTo(x, y, z | guid) walks a navmesh route (doors, stairs), with AgentPath, AgentWalk and AgentStop; Agent.intent(name, {when, run}) and Agent.decide(name, {question, options, on}) hand intents and decisions to a fast decision loop when one runs.
Built in: require("wp.engine") (a Playerbots-style decision engine: triggers, actions with relevance, prerequisites/alternatives/continuers, strategies per combat state) and require("wp.values") (health, power, group, auras, cooldowns, spell readiness, range, threat, bags, position).`;

const MODULES_DESCRIPTION = `List your saved Lua modules, or read one when \`name\` is given (dotted names like grind.loop). Modules persist across reloads and restarts of the client.`;

const SAVE_MODULE_DESCRIPTION = `Save a Lua module (source code) under a dotted name like grind.loop, for require("grind.loop") in the lua tool. Saving does not run it: call require (first load) or AgentReload("grind.loop") (after a change). Name it autoexec to run it whenever the client enters the world.`;

const SCREENSHOT_DESCRIPTION = `Capture the next rendered frame of the WrathPlus game client as a PNG image. scale (0-1, default 0.5) shrinks it; use 1 for full resolution when reading small text.`;

const EVENTS_DESCRIPTION = `Read the game UI events (the same events addons receive: PLAYER_TARGET_CHANGED, CHAT_MSG_SAY, UNIT_HEALTH, COMBAT_LOG_EVENT_UNFILTERED, ...) that fired after cursor \`since\`.
Returns JSON {events:[{seq, time, event, args}], next, missed}. Pass \`next\` as \`since\` on the following call to continue where you left off; start with since=0. \`missed\` counts events that dropped out of the retained window before you read them.
limit caps how many come back (default 1000).`;

const EVENTS_WAIT = "\nwait_ms (up to 30000) long-polls until at least one event arrives.";

export interface ToolOptions {
  /** Offer events' wait_ms long-poll (default true). A host that wakes its agent on events itself
   *  turns it off, so the agent ends its turn instead of sleeping inside one. */
  longPoll?: boolean;
}

const STATUS_DESCRIPTION = `Report the WrathPlus game client's state: {state, in_world, ui_ready, time, player}. state is the client phase (login, character select, in world, ...); in world, player summarises name, level, class, race, health, power, dead, combat, zone, subzone, map position and target.`;

function text(value: unknown, isError = false): CallToolResult {
  return {
    content: [{ type: "text", text: JSON.stringify(value, null, 2) }],
    ...(isError ? { isError: true } : {}),
  };
}

function failure(error: unknown): CallToolResult {
  const message =
    error instanceof AgentError || error instanceof Error ? error.message : String(error);
  return { content: [{ type: "text", text: message }], isError: true };
}

async function guarded(run: () => Promise<CallToolResult>): Promise<CallToolResult> {
  try {
    return await run();
  } catch (error) {
    return failure(error);
  }
}

/** Register the tools on `server`, each calling the control port through `client`. */
export function registerTools(server: McpServer, client: AgentClient, options: ToolOptions = {}): void {
  const longPoll = options.longPoll ?? true;
  server.registerTool(
    "lua",
    {
      title: "Run WoW Lua",
      description: LUA_DESCRIPTION,
      inputSchema: {
        code: z.string().min(1).describe("The Lua chunk or bare expression to run."),
        budget: z
          .number()
          .int()
          .positive()
          .optional()
          .describe("Instruction budget for this chunk; the default only stops runaway loops."),
      },
    },
    ({ code, budget }) =>
      guarded(async () => {
        const answer = await client.lua(code, budget);
        const failed =
          !!answer && typeof answer === "object" && "ok" in answer && answer.ok === false;
        return text(answer, failed);
      }),
  );

  server.registerTool(
    "screenshot",
    {
      title: "Screenshot",
      description: SCREENSHOT_DESCRIPTION,
      inputSchema: {
        scale: z
          .number()
          .gt(0)
          .max(1)
          .optional()
          .describe("Downscale factor, 0-1 (default 0.5)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ scale }) =>
      guarded(async () => {
        const png = await client.screenshot(scale ?? 0.5);
        return {
          content: [
            { type: "image", data: Buffer.from(png).toString("base64"), mimeType: "image/png" },
          ],
        };
      }),
  );

  server.registerTool(
    "events",
    {
      title: "Read UI events",
      description: EVENTS_DESCRIPTION + (longPoll ? EVENTS_WAIT : ""),
      inputSchema: {
        since: z.number().int().min(0).optional().describe("Cursor: the `next` of the last call (0 to start)."),
        ...(longPoll
          ? { wait_ms: z.number().int().min(0).max(30_000).optional().describe("Long-poll up to this many ms for an event.") }
          : {}),
        limit: z.number().int().min(1).max(10_000).optional().describe("Most events to return (default 1000)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ since, wait_ms, limit }: { since?: number; wait_ms?: number; limit?: number }) =>
      guarded(async () => text(await client.events({ since: since ?? 0, wait_ms: longPoll ? wait_ms : undefined, limit }))),
  );

  server.registerTool(
    "modules",
    {
      title: "Saved Lua modules",
      description: MODULES_DESCRIPTION,
      inputSchema: {
        name: z.string().optional().describe("A module to read; omit to list them all."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ name }) =>
      guarded(async () =>
        name
          ? { content: [{ type: "text", text: await client.readModule(name) }] }
          : text(await client.modules()),
      ),
  );

  server.registerTool(
    "save_module",
    {
      title: "Save a Lua module",
      description: SAVE_MODULE_DESCRIPTION,
      inputSchema: {
        name: z.string().regex(/^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)*$/).describe("Dotted module name."),
        source: z.string().describe("The module's Lua source; usually ends with `return M`."),
      },
    },
    ({ name, source }) => guarded(async () => text(await client.saveModule(name, source))),
  );

  server.registerTool(
    "status",
    {
      title: "Client status",
      description: STATUS_DESCRIPTION,
      annotations: { readOnlyHint: true },
    },
    () => guarded(async () => text(await client.status())),
  );
}

export function createServer(client: AgentClient, options: ToolOptions = {}): McpServer {
  const server = new McpServer({ name: "wrathplus-agent", version: "0.1.0" });
  registerTools(server, client, options);
  return server;
}
