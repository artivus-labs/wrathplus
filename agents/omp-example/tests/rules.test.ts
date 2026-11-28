import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Sample } from "../../mcp/src/scorecard.ts";
import { sample } from "../../mcp/tests/fixtures.ts";
import { KEEP, NO_RULES, OLDER_THAN_RECORD, readRules, Reviewer } from "../src/rules.ts";
import { wakeMessage } from "../src/wake.ts";

const MIN = 60_000;
const T0 = Date.UTC(2026, 10, 27, 12);

/** Samples every 30 s from `from` for `minutes`, the experience rising by `xp` in all. */
function play(from: number, minutes: number, xp = 0, start = 0): Sample[] {
  const n = minutes * 2;
  return Array.from({ length: n + 1 }, (_, i) => sample(from + i * 30_000, { xp: start + Math.round((xp * i) / n) }));
}

test("rules come from module headers and notes/rules.md, each with what it has cost since it was set", () => {
  const home = mkdtempSync(join(tmpdir(), "wp-omp-rules-"));
  const scripts = join(home, "scripts");
  mkdirSync(join(scripts, "zone"), { recursive: true });
  writeFileSync(
    join(scripts, "zone", "exclude.lua"),
    "-- zone.exclude: drops zones from Goals.zones()\n-- rule: skip Redridge Mountains | why: 5 deaths in 2 h to Black Dragon Whelps | set: 2026-11-27T12:00Z\nlocal M = {}\nreturn M\n",
  );
  mkdirSync(join(home, "work", "notes"), { recursive: true });
  writeFileSync(join(home, "work", "notes", "rules.md"), "# My rules\n- rule: no Deeprun Tram after a death | why: the corpse run\n");
  const rules = readRules(scripts, join(home, "work"));
  expect(rules.map((r) => [r.where, r.rule])).toEqual([
    ["zone.exclude", "skip Redridge Mountains"],
    ["notes/rules.md", "no Deeprun Tram after a death"],
  ]);
  expect(readRules(undefined, join(home, "nowhere"))).toEqual([]);
  const reviewer = new Reviewer({}, T0 + 60 * MIN);
  const review = reviewer.review(play(T0, 90, 0, 500), rules, T0 + 90 * MIN);
  expect(review.rules).toEqual([
    'rule "skip Redridge Mountains" (set 2026-11-27T12:00Z; why: 5 deaths in 2 h to Black Dragon Whelps; in zone.exclude) · since: 0 XP over 1.5 h, 0 quests, 0 deaths, 0 new places',
    'rule "no Deeprun Tram after a death" (why: the corpse run; in notes/rules.md) · since: not known (no set date)',
  ]);
  expect(review.decision).toBeUndefined();
  rmSync(home, { recursive: true });
});

test("three check-ins in a row with no experience ask for a decision; a window out of the world does not count", () => {
  const rule = { rule: "skip Westfall", set: "2026-11-27T12:00Z", setAt: T0 };
  const reviewer = new Reviewer({}, T0);
  let samples = play(T0, 20, 0, 100);
  expect(reviewer.review(samples, [rule], T0 + 20 * MIN).decision).toBeUndefined();
  samples = [...samples, ...play(T0 + 20 * MIN, 20, 0, 100)];
  expect(reviewer.review(samples, [rule], T0 + 40 * MIN).decision).toBeUndefined();
  // Out of the world for this check-in: no samples, so it neither counts nor ends the run.
  expect(reviewer.review(samples, [rule], T0 + 60 * MIN).decision).toBeUndefined();
  samples = [...samples, ...play(T0 + 60 * MIN, 20, 0, 100)];
  const review = reviewer.review(samples, [rule], T0 + 80 * MIN);
  expect(review.decision).toStartWith("STALL: no experience in your last 3 check-ins (1.0 h).");
  expect(review.decision).toContain("revise or drop one of your own rules listed above");
  expect(review.decision).toContain(KEEP);
  expect(review.decision).not.toContain(NO_RULES);
  // Experience ends it.
  samples = [...samples, ...play(T0 + 80 * MIN, 20, 400, 100)];
  expect(reviewer.review(samples, [rule], T0 + 100 * MIN).decision).toBeUndefined();
});

test("a stall with no rules says how to record one; the quest span is its own threshold", () => {
  const reviewer = new Reviewer({ xpWindows: 0, questHours: 0.5 }, T0);
  const samples = play(T0, 40, 2000);
  const review = reviewer.review(samples, [], T0 + 40 * MIN);
  expect(review.decision).toStartWith("STALL: no quest handed in for 40 min.");
  expect(review.decision).toEndWith(NO_RULES);
});

test("the review rides on check-ins and reflections only; a reflection passes it to the reflector", () => {
  const review = { rules: ['rule "skip Redridge" · since: 0 XP over 2.0 h, 0 quests, 0 deaths'], decision: "STALL: no experience in your last 3 check-ins (1.0 h). Decide." };
  const checkin = wakeMessage("checkin", [], "s", 20, [], review);
  expect(checkin).toContain('Rules you set yourself, and what has happened since each was set:\n- rule "skip Redridge"');
  expect(checkin).toContain("\n\nSTALL: no experience");
  expect(checkin).not.toContain("Give the reflector");
  expect(wakeMessage("reflect", [], "s", 20, [], review)).toContain("Give the reflector the rules and any stall above in its task.");
  expect(wakeMessage("events", ["You died."], "s", 20, [], review)).not.toContain("Rules you set yourself");
  expect(wakeMessage("checkin", [], "s", 20, [], { rules: [] })).not.toContain("Rules you set yourself");
});

test("a rule line counts from set: (when made), else from recorded:, and a stall says how to give set:", () => {
  // Made at T0, written down an hour later: the cost covers both hours.
  const made = { rule: "skip Westfall", set: "2026-11-27T12:00Z", setAt: T0, recorded: "2026-11-27T13:00Z", recordedAt: T0 + 60 * MIN };
  const only = { rule: "skip Duskwood", recorded: "2026-11-27T13:00Z", recordedAt: T0 + 60 * MIN };
  const reviewer = new Reviewer({ xpWindows: 1 }, T0 + 100 * MIN);
  const review = reviewer.review(play(T0, 120, 0, 100), [made, only], T0 + 120 * MIN);
  expect(review.rules).toEqual([
    'rule "skip Westfall" (set 2026-11-27T12:00Z) · since: 0 XP over 2.0 h, 0 quests, 0 deaths, 0 new places',
    'rule "skip Duskwood" (recorded 2026-11-27T13:00Z) · since recorded: 0 XP over 1.0 h, 0 quests, 0 deaths, 0 new places',
  ]);
  expect(review.note).toBe(OLDER_THAN_RECORD);
  const message = wakeMessage("checkin", [], "s", 20, [], review);
  expect(message).toContain(`since recorded: 0 XP over 1.0 h, 0 quests, 0 deaths, 0 new places\n${OLDER_THAN_RECORD}\n\nSTALL:`);
  // No stall: no note; no record-only rule: no note.
  expect(new Reviewer({}, T0 + 100 * MIN).review(play(T0, 120, 0, 100), [only], T0 + 120 * MIN).note).toBeUndefined();
  expect(new Reviewer({ xpWindows: 1 }, T0 + 100 * MIN).review(play(T0, 120, 0, 100), [made], T0 + 120 * MIN).note).toBeUndefined();
});
