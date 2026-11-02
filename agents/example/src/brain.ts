// The planner: Claude through the Agent SDK, with only the WrathPlus MCP tools, fed a stream of
// user messages (the goal first, then game events and operator notes as they arrive).

import {
  createSdkMcpServer,
  query,
  tool,
  type Options,
  type SDKMessage,
  type SDKUserMessage,
} from "@anthropic-ai/claude-agent-sdk";
import { z } from "zod";
import type { Policy } from "./policy.ts";
import { SYSTEM_PROMPT } from "./prompt.ts";

/** The MCP tools the planner may use, all from the wrathplus server; nothing else. */
export const TOOLS = ["lua", "screenshot", "events", "status", "modules", "save_module"].map(
  (name) => `mcp__wrathplus__${name}`,
);

/** A queue of user messages the SDK reads as its streaming prompt. */
export class Inbox implements AsyncIterable<SDKUserMessage> {
  #items: SDKUserMessage[] = [];
  #wake: (() => void) | undefined;
  #closed = false;

  push(text: string): void {
    if (this.#closed) return;
    this.#items.push({
      type: "user",
      message: { role: "user", content: text },
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
      systemPrompt: SYSTEM_PROMPT,
      // No built-in tools (files, shell, web): the agent acts only through the game client.
      tools: [],
      allowedTools: options.policy ? [...TOOLS, "mcp__plan__set_plan", "mcp__plan__fast_loop_status"] : TOOLS,
      permissionMode: "dontAsk",
      mcpServers: {
        wrathplus: {
          type: "stdio",
          command: process.execPath,
          args: ["run", options.mcpEntry],
          env: { WRATHPLUS_AGENT_URL: options.url, WRATHPLUS_AGENT_TOKEN: options.token },
        },
        ...(options.policy ? { plan: planServer(options.policy) } : {}),
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
          parts.push(`→ ${block.name.replace(/^mcp__(wrathplus|plan)__/, "")} ${input.length > 300 ? input.slice(0, 300) + "…" : input}`);
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
        "Hand the fast decision loop (Jev, about once a second) your current plan and the intents it may act on. Each intent is a name, a plain description of when it applies (Jev chooses by these), and the Lua to run when chosen (the same Lua as the lua tool; keep each one short and safe to repeat, e.g. AgentInteract(guid) toward the nearest quest giver, CastSpellByName, AcceptQuest()). A built-in 'wait' intent covers letting things run. Replaces the previous plan and intents.",
        {
          plan: z.string().describe("The plan in a few sentences: the goal and the steps, for the fast loop to follow."),
          intents: z
            .array(
              z.object({
                name: z.string().regex(/^[a-z][a-z0-9_]*$/).describe("snake_case name"),
                when: z.string().describe("When this is the right thing to do, in plain words."),
                lua: z.string().describe("Lua to run when chosen."),
                every: z.number().positive().optional().describe("Seconds before it may run again (default 3)."),
              }),
            )
            .max(254),
        },
        async ({ plan, intents }) => {
          policy.update(plan, intents);
          return { content: [{ type: "text", text: `Plan set with ${intents.length} intents (+ wait).` }] };
        },
      ),
      tool(
        "fast_loop_status",
        "What the fast loop has decided recently: its choices with confidence, whether each ran and what the Lua returned, and how stalled it judges the plan to be.",
        {},
        async () => ({ content: [{ type: "text", text: policy.describe() }] }),
      ),
    ],
  });
}
