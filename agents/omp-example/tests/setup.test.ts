import { expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { LineDecoder } from "../src/rpc.ts";
import { ompCli, ompEnv, prepareWorkDir, settingsFromEnv, trialRunning } from "../src/setup.ts";

const example = resolve(import.meta.dir, "..");
const base = { WRATHPLUS_AGENT_URL: "http://127.0.0.1:17950", WRATHPLUS_AGENT_TOKEN: "t".repeat(24) };

test("settings: the port is required, the rest has defaults", () => {
  expect(() => settingsFromEnv({})).toThrow(/WRATHPLUS_AGENT_URL.*WRATHPLUS_AGENT_TOKEN/);
  const s = settingsFromEnv({ ...base, WRATHPLUS_OMP_STATE: "/tmp/x" }, example);
  expect(s.workDir).toBe("/tmp/x/work");
  expect(s.model).toBe("anthropic/claude-sonnet-5-5");
  expect(s.repoDir).toBe(resolve(example, "..", ".."));
  expect([s.checkinMinutes, s.reflectHours, s.sessionHours, s.maxCost]).toEqual([20, 3, 6, undefined]);
  expect(s.repeatMinutes).toBe(15);
  expect(settingsFromEnv({ ...base, WRATHPLUS_OMP_REPEAT_MINUTES: "0" }, example).repeatMinutes).toBe(0);
  expect(s.noticeSeconds).toBe(15);
  expect(settingsFromEnv({ ...base, WRATHPLUS_OMP_NOTICE_SECONDS: "0" }, example).noticeSeconds).toBe(0);
  const t = settingsFromEnv({ ...base, WRATHPLUS_OMP_REFLECT_HOURS: "0", WRATHPLUS_OMP_CHECKIN_MINUTES: "x", WRATHPLUS_OMP_MAX_COST: "5" }, example);
  expect([t.reflectHours, t.checkinMinutes, t.maxCost]).toEqual([0, 20, 5]);
  expect([s.stallXpCheckins, s.stallQuestHours, s.scorecardSeconds]).toEqual([3, 4, 30]);
  const u = settingsFromEnv({ ...base, WRATHPLUS_OMP_STALL_XP_CHECKINS: "6", WRATHPLUS_OMP_STALL_QUEST_HOURS: "0" }, example);
  expect([u.stallXpCheckins, u.stallQuestHours]).toEqual([6, 0]);
});

test("the working dir gets AGENTS.md and the config each start; skills only when missing", () => {
  const state = mkdtempSync(join(tmpdir(), "omp-state-"));
  const s = settingsFromEnv({ ...base, WRATHPLUS_OMP_STATE: state }, example);
  prepareWorkDir(s);
  const work = s.workDir;
  expect(readFileSync(join(work, "AGENTS.md"), "utf8")).toBe(readFileSync(join(example, "AGENTS.md"), "utf8"));
  expect(existsSync(join(work, ".omp", "config.yml"))).toBe(true);
  expect(existsSync(join(work, ".omp", "mcp.json"))).toBe(true);
  expect(existsSync(join(work, ".omp", "agents", "reflector.md"))).toBe(true);
  expect(existsSync(join(work, "notes"))).toBe(true);
  const skill = join(work, ".omp", "skills", "reflect", "SKILL.md");
  writeFileSync(skill, "the agent's own edit");
  writeFileSync(join(work, "AGENTS.md"), "stale");
  mkdirSync(join(work, ".omp", "skills", "mine"), { recursive: true });
  prepareWorkDir(s);
  expect(readFileSync(skill, "utf8")).toBe("the agent's own edit");
  expect(readFileSync(join(work, "AGENTS.md"), "utf8")).not.toBe("stale");
  expect(existsSync(join(work, ".omp", "skills", "mine"))).toBe(true);
});

test("OMP's environment: its own dirs, no auto-QA, no telemetry", () => {
  const s = settingsFromEnv({ ...base, WRATHPLUS_OMP_STATE: join(homedir(), ".local", "share", "agent-x") }, example);
  const env = ompEnv(s, { PATH: "/bin" });
  expect(env.PI_CODING_AGENT_DIR).toBe(join(homedir(), ".local", "share", "agent-x", "omp-agent"));
  expect(env.PI_CONFIG_DIR).toBe(join(".local", "share", "agent-x", "omp-root"));
  expect([env.PI_AUTO_QA, env.OTEL_SDK_DISABLED, env.PATH]).toEqual(["0", "true", "/bin"]);
  expect(env.WRATHPLUS_REPO).toBe(s.repoDir);
  const outside = ompEnv(settingsFromEnv({ ...base, WRATHPLUS_OMP_STATE: "/var/tmp/agent-y" }, example), {});
  expect(outside.PI_CONFIG_DIR).toBeUndefined();
});

test("omp's entry point is the pinned package's bin", () => {
  expect(ompCli(example)).toEndWith(join("@oh-my-pi", "pi-coding-agent", "dist", "cli.js"));
});

test("a trial is running from its start until its verdict or end", () => {
  const lines = (...entries: object[]) => entries.map((e) => JSON.stringify(e)).join("\n");
  expect(trialRunning("")).toBe(false);
  expect(trialRunning(lines({ kind: "proposed", trial: 1 }))).toBe(false);
  expect(trialRunning(lines({ kind: "proposed", trial: 1 }, { kind: "started", trial: 1 }, { kind: "window", trial: 1 }))).toBe(true);
  expect(trialRunning(lines({ kind: "started", trial: 1 }, { kind: "verdict", trial: 1 }))).toBe(false);
  expect(trialRunning(`${lines({ kind: "started", trial: 1 }, { kind: "ended", trial: 1 }, { kind: "started", trial: 2 })}\n{"kind":"win`)).toBe(true);
});

test("RPC output: JSON lines across chunk boundaries", () => {
  const frames: unknown[] = [];
  const bad: string[] = [];
  const d = new LineDecoder();
  const enc = new TextEncoder();
  d.push(enc.encode('{"type":"ready"}\n{"type":"resp'), (f) => frames.push(f), (l) => bad.push(l));
  d.push(enc.encode('onse","id":"l1"}\n\nnot json\n'), (f) => frames.push(f), (l) => bad.push(l));
  expect(frames).toEqual([{ type: "ready" }, { type: "response", id: "l1" }]);
  expect(bad).toEqual(["not json"]);
});
