// Shared test helpers: a sample builder, a scratch folder, and a stand-in control port for the
// judge (module files in memory, Lua calls recorded).

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AgentError } from "../src/client.ts";
import type { JudgePort } from "../src/judge.ts";
import type { Sample } from "../src/scorecard.ts";

export function scratch(): string {
  return mkdtempSync(join(tmpdir(), "wp-judge-"));
}

export function sample(at: number, over: Partial<Sample> = {}): Sample {
  return {
    at,
    client_time: at / 1000,
    name: "Agentsix",
    realm: "Beta",
    level: 10,
    xp: 0,
    xp_max: 10_000,
    money: 0,
    dead: false,
    combat: true,
    idle: false,
    zone: "Elwynn Forest",
    subzone: "Goldshire",
    gear: { average: 8, items: 10, empty: 7, uncached: 0 },
    professions: {},
    spells: 20,
    quests: {},
    deaths: 0,
    tampered: [],
    ui_reloaded: false,
    ...over,
  };
}

/** Module files in memory; every Lua chunk is recorded and answered ok unless `failLoad` says. */
export class FakePort implements JudgePort {
  files = new Map<string, string>();
  lua_calls: string[] = [];
  /** Sources whose load (AgentReload after saving them) fails. */
  failLoad = new Set<string>();

  async lua(code: string): Promise<unknown> {
    this.lua_calls.push(code);
    const name = /AgentReload\("([^"]+)"\)/.exec(code)?.[1];
    if (name && this.failLoad.has(this.files.get(name) ?? "")) return { ok: false, error: "syntax error near 'end'" };
    return { ok: true, values: [true] };
  }
  async readModule(name: string): Promise<string> {
    const source = this.files.get(name);
    if (source === undefined) throw new AgentError(`no module ${name}`, 404);
    return source;
  }
  async saveModule(name: string, source: string): Promise<unknown> {
    this.files.set(name, source);
    return { saved: name };
  }
  async deleteModule(name: string): Promise<unknown> {
    if (!this.files.delete(name)) throw new AgentError(`no module ${name}`, 404);
    return { deleted: name };
  }
}
