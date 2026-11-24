// The MCP tools an agent plays through: Lua, screenshots, events and status.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { AgentError, type AgentClient } from "./client.ts";
import { type Judge, trials } from "./judge.ts";
import { characterKey, type SampleStore, type Sampler, scorecard } from "./scorecard.ts";

/** The agent helpers an outside agent needs, and which of them finish in the background. Every
 *  claim here is checked against crates/wrathplus-agent (src/world.rs, src/nav.rs, lua/wp/*.lua);
 *  keep it under 1500 characters. */
export const HELPERS = `Helpers (guids: "0x..." strings). What walks or fights returns at once, ending as an AGENT_* event (events names=["AGENT_"]).
- AgentNearby([yards]), nearest first: {guid, name, kind, distance, x, y, z, level, health, dead, reaction, creature_type, quest, npc, lootable}. AgentPosition(): {x, y, z, map}. AgentTarget(guid).
- AgentMoveTo(x, y[, z] | guid): walks a navmesh route; returns the route, {pending = true} while the realm routes it, or nil, why. AgentWalk(): {status = idle|routing|walking|arrived|stopped|failed|stuck, note, cinematic}, ending as AGENT_WALK_DONE(status). AgentStop(). A later walk or fight replaces it.
- AgentInteract(guid): talk to, loot, use or attack it; walks there itself. At a vendor the window opens a moment later; Upkeep may sell junk.
- Combat.engage(guid): true if in sight; chases, fights, loots: AGENT_COMBAT_DONE(guid, killed|lost|stopped|died). Combat.engaged(guid): over, won, outcome. Combat.stop().
- Quests.pick_up(), Quests.hand_in(): visit givers within 30 yd: AGENT_QUESTS_DONE(kind asked, picked, handed), one per call. Quests.list().
- Move.to(unit | x, y[, z]): AGENT_MOVE_DONE(target, arrived|lost|failed). Move.follow(unit), Move.free().
- Goals.on = true levels by itself. Goals.list(), Goals.facts(), Goals.state; Goals.push{name, why, priority, want}; Goals.hold{reason, seconds} pauses it. Agent.activity(): {name, seconds}, nil between actions.
- StopCinematic() skips a cinematic (status.cinematic); a new character starts in one.`;

const LUA_DESCRIPTION = `Run Lua in the WrathPlus game client's UI VM (the WoW 3.3.5a addon API) to read game state and act.
It is the stock World of Warcraft 3.3.5a (build 12340) API as an addon sees it: read (UnitName("player"), UnitHealth("target"), GetQuestLogTitle(i), GetContainerItemInfo(bag, slot), ...) and act (CastSpellByName, UseContainerItem, SendChatMessage, ...). Movement verbs work from here (MoveForwardStart/MoveForwardStop, TurnLeftStart/TurnLeftStop, JumpOrAscendStart, ...): start one, stop it in a later call.
A bare expression (\`UnitLevel("player")\`) returns its values; otherwise use \`return a, b\`. The answer is JSON {ok, values, output}, output being what print()ed, or {ok:false, error, output} on a Lua error.
A chunk runs to completion inside one game frame: never loop waiting for the world to change. Return, then call again or wait on the events tool.
${HELPERS}
Search before writing a helper: lua_search finds what exists (built-ins, your modules, the API's names) and lua_doc reads one.
For code that runs by itself every frame, save a module (save_module) and require("name") it (AgentReload("name") after a change; one named autoexec runs on entering the world). Frames work as in addons: CreateFrame("Frame"):SetScript("OnUpdate", fn) ticks, :RegisterEvent(...) reacts.
Also: AgentFindNpc(service[, yards]) (the nearest "repair", "vendor", "classtrainer", "innkeeper"... in sight), AgentServices([kinds[, radius]]) (where services are on your map, from the realm: {pending = true} first, then after AGENT_SERVICES_READY the places with their distance, nearest first, a game-event spawn marked event = true with event_active saying whether its event has it out, an auctioneer with house = "alliance", "horde" or "neutral", mailboxes too (kinds "auctioneer", "mailbox"); 2000 yd and 20 places by default; the answer is reused for 120 s within 100 yd), AgentPath (a route preview; {pending = true} first when the realm routes, then the route after AGENT_PATH_READY), Rotation.list (what Combat casts), Agent.intent(name, {when, run}) and Agent.decide(name, {question, options, on}) for a fast decision loop when one runs, Agent.ask(question, context, on[, {timeout, default}]) to hand a messy moment (someone talking, an odd mechanic, a hard choice) to you, the planner: it signals AGENT_ASK(id, question, context, digest of recent events), so wait on AGENT_ASK in events and answer with Agent.answer(id, value), which calls on(value); Agent.open_asks() lists the unanswered ones, require("wp.engine") (a Playerbots-style decision engine) and require("wp.values") (health, power, auras, cooldowns, range, threat, bags).`;

const LUA_SEARCH_DESCRIPTION = `Search the Lua you can call (built-ins, your modules, API names) before writing your own.
builtin: the built-in modules' functions (Quests.pick_up, Goals.push, wp.values.health...) and the Agent* functions, each with its signature, doc and file:line. own: your saved modules, the lines that match. api: the names of the global functions in the game's Lua (the stock 3.3.5a addon API and anything defined since), names only.
Case-insensitive; names that match come first, then docs, then source lines. Read one in full with lua_doc.`;

const LUA_DOC_DESCRIPTION = `Read one Lua function or module in full: signature, doc and source.
For a built-in, its signature, doc and source; for one of your modules, its source.
Give the name lua_search returned (Quests.pick_up, AgentMoveTo, grind.loop); a bare function name works when only one function has it. A stock API name only says it exists: WrathPlus ships no stock API docs. Nothing by that name: an error with the nearest names.`;

const MODULES_DESCRIPTION = `List your saved Lua modules, or read one when \`name\` is given (dotted names like grind.loop). Modules persist across reloads and restarts of the client.`;

const SAVE_MODULE_DESCRIPTION = `Save a Lua module (source) under a dotted name like grind.loop, for require("grind.loop") in lua. Saving does not run it: call require (first load) or AgentReload("grind.loop") (after a change). Name it autoexec to run it whenever the client enters the world.`;

const SAVE_MODULE_JUDGED = ` To have a change judged against the current version before it is kept, use propose_module instead.`;

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
  /** The scorecard and the judge: with them, the scorecard, propose_module and ledger tools. */
  judging?: () => Judging | string;
}

export interface Judging {
  judge: Judge;
  store: SampleStore;
  sampler: Sampler;
  /** A gap between samples longer than this is not counted (ms). */
  maxGapMs: number;
  now?: () => number;
}

const SCORECARD_DESCRIPTION = `Your character's scorecard: many goals at once, sampled from the client and kept on disk.
It samples about every 30 s while you are in the world.
current: level, xp, money (copper), gear (average equipped item level), professions (rank, max), spells known, zone, zones and subzones visited, quests done (handed in) and accepted, deaths.
needs: targets that rise with level, each a gap (0 = met): every known profession's skill near five a level, average item level near your level, a gold reserve. Falling behind on them is a regression the judge and your reviews see.
trends: the last 15, 60 and 240 minutes and all of it, as counts and rates per hour: xp, gold, quests, exploration (new zones and subzones), profession skill-ups, deaths, idle share (sampled: alive, out of combat, still, not casting and full), needs gap per hour; breaks lists restarts and gaps.`;

const PROPOSE_DESCRIPTION = `Propose a change to one of your Lua modules, to be judged before it is kept.
Give the claim it should bear out ("pulls one mob at a time: fewer deaths, same xp/h"). The candidate is kept as a new version but not loaded. A deterministic judge then trials it against the module as it stands (the champion) in alternating windows of play (by default 12 minutes each, 3 pairs), loading one or the other between windows, and scores each window on the scorecard's objectives. The candidate is promoted only if it improves at least one objective and regresses none beyond that objective's tolerance; otherwise the champion is put back. Read the outcome with ledger.
Meanwhile do not save_module the same module: a change mid-trial stops it. A loaded module that returns a table with a stop() function has it called before each swap; make the module safe to reload.`;

const LEDGER_DESCRIPTION = `The judge's ledger: every module change proposed, how its trial went and the verdict.
For each: the claim, the trial windows (arm, start, end, void and why, rates), and the verdict with each objective's champion and candidate values, the mean difference, the tolerance and the effect size (t, d), and the reasons. Also the trial running now and the queue.`;

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
  if (error instanceof AgentError && error.details !== undefined) {
    return text({ message: error.message, status: error.status, details: error.details }, true);
  }
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
    "lua_search",
    {
      title: "Search the Lua you can call",
      description: LUA_SEARCH_DESCRIPTION,
      inputSchema: {
        query: z.string().describe("Words to look for in names, docs and source (case-insensitive)."),
        kinds: z
          .array(z.enum(["builtin", "own", "api"]))
          .optional()
          .describe("Where to look (default all three)."),
        limit: z.number().int().min(1).max(200).optional().describe("Most results (default 20)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ query, kinds, limit }) => guarded(async () => text(await client.luaSearch(query, kinds, limit))),
  );

  server.registerTool(
    "lua_doc",
    {
      title: "Read a Lua function or module",
      description: LUA_DOC_DESCRIPTION,
      inputSchema: {
        name: z.string().min(1).describe("A name from lua_search: Quests.pick_up, AgentMoveTo, grind.loop."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ name }) => guarded(async () => text(await client.luaDoc(name))),
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
      description: SAVE_MODULE_DESCRIPTION + (options.judging ? SAVE_MODULE_JUDGED : ""),
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
  if (options.judging) registerJudging(server, options.judging);
}

/** The scorecard and the judge's tools. Nothing here writes the judge's config or ledger but a
 *  proposal, which only appends. */
function registerJudging(server: McpServer, ready: () => Judging | string): void {
  // The judge is set up once the client has said where its modules live; until then, why not.
  const withJudging = (run: (j: Judging) => Promise<CallToolResult>) =>
    guarded(async () => {
      const j = ready();
      return typeof j === "string" ? failure(new Error(j)) : run(j);
    });
  server.registerTool(
    "scorecard",
    {
      title: "Character scorecard",
      description: SCORECARD_DESCRIPTION,
      inputSchema: {
        sample: z.boolean().optional().describe("Take a fresh sample first (default false)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ sample }) =>
      withJudging(async ({ store, sampler, maxGapMs, now }) => {
        if (sample) await sampler.tick();
        const last = sampler.last;
        if (!last) return text({ note: sampler.lastSkip ?? "no sample yet" });
        const card = scorecard(store, characterKey(last.name, last.realm), (now ?? Date.now)(), { maxGapMs });
        return text(sampler.lastSkip ? { ...card, note: `latest attempt: ${sampler.lastSkip}` } : card);
      }),
  );

  server.registerTool(
    "propose_module",
    {
      title: "Propose a module change",
      description: PROPOSE_DESCRIPTION,
      inputSchema: {
        name: z.string().regex(/^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)*$/).describe("Dotted module name."),
        source: z.string().describe("The candidate's full Lua source."),
        claim: z.string().min(1).describe("What it should improve, and what it should not make worse."),
      },
    },
    ({ name, source, claim }) => withJudging(async ({ judge }) => text(judge.propose(name, source, claim))),
  );

  server.registerTool(
    "ledger",
    {
      title: "Judge ledger",
      description: LEDGER_DESCRIPTION,
      inputSchema: {
        module: z.string().optional().describe("Only this module's trials."),
        limit: z.number().int().min(1).max(200).optional().describe("Most recent trials (default 20)."),
      },
      annotations: { readOnlyHint: true },
    },
    ({ module, limit }) =>
      withJudging(async ({ judge }) =>
        text({ ...judge.status(), trials: trials(judge.ledger.entries(), module).slice(-(limit ?? 20)) }),
      ),
  );
}

export function createServer(client: AgentClient, options: ToolOptions = {}): McpServer {
  const server = new McpServer({ name: "wrathplus-agent", version: "0.1.0" });
  registerTools(server, client, options);
  return server;
}
