// The planner: Claude through the Agent SDK, with only the WrathPlus MCP tools, fed a stream of
// user messages (the goal first, then game events and operator notes as they arrive).

import {
  createSdkMcpServer,
  query,
  tool,
  type HookCallbackMatcher,
  type Options,
  type SDKMessage,
  type SDKUserMessage,
} from "@anthropic-ai/claude-agent-sdk";
import { z } from "zod";
import type { Policy } from "./policy.ts";
import { claudeEnv } from "./env.ts";
import { NOTE_TOOLS, notesServer, type Notes } from "./notes.ts";
import { sleepGuard, WAKE_TOOLS, wakeServer, type Waker } from "./wake.ts";
import { systemPrompt } from "./prompt.ts";
import { dropGoalLua, GOAL_NAME, GOALS_LUA, holdLua, DEFAULT_HOLD_SECONDS, MAX_HOLD_SECONDS, outcome, pushGoalLua, type Lua } from "./goals.ts";
import { harnessServer, MODULE_TOOLS, SAVE_MODULE_TOOL, versionHooks, type ModuleFiles, type ModuleVersions } from "./modules.ts";

/** The MCP tools the planner may use, all from the wrathplus server; nothing else. */
export const TOOLS = ["lua", "screenshot", "events", "status", "modules", "save_module"].map(
  (name) => `mcp__wrathplus__${name}`,
);

/** A queue of user messages the SDK reads as its streaming prompt. */
export class Inbox implements AsyncIterable<SDKUserMessage> {
  #items: SDKUserMessage[] = [];
  #wake: (() => void) | undefined;
  #closed = false;

  /** A message for the planner, with a PNG (base64) of the game when there is one. */
  push(text: string, png?: string): void {
    if (this.#closed) return;
    const content = png
      ? [
          { type: "image" as const, source: { type: "base64" as const, media_type: "image/png" as const, data: png } },
          { type: "text" as const, text },
        ]
      : text;
    this.#items.push({
      type: "user",
      message: { role: "user", content },
      parent_tool_use_id: null,
    });
    this.#wake?.();
  }

  close(): void {
    this.#closed = true;
    this.#wake?.();
  }

  async *[Symbol.asyncIterator](): AsyncIterator<SDKUserMessage> {
    for (;;) {
      const next = this.#items.shift();
      if (next) {
        yield next;
        continue;
      }
      if (this.#closed) return;
      await new Promise<void>((resolve) => (this.#wake = resolve));
      this.#wake = undefined;
    }
  }
}

export interface BrainOptions {
  /** The client's control port, e.g. http://127.0.0.1:17950. */
  url: string;
  token: string;
  /** agents/mcp/src/index.ts in this checkout. */
  mcpEntry: string;
  model: string;
  effort?: Options["effort"];
  /** Only for a short smoke run; a real run has no turn limit. */
  maxTurns?: number;
  abortController: AbortController;
  /** The fast tier (Jev). With it, the planner gets the plan tools. */
  policy?: Policy;
  /** Memory, to-dos and session search. */
  notes?: Notes;
  /** Compact the conversation once it reaches this many tokens. */
  compactAt?: number;
  /** The planner's alarm clock (wake_me); with it, the events tool does not long-poll. */
  waker?: Waker;
  /** Lua over the control port: with it, the planner gets the goal tools. */
  lua?: Lua;
  /** Versions of its saved modules (in its notes folder): with them, the history and rollback tools. */
  modules?: { versions: ModuleVersions; files: ModuleFiles };
}

/** Every tool the planner may call: the client's, and those of the parts it was given. */
export function allowedTools(options: Pick<BrainOptions, "policy" | "notes" | "waker" | "lua" | "modules">): string[] {
  return [
    ...TOOLS,
    ...(options.policy ? ["mcp__plan__set_plan", "mcp__plan__fast_loop_status"] : []),
    ...(options.notes ? NOTE_TOOLS : []),
    ...(options.waker ? WAKE_TOOLS : []),
    ...(options.lua ? GOAL_TOOLS : []),
    ...(options.lua && options.modules ? MODULE_TOOLS : []),
  ];
}

/** The sleep guard (after wake_me), and module versions kept around every save_module. */
function hooks(options: BrainOptions): Options["hooks"] {
  const pre: HookCallbackMatcher[] = options.waker ? [{ hooks: [sleepGuard(options.waker)] }] : [];
  const post: HookCallbackMatcher[] = [];
  if (options.modules) {
    const kept = versionHooks(options.modules.versions, options.modules.files);
    pre.push({ matcher: SAVE_MODULE_TOOL, hooks: [kept.pre] });
    post.push({ matcher: SAVE_MODULE_TOOL, hooks: [kept.post] });
  }
  return { PreToolUse: pre, PostToolUse: post };
}

/** Start the planner on the inbox; yields every SDK message for the caller to log. */
export function think(inbox: Inbox, options: BrainOptions): AsyncIterable<SDKMessage> {
  return query({
    prompt: inbox,
    options: {
      model: options.model,
      // The slow tier thinks: adaptive thinking at high effort, summarised for the log.
      thinking: { type: "adaptive", display: "summarized" },
      effort: options.effort ?? "high",
      systemPrompt: systemPrompt(options.policy !== undefined),
      // No built-in tools (files, shell, web): the agent acts only through the game client.
      tools: [],
      allowedTools: allowedTools(options),
      // Compaction well inside the window: recall degrades long before the window fills.
      settings: { autoCompactEnabled: true, autoCompactWindow: options.compactAt ?? 400_000 },
      permissionMode: "dontAsk",
      // The Claude Code login, not a stray ANTHROPIC_API_KEY from the shell (`env.ts`).
      env: claudeEnv(),
      // Isolated from this machine's Claude Code setup: no CLAUDE.md/AGENTS.md from the checkout, no
      // user settings, hooks or MCP servers. It works in its own notes folder.
      settingSources: [],
      ...(options.notes ? { cwd: options.notes.home } : {}),
      hooks: hooks(options),
      mcpServers: {
        wrathplus: {
          type: "stdio",
          command: process.execPath,
          args: ["run", options.mcpEntry],
          env: {
            WRATHPLUS_AGENT_URL: options.url,
            WRATHPLUS_AGENT_TOKEN: options.token,
            // It sleeps by ending its turn, not inside a tool call.
            ...(options.waker ? { WRATHPLUS_MCP_LONG_POLL: "0" } : {}),
          },
        },
        ...(options.policy ? { plan: planServer(options.policy) } : {}),
        ...(options.notes ? { notes: notesServer(options.notes) } : {}),
        ...(options.waker ? { wake: wakeServer(options.waker) } : {}),
        ...(options.lua ? { goals: goalsServer(options.lua) } : {}),
        ...(options.lua && options.modules ? { harness: harnessServer(options.modules.versions, options.modules.files, options.lua) } : {}),
      },
      ...(options.maxTurns ? { maxTurns: options.maxTurns } : {}),
      abortController: options.abortController,
    },
  });
}

/** One line per SDK message worth showing a person watching the run. */
export function summarize(message: SDKMessage): string | null {
  switch (message.type) {
    case "assistant": {
      const parts: string[] = [];
      for (const block of message.message.content) {
        if (block.type === "thinking" && block.thinking.trim()) parts.push(`(thinking) ${block.thinking.trim().slice(0, 400)}`);
        if (block.type === "text" && block.text.trim()) parts.push(block.text.trim());
        if (block.type === "tool_use") {
          const input = JSON.stringify(block.input);
          parts.push(`→ ${block.name.replace(/^mcp__(wrathplus|plan|notes|wake|goals|harness)__/, "")} ${input.length > 300 ? input.slice(0, 300) + "…" : input}`);
        }
      }
      return parts.length ? parts.join("\n") : null;
    }
    case "result":
      return message.subtype === "success"
        ? `[done after ${message.num_turns} turns, $${message.total_cost_usd.toFixed(2)} equivalent]`
        : `[stopped: ${message.subtype}]`;
    default:
      return null;
  }
}

/** The planner's handle on the fast tier: set the plan and intents, read what Jev has done. */
export function planServer(policy: Policy) {
  return createSdkMcpServer({
    name: "plan",
    version: "0.1.0",
    tools: [
      tool(
        "set_plan",
        "Hand the fast decision loop (Jev) your plan and the intents it may choose from. Each intent is an activity: a name, a plain description of when it applies (Jev chooses by these), the Lua that starts it, and optionally Lua that returns true once it has finished (until) and a timeout. Jev chooses when something happens (an activity ends, combat starts, a window opens, you send a plan) and sleeps while an activity runs, so make activities substantial: start your module's behaviour and let it run until done, rather than one tiny step per choice. Intents your own modules register with Agent.intent are offered too. Replaces the previous plan and its intents.",
        {
          plan: z.string().describe("The plan in a few sentences: the goal and the steps, for the fast loop to follow."),
          intents: z
            .array(
              z.object({
                name: z.string().regex(/^[a-z][a-z0-9_]*$/).describe("snake_case name"),
                when: z.string().describe("When this is the right thing to do, in plain words."),
                lua: z.string().describe("Lua that starts it."),
                until: z.string().optional().describe("Lua returning true once it has finished (and false as a second value if it failed). Without it the intent ends as soon as its Lua returns."),
                timeout: z.number().positive().optional().describe("Seconds before it counts as failed."),
              }),
            )
            .max(254),
          steps: z
            .array(
              z.object({
                name: z.string().describe("What this step does, in a few words."),
                lua: z.string().describe("Lua that starts it."),
                until: z.string().optional().describe("Lua returning true once it has finished (and false as a second value if it failed). Without it the step is done as soon as its Lua returns."),
                timeout: z.number().positive().optional().describe("Seconds before the step counts as failed."),
              }),
            )
            .optional()
            .describe("A sequence the client runs in order, by itself, as one activity (follow_steps): each step starts when the one before is done. You are woken when all are done or one fails or times out. Use it for anything with several legs (a route in waypoints, walk then talk then hand in) instead of doing one leg per turn."),
        },
        async ({ plan, intents, steps }) => ({
          content: [{ type: "text", text: await policy.update(plan, intents, steps ?? []) }],
        }),
      ),
      tool(
        "fast_loop_status",
        "What the fast loop has decided recently: why it woke, what it chose (with confidence), whether that started a new activity, how stalled it judges the plan, and how many Jev calls it has made.",
        {},
        async () => ({ content: [{ type: "text", text: policy.describe() }] }),
      ),
    ],
  });
}

export const GOAL_TOOLS = ["push_goal", "drop_goal", "goals", "hold"].map((name) => `mcp__goals__${name}`);

const reply = (value: string) => ({ content: [{ type: "text" as const, text: value }] });

/** The planner's goals: the client's GOAP runner plans and runs actions toward the top one. */
export function goalsServer(lua: Lua) {
  return createSdkMcpServer({
    name: "goals",
    version: "0.1.0",
    tools: [
      tool(
        "push_goal",
        "Set a goal for the client's goal runner (Goals.push): it plans actions toward the highest-priority achievable goal and runs them, one activity after another, without you. A goal of the same name is replaced. The standing goal `progress` (quests, training, upkeep, grinding, travel to a better zone) is always there at the lowest priority, so push goals for what matters more now: a quest chain, a zone, a level, gear, a profession, staying with your companion. done_lua says when it is achieved; teach the actions it needs with Goals.action in your modules.",
        {
          name: z.string().regex(GOAL_NAME).describe("snake_case name"),
          why: z.string().describe("Why this goal, in a sentence: the runner shows it, and you will read it back at your reviews."),
          priority: z.number().int().min(1).max(1000).optional().describe("Higher comes first (default 50; progress is lowest)."),
          done_lua: z.string().optional().describe("Lua returning finished[, ok]; for example `return UnitLevel(\"player\") >= 10`. Without it the goal stays until dropped."),
          standing: z.boolean().optional().describe("Never finished: kept as an ongoing aim (like progress) rather than done once."),
          want: z
            .record(z.string(), z.union([z.boolean(), z.number(), z.string()]))
            .optional()
            .describe(
              "The facts the goal wants, which the planner plans toward: built-in facts (the goals tool lists them, e.g. { quest_ready: true }, { zone_fit: \"ok\" }) or ones you add with Goals.fact. Without want, the goal wants a fact of its own name; if no action's effects reach what it wants, it shows as blocked: teach one (Goals.action) or want existing facts.",
            ),
        },
        async (goal) => reply(outcome(await lua(pushGoalLua(goal)), `Goal ${goal.name} set. The goals tool shows its plan.`)),
      ),
      tool(
        "drop_goal",
        "Remove a goal (Goals.drop). The runner moves on to the next.",
        { name: z.string().regex(GOAL_NAME) },
        async ({ name }) => reply(outcome(await lua(dropGoalLua(name)), `Goal ${name} dropped.`)),
      ),
      tool(
        "goals",
        "The goals, highest first, each with its status (active, blocked, done, failed), the plan of actions toward it and a note; the facts the planner plans over (quest_ready, bags_full, zone_fit, mobs_for_xp...); any hold; the running activity. Read it to see why the runner does what it does, or why it is stuck.",
        {},
        async () => {
          const result = await lua(GOALS_LUA);
          return reply(result.ok && result.values?.[0] !== false ? JSON.stringify(result.values?.[0] ?? null, null, 2) : outcome(result, ""));
        },
      ),
      tool(
        "hold",
        `Pause the goal runner for a while (Goals.hold), with the reason: waiting for your companion, for a respawn, for a group to form. Keep it short: ${DEFAULT_HOLD_SECONDS} s unless you give seconds, a few minutes at most (hard limit ${MAX_HOLD_SECONDS} s); when it runs out, the fast loop decides whether to keep waiting or get on, from who is actually there. seconds 0 ends the current hold.`,
        {
          reason: z.string().describe("What you are waiting for, plainly: the fast loop reads it."),
          seconds: z
            .number()
            .int()
            .min(0)
            .max(MAX_HOLD_SECONDS)
            .optional()
            .describe(`Default ${DEFAULT_HOLD_SECONDS}; 0 ends the current hold.`),
        },
        async ({ reason, seconds = DEFAULT_HOLD_SECONDS }) =>
          reply(outcome(await lua(holdLua(reason, seconds)), seconds > 0 ? `Holding for ${seconds}s: ${reason}.` : "Hold released; the runner carries on.")),
      ),
    ],
  });
}
