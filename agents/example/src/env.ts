// Keys for the agents, from `agents/.env` in the main checkout (git-ignored, shared by linked
// worktrees; see `agents/.env.example`): KEY=value lines, `#` comments. The shell wins.
//
// The planner runs on this machine's Claude Code login (a Claude subscription) unless
// WRATHPLUS_ANTHROPIC_API_KEY is set: a stray ANTHROPIC_API_KEY in the shell would otherwise
// replace the login, and an old one fails with "Invalid API key".

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { mainCheckout } from "./manager.ts";

export const envFile = () => join(mainCheckout(), "agents", ".env");

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
