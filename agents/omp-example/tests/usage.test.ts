import { expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { appendJsonl, totals, usageFromFrame, usageOf } from "../src/usage.ts";

const assistant = {
  role: "assistant",
  provider: "anthropic",
  model: "claude-sonnet-5-5",
  stopReason: "toolUse",
  usage: { input: 120, output: 340, cacheRead: 20000, cacheWrite: 1500, totalTokens: 21960, cost: { total: 0.0123 } },
};

test("an assistant message's usage", () => {
  expect(usageOf(assistant)).toEqual({
    model: "anthropic/claude-sonnet-5-5",
    input: 120,
    output: 340,
    cacheRead: 20000,
    cacheWrite: 1500,
    cost: 0.0123,
    stopReason: "toolUse",
  });
  expect(usageOf({ role: "user", content: "hi" })).toBeNull();
  expect(usageOf({ role: "assistant" })).toBeNull();
});

test("frames: the planner's message_end and a subagent's", () => {
  const now = new Date("2026-11-23T00:00:00Z");
  expect(usageFromFrame({ type: "message_end", message: assistant }, "s1", now)).toMatchObject({ kind: "assistant", session: "s1", ts: now.toISOString(), cost: 0.0123 });
  expect(usageFromFrame({ type: "subagent_event", event: { type: "message_end", message: assistant } }, "s1", now)).toMatchObject({ kind: "subagent", input: 120 });
  expect(usageFromFrame({ type: "message_end", message: { role: "toolResult" } })).toBeNull();
  expect(usageFromFrame({ type: "agent_end" })).toBeNull();
});

test("totals over a usage file, from a time on", () => {
  const dir = mkdtempSync(join(tmpdir(), "omp-usage-"));
  const path = join(dir, "usage.jsonl");
  appendJsonl(path, usageFromFrame({ type: "message_end", message: assistant }, "s", new Date("2026-11-23T00:00:00Z")));
  appendJsonl(path, usageFromFrame({ type: "subagent_event", event: { type: "message_end", message: assistant } }, "s", new Date("2026-11-23T01:00:00Z")));
  appendJsonl(path, { ts: "2026-11-23T01:00:01Z", kind: "session_stats", stats: { cost: 9 } });
  const all = totals(path);
  expect(all.calls).toBe(2);
  expect(all.cost).toBeCloseTo(0.0246);
  expect(all.cacheRead).toBe(40000);
  expect(totals(path, "2026-11-23T00:30:00Z").calls).toBe(1);
  expect(totals(join(dir, "missing.jsonl")).calls).toBe(0);
});
