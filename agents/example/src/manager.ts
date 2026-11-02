// The agents the Manager (the launcher) knows, and their clients: the same registry and private
// files its Agents page uses (`crates/wrathplus-manager/src/agents.rs`), so an agent made there
// runs here, and one running here shows there.
//
//   <state home>/agents/agents.json          [{ name, account, port }]
//   <state home>/agents/<ACCOUNT>/password    the realm account's password (0600)
//   <state home>/agents/<ACCOUNT>/token       the control port's token (0600)
//   <state home>/agents/<ACCOUNT>/home/       the client's own settings, cache and saved variables
//   <state home>/agents/<ACCOUNT>/notes/      the example agent's memory, to-dos and session logs
//   <state home>/agents/<ACCOUNT>/client.log  the client's log
//   <state home>/agents/<ACCOUNT>/running.json  while this example agent drives it

import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { Subprocess } from "bun";

/** The managed realm (`packaging/server/runtime.sh`), as the Manager's agents use it. */
const REALM_ADDRESS = "127.0.0.1:13724";
const REALM_NAME = "WrathPlus Local";

export interface ManagedAgent {
  name: string;
  account: string;
  port: number;
}

export interface Running {
  pid: number;
  port: number;
  model: string;
  goal: string;
  started: string;
}

/** The checkout this file is in: the main one, even from a linked worktree (state lives there). */
export function mainCheckout(from = import.meta.dir): string {
  const run = Bun.spawnSync(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd: from });
  if (run.exitCode !== 0) throw new Error("Not inside a WrathPlus checkout (git rev-parse failed).");
  return dirname(run.stdout.toString().trim());
}

/** `wrathplus_formats::default_state_home`: a legacy `benilla-config` wins, else `wrathplus-config`. */
export function stateHome(env = process.env): string {
  if (env.WRATHPLUS_STATE_HOME) return env.WRATHPLUS_STATE_HOME;
  const root = mainCheckout();
  const legacy = join(root, "benilla-config");
  return existsSync(legacy) ? legacy : join(root, "wrathplus-config");
}

export const agentDir = (home: string, account: string) => join(home, "agents", account);

export function loadAgents(home = stateHome()): ManagedAgent[] {
  const path = join(home, "agents", "agents.json");
  return existsSync(path) ? (JSON.parse(readFileSync(path, "utf8")) as ManagedAgent[]) : [];
}

/** By name or account, case-insensitive. */
export function findAgent(agents: ManagedAgent[], key: string): ManagedAgent | undefined {
  const k = key.toLowerCase();
  return agents.find((a) => a.name.toLowerCase() === k || a.account.toLowerCase() === k);
}

function secret(dir: string, file: string): string {
  const path = join(dir, file);
  if (!existsSync(path)) throw new Error(`${path} is missing: recreate the agent in the Manager.`);
  return readFileSync(path, "utf8").trim();
}

export const tokenOf = (home: string, agent: ManagedAgent) => secret(agentDir(home, agent.account), "token");
export const urlOf = (agent: ManagedAgent) => `http://127.0.0.1:${agent.port}`;
export const viewOf = (home: string, agent: ManagedAgent) => `${urlOf(agent)}/view?token=${tokenOf(home, agent)}`;

interface ManagerConfig {
  data: string;
  locale: string;
  repo: string;
  target: string;
}

function managerConfig(home: string): ManagerConfig {
  const path = join(home, "manager.json");
  if (!existsSync(path)) throw new Error(`${path} is missing: set the Manager up first (Client page).`);
  return JSON.parse(readFileSync(path, "utf8")) as ManagerConfig;
}

/** The client binary: WRATHPLUS_CLIENT, else the Manager's discovery order (`setup::discover`). */
export function clientBinary(home: string, env = process.env): string {
  if (env.WRATHPLUS_CLIENT) return env.WRATHPLUS_CLIENT;
  const config = managerConfig(home);
  const name = process.platform === "win32" ? "wrathplus.exe" : "wrathplus";
  const found = [
    join(config.target, "manager/bin", name),
    join(config.target, name),
    join(config.target, "bin", name),
    join(config.target, "play", name),
    join(config.target, "release", name),
    join(config.repo, "target/azb-live/bin", name),
  ].find((path) => existsSync(path) && (statSync(path).mode & 0o111) !== 0);
  if (!found) throw new Error("No client build found: build it on the Manager's Client page, or set WRATHPLUS_CLIENT.");
  return found;
}

/** The realm's navmesh, for AgentMoveTo: WOW_NAV_MMAPS, else the managed server's data folder. */
export function mmaps(home: string, env = process.env): string | undefined {
  if (env.WOW_NAV_MMAPS) return env.WOW_NAV_MMAPS;
  const path = join(home, "manager-server.json");
  if (!existsSync(path)) return undefined;
  const data = (JSON.parse(readFileSync(path, "utf8")) as { data?: string }).data;
  return data && existsSync(join(data, "mmaps")) ? join(data, "mmaps") : undefined;
}

/** Is something answering on the agent's control port? */
export async function answering(agent: ManagedAgent, token: string): Promise<Record<string, unknown> | null> {
  try {
    const response = await fetch(`${urlOf(agent)}/status`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(1500),
    });
    return response.ok ? ((await response.json()) as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** Start the agent's headless client, as the Manager's Start does, logged in to the managed realm. */
export function startClient(home: string, agent: ManagedAgent, character?: string): Subprocess {
  const dir = agentDir(home, agent.account);
  const config = managerConfig(home);
  const nav = mmaps(home);
  const log = Bun.file(join(dir, "client.log"));
  return Bun.spawn(
    [
      clientBinary(home),
      ...["--server", REALM_ADDRESS, "--account", agent.account, "--password-env", "WRATHPLUS_AGENT_PASSWORD"],
      ...["--realm", REALM_NAME, "--agent-port", String(agent.port)],
      ...(character ? ["--character", character, "--enter-world"] : []),
    ],
    {
      cwd: dir,
      env: {
        ...process.env,
        WOW_DATA: config.data,
        WOW_CLIENT_PROFILE: "wrath",
        WOW_LOCALE: config.locale,
        BENILLA_HOME: join(dir, "home"),
        WOW_HEADLESS: "1",
        WOW_AGENT_TOKEN: tokenOf(home, agent),
        WRATHPLUS_AGENT_PASSWORD: secret(dir, "password"),
        ...(nav ? { WOW_NAV_MMAPS: nav } : {}),
      },
      stdin: "ignore",
      stdout: log,
      stderr: log,
    },
  );
}

/** Wait until the client is past loading: in the world, or at character select with its UI up. */
export async function waitReady(agent: ManagedAgent, token: string, client: Subprocess, timeoutMs = 120_000): Promise<Record<string, unknown>> {
  const until = Date.now() + timeoutMs;
  while (Date.now() < until) {
    if (client.exitCode !== null) throw new Error(`The client exited (code ${client.exitCode}); see its client.log.`);
    const status = await answering(agent, token);
    const player = status?.player as { level?: number } | undefined;
    if (status?.in_world && (player?.level ?? 0) > 0) return status;
    if (status?.ui_ready && status.state === "CharSelect") return status;
    await Bun.sleep(1000);
  }
  throw new Error(`The client did not reach the world or character select in ${timeoutMs / 1000}s.`);
}

export function markRunning(home: string, agent: ManagedAgent, running: Running): void {
  writeFileSync(join(agentDir(home, agent.account), "running.json"), JSON.stringify(running, null, 2));
}

export function clearRunning(home: string, agent: ManagedAgent): void {
  rmSync(join(agentDir(home, agent.account), "running.json"), { force: true });
}

/** This example agent's record for a running agent, if its process is still alive. */
export function runningOf(home: string, agent: ManagedAgent): Running | null {
  const path = join(agentDir(home, agent.account), "running.json");
  if (!existsSync(path)) return null;
  const running = JSON.parse(readFileSync(path, "utf8")) as Running;
  try {
    process.kill(running.pid, 0);
    return running;
  } catch {
    return null;
  }
}

const SOAP_URL = "http://127.0.0.1:17878/";
const SOAP_ADMIN = "WPMANAGER";
const FIRST_PORT = 17950;

/** `Scout` → `AGENTSCOUT`, as the Manager names agent accounts (`agents::account_for`). */
export function accountFor(name: string): string {
  const base = name.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
  if (!base) throw new Error("Give the agent a name with letters or digits.");
  return `AGENT${base}`.slice(0, 16);
}

const xml = (text: string) => text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

/** A command on the managed realm's admin port, as the Manager sends them. */
export async function adminCommand(home: string, command: string): Promise<string> {
  const secretPath = join(home, "managed-server", "wrathplus-managed-local", "manager-admin.secret");
  if (!existsSync(secretPath)) throw new Error("The managed realm has no admin login yet: start it from the Manager's Server page.");
  const auth = Buffer.from(`${SOAP_ADMIN}:${readFileSync(secretPath, "utf8").trim()}`).toString("base64");
  const response = await fetch(SOAP_URL, {
    method: "POST",
    headers: { Authorization: `Basic ${auth}`, "Content-Type": "text/xml; charset=utf-8" },
    body: `<?xml version="1.0" encoding="utf-8"?><SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" xmlns:ns1="urn:AC"><SOAP-ENV:Body><ns1:executeCommand><command>${xml(command)}</command></ns1:executeCommand></SOAP-ENV:Body></SOAP-ENV:Envelope>`,
    signal: AbortSignal.timeout(60_000),
  }).catch((e) => {
    throw new Error(`The realm's command port is not answering (${e}); is the server running?`);
  });
  const text = await response.text();
  const fault = /<faultstring>([\s\S]*?)<\/faultstring>/.exec(text)?.[1];
  if (!response.ok || fault) throw new Error(`Realm command failed: ${(fault ?? text).trim().slice(0, 200)}`);
  return /<result>([\s\S]*?)<\/result>/.exec(text)?.[1]?.trim() ?? "";
}

function secretWord(length: number): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  return Array.from(crypto.getRandomValues(new Uint8Array(length)), (b) => alphabet[b % alphabet.length]).join("");
}

/** Make the realm account and remember the agent, as the Manager's Add does (`agents::create`). */
export async function createAgent(home: string, name: string): Promise<ManagedAgent> {
  const agents = loadAgents(home);
  const account = accountFor(name);
  if (agents.some((a) => a.account === account)) throw new Error(`An agent already uses the account ${account}.`);
  const password = secretWord(14);
  await adminCommand(home, `.account create ${account} ${password}`);
  // Wrath content needs the expansion flag on the account. The realm creates accounts
  // asynchronously, so the flag can briefly find no account yet.
  for (let attempt = 1; ; attempt++) {
    try {
      await adminCommand(home, `.account set addon ${account} 2`);
      break;
    } catch (error) {
      if (attempt >= 20 || !String(error).includes("not exist")) throw error;
      await Bun.sleep(250);
    }
  }
  const dir = agentDir(home, account);
  mkdirSync(join(dir, "home"), { recursive: true, mode: 0o700 });
  writeFileSync(join(dir, "password"), password, { mode: 0o600 });
  writeFileSync(join(dir, "token"), secretWord(32), { mode: 0o600 });
  let port = FIRST_PORT;
  while (agents.some((a) => a.port === port)) port++;
  const agent = { name: name.trim(), account, port };
  writeFileSync(join(home, "agents", "agents.json"), JSON.stringify([...agents, agent], null, 2));
  return agent;
}
