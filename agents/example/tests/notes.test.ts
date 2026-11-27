import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Notes, notesServer, sharedRule } from "../src/notes.ts";
import { connect } from "./mcp.ts";

const scratch = () => mkdtempSync(join(tmpdir(), "wp-notes-"));

test("memory and to-dos persist in the agent's folder", () => {
  const home = scratch();
  const notes = new Notes(home);
  expect(notes.memory()).toBe("");
  notes.appendMemory("Kobold Vermin are at the camp north-east of the abbey.");
  notes.appendMemory("UnitReaction-style values come back as strings.");
  expect(new Notes(home).memory()).toBe(
    "Kobold Vermin are at the camp north-east of the abbey.\nUnitReaction-style values come back as strings.\n",
  );
  expect(notes.writeMemory("x".repeat(30_000))).toContain("Not saved");
  notes.writeTodos([
    { text: "Hand in A Threat Within", status: "done" },
    { text: "Kill 8 kobolds", status: "in_progress", note: "3/8" },
  ]);
  expect(new Notes(home).todos().map((t) => [t.id, t.status])).toEqual([
    [1, "done"],
    [2, "in_progress"],
  ]);
  rmSync(home, { recursive: true });
});

test("sessions are logged and searchable, and the next session is briefed from the last", () => {
  const home = scratch();
  const first = new Notes(home);
  first.begin("11111111-aaaa");
  first.record("planner", "Walking to Marshal McBride at -8902, -162.");
  first.record("result", "Quest log: Kobold Camp Cleanup 8/8");
  expect(first.search("mcbride")).toContain("Marshal McBride at -8902");
  expect(first.search("no such words")).toContain("No matches");
  const second = new Notes(home);
  second.writeTodos([{ text: "Find the next quest", status: "pending" }]);
  const briefing = second.briefing();
  expect(briefing).toContain("MEMORY.md is empty");
  expect(briefing).toContain("1. [pending] Find the next quest");
  expect(briefing).toContain("Kobold Camp Cleanup 8/8");
  // Once the second session logs, the briefing still shows the first one as "last".
  Bun.sleepSync(5);
  second.begin("22222222-bbbb");
  second.record("planner", "New session.");
  expect(second.lastSession()).toContain("Walking to Marshal McBride");
  rmSync(home, { recursive: true });
});

test("rules the agent sets itself round-trip with why, when and what carries them out", async () => {
  const home = scratch();
  const notes = new Notes(home);
  const set = new Date("2026-11-26T14:10:00Z");
  const first = notes.setRule(
    { rule: "no walk south of y -1000", why: "2 deaths on the Rockjaw road", enforced_by: "paladin.zonehold (zone_fit forced ok)" },
    set,
  );
  expect(typeof first === "object" && first.id).toBe(1);
  expect(new Notes(home).rules()).toEqual([
    { id: 1, rule: "no walk south of y -1000", why: "2 deaths on the Rockjaw road", set_at: set.toISOString(), enforced_by: "paladin.zonehold (zone_fit forced ok)" },
  ]);
  expect(sharedRule(notes.rules()[0]!)).toEqual({
    id: "1",
    rule: "no walk south of y -1000",
    why: "2 deaths on the Rockjaw road",
    set: "2026-11-26 14:10Z",
    setAt: set.getTime(),
    where: "paladin.zonehold (zone_fit forced ok)",
  });
  expect(notes.keepRule(1, "Rockjaw still kills at level 14", new Date("2026-11-28T08:00:00Z"))).toContain("Kept rule #1");
  expect(sharedRule(notes.rules()[0]!).kept).toEqual({ at: "2026-11-28 08:00Z", because: "Rockjaw still kills at level 14" });
  expect(notes.keepRule(9, "x")).toContain("No rule #9");
  // A revision drops the old rule and records the new one; a drop keeps the history.
  const revised = notes.setRule({ rule: "no walk south of y -1300", why: "the deaths were at y -1150", replaces: 1 });
  expect(typeof revised === "object" && revised.replaces).toBe(1);
  expect(notes.rules().map((r) => r.id)).toEqual([2]);
  expect(notes.setRule({ rule: "x", why: "y", replaces: 1 })).toContain("No rule #1 in force");
  expect(notes.dropRule(2, "Loch Modan reached by the tram")).toBe("Dropped rule #2: no walk south of y -1300.");
  expect(notes.rules()).toEqual([]);
  expect(notes.ruleRecords().map((r) => [r.id, r.dropped?.why])).toEqual([
    [1, "revised as: no walk south of y -1300"],
    [2, "Loch Modan reached by the tram"],
  ]);
  expect(notes.describeRules()).toBe("No rules recorded.");
  // Through the planner's tools, and in the briefing.
  const call = await connect(notesServer(notes));
  expect((await call("rule_set", { rule: "skip Rockjaw quests", why: "objectives in the camp", enforced_by: "paladin.skipquest" })).text).toBe(
    "Rule #3 recorded: skip Rockjaw quests.",
  );
  expect((await call("rules")).text).toMatch(/^#3 skip Rockjaw quests \(set .*Z; why: objectives in the camp; in paladin.skipquest\)$/);
  expect((await call("rule_keep", { id: 3, because: "still lethal" })).text).toContain("Kept rule #3");
  expect(notes.briefing()).toContain("Rules you set yourself");
  expect((await call("rule_drop", { id: 3, why: "done" })).text).toBe("Dropped rule #3: skip Rockjaw quests. Undo what carries it out too: paladin.skipquest.");
  rmSync(home, { recursive: true });
});
