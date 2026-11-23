// Decaying needs: targets that rise with level, so a character that only levels falls behind on
// them. Each is reported as a gap (0 = met) and as that gap's share of its target, which the
// scorecard sums into one needs number a trial can compare. The table is the one place to adjust.

/** The parts of a sample the needs read. */
export interface NeedInput {
  level: number;
  money: number;
  gear: { average: number; items: number };
  professions: Record<string, { rank: number; max: number }>;
}

export interface NeedsTable {
  /** Each known profession's skill within `slack` of `target(level)`. */
  profession: { target: (level: number) => number; slack: number };
  /** The equipped items' average item level within `slack` of `target(level)`. */
  gear: { target: (level: number) => number; slack: number };
  /** Money (copper) held in reserve: at least `reserve(level)`. */
  gold: { reserve: (level: number) => number };
}

/** 3.3.5's rank caps open every 5-15 levels up to 450; five points a level keeps pace with them.
 *  Gear: what a quest reward at your level is. Gold: training, repairs and the riding skill. */
export const NEEDS: NeedsTable = {
  profession: { target: (level) => Math.min(450, 5 * level), slack: 25 },
  gear: { target: (level) => level, slack: 5 },
  gold: { reserve: (level) => 25 * level * level },
};

export interface NeedGap {
  need: string;
  /** What is wanted and what there is, in the need's own unit. */
  target: number;
  have: number;
  /** How far short, in that unit (0 = met). */
  gap: number;
  /** The gap as a share of the target, 0-1. */
  share: number;
  unit: string;
}

function need(name: string, target: number, have: number, slack: number, unit: string): NeedGap {
  const gap = Math.max(0, target - slack - have);
  return { need: name, target, have, gap, share: target > 0 ? Math.min(1, gap / target) : 0, unit };
}

/** Every need's gap for a character now. */
export function gaps(s: NeedInput, table: NeedsTable = NEEDS): NeedGap[] {
  const out: NeedGap[] = [];
  for (const [name, p] of Object.entries(s.professions).sort(([a], [b]) => a.localeCompare(b))) {
    out.push(need(`profession:${name}`, table.profession.target(s.level), p.rank, table.profession.slack, "skill"));
  }
  if (s.gear.items > 0) {
    out.push(need("gear", table.gear.target(s.level), Math.round(s.gear.average * 10) / 10, table.gear.slack, "item level"));
  }
  out.push(need("gold", table.gold.reserve(s.level), s.money, 0, "copper"));
  return out;
}
