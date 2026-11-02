#!/usr/bin/env bun
// The example agent: Claude plans and writes Lua through the client's MCP tools; game events
// arrive as messages while it works. Lines typed on stdin go to it too, as operator notes.
//
//   WRATHPLUS_AGENT_URL=http://127.0.0.1:17950 WRATHPLUS_AGENT_TOKEN=… \
//     bun run agents/example/src/index.ts "Reach level 5 in Northshire"
//
// Runs on the Claude Code login of this machine (a Claude subscription works); no API key needed.

import { fileURLToPath } from "node:url";
import { Inbox, summarize, think } from "./brain.ts";
import { Jev } from "./jev.ts";
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
    checkInSeconds: Number(env.WRATHPLUS_AGENT_CHECKIN_SECONDS ?? 120),
    ...(maxTurns && maxTurns > 0 ? { maxTurns } : {}),
  };
}

async function main(): Promise<void> {
  const config = configFromEnv(process.argv.slice(2), process.env);
  const abortController = new AbortController();
  process.on("SIGINT", () => abortController.abort());
  const inbox = new Inbox();
  // The fast tier, when a TypeSafe key is available (TYPESAFE_API_KEY).
  const policy = process.env.TYPESAFE_API_KEY
    ? new Policy({
        url: config.url,
        token: config.token,
        jev: new Jev(process.env.TYPESAFE_API_KEY, process.env.WRATHPLUS_JEV_MODEL ?? "jev-latest"),
        intervalMs: Number(process.env.WRATHPLUS_JEV_INTERVAL_MS ?? 1500),
        onStalled: (summary) => inbox.push(summary),
        log: (line) => console.log(line),
      })
    : undefined;
  if (policy) void policy.run(abortController.signal);
  inbox.push(kickoff(config.goal));
  let self: string | undefined;
  void watch({
    url: config.url,
    token: config.token,
    self: () => self,
    signal: abortController.signal,
    onMessage: (text) => {
      console.log(`\n[events] ${text.replace(/\n/g, " ")}`);
      for (const line of text.split("\n- ").slice(1)) policy?.note(line);
      inbox.push(text);
    },
  });
  // Operator notes from stdin.
  void (async () => {
    for await (const line of console) {
      if (line.trim()) inbox.push(`Note from the operator: ${line.trim()}`);
    }
  })();
  const mcpEntry = fileURLToPath(new URL("../../mcp/src/index.ts", import.meta.url));
  let checkIn: ReturnType<typeof setTimeout> | undefined;
  console.log(`WrathPlus example agent · ${config.model} · ${config.url}\nGoal: ${config.goal}\n`);
  console.log(policy ? "Fast tier: Jev on" : "Fast tier: off (no TYPESAFE_API_KEY)");
  for await (const message of think(inbox, { ...config, mcpEntry, abortController, ...(policy ? { policy } : {}) })) {
    if (message.type === "system" && message.subtype === "init") {
      console.log(`[session ${message.session_id}]`);
    }
    const line = summarize(message);
    if (line) console.log(line);
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
          () => inbox.push("Check in: look at the game, review your Lua's progress and carry on toward your goal."),
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
