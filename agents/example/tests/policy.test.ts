import { expect, test } from "bun:test";
import { Jev } from "../src/jev.ts";
import { luaString, Policy, registerLua } from "../src/policy.ts";

type Asked = { state: { woke_because: string[] }; questions: Record<string, { criteria?: Record<string, string> }> };

/** A fake client port and Jev: the state the client reports is settable per test. */
function fake(state: () => Record<string, unknown>, answers: () => Record<string, unknown>) {
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
  const policy = new Policy({ url: base, token: "t", jev: new Jev("k", "jev-test", base), onNotify: (s) => notes.push(s) });
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
