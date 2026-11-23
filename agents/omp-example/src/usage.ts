// Token usage per model call, from OMP's RPC events, appended to a JSONL file so spend is visible:
// one line per assistant message (the planner's and its subagents'), plus the session's own
// running totals after each turn.

import { appendFileSync, existsSync, readFileSync } from "node:fs";

export interface UsageLine {
  ts: string;
  kind: "assistant" | "subagent";
  session?: string;
  model?: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  /** USD, as OMP prices it from its model catalog. */
  cost: number;
  stopReason?: string;
}

const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);

/** The usage an assistant message carries, or null for anything else. */
export function usageOf(message: unknown): Omit<UsageLine, "ts" | "kind" | "session"> | null {
  if (!message || typeof message !== "object") return null;
  const m = message as Record<string, unknown>;
  if (m.role !== "assistant" || !m.usage || typeof m.usage !== "object") return null;
  const u = m.usage as Record<string, unknown>;
  const cost = u.cost && typeof u.cost === "object" ? num((u.cost as Record<string, unknown>).total) : num(u.cost);
  return {
    model: typeof m.model === "string" ? `${typeof m.provider === "string" ? `${m.provider}/` : ""}${m.model}` : undefined,
    input: num(u.input),
    output: num(u.output),
    cacheRead: num(u.cacheRead),
    cacheWrite: num(u.cacheWrite),
    cost,
    stopReason: typeof m.stopReason === "string" ? m.stopReason : undefined,
  };
}

/** The usage line for an RPC frame: an assistant `message_end`, the planner's own or a
 *  subagent's (forwarded as `subagent_event` with the subagent subscription at "events"). */
export function usageFromFrame(frame: unknown, session?: string, now = new Date()): UsageLine | null {
  if (!frame || typeof frame !== "object") return null;
  const f = frame as Record<string, unknown>;
  if (f.type === "message_end") {
    const u = usageOf(f.message);
    return u ? { ts: now.toISOString(), kind: "assistant", session, ...u } : null;
  }
  if (f.type === "subagent_event") {
    const inner = (f.event ?? f.payload) as Record<string, unknown> | undefined;
    if (!inner || inner.type !== "message_end") return null;
    const u = usageOf(inner.message);
    return u ? { ts: now.toISOString(), kind: "subagent", session, ...u } : null;
  }
  return null;
}

export function appendJsonl(path: string, value: unknown): void {
  appendFileSync(path, `${JSON.stringify(value)}\n`);
}

export interface UsageTotals {
  calls: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
}

/** Sum the per-call lines of a usage file written since `since` (ISO time; all when omitted). */
export function totals(path: string, since?: string): UsageTotals {
  const t: UsageTotals = { calls: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 };
  if (!existsSync(path)) return t;
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    if (!raw.trim()) continue;
    let line: Partial<UsageLine>;
    try {
      line = JSON.parse(raw) as Partial<UsageLine>;
    } catch {
      continue;
    }
    if (line.kind !== "assistant" && line.kind !== "subagent") continue;
    if (since && (line.ts ?? "") < since) continue;
    t.calls += 1;
    t.input += num(line.input);
    t.output += num(line.output);
    t.cacheRead += num(line.cacheRead);
    t.cacheWrite += num(line.cacheWrite);
    t.cost += num(line.cost);
  }
  return t;
}
