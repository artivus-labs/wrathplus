// The planner: Claude through the Agent SDK, with only the WrathPlus MCP tools, fed a stream of
// user messages (the goal first, then game events and operator notes as they arrive).

import { query, type Options, type SDKMessage, type SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
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
}

/** Start the planner on the inbox; yields every SDK message for the caller to log. */
export function think(inbox: Inbox, options: BrainOptions): AsyncIterable<SDKMessage> {
  return query({
    prompt: inbox,
    options: {
      model: options.model,
      ...(options.effort ? { effort: options.effort } : {}),
      systemPrompt: SYSTEM_PROMPT,
      // No built-in tools (files, shell, web): the agent acts only through the game client.
      tools: [],
      allowedTools: TOOLS,
      permissionMode: "dontAsk",
      mcpServers: {
        wrathplus: {
          type: "stdio",
          command: process.execPath,
          args: ["run", options.mcpEntry],
          env: { WRATHPLUS_AGENT_URL: options.url, WRATHPLUS_AGENT_TOKEN: options.token },
        },
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
        if (block.type === "text" && block.text.trim()) parts.push(block.text.trim());
        if (block.type === "tool_use") {
          const input = JSON.stringify(block.input);
          parts.push(`→ ${block.name.replace("mcp__wrathplus__", "")} ${input.length > 300 ? input.slice(0, 300) + "…" : input}`);
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
