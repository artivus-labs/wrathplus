// The planner's handle on the client's goal system (`Goals`, wp.goals): it sets goals, the
// client's GOAP planner plans actions toward the top one and runs them as Agent activities, and a
// hold pauses the runner for a bounded time. Everything here is Lua sent over the control port.

/** Lua source as a long-bracket string literal that cannot be closed by its own contents. */
export function luaString(text: string): string {
  let level = 0;
  while (text.includes(`]${"=".repeat(level)}]`)) level++;
  const eq = "=".repeat(level);
  // A newline right after the opening bracket is dropped by Lua, so one is added on purpose.
  return `[${eq}[\n${text}]${eq}]`;
}

export interface LuaResult {
  ok: boolean;
  values?: unknown[];
  error?: string;
}

export type Lua = (code: string) => Promise<LuaResult>;

/** A `Lua` over the client's control port. */
export function controlLua(url: string, token: string): Lua {
  return async (code) => {
    const response = await fetch(`${url}/lua`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
      body: code,
    });
    if (!response.ok) return { ok: false, error: `HTTP ${response.status}` };
    return (await response.json()) as LuaResult;
  };
}

/** Goal names are snake_case words, like intents. */
export const GOAL_NAME = /^[a-z][a-z0-9_]*$/;

/** The longest hold the planner may ask for in one go (s); Jev reassesses it on expiry. */
export const MAX_HOLD_SECONDS = 3600;
/** A hold's length when none is given: short, since the fast loop extends one worth keeping. */
export const DEFAULT_HOLD_SECONDS = 120;

const NEED_GOALS = `if not Goals then return false, "the client has no Goals module loaded (wp.goals)" end`;

export interface GoalSpec {
  name: string;
  why: string;
  priority?: number;
  /** Lua source returning finished[, ok]; compiled in the client with load. */
  done_lua?: string;
  standing?: boolean;
  /** The facts it wants (true, or equal): what the planner searches toward, e.g. { quest_ready: true }. */
  want?: Record<string, boolean | number | string>;
  /** The zone travel_to_zone heads for (`data.zone`); the client refuses one above our level. */
  zone?: string;
}

const FACT_NAME = /^[a-z_][a-z0-9_]*$/;

export function pushGoalLua(goal: GoalSpec): string {
  const fields = [`name = ${JSON.stringify(goal.name)}`, `why = ${luaString(goal.why)}`];
  if (goal.priority !== undefined) fields.push(`priority = ${Number(goal.priority)}`);
  if (goal.done_lua) fields.push(`done = ${luaString(goal.done_lua)}`);
  if (goal.standing) fields.push("standing = true");
  if (goal.want) {
    const value = (v: boolean | number | string) => (typeof v === "string" ? luaString(v) : String(v));
    const pairs = Object.entries(goal.want).filter(([k]) => FACT_NAME.test(k));
    if (pairs.length) fields.push(`want = { ${pairs.map(([k, v]) => `${k} = ${value(v)}`).join(", ")} }`);
  }
  if (goal.zone) fields.push(`data = { zone = ${luaString(goal.zone)} }`);
  // A refusal (a zone above our level) comes back as the reason, not as a Lua error.
  return `${NEED_GOALS}\nlocal ok, err = pcall(Goals.push, { ${fields.join(", ")} })\nif not ok then return false, tostring(err) end\nreturn true`;
}

export function dropGoalLua(name: string): string {
  return `${NEED_GOALS}\nGoals.drop(${JSON.stringify(name)})\nreturn true`;
}

/** The goals (highest first), the facts the planner plans over, and any hold. */
export const GOALS_LUA = `${NEED_GOALS}
local reason, waited, remaining
if Goals.holding then reason, waited, remaining = Goals.holding() end
return { goals = Goals.list(), facts = Goals.facts(),
  hold = reason and { reason = reason, waited = waited, remaining = remaining } or nil,
  activity = Agent and Agent.activity and Agent.activity() or nil }`;

/** A hold of `seconds`, or with 0 a release of the current one. */
export function holdLua(reason: string, seconds: number): string {
  if (seconds <= 0) return `${NEED_GOALS}\nGoals.release()\nreturn true`;
  return `${NEED_GOALS}\nGoals.hold{ reason = ${luaString(reason)}, seconds = ${Math.round(seconds)} }\nreturn true`;
}

/** A result as a line for the planner: what was done, or why it was not. */
export function outcome(result: LuaResult, done: string): string {
  if (!result.ok) return `Failed: ${result.error ?? "unknown error"}`;
  if (result.values?.[0] === false) return `Failed: ${String(result.values[1] ?? "no reason given")}`;
  return done;
}
