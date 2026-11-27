import { describe, expect, test } from "bun:test";
import { mkdirSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  costFromSamples,
  costText,
  decisionRequest,
  latestSamplesFile,
  parseRuleLine,
  parseWhen,
  readSamples,
  ruleLine,
  rulesInFolder,
  rulesInModule,
  rulesInNotes,
  StallWatch,
  stallText,
} from "../src/rules.ts";
import { sample, scratch } from "./fixtures.ts";

const HOUR = 3_600_000;

describe("rule lines", () => {
  test("a module header line round-trips its fields", () => {
    const rule = parseRuleLine(
      "-- rule: skip Redridge Mountains | why: 5 deaths in 2 h to Black Dragon Whelps | set: 2026-11-27 16:25 | applies: Redridge | kept: 2026-11-28 08:00 because no gear yet",
      "zone.exclude",
    );
    expect(rule).toEqual({
      rule: "skip Redridge Mountains",
      why: "5 deaths in 2 h to Black Dragon Whelps",
      set: "2026-11-27 16:25",
      setAt: parseWhen("2026-11-27 16:25")!,
      applies: "Redridge",
      kept: { at: "2026-11-28 08:00", because: "no gear yet" },
      where: "zone.exclude",
    });
    expect(parseRuleLine("- rule: keep north of y -1000")).toEqual({ rule: "keep north of y -1000" });
    expect(parseRuleLine("-- zone.exclude: Goals.zones() lists the zones")).toBeUndefined();
    expect(parseRuleLine("rule:   ")).toBeUndefined();
  });

  test("dates parse with or without a time or a zone", () => {
    expect(parseWhen("2026-11-26T14:10Z")).toBe(Date.UTC(2026, 10, 26, 14, 10));
    expect(parseWhen("2026-11-26 14:10 UTC")).toBe(Date.UTC(2026, 10, 26, 14, 10));
    expect(parseWhen("2026-11-26 9:05")).toBe(new Date(2026, 10, 26, 9, 5).getTime());
    expect(parseWhen("2026-11-26")).toBe(new Date(2026, 10, 26).getTime());
    expect(parseWhen("last Tuesday")).toBeUndefined();
    expect(parseWhen(undefined)).toBeUndefined();
  });

  test("only the header comment of a module is read; notes are read throughout", () => {
    // A module describing its own logic as "Rule: condition -> action" is not a self-imposed rule.
    const source =
      "-- mob.avoid: refuses families that kill us\n-- Rule: out of combat, hp < 90% -> Professions.bandage('player').\n-- rule: no pulls on Rockjaw | why: 7 deaths | set: 2026-11-25\nlocal M = {}\n-- rule: not a header line | why: x\nreturn M\n";
    expect(rulesInModule(source, "mob.avoid").map((r) => r.rule)).toEqual(["no pulls on Rockjaw"]);
    const notes = "# Rules\n- rule: no Deeprun Tram at night | why: lag | set: 2026-11-20\nsome text\nrule: stay in Westfall | set: 2026-11-26\n";
    expect(rulesInNotes(notes, "notes/rules.md").map((r) => [r.rule, r.where])).toEqual([
      ["no Deeprun Tram at night", "notes/rules.md"],
      ["stay in Westfall", "notes/rules.md"],
    ]);
  });

  test("a modules folder is walked into dotted module names", () => {
    const dir = scratch();
    mkdirSync(join(dir, "zone"), { recursive: true });
    writeFileSync(join(dir, "zone", "exclude.lua"), "-- rule: skip Redridge | set: 2026-11-27\nreturn {}\n");
    writeFileSync(join(dir, "autoexec.lua"), "require('zone.exclude')\n");
    expect(rulesInFolder(dir).map((r) => [r.where, r.rule])).toEqual([["zone.exclude", "skip Redridge"]]);
    expect(rulesInFolder(join(dir, "missing"))).toEqual([]);
    rmSync(dir, { recursive: true });
  });
});

describe("cost", () => {
  test("the cost since a rule was set comes from the scorecard's samples", () => {
    const t0 = Date.UTC(2026, 10, 27, 12);
    const samples = [
      sample(t0 - HOUR, { xp: 0 }),
      sample(t0, { xp: 500 }),
      // Under the rule: an hour and a half, grey mobs, nothing gained.
      ...[1, 2, 3].map((i) => sample(t0 + (i * HOUR) / 2, { xp: 500 })),
    ];
    const cost = costFromSamples(samples, t0, 3 * HOUR)!;
    expect(cost).toEqual({ hours: 1.5, xp: 0, quests: 0, deaths: 0, places: 0 });
    expect(costText(cost)).toBe("0 XP over 1.5 h, 0 quests, 0 deaths, 0 new places");
    expect(costFromSamples(samples, t0 + 10 * HOUR, 3 * HOUR)).toBeUndefined();
  });

  test("a rule's line: what, when, why, where, and what it cost since", () => {
    const rule = parseRuleLine("rule: no walk south of y -1000 | why: 2 deaths on the Rockjaw road | set: 2026-11-26 14:10Z", "MEMORY.md")!;
    expect(ruleLine(rule, { hours: 9, xp: 0, quests: 0, deaths: 0 })).toBe(
      'rule "no walk south of y -1000" (set 2026-11-26 14:10Z; why: 2 deaths on the Rockjaw road; in MEMORY.md) · since: 0 XP over 9.0 h, 0 quests, 0 deaths',
    );
    expect(ruleLine({ rule: "x" }, undefined)).toBe('rule "x" · since: not known (no set date)');
    expect(ruleLine({ ...rule, id: "2", kept: { at: "11-28", because: "the road kills" } }, undefined)).toContain(
      '#2 rule "no walk south of y -1000"',
    );
    expect(ruleLine({ ...rule, kept: { because: "the road kills" } }, { hours: 0.5, xp: 1200, quests: 1, deaths: 1 })).toEndWith(
      "since: 1,200 XP over 30 min, 1 quest, 1 death · kept because the road kills",
    );
  });

  test("the samples file of the character sampled last", () => {
    const dir = scratch();
    for (const [name, age] of [["Beta-Old", 1000], ["Beta-New", 0]] as const) {
      mkdirSync(join(dir, "characters", name), { recursive: true });
      const path = join(dir, "characters", name, "samples.jsonl");
      writeFileSync(path, `${JSON.stringify(sample(1))}\nnot json\n`);
      const at = new Date(Date.now() - age * 1000);
      utimesSync(path, at, at);
    }
    const path = latestSamplesFile(dir)!;
    expect(path).toEndWith(join("Beta-New", "samples.jsonl"));
    expect(readSamples(path)).toHaveLength(1);
    expect(latestSamplesFile(join(dir, "none"))).toBeUndefined();
    rmSync(dir, { recursive: true });
  });
});

describe("stalls", () => {
  test("experience flat for three counted windows is a stall; any experience resets it", () => {
    const watch = new StallWatch();
    expect(watch.add({ minutes: 30, xp: 0, quests: 0 })).toBeUndefined();
    expect(watch.add({ minutes: 30, xp: 0, quests: 0 })).toBeUndefined();
    // A short window (a death's review) adds its minutes but is not a window of its own.
    expect(watch.add({ minutes: 5, xp: 0, quests: 0 })).toBeUndefined();
    // A window out of the world neither counts nor resets.
    expect(watch.add({ minutes: 30, xp: 0, quests: 0, counted: false })).toBeUndefined();
    const stall = watch.add({ minutes: 30, xp: 0, quests: 0 })!;
    expect(stall).toEqual({ xpWindows: 3, xpMinutes: 95, questMinutes: 95, xp: true, quests: false });
    expect(stallText(stall)).toBe("STALL: no experience in your last 3 reviews (1.6 h).");
    expect(watch.add({ minutes: 30, xp: 40, quests: 0 })).toBeUndefined();
  });

  test("no quest for the quest span is a stall of its own, while experience comes in", () => {
    const watch = new StallWatch({ questHours: 2 });
    for (let i = 0; i < 3; i++) expect(watch.add({ minutes: 30, xp: 900, quests: 0 })).toBeUndefined();
    const stall = watch.add({ minutes: 30, xp: 900, quests: 0 })!;
    expect(stall.quests).toBe(true);
    expect(stall.xp).toBe(false);
    expect(stallText(stall, "check-ins")).toBe("STALL: no quest handed in for 2.0 h.");
    expect(watch.add({ minutes: 30, xp: 900, quests: 1 })).toBeUndefined();
  });

  test("thresholds are configurable, and 0 turns one off", () => {
    const watch = new StallWatch({ xpWindows: 0, questHours: 0 });
    for (let i = 0; i < 20; i++) expect(watch.add({ minutes: 60, xp: 0, quests: 0 })).toBeUndefined();
    const quick = new StallWatch({ xpWindows: 1 });
    expect(quick.add({ minutes: 15, xp: 0, quests: 3 })?.xp).toBe(true);
  });

  test("the decision request names the stall, asks for a change or a reasoned keep, and suggests no change", () => {
    const stall = { xpWindows: 18, xpMinutes: 540, questMinutes: 540, xp: true, quests: true };
    const text = decisionRequest(stall, "rule_keep(id, because)", 1);
    expect(text).toStartWith("STALL: no experience in your last 18 reviews (9.0 h); no quest handed in for 9.0 h.");
    expect(text).toContain("revise or drop one of your own rules");
    expect(text).toContain("rule_keep(id, because)");
    expect(text).toContain('"No change" without a reason is not an answer.');
    expect(decisionRequest(stall, "x", 0)).not.toContain("your own rules");
  });
});
