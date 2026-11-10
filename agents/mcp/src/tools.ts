// The MCP tools an agent plays through: Lua, screenshots, events and status.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { AgentError, type AgentClient } from "./client.ts";

/** The agent helpers an outside agent needs, and which of them finish in the background. Every
 *  claim here is checked against crates/wrathplus-agent (src/world.rs, src/nav.rs, lua/wp/*.lua);
 *  keep it under 1500 characters. */
export const HELPERS = `Helpers (guids: "0x..." strings). What walks or fights returns at once and ends as an AGENT_* event (events names=["AGENT_"]).
- AgentNearby([yards]), nearest first: {guid, name, kind, distance, x, y, z, level, health, dead, reaction, creature_type, quest, npc, lootable}. AgentPosition(): {x, y, z, facing, map}. AgentTarget(guid).
- AgentMoveTo(x, y[, z] | guid): walks a navmesh route; returns the route, {pending = true} while the realm routes it, or nil, why. AgentWalk(): {status = idle|routing|walking|arrived|stopped|failed|stuck, note}, ending as AGENT_WALK_DONE(status). AgentStop(). A later AgentMoveTo, AgentInteract or Combat.engage replaces it.
- AgentInteract(guid): talk to, loot, use or attack it; walks there itself.
- Combat.engage(guid): true if in sight; chases, fights, loots: AGENT_COMBAT_DONE(guid, killed|lost|stopped|died). Combat.engaged(guid): over, won, outcome. Combat.stop().
- Quests.pick_up(), Quests.hand_in(): visit givers within 30 yd: AGENT_QUESTS_DONE(kind, picked, handed). Quests.list(): {id, title, complete, objectives}.
- Move.to(unit | x, y[, z]): AGENT_MOVE_DONE(target, arrived|lost|failed). Move.follow(unit), Move.free().
- Goals.on = true levels by itself. Goals.list(), Goals.facts(), Goals.state; Goals.push{name, why, priority, want}; Goals.hold{reason, seconds} pauses it. Agent.activity(): {name, seconds}, nil between actions.
- StopCinematic() skips a cinematic (status.cinematic); a new character starts in one, with nothing near or walkable.`;

const LUA_DESCRIPTION = `Run Lua in the WrathPlus game client's UI VM (the WoW 3.3.5a addon API) to read game state and act.
It is the stock World of Warcraft 3.3.5a (build 12340) API as an addon sees it: read (UnitName("player"), UnitHealth("target"), GetQuestLogTitle(i), GetContainerItemInfo(bag, slot), ...) and act (CastSpellByName, UseContainerItem, SendChatMessage, ...). Movement verbs work from here (MoveForwardStart/MoveForwardStop, TurnLeftStart/TurnLeftStop, JumpOrAscendStart, ...): start one, stop it in a later call.
A bare expression (\`UnitLevel("player")\`) returns its values; otherwise use \`return a, b\`. The answer is JSON {ok, values, output}, output being what print()ed, or {ok:false, error, output} on a Lua error.
A chunk runs to completion inside one game frame: never loop waiting for the world to change. Return, then call again or wait on the events tool.
${HELPERS}
For code that runs by itself every frame, save a module (save_module) and require("name") it (AgentReload("name") after a change; one named autoexec runs on entering the world). Frames work as in addons: CreateFrame("Frame"):SetScript("OnUpdate", fn) ticks, :RegisterEvent(...) reacts.
Also: AgentFindNpc(service[, yards]) (the nearest "repair", "vendor", "classtrainer", "innkeeper"... in sight), AgentServices([kinds[, radius]]) (where services are on your map, from the realm: {pending = true} first, then the list after AGENT_SERVICES_READY), AgentPath (a route preview; {pending = true} first when the realm routes, then the route after AGENT_PATH_READY), Rotation.list (what Combat casts), Agent.intent(name, {when, run}) and Agent.decide(name, {question, options, on}) for a fast decision loop when one runs, require("wp.engine") (a Playerbots-style decision engine) and require("wp.values") (health, power, auras, cooldowns, range, threat, bags).`;

const MODULES_DESCRIPTION = `List your saved Lua modules, or read one when \`name\` is given (dotted names like grind.loop). Modules persist across reloads and restarts of the client.`;

const SAVE_MODULE_DESCRIPTION = `Save a Lua module (source) under a dotted name like grind.loop, for require("grind.loop") in lua. Saving does not run it: call require (first load) or AgentReload("grind.loop") (after a change). Name it autoexec to run it whenever the client enters the world.`;

const SCREENSHOT_DESCRIPTION = `Capture the next rendered frame of the WrathPlus game client as a PNG image. scale (0-1, default 0.5) shrinks it; use 1 for full resolution when reading small text.`;

const EVENTS_DESCRIPTION = `Read game UI events after cursor \`since\` (omit it to start from now), optionally filtered by name.
They are the events addons receive (PLAYER_TARGET_CHANGED, CHAT_MSG_SAY, UNIT_HEALTH, COMBAT_LOG_EVENT_UNFILTERED, ...) plus the agent's own AGENT_* signals. Returns JSON {events:[{seq, time, event, args}], next, missed}: pass \`next\` as \`since\` on the following call.
Without \`since\` the read starts from now: nothing already logged comes back, only \`next\` to carry on from. since=0 reads the whole retained buffer (up to 10000 events, mostly other units' COMBAT_LOG_EVENT and UNIT_* noise: filter it).
names keeps only these events and exclude drops them: whole names ("PLAYER_DEAD"), or prefixes when they end in "_" or "*" (["AGENT_", "CHAT_MSG_", "QUEST_", "UI_ERROR_MESSAGE"]).
limit caps how many come back (default 100, oldest first); when it cuts a read short, \`next\` carries on after the last one returned. \`missed\` counts events that dropped out of the retained window before you read them.`;

/** The longest wait the tool passes on: under the 30 s at which hosts time a tool call out. */
export const MAX_WAIT_MS = 25_000;

const EVENTS_WAIT = `\nwait_ms long-polls until an event arrives (with names or exclude: one that matches), for at most ${MAX_WAIT_MS} ms; a longer wait is cut to that, because hosts time a tool call out at about 30 s. No event in time is an empty read: call again with \`next\`. To wait for the end of something you start (a walk, a fight), read \`next\` first, start it, then wait with since=next and names=["AGENT_"]: an event that fires before a from-now read begins is not in it.`;

export interface ToolOptions {
  /** Offer events' wait_ms long-poll (default true). A host that wakes its agent on events itself
   *  turns it off, so the agent ends its turn instead of sleeping inside one. */
  longPoll?: boolean;
}

const STATUS_DESCRIPTION = `Report the WrathPlus client's state: {state, in_world, ui_ready, cinematic, time, player, nav}.
state is the client phase (Login, CharSelect, InWorld, ...). ui_ready means its Lua answers, and out of the world that the login and character-select API (GetNumCharacters, CreateCharacter, EnterWorld...) is installed, so wait for it before calling those.
player is null out of the world, and for about a second after entering it, until the character's own record has arrived: wait for in_world with a player before acting. Then it summarises name, level, class, race, health, power, dead, combat, zone, subzone, map position and target.
cinematic is true while a cinematic holds the client (a new character's intro fly-by): nothing is near (AgentNearby) and walking is refused until it ends by itself or StopCinematic() (lua tool) skips it.
nav says where walking routes (AgentMoveTo, AgentPath) come from: source "local" (the client's own mmaps), "realm" (asked of the realm over the game connection: a walk is "routing" until its route arrives) or "none" (no walking routes: detail says how to get them).`;

interface EventsArgs {
  since?: number;
  wait_ms?: number;
  limit?: number;
  names?: string[];
  exclude?: string[];
}

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
        since: z
          .number()
          .int()
          .min(0)
          .optional()
          .describe("Cursor: the `next` of the last call. Omit to start from now; 0 reads the whole buffer."),
        names: z
          .array(z.string().min(1))
          .optional()
          .describe('Only these events: names, or prefixes ending in "_" or "*", e.g. ["AGENT_", "CHAT_MSG_"].'),
        exclude: z
          .array(z.string().min(1))
          .optional()
          .describe('Never these events, in the same form, e.g. ["COMBAT_LOG*", "UNIT_"].'),
        ...(longPoll
          ? {
              wait_ms: z
                .number()
                .int()
                .min(0)
                .optional()
                .describe(`Long-poll up to this many ms for a (matching) event; capped at ${MAX_WAIT_MS}.`),
            }
          : {}),
        limit: z.number().int().min(1).max(10_000).optional().describe("Most events to return (default 100)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ since, wait_ms, limit, names, exclude }: EventsArgs) =>
      guarded(async () =>
        text(
          await client.events({
            since,
            wait_ms: longPoll && wait_ms !== undefined ? Math.min(wait_ms, MAX_WAIT_MS) : undefined,
            limit,
            names,
            exclude,
          }),
        ),
      ),
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
