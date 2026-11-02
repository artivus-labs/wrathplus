#!/usr/bin/env bun
// The example agent: Claude plans and writes Lua through the client's MCP tools; game events
// arrive as messages while it works. Lines typed on stdin go to it too, as operator notes.
//
//   WRATHPLUS_AGENT_URL=http://127.0.0.1:17950 WRATHPLUS_AGENT_TOKEN=… \
//     bun run agents/example/src/index.ts "Reach level 5 in Northshire"
//
// Runs on the Claude Code login of this machine (a Claude subscription works); no API key needed.

import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { Inbox, summarize, think } from "./brain.ts";
import { Jev } from "./jev.ts";
import { Notes } from "./notes.ts";
import { Policy } from "./policy.ts";
import { kickoff } from "./prompt.ts";
import { watch } from "./watch.ts";

export interface Config {
  url: string;
  token: string;
  goal: string;
  model: string;
  maxTurns?: number;
  /** After a finished turn with no news for this long, nudge the planner to carry on (0: never). */
  checkInSeconds: number;
}

export function configFromEnv(argv: string[], env: Record<string, string | undefined>): Config {
  const url = env.WRATHPLUS_AGENT_URL;
  const token = env.WRATHPLUS_AGENT_TOKEN;
  if (!url) throw new Error("Set WRATHPLUS_AGENT_URL to the client's control port (http://127.0.0.1:<port>).");
  if (!token) throw new Error("Set WRATHPLUS_AGENT_TOKEN to the client's WOW_AGENT_TOKEN.");
  const goal = argv.join(" ").trim() || "Play your character: quest, level up and explore, as a thoughtful player would.";
  const maxTurns = env.WRATHPLUS_AGENT_MAX_TURNS ? Number(env.WRATHPLUS_AGENT_MAX_TURNS) : undefined;
  return {
    url: url.replace(/\/+$/, ""),
    token,
    goal,
    model: env.WRATHPLUS_AGENT_MODEL ?? "claude-sonnet-5-5",
    checkInSeconds: Number(env.WRATHPLUS_AGENT_CHECKIN_SECONDS ?? 0),
    ...(maxTurns && maxTurns > 0 ? { maxTurns } : {}),
  };
}

async function main(): Promise<void> {
  const config = configFromEnv(process.argv.slice(2), process.env);
  const abortController = new AbortController();
  process.on("SIGINT", () => abortController.abort());
  const inbox = new Inbox();
  // The agent's own notes: memory, to-dos and its session log, kept per control port.
  const notes = new Notes(
    process.env.WRATHPLUS_AGENT_HOME ??
      join(homedir(), ".local/share/wrathplus-agents", `agent-${new URL(config.url).port || "default"}`),
  );
  // Everything that reaches the planner also goes in the session log.
  const tell = (kind: string, text: string) => {
    notes.record(kind, text);
    inbox.push(text);
  };
  // The fast tier, when a TypeSafe key is available (TYPESAFE_API_KEY).
  const policy = process.env.TYPESAFE_API_KEY
    ? new Policy({
        url: config.url,
        token: config.token,
        jev: new Jev(process.env.TYPESAFE_API_KEY, process.env.WRATHPLUS_JEV_MODEL ?? "jev-latest"),
        heartbeatMs: Number(process.env.WRATHPLUS_JEV_HEARTBEAT_MS ?? 20_000),
        onNotify: (summary) => tell("fast loop", summary),
        log: (line) => console.log(line),
      })
    : undefined;
  if (policy) void policy.run(abortController.signal);
  const opening = kickoff(config.goal, notes.briefing());
  inbox.push(opening);
  let self: string | undefined;
  void watch({
    url: config.url,
    token: config.token,
    self: () => self,
    signal: abortController.signal,
    onEvents: (events) => policy?.notice(events),
    onMessage: (text) => {
      console.log(`\n[events] ${text.replace(/\n/g, " ")}`);
      tell("events", text);
    },
  });
  // Operator notes from stdin.
  void (async () => {
    for await (const line of console) {
      if (line.trim()) tell("operator", `Note from the operator: ${line.trim()}`);
    }
  })();
  const mcpEntry = fileURLToPath(new URL("../../mcp/src/index.ts", import.meta.url));
  let checkIn: ReturnType<typeof setTimeout> | undefined;
  console.log(`WrathPlus example agent · ${config.model} · ${config.url}\nGoal: ${config.goal}\n`);
  console.log(policy ? "Fast tier: Jev on" : "Fast tier: off (no TYPESAFE_API_KEY)");
  console.log(`Notes: ${notes.home}`);
  const compactAt = Number(process.env.WRATHPLUS_AGENT_COMPACT_AT ?? 400_000);
  let begun = false;
  for await (const message of think(inbox, { ...config, mcpEntry, abortController, notes, compactAt, ...(policy ? { policy } : {}) })) {
    if (message.type === "system" && message.subtype === "init") {
      console.log(`[session ${message.session_id}]`);
      if (!begun) {
        begun = true;
        notes.begin(message.session_id);
        notes.record("goal", opening);
      }
    }
    if (message.type === "system" && message.subtype === "compact_boundary") {
      const tokens = message.compact_metadata.pre_tokens;
      console.log(`[compacted at ${tokens} tokens]`);
      tell(
        "compaction",
        `Your conversation was compacted (it had reached ${tokens} tokens). Your notes are unchanged:\n\n${notes.briefing()}`,
      );
    }
    const line = summarize(message);
    if (line) {
      console.log(line);
      notes.record(message.type === "result" ? "turn end" : "planner", line);
    }
    if (message.type === "user" && Array.isArray(message.message.content)) {
      for (const block of message.message.content) {
        if (typeof block === "object" && block.type === "tool_result") {
          const content = Array.isArray(block.content)
            ? block.content.map((c) => (c.type === "text" ? c.text : `[${c.type}]`)).join(" ")
            : String(block.content ?? "");
          notes.record("result", content.slice(0, 1500));
        }
      }
    }
    if (message.type === "result") {
      // A smoke run (a turn limit) ends with its first turn; a real run waits for news.
      if (config.maxTurns) {
        abortController.abort();
        inbox.close();
        break;
      }
      clearTimeout(checkIn);
      if (config.checkInSeconds > 0) {
        checkIn = setTimeout(
          () => tell("check-in", "Check in: look at the game, review your Lua's progress and carry on toward your goal."),
          config.checkInSeconds * 1000,
        );
      }
    } else if (message.type === "assistant") {
      clearTimeout(checkIn);
    }
    // Learn the character's name from the first status call that returns one.
    if (!self && message.type === "user") {
      const text = JSON.stringify(message.message.content);
      self = /\\"name\\":\s*\\"([A-Z][a-z]+)\\"/.exec(text)?.[1];
    }
  }
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}
