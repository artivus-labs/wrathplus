// The month's spend, persisted on disk, and the budget check.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { costUsd, type Price, type Usage } from "./pricing.ts";

/** Calendar month in UTC, "YYYY-MM". */
export function monthOf(date: Date): string {
  return date.toISOString().slice(0, 7);
}

/** First instant of the next UTC month. */
export function nextMonthStart(date: Date): Date {
  return new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 1));
}

export interface MonthTotals {
  month: string;
  usd: number;
  /** Calls answered by the API (and billed). */
  calls: number;
  /** Calls refused here because the budget was spent. */
  refused: number;
  /** Calls the API answered with an error (not billed). */
  errors: number;
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
  /** Budget percentages already logged this month. */
  alerted: number[];
}

interface State {
  current: MonthTotals;
  /** Totals of earlier months, newest last (at most 12). */
  history: Pick<MonthTotals, "month" | "usd" | "calls" | "refused">[];
}

export const THRESHOLDS = [50, 80, 100];

function empty(month: string): MonthTotals {
  return {
    month,
    usd: 0,
    calls: 0,
    refused: 0,
    errors: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_input_tokens: 0,
    cache_read_input_tokens: 0,
    alerted: [],
  };
}

export interface MeterOptions {
  /** JSON state file; written atomically after every change. */
  path: string;
  capUsd: number;
  now?: () => Date;
  log?: (line: string) => void;
}

export class Meter {
  readonly capUsd: number;
  private readonly path: string;
  private readonly now: () => Date;
  private readonly log: (line: string) => void;
  private state: State;

  constructor(options: MeterOptions) {
    if (!(options.capUsd > 0)) throw new Error("monthly budget must be a positive number of USD");
    this.path = options.path;
    this.capUsd = options.capUsd;
    this.now = options.now ?? (() => new Date());
    this.log = options.log ?? ((line) => console.log(line));
    this.state = this.load();
    this.roll();
  }

  private load(): State {
    if (!existsSync(this.path)) return { current: empty(monthOf(this.now())), history: [] };
    const parsed = JSON.parse(readFileSync(this.path, "utf8")) as State;
    return { current: { ...empty(parsed.current.month), ...parsed.current }, history: parsed.history ?? [] };
  }

  private save(): void {
    mkdirSync(dirname(this.path), { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(tmp, `${JSON.stringify(this.state, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, this.path);
  }

  /** Start a fresh month when the calendar month (UTC) has changed. */
  private roll(): MonthTotals {
    const month = monthOf(this.now());
    const current = this.state.current;
    if (current.month !== month) {
      const { usd, calls, refused } = current;
      this.state.history = [...this.state.history, { month: current.month, usd, calls, refused }].slice(-12);
      this.state.current = empty(month);
      this.log(`llm-guard: new month ${month}; ${current.month} closed at $${usd.toFixed(4)} over ${calls} calls`);
      this.save();
    }
    return this.state.current;
  }

  /** True while this month's spend is below the budget. */
  allow(): boolean {
    return this.roll().usd < this.capUsd;
  }

  refuse(): void {
    this.roll().refused += 1;
    this.save();
  }

  error(): void {
    this.roll().errors += 1;
    this.save();
  }

  /** Add one answered call; returns its cost in USD. */
  record(usage: Usage, price: Price): number {
    const totals = this.roll();
    const cost = costUsd(usage, price);
    totals.usd += cost;
    totals.calls += 1;
    totals.input_tokens += usage.input_tokens ?? 0;
    totals.output_tokens += usage.output_tokens ?? 0;
    totals.cache_creation_input_tokens += usage.cache_creation_input_tokens ?? 0;
    totals.cache_read_input_tokens += usage.cache_read_input_tokens ?? 0;
    for (const percent of THRESHOLDS) {
      if (totals.alerted.includes(percent) || totals.usd < (this.capUsd * percent) / 100) continue;
      totals.alerted.push(percent);
      const tail =
        percent >= 100
          ? `; refusing calls until ${nextMonthStart(this.now()).toISOString()}`
          : "";
      this.log(
        `llm-guard: ${totals.month} reached ${percent}% of the $${this.capUsd} budget ($${totals.usd.toFixed(4)}, ${totals.calls} calls)${tail}`,
      );
    }
    this.save();
    return cost;
  }

  /** The month so far, for GET /usage. */
  snapshot() {
    const totals = this.roll();
    const { alerted: _alerted, ...rest } = totals;
    return {
      ...rest,
      usd: Number(totals.usd.toFixed(6)),
      cap_usd: this.capUsd,
      percent: Number(((totals.usd / this.capUsd) * 100).toFixed(2)),
      over_budget: totals.usd >= this.capUsd,
      resets_at: nextMonthStart(this.now()).toISOString(),
      prompt_tokens: totals.input_tokens + totals.cache_creation_input_tokens + totals.cache_read_input_tokens,
      history: this.state.history,
    };
  }
}
