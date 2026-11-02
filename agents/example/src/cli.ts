#!/usr/bin/env bun
// Run the example agent on one of the Manager's agents: its headless client starts with it and
// stops with it. Several at once is several terminals; `dashboard` shows them all on one page.
//
//   bun run agent add <name>          a realm account, control port and token for a new agent
//   bun run agent list
//   bun run agent run <name> [--character <name>] [--model haiku|sonnet|<id>] [goal...]
//   bun run agent dashboard [--port 17900]
//
// Agents (each its own realm account, control port and token) are the Manager's: made here with
// `add` or on its Agents page. Keys come from agents/.env (see agents/.env.example).

import { join } from "node:path";
import { parseArgs } from "node:util";
import { serveDashboard } from "./dashboard.ts";
import { claudeAuth, envFile, loadEnv } from "./env.ts";
import { runAgent, type Config } from "./index.ts";
import {
  agentDir,
  answering,
  clearRunning,
  clientBinary,
  createAgent,
  findAgent,
  loadAgents,
  markRunning,
  runningOf,
  startClient,
  stateHome,
  tokenOf,
  urlOf,
  viewOf,
  waitReady,
} from "./manager.ts";

const MODELS: Record<string, string> = { haiku: "claude-haiku-5-5", sonnet: "claude-sonnet-5-5" };
const DEFAULT_GOAL = "Play your character: quest, level up and explore, as a thoughtful player would.";

const usage = () => `Usage:
  bun run agent add <name>
  bun run agent list
  bun run agent run <name> [--character <name>] [--model haiku|sonnet|<id>] [goal...]
  bun run agent dashboard [--port 17900]

Agents are shared with the Manager's Agents page. Keys: ${envFile()}`;

async function list(): Promise<void> {
  const home = stateHome();
  const agents = loadAgents(home);
  if (!agents.length) {
    console.log("No agents yet: bun run agent add <name>.");
    return;
  }
  for (const agent of agents) {
    const status = await answering(agent, tokenOf(home, agent));
    const running = runningOf(home, agent);
    const player = status?.player as { name?: string; level?: number } | undefined;
    const state = !status
      ? "client stopped"
      : status.in_world
        ? `in world as ${player?.name ?? "?"} (level ${player?.level ?? "?"})`
        : String(status.state);
    const driver = running ? `, driven by the example agent (${running.model})` : "";
    console.log(`${agent.name.padEnd(12)} ${agent.account.padEnd(16)} :${agent.port}  ${state}${driver}`);
  }
  console.log(`\nWatch them all: bun run agent dashboard`);
}

async function run(argv: string[]): Promise<void> {
  const { values, positionals } = parseArgs({
    args: argv,
    options: { character: { type: "string" }, model: { type: "string" } },
    allowPositionals: true,
  });
  const [name, ...goalWords] = positionals;
  if (!name) throw new Error(usage());
  const home = stateHome();
  const agent = findAgent(loadAgents(home), name);
  if (!agent) throw new Error(`No agent "${name}": bun run agent add <name> (bun run agent list).`);
  if (runningOf(home, agent)) throw new Error(`${agent.name} is already being driven by another example agent.`);
  const token = tokenOf(home, agent);
  const model = MODELS[values.model ?? ""] ?? values.model ?? process.env.WRATHPLUS_AGENT_MODEL ?? MODELS.haiku!;

  // Its client: the one already running (started from the Manager), or a new one that stops with us.
  let client = (await answering(agent, token)) ? undefined : startClient(home, agent, values.character);
  const abortController = new AbortController();
  const stop = () => {
    abortController.abort();
    client?.kill();
    clearRunning(home, agent);
  };
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
    process.on(signal, () => {
      stop();
      process.exit(130);
    });
  }
  process.on("exit", () => client?.kill());
  if (client) {
    const log = join(agentDir(home, agent.account), "client.log");
    console.log(`Starting ${agent.name}'s client (${clientBinary(home)})...`);
    await waitReady(agent, token, client, log);
  } else {
    console.log(`${agent.name}'s client is already running; driving it.`);
  }

  const config: Config = {
    url: urlOf(agent),
    token,
    goal: goalWords.join(" ").trim() || DEFAULT_GOAL,
    model,
    checkInSeconds: Number(process.env.WRATHPLUS_AGENT_CHECKIN_SECONDS ?? 0),
  };
  markRunning(home, agent, { pid: process.pid, port: agent.port, model, goal: config.goal, started: new Date().toISOString() });
  console.log(`Watch: ${viewOf(home, agent)}`);
  console.log(`Planner: ${model} via ${claudeAuth()}; fast tier: ${process.env.TYPESAFE_API_KEY ? "Jev" : "off (no TYPESAFE_API_KEY)"}\n`);
  try {
    await runAgent(config, { abortController, notesHome: join(agentDir(home, agent.account), "notes") });
  } finally {
    stop();
  }
}

async function main(): Promise<void> {
  loadEnv();
  const [command, ...rest] = process.argv.slice(2);
  switch (command) {
    case "add": {
      if (!rest[0]) throw new Error(usage());
      const agent = await createAgent(stateHome(), rest.join(" "));
      console.log(`Added ${agent.name}: account ${agent.account}, control port ${agent.port}. It has no character yet;\n` +
        `give it one with --character on the first run, or let it make one at character select.`);
      return;
    }
    case "list":
      return list();
    case "run":
      return run(rest);
    case "dashboard": {
      const { values } = parseArgs({ args: rest, options: { port: { type: "string" } } });
      const server = serveDashboard(Number(values.port ?? 17900));
      console.log(`Dashboard: ${server.url}`);
      return;
    }
    default:
      console.log(usage());
  }
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}
