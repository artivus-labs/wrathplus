import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Notes } from "../src/notes.ts";

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
