import { expect, test } from "bun:test";
import { goalsServer } from "../src/brain.ts";
import type { LuaResult } from "../src/goals.ts";
import type { Jev } from "../src/jev.ts";
import { RUNNER_NOTICE, RunnerLock } from "../src/runner.ts";
import { Waker, wakeServer } from "../src/wake.ts";
import { BRIEF_LUA, describeBrief, gameIdle, HoldGate, idleWakeRefusal, PERSON_WAIT, readBrief, similarReasons, type Brief } from "../src/waits.ts";
import { connect } from "./mcp.ts";

/** Agentfour in Goldshire: nothing running, the runner with nothing it can do. */
const STUCK = {
  on: true,
  state: "nothing achievable",
  goals: [
    { name: "safe_zone", priority: 60, status: "blocked", plan: {}, note: "no plan reaches safe_zone = true now", why: "Stop dying" },
    { name: "progress", priority: 0, status: "active", plan: {}, why: "Keep levelling" },
  ],
  facts: { quest_ready: true, zone_fit: "low", hp_low: false, mobs_for_xp: false },
  zones: [
    { name: "Elwynn Forest", min: 1, max: 10, fit: "low", distance: 300, current: true },
    { name: "Westfall", min: 10, max: 20, fit: "ok", distance: 1400 },
  ],
  party: [{ name: "Larie", online: false }],
  walking: false,
  quests_busy: false,
  combat: false,
  dead: false,
  level: 11,
};

/** A client whose brief is `state()`; every other Lua call succeeds and is recorded. */
function client(state: () => Record<string, unknown> | false = () => STUCK) {
  const sent: string[] = [];
  const lua = async (code: string): Promise<LuaResult> => {
    sent.push(code);
    return { ok: true, values: [code === BRIEF_LUA ? state() : true] };
  };
  return { lua, sent, holds: () => sent.filter((c) => c.includes("Goals.hold{")) };
}

const answering = (choice: string, confidence = 0.9) => {
  const asked: { state: Record<string, unknown>; questions: Record<string, unknown> }[] = [];
  const jev = {
    ask: async (state: Record<string, unknown>, questions: Record<string, unknown>) => {
      asked.push({ state, questions });
      if (choice === "error") throw new Error("Jev HTTP 500");
      return { model: "jev-test", answers: { waits_for: { type: "choice", choice, confidence, probabilities: {} } } };
    },
  } as unknown as Jev;
  return { jev, asked };
};

test("the pattern refuses waits on a person and lets game waits through", () => {
  for (const reason of [
    "Still no operator zone decision; holding in Goldshire",
    "check for the operator's answer",
    "Waiting for the user to confirm Westfall",
    "awaiting instructions",
    "need permission to leave Elwynn",
    "waiting on your go-ahead",
  ]) {
    expect(PERSON_WAIT.test(reason)).toBe(true);
  }
  for (const reason of ["Pallytest is coming back from the vendor", "Hogger respawns in a minute", "Bloodrage cooldown", "the boat to Menethil"]) {
    expect(PERSON_WAIT.test(reason)).toBe(false);
  }
});

test("the same wait in other words is the same wait", () => {
  expect(similarReasons("Still no operator zone decision; holding in Goldshire", "No operator zone decision yet (3rd check), holding in Goldshire")).toBe(true);
  expect(similarReasons("Waiting for Larie", "waiting for larie.")).toBe(true);
  expect(similarReasons("Waiting for Larie to come online", "Still waiting: Larie")).toBe(true);
  expect(similarReasons("Hogger respawn", "Larie is selling at the vendor")).toBe(false);
  expect(similarReasons("zone decision", "Hogger respawn decision")).toBe(false);
});

test("without the fast tier a hold on a person is refused with the goals and facts; a game wait is placed", async () => {
  const c = client();
  let now = 0;
  const gate = new HoldGate({ lua: c.lua, now: () => now });
  const call = await connect(goalsServer(c.lua, gate));

  const refused = await call("hold", { reason: "Still no operator zone decision", seconds: 600 });
  expect(refused.error).toBe(true);
  expect(refused.text).toContain('Hold refused: "Still no operator zone decision" waits for a person');
  expect(refused.text).toContain("There is no operator and no user");
  expect(refused.text).toContain("decide now with what you know");
  // What it has to decide with: each goal and why it is blocked, the facts, the zones that fit.
  expect(refused.text).toContain("- safe_zone (priority 60): blocked: no plan reaches safe_zone = true now. Why: Stop dying");
  expect(refused.text).toContain("Facts that hold now: quest_ready, zone_fit = low.");
  expect(refused.text).toContain("Westfall (levels 10-20, 1400 yd)");
  expect(c.holds()).toHaveLength(0);

  const placed = await call("hold", { reason: "Hogger respawns in a minute", seconds: 90 });
  expect(placed.error).toBe(false);
  expect(placed.text).toBe("Holding for 90s: Hogger respawns in a minute. Held 0s in all this session, before this hold.");
  expect(c.holds()).toHaveLength(1);
  // The total counts what was actually held: 60 s of this one, then all 120 s of the next.
  now = 60_000;
  expect((await call("hold", { reason: "Hogger respawns in a minute", seconds: 120 })).text).toContain("Held 60s in all this session, before this hold.");
  now = 600_000;
  expect(gate.totalSeconds).toBe(180);
  expect((await call("hold", { reason: "done", seconds: 0 })).text).toBe("Hold released; the runner carries on. Held 3 min in all this session.");
});

test("with the fast tier Jev reads the reason: a person is refused, the game is not, an error falls back to the pattern", async () => {
  const c = client();
  const person = answering("person");
  const refused = await new HoldGate({ lua: c.lua, jev: person.jev }).check("holding until I hear back about Westfall");
  expect(refused).toContain("waits for a person");
  expect(person.asked[0]?.state).toEqual({ reason: "holding until I hear back about Westfall", party: [{ name: "Larie", online: false }] });
  expect(Object.keys((person.asked[0]?.questions.waits_for as { criteria: object }).criteria)).toEqual(["game", "person"]);

  // "answer" is a game word here, and Jev says so: the pattern alone would have refused it.
  expect(await new HoldGate({ lua: c.lua, jev: answering("game").jev }).check("Pallytest's answer to my party invite")).toBeNull();
  // Unsure that it is a person: let it through (Jev reassesses the hold when it runs out).
  expect(await new HoldGate({ lua: c.lua, jev: answering("person", 0.4).jev }).check("something is about to happen")).toBeNull();
  const down = new HoldGate({ lua: c.lua, jev: answering("error").jev });
  expect(await down.check("waiting for the operator")).toContain("Hold refused");
  expect(await down.check("Hogger respawn")).toBeNull();
});

test("a wait the fast loop ended is refused again for a while, in whatever words", async () => {
  const c = client();
  let now = 0;
  const gate = new HoldGate({ lua: c.lua, jev: answering("game").jev, now: () => now, refuseMs: 1_800_000 });
  expect(await gate.check("Waiting for Larie to come back online")).toBeNull();
  gate.placed(600);
  now = 600_000;
  gate.stopped("Waiting for Larie to come back online");
  now = 660_000;
  const again = await gate.check("Still waiting for Larie (2nd check)");
  expect(again).toContain('this is the wait the fast loop ended 1 min ago ("Waiting for Larie to come back online": stop waiting');
  expect(again).toContain("refused for another 29 min");
  expect(again).toContain("You have held for 10 min in all this session.");
  // Another wait is its own question; and the same one may be asked again later.
  expect(await gate.check("Hogger respawns in a minute")).toBeNull();
  now = 600_000 + 1_800_000;
  expect(await gate.check("Waiting for Larie to come back online")).toBeNull();
});

test("idle means nothing runs and nothing will start by itself", () => {
  const brief = STUCK as unknown as Brief;
  expect(gameIdle({ ...brief, goals: brief.goals.map((g) => ({ ...g, plan: [] })) })).toBe(true);
  expect(gameIdle({ ...brief, on: false, goals: [{ name: "progress", status: "active", plan: ["grind"] }] })).toBe(true);
  expect(gameIdle({ ...brief, goals: [{ name: "progress", status: "active", plan: ["grind"] }] })).toBe(false);
  expect(gameIdle({ ...brief, activity: { name: "goal:grind", seconds: 3 } })).toBe(false);
  expect(gameIdle({ ...brief, walking: true })).toBe(false);
  expect(gameIdle({ ...brief, combat: true })).toBe(false);
});

test("a long or unlimited wake-up is refused while the game is idle, with what to do instead", async () => {
  const c = client();
  const refusal = await idleWakeRefusal(c.lua, 1800);
  expect(refusal).toContain("Not set: nothing is running in the game");
  expect(refusal).toContain("a wake-up of 1800s would leave your character standing there");
  expect(refusal).toContain("There is no operator and no user");
  expect(refusal).toContain("- progress (priority 0): blocked: no plan reaches it now");
  expect(refusal).toContain("ask for at most 300s");
  expect(await idleWakeRefusal(c.lua, undefined)).toContain("a wake-up with no time limit");
  // A short look-again is fine, and so is any sleep while something runs (not even asked about).
  expect(await idleWakeRefusal(c.lua, 300)).toBeNull();
  expect(c.sent).toHaveLength(2);
  const busy = client(() => ({ ...STUCK, activity: { name: "goal:travel_to_zone", seconds: 40 } }));
  expect(await idleWakeRefusal(busy.lua, 1800)).toBeNull();
  // A client with no goal runner cannot be judged: the wake-up stands.
  expect(await idleWakeRefusal(client(() => false).lua, 1800)).toBeNull();
});

test("wake_me refused while idle leaves the turn open; set, it ends the turn as before", async () => {
  const c = client();
  const waker = new Waker(() => {});
  const call = await connect(wakeServer(waker, (seconds) => idleWakeRefusal(c.lua, seconds)));
  const refused = await call("wake_me", { seconds: 1800, reason: "check for the operator's answer" });
  expect(refused.error).toBe(true);
  expect(refused.text).toContain("Your turn has not ended.");
  expect(waker.asleep).toBe(false);
  expect(waker.pending).toBe(false);
  const set = await call("wake_me", { seconds: 120, events: ["AGENT_WALK_DONE"], reason: "the walk" });
  expect(set.error).toBe(false);
  expect(set.text).toBe("I will wake you in 120s or on AGENT_WALK_DONE. End your turn now.");
  expect(waker.asleep).toBe(true);
  waker.cancel();
});

test("the brief reads lists defensively and says so when the client has no goals", async () => {
  expect(describeBrief(null)).toBe("The goals tool shows your goals, their plans and the facts.");
  const brief = await readBrief(client(() => ({ ...STUCK, goals: {}, zones: {}, party: {} })).lua);
  expect(brief?.goals).toEqual([]);
  expect(describeBrief(brief)).toContain("- none");
});

test("the runner stays on: switched off it is turned back on, and the planner is told once", async () => {
  let on = false;
  const told: string[] = [];
  const lua = async (): Promise<LuaResult> => {
    if (on) return { ok: true, values: ["on"] };
    on = true;
    return { ok: true, values: ["reasserted"] };
  };
  const lock = new RunnerLock(lua, (text) => told.push(text));
  // The harness turning it on at the start is not news.
  expect(await lock.check()).toBe("reasserted");
  expect(await lock.check()).toBe("on");
  expect(told).toEqual([]);
  // The planner's Lua sets Goals.on = false: on again at the next check, and it hears why.
  on = false;
  expect(await lock.check()).toBe("reasserted");
  expect(told).toEqual([RUNNER_NOTICE]);
  expect(RUNNER_NOTICE).toContain("the runner is always on");
  expect(RUNNER_NOTICE).toContain("goals (push_goal, drop_goal), actions (Goals.action) and holds");
  // Again: turned back on, counted, not repeated to the planner.
  on = false;
  expect(await lock.check()).toBe("reasserted");
  expect(told).toHaveLength(1);
  expect(lock.reasserted).toBe(2);
});

test("a client that is not there yet is checked again, and its first turn-on is still the harness's", async () => {
  let answer: LuaResult = { ok: false, error: "HTTP 503" };
  const told: string[] = [];
  const lock = new RunnerLock(async () => answer, (text) => told.push(text));
  expect(await lock.check()).toBe("unavailable");
  answer = { ok: true, values: ["reasserted"] };
  expect(await lock.check()).toBe("reasserted");
  expect(told).toEqual([]);
});
