import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { harnessServer, ModuleVersions, rollback, versionHooks, type ModuleFiles } from "../src/modules.ts";
import { connect } from "./mcp.ts";

const scratch = () => mkdtempSync(join(tmpdir(), "wp-modules-"));

/** The client's module files, in memory. */
function client(initial: Record<string, string> = {}): ModuleFiles & { files: Map<string, string> } {
  const files = new Map(Object.entries(initial));
  return {
    files,
    read: async (name) => files.get(name),
    write: async (name, source) => void files.set(name, source),
  };
}

const hookInput = (event: "PreToolUse" | "PostToolUse", tool_name: string, tool_input: unknown) =>
  ({ hook_event_name: event, tool_name, tool_input, tool_response: {}, tool_use_id: "t", session_id: "s", transcript_path: "", cwd: "" }) as never;
const opts = { signal: new AbortController().signal };

test("versions are kept in order, an unchanged save adds none, and notes go in the changelog", () => {
  const home = scratch();
  const v = new ModuleVersions(home);
  expect(v.versions("grind.loop")).toEqual([]);
  expect(v.record("grind.loop", "return 1", "save")).toBe(1);
  expect(v.record("grind.loop", "return 1", "save")).toBe(1);
  expect(v.record("grind.loop", "return 2\n", "save")).toBe(2);
  expect(v.note("grind.loop", "pull one at a time", "deaths")).toBe("Noted against grind.loop v2.");
  expect(v.note("other", "x")).toContain("No versions of other");
  expect(new ModuleVersions(home).versions("grind.loop")).toEqual([1, 2]);
  expect(v.source("grind.loop", 1)).toBe("return 1");
  expect(v.changelog("grind.loop").map((c) => [c.version, c.kind, c.why ?? null, c.watch ?? null])).toEqual([
    [1, "save", null, null],
    [2, "save", null, null],
    [2, "note", "pull one at a time", "deaths"],
  ]);
  expect(v.describe("grind.loop")).toContain("Versions of grind.loop: v1, v2");
  expect(v.describe("grind.loop")).toContain("grind.loop v2 (note): pull one at a time [watch: deaths]");
  expect(() => v.versions("../escape")).toThrow("not a module name");
  rmSync(home, { recursive: true });
});

test("save_module is versioned around the call: the client's copy first, then the new one", async () => {
  const home = scratch();
  const v = new ModuleVersions(home);
  const c = client({ autoexec: "-- the old autoexec" });
  const hooks = versionHooks(v, c);
  // The first save this harness sees keeps what the client had.
  await hooks.pre(hookInput("PreToolUse", "mcp__wrathplus__save_module", { name: "autoexec", source: "-- new" }), "t", opts);
  c.files.set("autoexec", "-- new");
  await hooks.post(hookInput("PostToolUse", "mcp__wrathplus__save_module", { name: "autoexec", source: "-- new" }), "t", opts);
  // Later saves need no baseline; a brand-new module has none.
  await hooks.pre(hookInput("PreToolUse", "mcp__wrathplus__save_module", { name: "autoexec", source: "-- newer" }), "t", opts);
  c.files.set("autoexec", "-- newer");
  await hooks.post(hookInput("PostToolUse", "mcp__wrathplus__save_module", { name: "autoexec", source: "-- newer" }), "t", opts);
  await hooks.pre(hookInput("PreToolUse", "mcp__wrathplus__save_module", { name: "fresh", source: "x" }), "t", opts);
  // Other tools are not versioned.
  await hooks.post(hookInput("PostToolUse", "mcp__wrathplus__lua", { code: "return 1" }), "t", opts);
  expect(v.changelog().map((c) => `${c.module} v${c.version} ${c.kind}`)).toEqual([
    "autoexec v1 baseline",
    "autoexec v2 save",
    "autoexec v3 save",
  ]);
  expect(v.source("autoexec", 1)).toBe("-- the old autoexec");
  rmSync(home, { recursive: true });
});

test("a rollback puts the old version back, reloads a loaded module, and is itself a version", async () => {
  const home = scratch();
  const v = new ModuleVersions(home);
  v.record("grind.loop", "-- v1", "save");
  v.record("grind.loop", "-- v2 (worse)", "save");
  const c = client({ "grind.loop": "-- v2 (worse)" });
  const sent: string[] = [];
  let loaded = "reloaded";
  const lua = async (code: string) => (sent.push(code), { ok: true, values: [loaded] });
  expect(await rollback(v, c, lua, "grind.loop", 1, "deaths went from 1 to 4")).toBe("grind.loop is back to v1 (kept as v3); reloaded.");
  expect(c.files.get("grind.loop")).toBe("-- v1");
  expect(sent[0]).toContain('AgentReload("grind.loop")');
  expect(v.changelog().at(-1)).toMatchObject({ version: 3, kind: "rollback", why: "back to v1: deaths went from 1 to 4" });
  expect(await rollback(v, c, lua, "grind.loop", 9, "x")).toBe("grind.loop has no v9; kept: v1, v2, v3.");
  expect(await rollback(v, c, lua, "nothing", 1, "x")).toBe("No versions of nothing are kept.");
  loaded = "not loaded";
  expect(await rollback(v, c, lua, "grind.loop", 2, "try again")).toContain("it was not loaded, so require it");
  rmSync(home, { recursive: true });
});

test("the planner's module tools: history, notes and rollback", async () => {
  const home = scratch();
  const v = new ModuleVersions(home);
  v.record("rotation.mine", "-- a", "save");
  v.record("rotation.mine", "-- b", "save");
  const c = client();
  const call = await connect(harnessServer(v, c, async () => ({ ok: true, values: ["reloaded"] })));
  expect((await call("note_change", { name: "rotation.mine", why: "Rend first", watch: "seconds per fight" })).text).toBe("Noted against rotation.mine v2.");
  expect((await call("module_versions", { name: "rotation.mine" })).text).toContain("v1, v2");
  expect((await call("module_versions")).text).toContain("rotation.mine v2 (note): Rend first [watch: seconds per fight]");
  expect((await call("rollback_module", { name: "rotation.mine", version: 1, why: "slower" })).text).toBe("rotation.mine is back to v1 (kept as v3); reloaded.");
  expect(c.files.get("rotation.mine")).toBe("-- a");
  const failing = await connect(harnessServer(v, { read: async () => undefined, write: async () => { throw new Error("HTTP 500"); } }, async () => ({ ok: true })));
  expect((await failing("rollback_module", { name: "rotation.mine", version: 2, why: "x" })).text).toBe("Rollback failed: HTTP 500");
  rmSync(home, { recursive: true });
});
