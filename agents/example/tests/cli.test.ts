import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { recentLines } from "../src/dashboard.ts";
import { claudeEnv, loadEnv, parseEnv } from "../src/env.ts";
import { accountFor, exitReason, findAgent } from "../src/manager.ts";

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
