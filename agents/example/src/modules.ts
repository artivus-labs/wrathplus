// Versions of the agent's own Lua modules, kept in its notes folder so a change that makes things
// worse can be rolled back. Every save_module the planner makes is recorded (the module as the
// client then holds it), with a changelog of why each change was made and what to watch.
//
//   <home>/modules/<name>/v0001.lua   one file per version, in order
//   <home>/modules/changelog.jsonl    { at, module, version, kind, why?, watch?, lines }

import { createSdkMcpServer, tool, type HookCallback } from "@anthropic-ai/claude-agent-sdk";
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import type { Lua } from "./goals.ts";

export type ChangeKind = "baseline" | "save" | "rollback" | "note";

export interface Change {
  at: string;
  module: string;
  version: number;
  kind: ChangeKind;
  why?: string;
  watch?: string;
  lines: number;
}

const MODULE_NAME = /^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)*$/;

export class ModuleVersions {
  readonly dir: string;

  constructor(home: string) {
    this.dir = join(home, "modules");
    mkdirSync(this.dir, { recursive: true });
  }

  #moduleDir(name: string): string {
    if (!MODULE_NAME.test(name)) throw new Error(`not a module name: ${name}`);
    return join(this.dir, name);
  }

  /** The version numbers kept for a module, oldest first. */
  versions(name: string): number[] {
    const dir = this.#moduleDir(name);
    if (!existsSync(dir)) return [];
    return readdirSync(dir)
      .map((f) => /^v(\d+)\.lua$/.exec(f)?.[1])
      .filter((v): v is string => v !== undefined)
      .map(Number)
      .sort((a, b) => a - b);
  }

  source(name: string, version: number): string | undefined {
    const path = join(this.#moduleDir(name), `v${String(version).padStart(4, "0")}.lua`);
    return existsSync(path) ? readFileSync(path, "utf8") : undefined;
  }

  latest(name: string): { version: number; source: string } | undefined {
    const version = this.versions(name).at(-1);
    if (version === undefined) return undefined;
    const source = this.source(name, version);
    return source === undefined ? undefined : { version, source };
  }

  /** Keep `source` as the module's next version, unless it is the same as the latest. */
  record(name: string, source: string, kind: Exclude<ChangeKind, "note">, why?: string): number {
    const latest = this.latest(name);
    if (latest && latest.source === source) return latest.version;
    const version = (latest?.version ?? 0) + 1;
    const dir = this.#moduleDir(name);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `v${String(version).padStart(4, "0")}.lua`), source);
    this.#log({ module: name, version, kind, lines: source.split("\n").length, ...(why ? { why } : {}) });
    return version;
  }

  /** Why the latest version was made, and what should show whether it worked. */
  note(name: string, why: string, watch?: string): string {
    const latest = this.latest(name);
    if (!latest) return `No versions of ${name} are kept yet: save it with save_module first.`;
    this.#log({ module: name, version: latest.version, kind: "note", why, lines: latest.source.split("\n").length, ...(watch ? { watch } : {}) });
    return `Noted against ${name} v${latest.version}.`;
  }

  changelog(name?: string, limit = 30): Change[] {
    const path = join(this.dir, "changelog.jsonl");
    if (!existsSync(path)) return [];
    const all = readFileSync(path, "utf8")
      .split("\n")
      .filter(Boolean)
      .flatMap((line) => {
        try {
          return [JSON.parse(line) as Change];
        } catch {
          return [];
        }
      });
    return (name ? all.filter((c) => c.module === name) : all).slice(-limit);
  }

  #log(change: Omit<Change, "at">): void {
    appendFileSync(join(this.dir, "changelog.jsonl"), `${JSON.stringify({ at: new Date().toISOString(), ...change })}\n`);
  }

  /** The changelog as lines for the planner. */
  describe(name?: string, limit = 30): string {
    const changes = this.changelog(name, limit);
    if (!changes.length) return name ? `No versions of ${name} are kept yet.` : "No module changes recorded yet.";
    const head = name ? `Versions of ${name}: ${this.versions(name).map((v) => `v${v}`).join(", ")}\n` : "";
    return head + changes.map(line).join("\n");
  }
}

export function line(c: Change): string {
  const what = c.kind === "note" ? "note" : `${c.kind}, ${c.lines} lines`;
  return `${c.at.slice(0, 16).replace("T", " ")} ${c.module} v${c.version} (${what})${c.why ? `: ${c.why}` : ""}${c.watch ? ` [watch: ${c.watch}]` : ""}`;
}

/** The control port's module files (the same ones save_module writes). */
export interface ModuleFiles {
  read(name: string): Promise<string | undefined>;
  write(name: string, source: string): Promise<void>;
}

export function controlFiles(url: string, token: string): ModuleFiles {
  const headers = { Authorization: `Bearer ${token}` };
  return {
    async read(name) {
      const response = await fetch(`${url}/files/${encodeURIComponent(name)}`, { headers, signal: AbortSignal.timeout(5000) });
      return response.ok ? response.text() : undefined;
    },
    async write(name, source) {
      const response = await fetch(`${url}/files/${encodeURIComponent(name)}`, { method: "PUT", headers, body: source });
      if (!response.ok) throw new Error(`saving ${name}: HTTP ${response.status} ${(await response.text()).slice(0, 200)}`);
    },
  };
}

export const SAVE_MODULE_TOOL = "mcp__wrathplus__save_module";

/** Hooks around save_module: before the first save of a module this harness sees, keep what the
 *  client already has (the baseline); after every save, keep what the client now holds. */
export function versionHooks(versions: ModuleVersions, files: ModuleFiles): { pre: HookCallback; post: HookCallback } {
  const nameOf = (input: unknown): string | undefined => {
    const name = (input as { name?: unknown } | null)?.name;
    return typeof name === "string" && MODULE_NAME.test(name) ? name : undefined;
  };
  return {
    pre: async (input) => {
      if (input.hook_event_name !== "PreToolUse" || input.tool_name !== SAVE_MODULE_TOOL) return {};
      const name = nameOf(input.tool_input);
      if (name && !versions.latest(name)) {
        const current = await files.read(name).catch(() => undefined);
        if (current !== undefined) versions.record(name, current, "baseline", "as found on the client");
      }
      return {};
    },
    post: async (input) => {
      if (input.hook_event_name !== "PostToolUse" || input.tool_name !== SAVE_MODULE_TOOL) return {};
      const name = nameOf(input.tool_input);
      if (!name) return {};
      const saved = await files.read(name).catch(() => undefined);
      if (saved !== undefined) versions.record(name, saved, "save");
      return {};
    },
  };
}

/** Put an earlier version back on the client, reload it, and keep it as the newest version. */
export async function rollback(
  versions: ModuleVersions,
  files: ModuleFiles,
  lua: Lua,
  name: string,
  version: number,
  why: string,
): Promise<string> {
  const source = versions.source(name, version);
  if (source === undefined) {
    const kept = versions.versions(name);
    return kept.length ? `${name} has no v${version}; kept: ${kept.map((v) => `v${v}`).join(", ")}.` : `No versions of ${name} are kept.`;
  }
  await files.write(name, source);
  const now = versions.record(name, source, "rollback", `back to v${version}: ${why}`);
  const reload = await lua(`if not (__wp_loaded and __wp_loaded[${JSON.stringify(name)}]) then return "not loaded" end AgentReload(${JSON.stringify(name)}) return "reloaded"`);
  const state = reload.ok ? String(reload.values?.[0]) : `reload failed: ${reload.error}`;
  return `${name} is back to v${version} (kept as v${now}); ${state === "not loaded" ? "it was not loaded, so require it when you want it" : state}.`;
}

export const MODULE_TOOLS = ["module_versions", "note_change", "rollback_module"].map((name) => `mcp__harness__${name}`);

const text = (value: string) => ({ content: [{ type: "text" as const, text: value }] });

/** The planner's tools over its module history. */
export function harnessServer(versions: ModuleVersions, files: ModuleFiles, lua: Lua) {
  return createSdkMcpServer({
    name: "harness",
    version: "0.1.0",
    tools: [
      tool(
        "module_versions",
        "Your Lua modules' history: every save_module is kept as a version, with the changelog of why. With a name, that module's versions and changes; without, the latest changes to all of them. Read it before reworking a module and when judging whether a change helped.",
        { name: z.string().regex(MODULE_NAME).optional(), limit: z.number().int().min(1).max(200).optional() },
        async ({ name, limit }) => text(versions.describe(name, limit)),
      ),
      tool(
        "note_change",
        "Say why you made the latest save of a module and what should show whether it worked (a metric from the reflection scoreboard, a kind of failure that should stop). Call it right after save_module; your next reflection checks it.",
        { name: z.string().regex(MODULE_NAME), why: z.string(), watch: z.string().optional() },
        async ({ name, why, watch }) => text(versions.note(name, why, watch)),
      ),
      tool(
        "rollback_module",
        "Put an earlier version of a module back on the client (and reload it if it is loaded) when a change made things worse. The rollback is kept as a new version, so it can be undone too.",
        { name: z.string().regex(MODULE_NAME), version: z.number().int().min(1), why: z.string() },
        async ({ name, version, why }) => {
          try {
            return text(await rollback(versions, files, lua, name, version, why));
          } catch (error) {
            return text(`Rollback failed: ${error instanceof Error ? error.message : String(error)}`);
          }
        },
      ),
    ],
  });
}
