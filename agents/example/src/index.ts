#!/usr/bin/env bun
// The example agent: Claude plans and writes Lua through the client's MCP tools; game events
// arrive as messages while it works. Lines typed on stdin go to it too, as operator notes.
//
//   WRATHPLUS_AGENT_URL=http://127.0.0.1:17950 WRATHPLUS_AGENT_TOKEN=… \
//     bun run agents/example/src/index.ts "Reach level 5 in Northshire"
//
// Runs on the Claude Code login of this machine (a Claude subscription works); no API key needed.

import { appendFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { Inbox, summarize, think } from "./brain.ts";
import { controlLua } from "./goals.ts";
import { Jev } from "./jev.ts";
import { controlFiles, ModuleVersions } from "./modules.ts";
import { claudeAuth, loadEnv } from "./env.ts";
import { Notes } from "./notes.ts";
import { Policy } from "./policy.ts";
import { kickoff } from "./prompt.ts";
import { Reflection, XP_LUA, type XpSample } from "./reflect.ts";
import { Triage } from "./triage.ts";
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
  // Speech nearby that was not to us rides along with the next wake-up (the latest few lines), and
  // so do requests the fast tier carried out for the party.
  let heard: string[] = [];
  let handled: string[] = [];
  // Everything that reaches the planner also goes in the session log, with the character's
  // status at that moment, so a wake-up needs no calls to see where things stand.
  // A half-size frame of the game rides along with a wake-up, at most one every 30 s, so the
  // planner sees the scene without asking (WRATHPLUS_WAKE_SHOTS=0 turns it off).
  let shotAt = 0;
  const glance = async (): Promise<string | undefined> => {
    if (process.env.WRATHPLUS_WAKE_SHOTS === "0" || Date.now() - shotAt < 30_000) return undefined;
    try {
      const response = await fetch(`${config.url}/screenshot?scale=0.5`, {
        headers: { Authorization: `Bearer ${config.token}` },
        signal: AbortSignal.timeout(3_000),
      });
      if (!response.ok || !(response.headers.get("content-type") ?? "").startsWith("image/png")) return undefined;
      shotAt = Date.now();
      return Buffer.from(await response.arrayBuffer()).toString("base64");
    } catch {
      return undefined;
    }
  };
  const tell = (kind: string, text: string) => {
    void Promise.all([statusLine(config.url, config.token), glance()]).then(([status, png]) => {
      const nearby = heard.length ? `\n\nHeard nearby (no need to answer):\n- ${heard.join("\n- ")}` : "";
      const done = handled.length ? `\n\nDone for the party by the fast loop (already answered):\n- ${handled.join("\n- ")}` : "";
      heard = [];
      handled = [];
      const message = status ? `${text}${done}${nearby}\n\n[Now: ${status}]` : `${text}${done}${nearby}`;
      notes.record(kind, png ? `${message}\n\n[with a screenshot]` : message);
      inbox.push(message, png);
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
        goal: config.goal,
        onNotify: (summary) => tell("fast loop", summary),
        log: (line) => console.log(line),
      })
    : undefined;
  if (policy) void policy.run(abortController.signal);
  const lua = controlLua(config.url, config.token);
  // The goal runner (wp.goals) is off in a bare client, so a manual MCP session never plays by
  // itself; the example agent turns it on (WRATHPLUS_AGENT_GOALS=0 leaves it off) and off on exit.
  if (process.env.WRATHPLUS_AGENT_GOALS !== "0") {
    const on = await lua("if Goals then Goals.on = true return true end return false").catch(() => undefined);
    console.log(on?.ok && on.values?.[0] === true ? "Goal runner: on" : "Goal runner: unavailable (this client has no wp.goals)");
    abortController.signal.addEventListener("abort", () => void lua("if Goals then Goals.on = false end return true").catch(() => undefined));
  }
  // Every module the planner saves is kept as a version in its notes, for rollback (`modules.ts`).
  const versions = new ModuleVersions(notes.home);
  const files = controlFiles(config.url, config.token);
  // The reflection loop (`reflect.ts`): a review of how it is doing, between the planner's turns.
  let inTurn = false;
  const reflection =
    process.env.WRATHPLUS_AGENT_REFLECT === "0"
      ? undefined
      : new Reflection({
          onReflect: (text, score) => {
            appendFileSync(join(notes.home, "reflections.jsonl"), `${JSON.stringify({ at: new Date().toISOString(), score })}\n`);
            console.log(`[reflection] ${JSON.stringify(score)}`);
            tell("reflection", text);
          },
          quiet: () => !inTurn,
          fast: () => (policy ? { decisions: policy.decisions, holds: policy.holds, time: policy.time } : undefined),
          versions,
          periodMs: Number(process.env.WRATHPLUS_AGENT_REFLECT_MINUTES ?? 30) * 60_000,
        });
  if (reflection) {
    // Experience every minute for the scoreboard; whether a review is due every 15 s.
    let ticks = 0;
    const timer = setInterval(() => {
      if (ticks++ % 4 === 0) {
        void lua(XP_LUA)
          .then((r) => {
            const xp = r.ok ? (r.values?.[0] as XpSample | undefined) : undefined;
            if (xp && typeof xp.level === "number") reflection.sample(xp);
          })
          .catch(() => undefined);
      }
      reflection.check();
    }, 15_000);
    abortController.signal.addEventListener("abort", () => clearInterval(timer));
  }
  // Requests from the party, sorted by Jev before the planner hears them (`triage.ts`).
  const triage =
    policy && process.env.TYPESAFE_API_KEY && process.env.WRATHPLUS_AGENT_TRIAGE !== "0"
      ? new Triage(
          new Jev(process.env.TYPESAFE_API_KEY, process.env.WRATHPLUS_JEV_MODEL ?? "jev-latest"),
          lua,
          async (text, channel, to) => {
            await lua(`SendChatMessage(${JSON.stringify(text)}, ${JSON.stringify(channel)}, nil, ${JSON.stringify(to)}) return true`);
          },
        )
      : undefined;
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
    ...(triage ? { triage: async (request) => triage.handle(request, await statusLine(config.url, config.token), policy?.plan) } : {}),
    onHandled: (note) => {
      handled = [...handled, note].slice(-8);
    },
    onOverheard: (lines) => {
      heard = [...heard, ...lines].slice(-8);
    },
    signal: abortController.signal,
    onEvents: (events) => {
      policy?.notice(events);
      reflection?.notice(events);
      // Our own chat echoing back is not news to wake on.
      waker.notice(events.filter((e) => !(e.event.startsWith("CHAT_MSG_") && self !== undefined && e.args[1] === self)));
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
  for await (const message of think(inbox, { ...config, mcpEntry, abortController, notes, waker, compactAt, ...(policy ? { policy } : {}), lua, modules: { versions, files } })) {
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
      inTurn = false;
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
      inTurn = true;
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
