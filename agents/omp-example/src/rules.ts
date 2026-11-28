// The rules the agent set itself, with what each has cost, and the decision a stall asks for: for
// the check-ins' and reflections' messages. Rules are `-- rule:` lines in the header of the
// module that enforces them, or `rule:` lines in notes/rules.md (agents/mcp/src/rules.ts has the
// format); costs and stalls come from the scorecard's samples, which the MCP server keeps. The
// launcher only reports: keeping, revising or dropping a rule is the planner's call.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  costFromSamples,
  costStart,
  decisionRequest,
  fromRecordOnly,
  type Rule,
  ruleLine,
  rulesInFolder,
  rulesInNotes,
  StallWatch,
  type StallOptions,
} from "../../mcp/src/rules.ts";
import type { Sample } from "../../mcp/src/scorecard.ts";

export const NOTES_RULES = "notes/rules.md";

/** How a reasoned keep is recorded here, for the decision a stall asks for. */
export const KEEP =
  "add `| kept: <date> because <reason>` to the line of each rule you keep, and a dated `keep: because ...` line in notes/journal.md for anything else you keep";

/** Said with a stall when no rule is recorded. */
export const NO_RULES =
  "You have no rules recorded: if one of your modules or notes holds a limit you set yourself (a zone you skip, a mob family you refuse, a quest you abandon), add a `-- rule: what | why: reason | set: YYYY-MM-DD HH:MM` line to its header comment (or a `rule:` line in notes/rules.md) so its cost is shown here; `set:` is when you made the rule, not today.";

/** Under the rules at a stall, when a rule has `recorded:` but no `set:`. */
export const OLDER_THAN_RECORD =
  "\"since recorded\" counts from when you wrote the rule down. If you made it earlier, add `| set: <when you made it>` to its line.";

/** Every rule: module headers under the modules folder, then notes/rules.md in the working dir. */
export function readRules(modulesFolder: string | undefined, workDir: string): Rule[] {
  const notes = join(workDir, NOTES_RULES);
  return [
    ...(modulesFolder ? rulesInFolder(modulesFolder) : []),
    ...(existsSync(notes) ? rulesInNotes(readFileSync(notes, "utf8"), NOTES_RULES) : []),
  ];
}

/** What a check-in or reflection message adds: each rule with its cost, and a stall's decision. */
export interface Review {
  rules: string[];
  /** Under the rules at a stall: a rule whose cost counts only from its record. */
  note?: string;
  decision?: string;
}

/** Follows the check-in windows on the scorecard and builds each message's review. */
export class Reviewer {
  readonly stalls: StallWatch;
  #since: number;

  constructor(
    stall: Partial<StallOptions>,
    now: number,
    /** A gap between samples longer than this is time away (the MCP server's own rule). */
    readonly maxGapMs = 90_000,
  ) {
    this.stalls = new StallWatch(stall);
    this.#since = now;
  }

  /** The review for a window ending `now`, from the samples (all of them; empty without the
   *  scorecard) and the rules. */
  review(samples: Sample[], rules: Rule[], now: number): Review {
    const window = costFromSamples(samples, this.#since, this.maxGapMs);
    const wallMinutes = (now - this.#since) / 60_000;
    this.#since = now;
    // Mostly out of the world (or no scorecard): it neither counts toward a stall nor ends one.
    const minutes = (window?.hours ?? 0) * 60;
    const stall = this.stalls.add({
      minutes,
      xp: window?.xp ?? 0,
      quests: window?.quests ?? 0,
      counted: window !== undefined && minutes * 2 >= wallMinutes,
    });
    const lines = rules.map((r) => {
      const from = costStart(r);
      return ruleLine(r, from === undefined ? undefined : costFromSamples(samples, from, this.maxGapMs));
    });
    if (!stall) return { rules: lines };
    const decision = decisionRequest(stall, KEEP, rules.length, "check-ins");
    return {
      rules: lines,
      ...(rules.some(fromRecordOnly) ? { note: OLDER_THAN_RECORD } : {}),
      decision: rules.length ? decision : `${decision} ${NO_RULES}`,
    };
  }
}
