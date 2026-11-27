// The launcher's settings (all from the environment) and the agent's working directory.
//
// OMP runs with its cwd in a working directory of the agent's own, outside any checkout, so the
// only project context it finds is this example's AGENTS.md (a checkout's own AGENTS.md and
// CLAUDE.md are for people working on the code, not for a player). AGENTS.md and .omp/config.yml
// and mcp.json are copied in at every start, so they follow the checkout; the seed skills and the
// reflector agent are copied only when missing, because the agent edits and adds to its own.

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join, relative, resolve } from "node:path";

export interface Settings {
  /** This example's directory (AGENTS.md, .omp/). */
  exampleDir: string;
  /** The checkout's root, for the MCP server's path. */
  repoDir: string;
  /** Everything of this agent's: working dir, OMP's agent dir, usage, logs. */
  stateDir: string;
  workDir: string;
  agentUrl: string;
  token: string;
  /** `provider/model`, OMP's selector. */
  model: string;
  thinking: string;
  /** Minutes of quiet before a check-in prompt. */
  checkinMinutes: number;
  /** Hours between reflections (0: off). */
  reflectHours: number;
  /** Hours before the planner's session is replaced by a fresh one (0: never). */
  sessionHours: number;
  /** A turn running longer than this is aborted (minutes; 0: never). */
  turnMaxMinutes: number;
  /** A notable event repeating one that woke the planner this many minutes ago or less does not
   *  wake it again; it is counted for the next check-in (0: every one wakes it). */
  repeatMinutes: number;
  /** AGENT_NOTICED events are collected this many seconds into one wake (0: each wakes at once). */
  noticeSeconds: number;
  /** Passed as --max-cost when set (USD per session). */
  maxCost?: number;
  judgeDir?: string;
  /** Start a fresh session rather than continue the last one (on the first start only). */
  fresh: boolean;
}

const number = (v: string | undefined, fallback: number): number => {
  const n = Number(v);
  return v !== undefined && v.trim() !== "" && Number.isFinite(n) && n >= 0 ? n : fallback;
};

export function settingsFromEnv(env: Record<string, string | undefined>, exampleDir = resolve(import.meta.dir, "..")): Settings {
  const problems: string[] = [];
  const agentUrl = env.WRATHPLUS_AGENT_URL?.trim() ?? "";
  const token = env.WRATHPLUS_AGENT_TOKEN?.trim() ?? "";
  if (!agentUrl) problems.push("WRATHPLUS_AGENT_URL is not set (the client's control port, e.g. http://127.0.0.1:17950)");
  if (!token) problems.push("WRATHPLUS_AGENT_TOKEN is not set (the client's WOW_AGENT_TOKEN)");
  const stateDir = resolve(env.WRATHPLUS_OMP_STATE?.trim() || join(homedir(), ".local", "share", "wrathplus-omp-agent"));
  if (problems.length) throw new Error(problems.join("; "));
  const maxCost = env.WRATHPLUS_OMP_MAX_COST ? number(env.WRATHPLUS_OMP_MAX_COST, 0) : undefined;
  return {
    exampleDir,
    repoDir: resolve(exampleDir, "..", ".."),
    stateDir,
    workDir: join(stateDir, "work"),
    agentUrl,
    token,
    model: env.WRATHPLUS_OMP_MODEL?.trim() || "anthropic/claude-sonnet-5-5",
    thinking: env.WRATHPLUS_OMP_THINKING?.trim() || "medium",
    checkinMinutes: number(env.WRATHPLUS_OMP_CHECKIN_MINUTES, 20),
    reflectHours: number(env.WRATHPLUS_OMP_REFLECT_HOURS, 3),
    sessionHours: number(env.WRATHPLUS_OMP_SESSION_HOURS, 6),
    turnMaxMinutes: number(env.WRATHPLUS_OMP_TURN_MAX_MINUTES, 30),
    repeatMinutes: number(env.WRATHPLUS_OMP_REPEAT_MINUTES, 15),
    noticeSeconds: number(env.WRATHPLUS_OMP_NOTICE_SECONDS, 15),
    maxCost: maxCost && maxCost > 0 ? maxCost : undefined,
    judgeDir: env.WRATHPLUS_JUDGE_DIR?.trim() || undefined,
    fresh: env.WRATHPLUS_OMP_FRESH === "1",
  };
}

/** Copy the example's prompt and OMP config into the working directory. */
export function prepareWorkDir(s: Settings): void {
  const omp = join(s.workDir, ".omp");
  mkdirSync(join(s.workDir, "notes"), { recursive: true });
  mkdirSync(omp, { recursive: true });
  cpSync(join(s.exampleDir, "AGENTS.md"), join(s.workDir, "AGENTS.md"));
  for (const file of ["config.yml", "mcp.json"]) cpSync(join(s.exampleDir, ".omp", file), join(omp, file));
  for (const dir of ["skills", "agents"]) {
    const from = join(s.exampleDir, ".omp", dir);
    if (!existsSync(from)) continue;
    for (const name of readdirSync(from)) {
      const to = join(omp, dir, name);
      if (!existsSync(to)) cpSync(join(from, name), to, { recursive: true });
    }
  }
}

/** The environment OMP runs with: its own agent and config dirs under the state dir (so nothing of
 *  the user's own OMP setup, auth or sessions is read), no auto-QA reports, no OpenTelemetry. */
export function ompEnv(s: Settings, base: Record<string, string | undefined>): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = {
    ...base,
    PI_CODING_AGENT_DIR: join(s.stateDir, "omp-agent"),
    PI_AUTO_QA: "0",
    PI_NO_TITLE: "1",
    OTEL_SDK_DISABLED: "true",
    WRATHPLUS_REPO: s.repoDir,
  };
  // PI_CONFIG_DIR is a path under the home directory (logs, the daemon's files).
  const rel = relative(homedir(), join(s.stateDir, "omp-root"));
  if (!rel.startsWith("..") && !isAbsolute(rel)) env.PI_CONFIG_DIR = rel;
  return env;
}

/** omp's entry point in this example's node_modules. */
export function ompCli(exampleDir: string): string {
  const pkg = join(exampleDir, "node_modules", "@oh-my-pi", "pi-coding-agent");
  const manifest = JSON.parse(readFileSync(join(pkg, "package.json"), "utf8")) as { bin?: Record<string, string> };
  return join(pkg, manifest.bin?.omp ?? "dist/cli.js");
}

/** Whether the judge has a trial under way (started, with no verdict or end): replacing the session
 *  restarts the MCP server, which would abandon it. */
export function trialRunning(ledgerText: string): boolean {
  const started = new Set<number>();
  for (const raw of ledgerText.split("\n")) {
    if (!raw.trim()) continue;
    try {
      const e = JSON.parse(raw) as { kind?: string; trial?: number };
      if (typeof e.trial !== "number") continue;
      if (e.kind === "started") started.add(e.trial);
      if (e.kind === "verdict" || e.kind === "ended") started.delete(e.trial);
    } catch {
      // A torn last line: ignore it.
    }
  }
  return started.size > 0;
}
