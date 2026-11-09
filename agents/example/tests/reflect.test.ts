import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ModuleVersions } from "../src/modules.ts";
import type { Decision, HoldDecision, TimeShares } from "../src/policy.ts";
import { Reflection, type ReflectOptions, type Score } from "../src/reflect.ts";

const event = (name: string, args: unknown[] = []) => ({ seq: 1, time: 0, event: name, args });
const MIN = 60_000;

function setup(extra: Partial<ReflectOptions> = {}) {
  const clock = { now: 0 };
  const sent: { text: string; score: Score }[] = [];
  const r = new Reflection({ now: () => clock.now, onReflect: (text, score) => sent.push({ text, score }), ...extra });
  return { clock, sent, r };
}

test("a death is reviewed once it settles, with its cause from the fight review", () => {
  const { clock, sent, r } = setup();
  clock.now = 10 * MIN;
  r.notice([event("PLAYER_REGEN_DISABLED"), event("PLAYER_DEAD")]);
  r.notice([event("AGENT_COMBAT_REVIEW", ["You died fighting Defias Pillager. Last 4 fights: 3 won, 1 died, 21s on average. Rotation (default): Rend 3 cast."])]);
  expect(r.pending).toEqual(["you died"]);
  clock.now += 10_000;
  expect(r.check()).toBeNull(); // settling
  clock.now += 15_000;
  expect(r.check()).toBeNull(); // still in combat as far as it knows
  r.notice([event("PLAYER_ALIVE")]);
  const text = r.check();
  expect(text).toContain("Reflection (you died)");
  expect(text).toContain("You died fighting Defias Pillager");
  expect(text).toContain("- Deaths: 1 | 1");
  expect(text).toContain("rollback_module");
  expect(sent).toHaveLength(1);
  expect(r.pending).toEqual([]);
  // A new window: nothing more until something happens.
  clock.now += MIN;
  expect(r.check()).toBeNull();
});

test("reviews keep a minimum gap; a level-up during it waits", () => {
  const { clock, sent, r } = setup({ minGapMs: 5 * MIN, settleMs: 0 });
  clock.now = 6 * MIN;
  r.notice([event("PLAYER_LEVEL_UP", [6])]);
  expect(r.check()).toContain("you reached level 6");
  clock.now += MIN;
  r.notice([event("PLAYER_LEVEL_UP", [7])]);
  expect(r.check()).toBeNull();
  clock.now += 4 * MIN;
  expect(r.check()).toContain("Levels reached: 7.");
  expect(sent).toHaveLength(2);
});

test("repeated action failures trigger a review; goal actions are not counted twice", () => {
  const { clock, r } = setup();
  clock.now = 10 * MIN;
  r.notice([
    event("AGENT_ACTION_FAILED", ["progress", "grind", "no mobs"]),
    event("AGENT_INTENT_DONE", ["goal:grind", false, "no mobs"]),
    event("AGENT_INTENT_DONE", ["clear_camp", false, "replaced by a fight"]),
    event("AGENT_ACTION_FAILED", ["progress", "grind", "no mobs"]),
  ]);
  expect(r.pending).toEqual([]);
  r.notice([event("AGENT_INTENT_DONE", ["sell_junk", false, "no vendor"])]);
  expect(r.pending).toEqual(["3 actions failed in 10 min"]);
  const text = r.check() ?? "";
  expect(text).toContain('"grind (for progress): no mobs" ×2');
  expect(text).toContain("- Failed actions: 3");
});

test("a stall: no experience, kill, quest or goal for a while", () => {
  const { clock, r } = setup({ stallMs: 15 * MIN });
  r.sample({ level: 5, xp: 100, max: 2000 });
  clock.now = 14 * MIN;
  r.sample({ level: 5, xp: 100, max: 2000 });
  expect(r.check()).toBeNull();
  clock.now = 16 * MIN;
  expect(r.check()).toContain("no progress for 16 min");
  // Not again until progress has been made and stops again.
  clock.now = 40 * MIN;
  expect(r.check()).toBeNull();
  r.notice([event("AGENT_COMBAT_DONE", ["0xF1", "killed"])]);
  clock.now = 56 * MIN;
  expect(r.check()).toContain("no progress for 16 min");
});

test("the regular review waits for a quiet moment", () => {
  let quiet = false;
  const { clock, r } = setup({ periodMs: 30 * MIN, stallMs: 60 * MIN, quiet: () => quiet });
  r.sample({ level: 5, xp: 0, max: 1000 });
  r.notice([event("AGENT_COMBAT_DONE", ["0xF1", "killed"])]);
  clock.now = 31 * MIN;
  expect(r.check()).toBeNull();
  quiet = true;
  expect(r.check()).toContain("Reflection (a regular review)");
});

test("the scoreboard: XP/hour across a level-up, quests, fights, idle share since the last review", () => {
  let time: TimeShares = { busy: 0, idle: 0, hold: 0, other: 0 };
  const decisions: Decision[] = [
    { at: 1_000, reasons: ["activity ended"], choice: "wait", confidence: 0.4, ran: false, stuck: 0.1 },
    { at: 2_000, reasons: ["activity ended"], choice: "grind", confidence: 0.9, ran: true, result: "error: no", stuck: 0.1 },
  ];
  const holds: HoldDecision[] = [{ at: 3_000, reason: "waiting for Wifey", minutes_waited: 6, here: [], choice: "stop_waiting", confidence: 0.9 }];
  const home = mkdtempSync(join(tmpdir(), "wp-reflect-"));
  const versions = new ModuleVersions(home);
  const { clock, r } = setup({ fast: () => ({ decisions, holds, time }), versions });
  r.sample({ level: 5, xp: 1500, max: 2000 });
  versions.record("grind.loop", "-- v1", "save");
  versions.note("grind.loop", "smaller pulls", "deaths");
  clock.now = 30 * MIN;
  r.sample({ level: 6, xp: 500, max: 2500 });
  r.notice([event("AGENT_QUESTS_DONE", ["hand_in", 1, 2]), event("AGENT_COMBAT_DONE", ["g", "killed"]), event("AGENT_COMBAT_DONE", ["g", "lost"])]);
  r.notice([event("AGENT_LUA", ["require('x')", false, "module x not found"]), event("UI_ERROR_MESSAGE", ["Out of range."])]);
  r.notice([event("AGENT_GOAL_DONE", ["reach_level_6", true]), event("AGENT_GOAL_BLOCKED", ["reach_westfall", "no path"])]);
  time = { busy: 20 * MIN, idle: 6 * MIN, hold: 4 * MIN, other: 0 };
  const score = r.score();
  expect(score).toMatchObject({ xp_per_hour: 2000, deaths: 0, quests_done: 2, kills: 1, fights_lost: 1, idle_pct: 20, hold_pct: 13, level: 6 });
  const text = r.reflect();
  expect(text).toContain("- XP/hour: 2,000 | 2,000 (level 6)");
  expect(text).toContain("- Quests handed in: 2 | 2 (1 picked up)");
  expect(text).toContain("- Done: reach_level_6");
  expect(text).toContain("- Blocked: reach_westfall (no path)");
  expect(text).toContain("- 2 choices: wait ×1, grind ×1; 1 unsure (< 0.5); 1 failed to start");
  expect(text).toContain('- Hold "waiting for Wifey" ran out after 6 min (here: nobody): stop_waiting');
  expect(text).toContain('"module x not found (in: require(\'x\'))" ×1');
  expect(text).toContain('Game errors: "Out of range." ×1.');
  expect(text).toContain("grind.loop v1 (note): smaller pulls [watch: deaths]");
  // The next window measures time from this review on.
  time = { busy: 40 * MIN, idle: 6 * MIN, hold: 4 * MIN, other: 0 };
  expect(r.score().idle_pct).toBe(0);
  expect(r.reviews).toBe(1);
  rmSync(home, { recursive: true });
});
