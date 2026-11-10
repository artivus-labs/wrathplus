import { expect, test } from "bun:test";
import { Jev } from "../src/jev.ts";
import { holdContext, idleNotice, luaString, Policy, registerLua, WAIT_WHEN, type PolicyOptions } from "../src/policy.ts";

type Asked = { state: { woke_because: string[]; plan: string; hold_expired?: Record<string, unknown> }; questions: Record<string, { criteria?: Record<string, string> }> };

/** A fake client port and Jev: the state the client reports is settable per test. */
function fake(state: () => Record<string, unknown>, answers: () => Record<string, unknown>, extra: Partial<PolicyOptions> = {}) {
  const lua: string[] = [];
  const asked: Asked[] = [];
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    async fetch(request) {
      const url = new URL(request.url);
      if (url.pathname === "/lua") {
        const code = await request.text();
        lua.push(code);
        return Response.json({ ok: true, values: code.includes("AgentNearby") ? [state()] : [true] });
      }
      asked.push((await request.json()) as Asked);
      return Response.json({ model: "jev-test", answers: answers() });
    },
  });
  const base = server.url.href.replace(/\/$/, "");
  const notes: string[] = [];
  const policy = new Policy({ url: base, token: "t", jev: new Jev("k", "jev-test", base), onNotify: (s) => notes.push(s), ...extra });
  return { lua, asked, notes, policy, stop: () => server.stop(true) };
}

const choose = (name: string, stuck = 0.1, done = 0.1) => ({
  next: { type: "choice", choice: name, confidence: 0.9, probabilities: {} },
  stuck: { type: "noul", noul: stuck },
  done: { type: "noul", noul: done },
});

test("Jev chooses only when awake and idle; a running activity sleeps through non-interrupting events", async () => {
  let activity: unknown = null;
  const f = fake(
    () => ({ registered: [{ name: "clear_camp", when: "Kobolds remain." }], decisions: [], activity }),
    () => choose("clear_camp"),
  );
  const first = await f.policy.tick(["new plan"], true);
  expect(first?.ran).toBe(true);
  expect(f.lua).toContain('return Agent.run("clear_camp")');
  expect(f.asked[0]?.state.woke_because).toEqual(["new plan"]);
  expect(Object.keys(f.asked[0]?.questions ?? {})).toEqual(["next", "stuck", "done"]);
  // Busy: a loot window or a heartbeat does not call Jev.
  activity = { name: "clear_camp", seconds: 12 };
  expect(await f.policy.tick(["loot window"], false)).toBeNull();
  expect(await f.policy.tick(["heartbeat"], false)).toBeNull();
  expect(f.asked).toHaveLength(1);
  // Combat interrupts: Jev reconsiders, and choosing what already runs restarts nothing.
  const again = await f.policy.tick(["combat started"], true);
  expect(f.asked).toHaveLength(2);
  expect(again?.ran).toBe(false);
  expect(f.policy.calls).toBe(2);
  f.stop();
});

test("open decisions are answered even while busy, without re-choosing the activity", async () => {
  const f = fake(
    () => ({
      registered: [{ name: "clear_camp", when: "x" }],
      activity: { name: "clear_camp", seconds: 3 },
      decisions: [{ id: 4, name: "flee_or_fight", question: "Fight on?", options: { fight: "Stay", flee: "Run" } }],
    }),
    () => ({ decision_4: { type: "choice", choice: "fight", confidence: 0.7, probabilities: {} } }),
  );
  expect(await f.policy.tick(["decision opened"], false)).toBeNull();
  expect(Object.keys(f.asked[0]?.questions ?? {})).toEqual(["decision_4"]);
  expect(f.lua).toContain('return Agent.resolve(4, "fight")');
  f.stop();
});

test("the planner hears about repeated failures and a plan that looks done, once each", async () => {
  const f = fake(() => ({ registered: [{ name: "a", when: "x" }], decisions: [], activity: null }), () => choose("wait", 0.1, 0.95));
  for (let i = 0; i < 3; i++) f.policy.notice([{ seq: i, time: i, event: "AGENT_INTENT_DONE", args: ["a", false, "too far"] }]);
  await f.policy.tick(["activity ended"], true);
  await f.policy.tick(["activity ended"], true);
  await f.policy.tick(["activity ended"], true);
  f.stop();
  expect(f.notes.filter((n) => n.includes("failed 3 times"))).toHaveLength(1);
  expect(f.notes.filter((n) => n.includes("goal achieved"))).toHaveLength(1);
});

test("planner intents become registered Lua activities, with their until and timeout", () => {
  const lua = registerLua([{ name: "go", when: "Go there", lua: "AgentMoveTo(1, 2, 3)", until: "return AgentWalk().status ~= 'walking'", timeout: 60 }]);
  expect(lua.split("\n")[0]).toContain("if spec.plan then Agent.remove(name) end");
  expect(lua).toContain(", plan = true })");
  expect(lua).toContain('Agent.intent("go", { when = [[\nGo there]], run = assert(loadstring([[\nAgentMoveTo(1, 2, 3)]])), done = assert(loadstring(');
  expect(lua).toContain("timeout = 60");
  // Long brackets grow until the text cannot close them.
  expect(luaString("a]]b")).toBe("[=[\na]]b]=]");
});

test("Jev errors carry the status and body", async () => {
  const server = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => Response.json({ detail: "nope" }, { status: 403 }) });
  const jev = new Jev("k", "jev-test", server.url.href.replace(/\/$/, ""));
  await expect(jev.ask({}, {})).rejects.toThrow("Jev HTTP 403");
  server.stop(true);
});

const expired = (reason: string, waited: number) => ({ seq: 1, time: 0, event: "AGENT_HOLD_EXPIRED", args: [reason, waited] });

test("a hold that ran out is Jev's decision, with who is actually here; keep_waiting holds again", async () => {
  let hold = "keep_waiting";
  const f = fake(
    () => ({
      registered: [],
      decisions: [],
      activity: null,
      party: [
        { name: "Wifey", online: true },
        { name: "Pallytest", online: true, yards: 12 },
        { name: "Dongle", online: false },
      ],
      goals: [{ name: "progress", priority: 0, status: "active", plan: ["grind"] }],
    }),
    () => ({
      hold: { type: "choice", choice: hold, confidence: 0.8, probabilities: {} },
      hold_for: { type: "choice", choice: "few_minutes", confidence: 0.6, probabilities: {} },
    }),
  );
  f.policy.notice([expired("waiting for Wifey", 120)]);
  expect(await f.policy.tick(["a hold ran out"], true)).toBeNull();
  expect(Object.keys(f.asked[0]?.questions ?? {})).toEqual(["hold", "hold_for"]);
  expect(f.asked[0]?.state.hold_expired).toEqual({
    reason: "waiting for Wifey",
    minutes_waited: 2,
    party: { here: ["Pallytest"], online_elsewhere: ["Wifey"], offline: ["Dongle"] },
    goals: [{ name: "progress", priority: 0, status: "active" }],
  });
  expect(f.lua.at(-1)).toContain("Goals.hold{ reason = [[\nwaiting for Wifey]], seconds = 180 }");
  // The kept hold runs out too: the total waited carries over.
  f.policy.notice([expired("waiting for Wifey", 180)]);
  hold = "stop_waiting";
  await f.policy.tick(["a hold ran out"], true);
  expect(f.asked[1]?.state.hold_expired?.minutes_waited).toBe(5);
  expect(f.lua.filter((c) => c.includes("Goals.hold{"))).toHaveLength(1);
  expect(f.policy.holds.map((h) => [h.choice, h.seconds ?? null, h.minutes_waited])).toEqual([
    ["keep_waiting", 180, 2],
    ["stop_waiting", null, 5],
  ]);
  // Waiting again later for the same reason starts from zero.
  f.policy.notice([expired("waiting for Wifey", 60)]);
  await f.policy.tick(["a hold ran out"], true);
  expect(f.asked[2]?.state.hold_expired?.minutes_waited).toBe(1);
  expect(JSON.parse(f.policy.describe()).holds).toHaveLength(3);
  f.stop();
});

test("no usable answer on an expired hold means stop waiting", async () => {
  const f = fake(() => ({ registered: [], decisions: [], activity: null }), () => ({}));
  f.policy.notice([expired("respawn", 60)]);
  await f.policy.tick(["a hold ran out"], true);
  expect(f.policy.holds[0]?.choice).toBe("stop_waiting");
  expect(f.lua.some((c) => c.includes("Goals.hold{"))).toBe(false);
  f.stop();
});

test("hold context: online and in sight is here; unknown online counts as online", () => {
  const context = holdContext({ reason: "r", waited: 90 }, 90, { party: [{ name: "A", yards: 40 }, { name: "B" }, { name: "C", online: false, yards: 5 }] });
  expect(context.party).toEqual({ here: ["A"], online_elsewhere: ["B"], offline: ["C"] });
  expect(context.minutes_waited).toBe(1.5);
  expect(context.goals).toEqual([]);
});

test("idle is not fine forever: wait is offered for a grace period, then dropped, and the planner hears, again while it lasts", async () => {
  let now = 0;
  let activity: unknown = null;
  let hold: unknown = null;
  const f = fake(
    () => ({ registered: [{ name: "grind", when: "Mobs near." }], decisions: [], activity, hold, goals: [{ name: "progress", status: "blocked", note: "no mobs" }] }),
    () => choose("wait"),
    { now: () => now, idleGraceMs: 60_000, idleNotifyMs: 180_000, idleRepeatMs: 600_000 },
  );
  const waitOffered = (i: number) => f.asked[i]?.questions.next?.criteria?.wait;
  await f.policy.tick(["heartbeat"], false);
  expect(waitOffered(0)).toBe(WAIT_WHEN.idle);
  expect(f.asked[0]?.state.plan).toContain("never stand idle");
  now = 61_000;
  await f.policy.tick(["heartbeat"], false);
  expect(waitOffered(1)).toBeUndefined();
  now = 181_000;
  await f.policy.tick(["heartbeat"], false);
  now = 240_000;
  await f.policy.tick(["heartbeat"], false);
  const told = f.notes.filter((n) => n.includes("stood idle"));
  expect(told).toHaveLength(1);
  expect(told[0]).toContain("- progress: blocked: no mobs");
  // A hold makes waiting right again, and ends the idle stretch.
  hold = { reason: "respawn", waited: 10, remaining: 50 };
  now = 250_000;
  await f.policy.tick(["heartbeat"], false);
  expect(waitOffered(4)).toBe(WAIT_WHEN.hold);
  // Busy: the ordinary wait.
  hold = null;
  activity = { name: "grind", seconds: 3 };
  now = 300_000;
  await f.policy.tick(["combat started"], true);
  expect(waitOffered(5)).toBe(WAIT_WHEN.busy);
  // Where the time went: 250 s idle, then 50 s on hold, until the busy tick.
  expect(f.policy.time).toEqual({ busy: 0, idle: 250_000, hold: 50_000, other: 0 });
  expect(JSON.parse(f.policy.describe()).time_share).toEqual({ busy: 0, idle: 83, hold: 17, other: 0 });
  f.stop();
});

test("a blocked goal and a goal action that keeps failing reach the planner", async () => {
  const f = fake(() => ({ registered: [], decisions: [], activity: null }), () => ({}));
  const blocked = { seq: 1, time: 0, event: "AGENT_GOAL_BLOCKED", args: ["reach_westfall", "no path"] };
  f.policy.notice([blocked, blocked]);
  for (let i = 0; i < 3; i++) f.policy.notice([{ seq: i, time: i, event: "AGENT_INTENT_DONE", args: ["goal:grind", false, "no mobs"] }]);
  expect(f.notes.filter((n) => n.includes('"reach_westfall" is blocked: no path'))).toHaveLength(1);
  expect(f.notes.filter((n) => n.includes('goal action "grind" has failed 3 times'))).toHaveLength(1);
  f.stop();
});

test("an idle character is told of again and again, more plainly, with what the runner would do", async () => {
  let now = 0;
  const f = fake(
    () => ({
      registered: [],
      decisions: [],
      activity: null,
      goals: [
        { name: "safe_zone", priority: 60, status: "blocked", note: "no plan reaches safe_zone = true now" },
        { name: "progress", priority: 0, status: "active", plan: ["travel_to_zone", "grind"] },
      ],
    }),
    () => ({}),
    { now: () => now, idleNotifyMs: 180_000, idleRepeatMs: 600_000 },
  );
  for (const at of [0, 180_000, 500_000, 780_000, 1_000_000, 1_380_000]) {
    now = at;
    await f.policy.tick(["heartbeat"], false);
  }
  const told = f.notes.filter((n) => n.includes("stood idle"));
  expect(told).toHaveLength(3);
  expect(told[0]).toContain("stood idle for 180s");
  expect(told[0]).not.toContain("There is no operator");
  expect(told[1]).toContain("stood idle for 780s: no activity, no hold, and nothing started. You were told once already");
  expect(told[1]).toContain("There is no operator and no user");
  expect(told[1]).toContain("- safe_zone (priority 60): blocked: no plan reaches safe_zone = true now");
  expect(told[1]).toContain("- progress (priority 0): active; next: travel_to_zone → grind");
  expect(told[1]).toContain("Do not end the turn with the character idle.");
  expect(told[2]).toContain("You were told 2 times already");
  expect(idleNotice(200, [])).toContain("- none (is wp.goals loaded?)");
  f.stop();
});

test("hold decisions are passed on, so an ended wait can be refused when asked again", async () => {
  const seen: [string, string, number | undefined][] = [];
  let hold = "keep_waiting";
  const f = fake(
    () => ({ registered: [], decisions: [], activity: null }),
    () => ({ hold: { type: "choice", choice: hold, confidence: 1, probabilities: {} }, hold_for: { type: "choice", choice: "few_minutes", confidence: 1, probabilities: {} } }),
    { onHold: (d) => seen.push([d.reason, d.choice, d.seconds]) },
  );
  f.policy.notice([{ seq: 1, time: 0, event: "AGENT_HOLD_EXPIRED", args: ["Still no operator zone decision", 600] }]);
  await f.policy.tick(["a hold ran out"], true);
  hold = "stop_waiting";
  f.policy.notice([{ seq: 2, time: 0, event: "AGENT_HOLD_EXPIRED", args: ["Still no operator zone decision", 180] }]);
  await f.policy.tick(["a hold ran out"], true);
  expect(seen).toEqual([
    ["Still no operator zone decision", "keep_waiting", 180],
    ["Still no operator zone decision", "stop_waiting", undefined],
  ]);
  // Jev is told that a wait for someone outside the game is one to stop.
  expect(f.asked[0]?.questions.hold?.criteria?.stop_waiting).toContain("nobody outside the game will ever answer");
  f.stop();
});
