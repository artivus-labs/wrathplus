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
import { claudeAuth, loadEnv } from "./env.ts";
import { Notes } from "./notes.ts";
import { Policy } from "./policy.ts";
import { kickoff } from "./prompt.ts";
import { statusLine, Waker } from "./wake.ts";
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
    model: env.WRATHPLUS_AGENT_MODEL ?? "claude-haiku-5-5",
    checkInSeconds: Number(env.WRATHPLUS_AGENT_CHECKIN_SECONDS ?? 0),
    ...(maxTurns && maxTurns > 0 ? { maxTurns } : {}),
  };
}

export interface RunOptions {
  /** Where the agent keeps its notes (memory, to-dos, session logs). */
  notesHome: string;
  abortController: AbortController;
  /** Read operator notes from stdin (one agent per terminal). */
  stdin?: boolean;
}

/** Run the planner (and the fast tier, with a TypeSafe key) against a client until aborted. */
export async function runAgent(config: Config, options: RunOptions): Promise<void> {
  const { abortController } = options;
  const inbox = new Inbox();
  const notes = new Notes(options.notesHome);
  // Speech nearby that was not to us rides along with the next wake-up (the latest few lines).
  let heard: string[] = [];
  // Everything that reaches the planner also goes in the session log, with the character's
  // status at that moment, so a wake-up needs no calls to see where things stand.
  const tell = (kind: string, text: string) => {
    void statusLine(config.url, config.token).then((status) => {
      const nearby = heard.length ? `\n\nHeard nearby (no need to answer):\n- ${heard.join("\n- ")}` : "";
      heard = [];
      const message = (status ? `${text}${nearby}\n\n[Now: ${status}]` : `${text}${nearby}`);
      notes.record(kind, message);
      inbox.push(message);
    });
  };
  const waker = new Waker((text) => tell("wake-up", text));
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
  // The party's names, refreshed every 15 s: their /say is meant for us.
  let party = new Set<string>();
  const partyLua = 'local t = {} for i = 1, 4 do local n = UnitName("party" .. i) if n then t[#t + 1] = n end end return t';
  const refreshParty = async () => {
    try {
      const response = await fetch(`${config.url}/lua`, {
        method: "POST",
        headers: { Authorization: `Bearer ${config.token}` },
        body: partyLua,
        signal: AbortSignal.timeout(2000),
      });
      const answer = (await response.json()) as { values?: unknown[] };
      const names = Array.isArray(answer.values?.[0]) ? (answer.values[0] as string[]) : [];
      party = new Set(names.map((n) => n.toLowerCase()));
    } catch {
      // The client may be loading; keep the last list.
    }
  };
  void refreshParty();
  const partyTimer = setInterval(() => void refreshParty(), 15_000);
  abortController.signal.addEventListener("abort", () => clearInterval(partyTimer));
  void watch({
    url: config.url,
    token: config.token,
    self: () => self,
    party: () => party,
    npcReplies: process.env.WRATHPLUS_AGENT_NPC_REPLIES !== "0",
    onOverheard: (lines) => {
      heard = [...heard, ...lines].slice(-8);
    },
    signal: abortController.signal,
    onEvents: (events) => {
      policy?.notice(events);
      waker.notice(events);
    },
    onMessage: (text) => {
      console.log(`\n[events] ${text.replace(/\n/g, " ")}`);
      tell("events", text);
    },
  });
  // Operator notes from stdin.
  if (options.stdin ?? true) {
    void (async () => {
      for await (const line of console) {
        if (line.trim()) tell("operator", `Note from the operator: ${line.trim()}`);
      }
    })();
  }
  const mcpEntry = fileURLToPath(new URL("../../mcp/src/index.ts", import.meta.url));
  let checkIn: ReturnType<typeof setTimeout> | undefined;
  console.log(`WrathPlus example agent · ${config.model} (${claudeAuth()}) · ${config.url}\nGoal: ${config.goal}\n`);
  console.log(policy ? "Fast tier: Jev on" : "Fast tier: off (no TYPESAFE_API_KEY)");
  console.log(`Notes: ${notes.home}`);
  const compactAt = Number(process.env.WRATHPLUS_AGENT_COMPACT_AT ?? 400_000);
  let begun = false;
  for await (const message of think(inbox, { ...config, mcpEntry, abortController, notes, waker, compactAt, ...(policy ? { policy } : {}) })) {
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
      waker.turnEnded();
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

/** By hand, against a client someone else started: the control port and token from the env. */
async function main(): Promise<void> {
  loadEnv();
  const config = configFromEnv(process.argv.slice(2), process.env);
  const abortController = new AbortController();
  process.on("SIGINT", () => abortController.abort());
  await runAgent(config, {
    abortController,
    notesHome:
      process.env.WRATHPLUS_AGENT_HOME ??
      join(homedir(), ".local/share/wrathplus-agents", `agent-${new URL(config.url).port || "default"}`),
  });
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}
