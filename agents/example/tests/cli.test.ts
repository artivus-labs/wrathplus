import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { recentLines } from "../src/dashboard.ts";
import { claudeEnv, envFile, loadEnv, parseEnv } from "../src/env.ts";
import { NO_FAST_TIER, SYSTEM_PROMPT, systemPrompt } from "../src/prompt.ts";
import { accountFor, exitReason, findAgent, needsRestart, stuckReason } from "../src/manager.ts";

test("agents/.env: KEY=value lines, comments and quotes; the shell wins", () => {
  expect(parseEnv("# keys\nTYPESAFE_API_KEY=abc\nexport A='q r'\nB=\"x\"\nnot a line\n=nokey\n")).toEqual({
    TYPESAFE_API_KEY: "abc",
    A: "q r",
    B: "x",
  });
  const dir = mkdtempSync(join(tmpdir(), "wp-env-"));
  writeFileSync(join(dir, ".env"), "TYPESAFE_API_KEY=from-file\nOTHER=1\n");
  const env: Record<string, string | undefined> = { TYPESAFE_API_KEY: "from-shell" };
  loadEnv(join(dir, ".env"), env);
  expect(env).toEqual({ TYPESAFE_API_KEY: "from-shell", OTHER: "1" });
});

test("the planner uses the Claude Code login unless an agent key is chosen", () => {
  const shell = { ANTHROPIC_API_KEY: "stale", PATH: "/bin" };
  expect(claudeEnv(shell)).toEqual({ PATH: "/bin" });
  expect(claudeEnv({ ...shell, WRATHPLUS_ANTHROPIC_API_KEY: "chosen" })).toEqual({ PATH: "/bin", ANTHROPIC_API_KEY: "chosen" });
  expect(claudeEnv({ ...shell, WRATHPLUS_ANTHROPIC_API_KEY: "" })).toEqual({ PATH: "/bin" });
});

test("agent accounts are named as the Manager names them, and found by name or account", () => {
  expect(accountFor("Scout")).toBe("AGENTSCOUT");
  expect(accountFor("night owl 2!")).toBe("AGENTNIGHTOWL2");
  expect(accountFor("a very long agent name")).toHaveLength(16);
  expect(() => accountFor("!!")).toThrow("letters or digits");
  const agents = [{ name: "One", account: "AGENTONE", port: 17950 }];
  expect(findAgent(agents, "one")?.port).toBe(17950);
  expect(findAgent(agents, "agentone")?.name).toBe("One");
  expect(findAgent(agents, "two")).toBeUndefined();
});

test("the dashboard shows the planner's latest lines without the status suffix", () => {
  const notes = mkdtempSync(join(tmpdir(), "wp-notes-"));
  const sessions = join(notes, "sessions");
  mkdirSync(sessions);
  const line = (kind: string, text: string) => JSON.stringify({ at: "2026-11-03T01:02:03.000Z", session: "s", kind, text });
  writeFileSync(
    join(sessions, "2026-11-03-a.jsonl"),
    [line("planner", "→ status {}"), line("result", "{...}"), line("events", "Game events:\n- You died.\n\n[Now: level 4]")].join("\n"),
  );
  expect(recentLines(notes)).toEqual(["01:02:03 → status {}", "01:02:03 Game events:\n- You died.\n"]);
  expect(recentLines(join(notes, "missing"))).toEqual([]);
});

test("a client that exits at start says why, with a hint for an old build", () => {
  const old = 'unknown argument "--agent-port"\nusage: wrathplus [--server <host[:port]>]\n2026-11-03T01:00:00Z  WARN x: noise\n';
  expect(exitReason(old)).toBe(
    ':\n  unknown argument "--agent-port"\n  usage: wrathplus [--server <host[:port]>]\nThis client build predates the agent port: rebuild it (Manager, Client page) or set WRATHPLUS_CLIENT.',
  );
  expect(exitReason("2026-11-03T01:00:00Z ERROR wrathplus: no WOW_DATA\n")).toBe(":\n  wrathplus: no WOW_DATA");
  expect(exitReason("")).toBe("");
});

test("a client the agent started is restarted after a disconnect or an exit, not a logout", () => {
  expect(needsRestart({ state: "Login" }, true, false)).toBe(true);
  expect(needsRestart({ state: "Login" }, false, false)).toBe(false);
  expect(needsRestart({ state: "CharSelect" }, true, false)).toBe(false);
  expect(needsRestart({ state: "InWorld", in_world: true }, true, false)).toBe(false);
  expect(needsRestart(null, true, true)).toBe(true);
});

test("a client that never gets past loading says where it stopped", () => {
  expect(stuckReason({ state: "Login", ui_ready: true })).toBe(
    " It is still at the login screen: is the realm running (Manager, Server: Start)?",
  );
  expect(stuckReason({ state: "Loading" })).toBe(" It stopped at Loading.");
  expect(stuckReason(null)).toBe(" Its control port never answered.");
});

test("agents/.env is found outside a git checkout too (a source archive)", () => {
  expect(envFile()).toEndWith(join("agents", ".env"));
  const src = join(mkdtempSync(join(tmpdir(), "wp-archive-")), "agents", "example", "src");
  mkdirSync(src, { recursive: true });
  expect(envFile(src)).toBe(join(src, "..", "..", ".env"));
});

test("without the fast tier the planner is told what is not there", () => {
  expect(systemPrompt(true)).toBe(SYSTEM_PROMPT);
  expect(systemPrompt(false)).toBe(SYSTEM_PROMPT + NO_FAST_TIER);
  expect(NO_FAST_TIER).toContain("no fast loop");
  expect(NO_FAST_TIER).toContain("wake_me");
});
