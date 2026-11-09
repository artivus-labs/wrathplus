import { expect, test } from "bun:test";
import { allowedTools, goalsServer, GOAL_TOOLS } from "../src/brain.ts";
import { dropGoalLua, GOALS_LUA, holdLua, pushGoalLua, type LuaResult } from "../src/goals.ts";
import { MODULE_TOOLS } from "../src/modules.ts";
import { sleepGuard, Waker } from "../src/wake.ts";
import { connect } from "./mcp.ts";

test("the goal tools send the Goals API's calls and say what happened", async () => {
  const sent: string[] = [];
  let answer: LuaResult = { ok: true, values: [true] };
  const call = await connect(goalsServer(async (code) => (sent.push(code), answer)));

  const pushed = await call("push_goal", { name: "reach_westfall", why: "Elwynn is too easy now.", priority: 70, done_lua: 'return GetRealZoneText() == "Westfall"' });
  expect(pushed.text).toBe("Goal reach_westfall set. The goals tool shows its plan.");
  expect(sent[0]).toBe(pushGoalLua({ name: "reach_westfall", why: "Elwynn is too easy now.", priority: 70, done_lua: 'return GetRealZoneText() == "Westfall"' }));
  expect(sent[0]).toContain('Goals.push{ name = "reach_westfall", why = [[\nElwynn is too easy now.]], priority = 70, done = [[\nreturn GetRealZoneText() == "Westfall"]] }');

  expect((await call("drop_goal", { name: "reach_westfall" })).text).toBe("Goal reach_westfall dropped.");
  expect(sent[1]).toBe(dropGoalLua("reach_westfall"));

  expect((await call("hold", { reason: "Pallytest is coming back from the vendor", seconds: 120 })).text).toBe(
    "Holding for 120s: Pallytest is coming back from the vendor.",
  );
  expect(sent[2]).toContain("Goals.hold{ reason = [[\nPallytest is coming back from the vendor]], seconds = 120 }");
  expect((await call("hold", { reason: "done", seconds: 0 })).text).toBe("Hold released; the runner carries on.");
  expect(sent[3]).toContain("Goals.release()");
  // No unlimited holds: the schema refuses one longer than an hour.
  expect((await call("hold", { reason: "forever", seconds: 86_400 })).error).toBe(true);
  expect(sent).toHaveLength(4);

  answer = { ok: true, values: [{ goals: [{ name: "progress", status: "active", plan: ["grind"] }], facts: { mobs_for_xp: true } }] };
  const listed = JSON.parse((await call("goals")).text);
  expect(listed.goals[0].plan).toEqual(["grind"]);
  expect(sent[4]).toBe(GOALS_LUA);

  // A client without wp.goals says so rather than failing silently.
  answer = { ok: true, values: [false, "the client has no Goals module loaded (wp.goals)"] };
  expect((await call("push_goal", { name: "x", why: "y" })).text).toBe("Failed: the client has no Goals module loaded (wp.goals)");
  answer = { ok: false, error: "HTTP 401" };
  expect((await call("goals")).text).toBe("Failed: HTTP 401");
});

test("goal names are snake_case", async () => {
  const call = await connect(goalsServer(async () => ({ ok: true, values: [true] })));
  expect((await call("push_goal", { name: "Reach Westfall", why: "y" })).error).toBe(true);
});

test("the planner gets the goal tools with Lua, and the module tools with its versions too", () => {
  expect(allowedTools({})).not.toContain("mcp__goals__push_goal");
  const lua = async () => ({ ok: true });
  expect(allowedTools({ lua })).toEqual(expect.arrayContaining(GOAL_TOOLS));
  expect(allowedTools({ lua })).not.toContain("mcp__harness__rollback_module");
  expect(allowedTools({ lua, modules: {} as never })).toEqual(expect.arrayContaining([...GOAL_TOOLS, ...MODULE_TOOLS]));
});

test("asleep after wake_me, the goal tools and a rollback are refused like the game's", async () => {
  const waker = new Waker(() => {});
  const guard = sleepGuard(waker);
  const pre = async (tool_name: string) =>
    JSON.stringify(
      await guard({ hook_event_name: "PreToolUse", tool_name, tool_input: {}, tool_use_id: "t", session_id: "s", transcript_path: "", cwd: "" } as never, "t", {
        signal: new AbortController().signal,
      }),
    );
  waker.set(30, [], "check");
  expect(await pre("mcp__goals__push_goal")).toContain('"deny"');
  expect(await pre("mcp__harness__rollback_module")).toContain('"deny"');
  expect(await pre("mcp__harness__module_versions")).toBe("{}");
  waker.cancel();
});

// The generated Lua, run against a stub of the contract's Goals API in a real Lua 5.1 (LuaJIT),
// when one is installed: it must compile and make the calls the contract describes.
test.skipIf(!Bun.which("luajit"))("the generated Lua runs against the Goals API contract", () => {
  const stub = `
local calls = {}
Goals = {
  push = function(g) calls[#calls + 1] = "push " .. g.name .. " " .. g.priority .. " " .. type(g.done) .. " " .. g.why end,
  drop = function(n) calls[#calls + 1] = "drop " .. n end,
  hold = function(h) calls[#calls + 1] = "hold " .. h.reason .. " " .. h.seconds end,
  release = function() calls[#calls + 1] = "release" end,
  list = function() return { { name = "progress", status = "active", plan = { "grind" } } } end,
  facts = function() return { mobs_for_xp = true } end,
  holding = function() return "waiting", 30, 90 end,
}
Agent = { activity = function() return nil end }
local function run(code) local f = assert(loadstring(code)) return f() end
`;
  const chunks = [
    pushGoalLua({ name: "ding_ten", why: "Talents at 10 ]] tricky", priority: 80, done_lua: 'return UnitLevel("player") >= 10' }),
    dropGoalLua("ding_ten"),
    holdLua("Wifey is on her way", 90),
    holdLua("", 0),
  ];
  const script = `${stub}
for _, code in ipairs({ ${chunks.map((c) => JSON.stringify(c)).join(", ")} }) do assert(run(code) == true) end
local state = run(${JSON.stringify(GOALS_LUA)})
print(table.concat(calls, "|"))
print(state.goals[1].name, state.facts.mobs_for_xp, state.hold.reason, state.hold.remaining)`;
  const out = Bun.spawnSync(["luajit", "-e", script]);
  expect(out.stderr.toString()).toBe("");
  expect(out.stdout.toString().trim().split("\n")).toEqual([
    "push ding_ten 80 string Talents at 10 ]] tricky|drop ding_ten|hold Wifey is on her way 90|release",
    "progress\ttrue\twaiting\t90",
  ]);
});

test("a goal's want is sent as Lua facts, and unsafe fact names are dropped", () => {
  const lua = pushGoalLua({ name: "to_westfall", why: "Elwynn is done.", want: { zone_fit: "ok", quest_ready: true, "bad name]]": 1 } });
  expect(lua).toContain("want = { zone_fit = [[\nok]], quest_ready = true }");
  expect(lua).not.toContain("bad name");
});
