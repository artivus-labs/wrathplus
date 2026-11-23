// Keys for the agents, from `agents/.env` in the main checkout (git-ignored, shared by linked
// worktrees; see `agents/.env.example`): KEY=value lines, `#` comments. The shell wins.
//
// The planner runs on this machine's Claude Code login (a Claude subscription) unless
// WRATHPLUS_ANTHROPIC_API_KEY is set: a stray ANTHROPIC_API_KEY in the shell would otherwise
// replace the login, and an old one fails with "Invalid API key".

import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { mainCheckout } from "./manager.ts";

/** `agents/.env` in the main checkout; outside a git checkout (a source archive), the one beside
 *  this package. */
export function envFile(from = import.meta.dir): string {
  try {
    return join(mainCheckout(from), "agents", ".env");
  } catch {
    return join(from, "..", "..", ".env");
  }
}

export function parseEnv(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const at = line.indexOf("=");
    if (at <= 0) continue;
    const key = line.slice(0, at).replace(/^export\s+/, "").trim();
    let value = line.slice(at + 1).trim();
    if (/^(['"]).*\1$/.test(value)) value = value.slice(1, -1);
    out[key] = value;
  }
  return out;
}

/** Load `agents/.env` into the process environment (shell values win). */
export function loadEnv(path = envFile(), env: Record<string, string | undefined> = process.env): void {
  if (!existsSync(path)) return;
  for (const [key, value] of Object.entries(parseEnv(readFileSync(path, "utf8")))) {
    if (env[key] === undefined) env[key] = value;
  }
}

/** The environment the planner's Claude Code process gets: the login, or the key asked for. */
export function claudeEnv(env: Record<string, string | undefined> = process.env): Record<string, string | undefined> {
  const { ANTHROPIC_API_KEY: _shell, WRATHPLUS_ANTHROPIC_API_KEY: chosen, ...rest } = env;
  return chosen ? { ...rest, ANTHROPIC_API_KEY: chosen } : rest;
}

export function claudeAuth(env: Record<string, string | undefined> = process.env): string {
  return env.WRATHPLUS_ANTHROPIC_API_KEY ? "API key (WRATHPLUS_ANTHROPIC_API_KEY)" : "Claude Code login";
}

const tokenFiles = new Map<string, string>();

/**
 * The control-port token in a file only this user can read (a fresh 0700 folder, removed at exit),
 * for the MCP server's `WRATHPLUS_AGENT_TOKEN_FILE`. The Agent SDK starts Claude Code with the MCP
 * servers' settings on its command line, where any local user could read a token with `ps`.
 */
export function tokenFile(token: string, dir = tmpdir()): string {
  const known = tokenFiles.get(`${dir}\0${token}`);
  if (known && existsSync(known)) return known;
  const folder = mkdtempSync(join(dir, "wrathplus-token-"));
  chmodSync(folder, 0o700);
  const path = join(folder, "token");
  writeFileSync(path, token, { mode: 0o600 });
  tokenFiles.set(`${dir}\0${token}`, path);
  if (tokenFiles.size === 1) process.on("exit", () => {
    for (const file of tokenFiles.values()) rmSync(dirname(file), { recursive: true, force: true });
  });
  return path;
}
