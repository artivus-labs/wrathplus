import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ModuleVersions } from "../src/modules.ts";
import type { Decision, HoldDecision, TimeShares } from "../src/policy.ts";
import { GUIDANCE, Reflection, STALL_GUIDANCE, stalled, unusedTrade, type ReflectOptions, type Score } from "../src/reflect.ts";

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

test("a window spent standing still leads with that, says no one is coming, and asks for the cause to be fixed", () => {
  let time: TimeShares = { busy: 0, idle: 0, hold: 0, other: 0 };
  const { clock, r } = setup({ fast: () => ({ decisions: [], holds: [], time }) });
  r.sample({ level: 11, xp: 100, max: 8700 });
  clock.now = 30 * MIN;
  time = { busy: 2 * MIN, idle: 10 * MIN, hold: 18 * MIN, other: 0 };
  r.sample({ level: 11, xp: 100, max: 8700 });
  const text = r.reflect();
  const parts = text.split("\n\n");
  expect(parts[0]).toStartWith("Reflection (asked for)");
  expect(parts[1]).toStartWith("THE PROBLEM: your character did nothing for 93% of this window (idle 33%, on hold 60%), 30 min in all.");
  expect(parts[1]).toContain("No one is coming: there is no operator and no user");
  expect(parts[2]).toStartWith("Scoreboard");
  expect(parts.at(-1)).toBe(STALL_GUIDANCE);
  expect(text).not.toContain(GUIDANCE);
  expect(STALL_GUIDANCE).toContain("Goals.zones()");
  expect(STALL_GUIDANCE).toContain("Goals.rest_below");
  expect(STALL_GUIDANCE).toContain("Goals.fits");
  expect(STALL_GUIDANCE).toContain('"No change"');
  expect(GUIDANCE).toContain("If nothing needs changing, say so in a line");

  // A working window keeps the ordinary guidance.
  clock.now = 60 * MIN;
  time = { busy: 30 * MIN, idle: 12 * MIN, hold: 18 * MIN, other: 0 };
  const next = r.reflect();
  expect(next).not.toContain("THE PROBLEM");
  expect(next.split("\n\n").at(-1)).toBe(GUIDANCE);
});

test("without the fast tier, a long window with no experience, kill or quest is a stall too", () => {
  const score = { window_minutes: 30, idle_pct: null, hold_pct: null } as Score;
  expect(stalled(score, { xp: 0, kills: 0, quests: 0 })).toContain("gained no experience, won no fight and handed in no quest");
  expect(stalled(score, { xp: 0, kills: 2, quests: 0 })).toBeNull();
  expect(stalled({ ...score, window_minutes: 5 }, { xp: 0, kills: 0, quests: 0 })).toBeNull();
  expect(stalled({ ...score, idle_pct: 50, hold_pct: 29 }, { xp: 0, kills: 0, quests: 0 })).toBeNull();
  expect(stalled({ ...score, idle_pct: 50, hold_pct: 30 }, { xp: 900, kills: 4, quests: 0 })).toContain("did nothing for 80%");
});

test("a built-in that keeps doing nothing is named with the agent's own loaded modules", () => {
  const { clock, r } = setup();
  clock.now = 10 * MIN;
  const noop = (minutes: number) =>
    event("AGENT_NOOP", ["pick_up_quests", `Smith Argus offered nothing we took; skipping them for ${minutes} min`, "autoexec, decline_elmore, pickup_filter"]);
  r.notice([noop(2)]);
  // Once is no pattern.
  expect(r.reflect()).not.toContain("Built-ins that keep doing nothing");
  r.notice([noop(2), noop(4), event("AGENT_NOOP", ["hand_in_quests", "Marshal Dughan took nothing we handed in; skipping them for 2 min", ""])]);
  const block = r
    .reflect()
    .split("\n\n")
    .find((p) => p.startsWith("Built-ins that keep doing nothing"));
  expect(block).toBe(
    [
      "Built-ins that keep doing nothing:",
      "- pick_up_quests ×2, last: Smith Argus offered nothing we took; skipping them for 4 min",
      "Your own modules loaded in the client: autoexec, decline_elmore, pickup_filter. Does one of them cause this: a handler that declines, closes or skips what the built-in does, or a wrapper that replaces it? Read them (modules) before you blame the built-in or the game, and fix or roll back the one that does, rather than adding another.",
    ].join("\n"),
  );
});

test("a note that says nothing is left is set against the live facts that say otherwise", () => {
  let memory = [
    "- Elwynn: Goldshire is exhausted for level 11 (only L5 spiders, boars, sheep, wolves).",
    "- Lyria trains warriors in Goldshire.",
    "Review: Goldshire quests are exhausted; Kobold Candles needs the mine.",
  ].join("\n");
  const { clock, r } = setup({ memory: () => memory });
  clock.now = 10 * MIN;
  // No facts read yet: nothing to compare.
  expect(r.reflect()).not.toContain("Your notes and the game disagree");
  r.observe({ quest_giver_near: false, objectives_open: true, mobs_for_xp: true, zone_fit: "low" });
  const said = r
    .reflect()
    .split("\n\n")
    .find((p) => p.startsWith("Your notes and the game disagree"));
  expect(said).toBe(
    'Your notes and the game disagree. MEMORY.md says "- Elwynn: Goldshire is exhausted for level 11 (only L5 spiders, boars, sheep, wolves)." and "Review: Goldshire quests are exhausted; Kobold Candles needs the mine.", but the client says now: your quest log has objectives still open (objectives_open); mobs that give you XP are in sight (mobs_for_xp). The facts are live and the note is old: check the note against them, and correct or remove it (memory_write) before you act on it.',
  );
  // Facts that agree with the note, or a note with no such words, say nothing.
  r.observe({ quest_giver_near: false, objectives_open: false, mobs_for_xp: false });
  expect(r.reflect()).not.toContain("Your notes and the game disagree");
  memory = "- Lyria trains warriors in Goldshire.";
  r.observe({ mobs_for_xp: true });
  expect(r.reflect()).not.toContain("Your notes and the game disagree");
});

test("resting a lot with meat and cloth in the bags and no profession is pointed out", () => {
  const { clock, r } = setup({ settleMs: 0, minGapMs: 0 });
  r.sample({ level: 11, xp: 100, max: 1000, trade: { professions: 0, reagents: 23, food: 0 } });
  clock.now = 10 * MIN;
  const rest = event("AGENT_RESTED", [180, null]);
  r.notice([rest, rest, rest, event("AGENT_FOOD_BOUGHT", [5, 23, 0])]);
  r.trigger("asked");
  expect(r.check()).toContain("Professions: you rested 3 times and bought 5 food while carrying 23 meat and cloth");
  // Known professions, no reagents, or little resting: nothing to say.
  const w = { rests: 4, foodBought: 0 };
  expect(unusedTrade({ ...w, trade: { professions: 2, reagents: 23, food: 4 } })).toBeNull();
  expect(unusedTrade({ ...w, trade: { professions: 0, reagents: 0, food: 0 } })).toBeNull();
  expect(unusedTrade({ rests: 1, foodBought: 2, trade: { professions: 0, reagents: 9, food: 0 } })).toBeNull();
  expect(unusedTrade({ ...w, trade: undefined })).toBeNull();
});
