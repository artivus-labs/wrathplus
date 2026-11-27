// The agent's own notes, kept in its home folder between sessions: a to-do list, a MEMORY.md for
// whatever it wants to remember, and a JSONL log of every session it can search. Plain files,
// nothing clever: the point is a consistent agent that keeps its bearings across restarts and
// compactions.
//
//   <home>/MEMORY.md            free-form notes
//   <home>/todo.json            [{ id, text, status: pending | in_progress | done, note? }]
//   <home>/sessions/*.jsonl     one line per message: { at, session, kind, text }
//   <home>/rules.json           rules it set itself: [{ id, rule, why, set_at, enforced_by?, kept?, dropped? }]

import { createSdkMcpServer, tool } from "@anthropic-ai/claude-agent-sdk";
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import type { Rule } from "../../mcp/src/rules.ts";

export interface Todo {
  id: number;
  text: string;
  status: "pending" | "in_progress" | "done";
  note?: string;
}

/** A rule the agent set itself (`rules.json`): what, why, when, and what enforces it. Dropped
 *  rules stay in the file, marked, as their history. */
export interface RuleRecord {
  id: number;
  rule: string;
  why: string;
  /** ISO time it was set. */
  set_at: string;
  applies?: string;
  /** What carries it out: a module, a fact override, a note in MEMORY.md, a habit. */
  enforced_by?: string;
  /** The last reasoned keep. */
  kept?: { at: string; because: string };
  dropped?: { at: string; why: string };
  /** The rule this one revised. */
  replaces?: number;
}

const MEMORY_LIMIT = 20_000;

const minute = (iso: string) => `${iso.slice(0, 16).replace("T", " ")}Z`;

/** A record as the shared rule shape (agents/mcp/src/rules.ts), for the reflection's cost lines. */
export function sharedRule(r: RuleRecord): Rule {
  const set = minute(r.set_at);
  return {
    id: String(r.id),
    rule: r.rule,
    why: r.why,
    set,
    ...(Number.isFinite(Date.parse(r.set_at)) ? { setAt: Date.parse(r.set_at) } : {}),
    ...(r.applies ? { applies: r.applies } : {}),
    ...(r.enforced_by ? { where: r.enforced_by } : {}),
    ...(r.kept ? { kept: { at: minute(r.kept.at), because: r.kept.because } } : {}),
  };
}

export class Notes {
  readonly sessions: string;
  #log: string | undefined;
  #session = "";

  constructor(readonly home: string) {
    this.sessions = join(home, "sessions");
    mkdirSync(this.sessions, { recursive: true });
  }

  memory(): string {
    const path = join(this.home, "MEMORY.md");
    return existsSync(path) ? readFileSync(path, "utf8") : "";
  }

  writeMemory(text: string): string {
    if (text.length > MEMORY_LIMIT) {
      return `Not saved: MEMORY.md would be ${text.length} characters; keep it under ${MEMORY_LIMIT} by summarising older notes.`;
    }
    writeFileSync(join(this.home, "MEMORY.md"), text);
    return `MEMORY.md saved (${text.length} characters).`;
  }

  appendMemory(text: string): string {
    const current = this.memory();
    return this.writeMemory(`${current}${current && !current.endsWith("\n") ? "\n" : ""}${text.trim()}\n`);
  }

  todos(): Todo[] {
    const path = join(this.home, "todo.json");
    if (!existsSync(path)) return [];
    try {
      return JSON.parse(readFileSync(path, "utf8")) as Todo[];
    } catch {
      return [];
    }
  }

  writeTodos(items: Omit<Todo, "id">[]): Todo[] {
    const todos = items.map((item, i) => ({ ...item, id: i + 1 }));
    writeFileSync(join(this.home, "todo.json"), JSON.stringify(todos, null, 2));
    return todos;
  }

  /** Every rule ever recorded, dropped ones included. */
  ruleRecords(): RuleRecord[] {
    const path = join(this.home, "rules.json");
    if (!existsSync(path)) return [];
    try {
      const all = JSON.parse(readFileSync(path, "utf8")) as unknown;
      return Array.isArray(all) ? (all as RuleRecord[]) : [];
    } catch {
      return [];
    }
  }

  /** The rules in force. */
  rules(): RuleRecord[] {
    return this.ruleRecords().filter((r) => !r.dropped);
  }

  #saveRules(all: RuleRecord[]): void {
    writeFileSync(join(this.home, "rules.json"), JSON.stringify(all, null, 2));
  }

  /** Record a rule (and drop the one it revises). */
  setRule(input: { rule: string; why: string; applies?: string; enforced_by?: string; replaces?: number }, now = new Date()): RuleRecord | string {
    const all = this.ruleRecords();
    const at = now.toISOString();
    if (input.replaces !== undefined) {
      const old = all.find((r) => r.id === input.replaces && !r.dropped);
      if (!old) return `No rule #${input.replaces} in force to revise (rules lists them).`;
      old.dropped = { at, why: `revised as: ${input.rule}` };
    }
    const record: RuleRecord = {
      id: all.reduce((max, r) => Math.max(max, r.id), 0) + 1,
      rule: input.rule.trim(),
      why: input.why.trim(),
      set_at: at,
      ...(input.applies?.trim() ? { applies: input.applies.trim() } : {}),
      ...(input.enforced_by?.trim() ? { enforced_by: input.enforced_by.trim() } : {}),
      ...(input.replaces !== undefined ? { replaces: input.replaces } : {}),
    };
    all.push(record);
    this.#saveRules(all);
    return record;
  }

  /** Keep a rule after looking at its cost, with the reason. */
  keepRule(id: number, because: string, now = new Date()): string {
    const all = this.ruleRecords();
    const rule = all.find((r) => r.id === id && !r.dropped);
    if (!rule) return `No rule #${id} in force (rules lists them).`;
    rule.kept = { at: now.toISOString(), because: because.trim() };
    this.#saveRules(all);
    return `Kept rule #${id}: ${rule.rule} (because ${rule.kept.because}).`;
  }

  /** Drop a rule: it stays in rules.json as history. */
  dropRule(id: number, why: string, now = new Date()): string {
    const all = this.ruleRecords();
    const rule = all.find((r) => r.id === id && !r.dropped);
    if (!rule) return `No rule #${id} in force (rules lists them).`;
    rule.dropped = { at: now.toISOString(), why: why.trim() };
    this.#saveRules(all);
    const by = rule.enforced_by ? ` Undo what carries it out too: ${rule.enforced_by}.` : "";
    return `Dropped rule #${id}: ${rule.rule}.${by}`;
  }

  /** The rules in force, one line each. */
  describeRules(): string {
    const rules = this.rules();
    if (!rules.length) return "No rules recorded.";
    return rules
      .map((r) => {
        const s = sharedRule(r);
        return `#${r.id} ${r.rule} (set ${s.set}; why: ${r.why}${r.applies ? `; applies to ${r.applies}` : ""}${r.enforced_by ? `; in ${r.enforced_by}` : ""})${r.kept ? ` · kept ${minute(r.kept.at)} because ${r.kept.because}` : ""}`;
      })
      .join("\n");
  }

  /** Start logging a session (the SDK's session id). */
  begin(session: string): void {
    this.#session = session;
    const stamp = new Date().toISOString().replace(/[:.]/g, "-");
    this.#log = join(this.sessions, `${stamp}-${session.slice(0, 8)}.jsonl`);
  }

  /** One line in the session log. */
  record(kind: string, text: string): void {
    if (!this.#log || !text.trim()) return;
    appendFileSync(
      this.#log,
      `${JSON.stringify({ at: new Date().toISOString(), session: this.#session, kind, text: text.slice(0, 4000) })}\n`,
    );
  }

  /** Search past sessions with ripgrep (case-insensitive), newest files first; in-process without it. */
  search(query: string, limit = 30): string {
    const files = readdirSync(this.sessions).filter((f) => f.endsWith(".jsonl")).sort().reverse();
    if (!files.length) return "No sessions recorded yet.";
    let lines: string[];
    if (Bun.which("rg")) {
      const run = Bun.spawnSync(["rg", "--no-heading", "-i", "-m", String(limit), "--", query, ...files], {
        cwd: this.sessions,
      });
      if (run.exitCode === 1) return `No matches for "${query}".`;
      if (run.exitCode !== 0 && run.exitCode !== null) return `Search failed: ${run.stderr.toString().slice(0, 300)}`;
      lines = run.stdout.toString().trim().split("\n").slice(0, limit);
    } else {
      // Without ripgrep: the same regex, case-insensitive, in-process.
      let pattern: RegExp;
      try {
        pattern = new RegExp(query, "i");
      } catch (error) {
        return `Search failed: ${(error as Error).message}`;
      }
      lines = files
        .flatMap((file) =>
          readFileSync(join(this.sessions, file), "utf8")
            .split("\n")
            .filter((line) => pattern.test(line))
            .slice(0, limit)
            .map((line) => `${file}:${line}`),
        )
        .slice(0, limit);
      if (!lines.length) return `No matches for "${query}".`;
    }
    return lines
      .map((line) => {
        const at = line.indexOf(":");
        try {
          const entry = JSON.parse(line.slice(at + 1)) as { at: string; kind: string; text: string };
          return `${entry.at} [${entry.kind}] ${entry.text.slice(0, 400)}`;
        } catch {
          return line.slice(0, 400);
        }
      })
      .join("\n");
  }

  /** The tail of the most recent earlier session, for picking up where it left off. */
  lastSession(lines = 15): string {
    const files = readdirSync(this.sessions).filter((f) => f.endsWith(".jsonl")).sort();
    const previous = files.filter((f) => join(this.sessions, f) !== this.#log).pop();
    if (!previous) return "";
    return readFileSync(join(this.sessions, previous), "utf8")
      .trim()
      .split("\n")
      .slice(-lines)
      .map((line) => {
        try {
          const entry = JSON.parse(line) as { kind: string; text: string };
          return `[${entry.kind}] ${entry.text.slice(0, 300)}`;
        } catch {
          return "";
        }
      })
      .filter(Boolean)
      .join("\n");
  }

  /** What the planner sees at the start of a session (and after a compaction). */
  briefing(): string {
    const memory = this.memory().trim();
    const open = this.todos().filter((t) => t.status !== "done");
    const last = this.lastSession();
    const rules = this.rules();
    return [
      memory ? `Your MEMORY.md:\n${memory}` : "Your MEMORY.md is empty.",
      rules.length ? `Rules you set yourself (rules; each reflection shows what they cost):\n${this.describeRules()}` : "",
      open.length
        ? `Your open to-dos:\n${open.map((t) => `${t.id}. [${t.status}] ${t.text}${t.note ? ` (${t.note})` : ""}`).join("\n")}`
        : "You have no open to-dos.",
      last ? `The end of your last session:\n${last}` : "",
    ]
      .filter(Boolean)
      .join("\n\n");
  }
}

export const NOTE_TOOLS = [
  "todo_read",
  "todo_write",
  "memory_read",
  "memory_append",
  "memory_write",
  "session_search",
  "rules",
  "rule_set",
  "rule_keep",
  "rule_drop",
].map(
  (name) => `mcp__notes__${name}`,
);

const text = (value: string) => ({ content: [{ type: "text" as const, text: value }] });

/** The planner's tools over its notes. */
export function notesServer(notes: Notes) {
  return createSdkMcpServer({
    name: "notes",
    version: "0.1.0",
    tools: [
      tool("todo_read", "Your to-do list: what you plan to do, are doing and have done.", {}, async () =>
        text(JSON.stringify(notes.todos(), null, 2)),
      ),
      tool(
        "todo_write",
        "Replace your to-do list. Keep it current: one item in_progress at a time, mark items done as soon as they are, add what you discover. It survives restarts and compactions.",
        {
          todos: z.array(
            z.object({
              text: z.string(),
              status: z.enum(["pending", "in_progress", "done"]),
              note: z.string().optional(),
            }),
          ),
        },
        async ({ todos }) => text(`Saved ${notes.writeTodos(todos).length} to-dos.`),
      ),
      tool("memory_read", "Read your MEMORY.md: the notes you keep across sessions.", {}, async () =>
        text(notes.memory() || "(empty)"),
      ),
      tool(
        "memory_append",
        "Add a note to MEMORY.md: something worth knowing next time (how the game or the client behaves, what worked in your Lua, where things are, what went wrong). Short and specific.",
        { note: z.string() },
        async ({ note }) => text(notes.appendMemory(note)),
      ),
      tool(
        "memory_write",
        "Replace MEMORY.md with a reorganised version (to tidy, merge or drop stale notes). Keep everything still true.",
        { content: z.string() },
        async ({ content }) => text(notes.writeMemory(content)),
      ),
      tool(
        "session_search",
        "Search everything you said and did in past sessions (case-insensitive; ripgrep regex). Use it to recall how you did something before, coordinates, names, errors.",
        { query: z.string(), limit: z.number().int().min(1).max(100).optional() },
        async ({ query, limit }) => text(notes.search(query, limit)),
      ),
      tool(
        "rules",
        "The rules you set yourself that are in force, with their ids. Rules in a module's header (`-- rule: ...`) are shown in reflections too.",
        {},
        async () => text(notes.describeRules()),
      ),
      tool(
        "rule_set",
        "Record a rule you set yourself: a limit on your own play (a road or area you will not enter, a zone you skip, a mob you refuse, a quest you abandon, a fact you override, a hold you keep), with why and what carries it out (a module, a fact override, a MEMORY.md note). Every reflection lists your rules with what playing under each has cost since it was set. Revising a rule: give replaces (its id). A rule a module enforces can instead be a header line in that module: `-- rule: what | why: reason | set: YYYY-MM-DD HH:MM` (the why or set field is required there).",
        {
          rule: z.string().min(1),
          why: z.string().min(1),
          applies: z.string().optional(),
          enforced_by: z.string().optional(),
          replaces: z.number().int().optional(),
        },
        async (input) => {
          const r = notes.setRule(input);
          return text(typeof r === "string" ? r : `Rule #${r.id} recorded: ${r.rule}.`);
        },
      ),
      tool(
        "rule_keep",
        "You looked at a rule's cost and keep it: record why. The reason is shown next to the rule at the next reflection.",
        { id: z.number().int(), because: z.string().min(1) },
        async ({ id, because }) => text(notes.keepRule(id, because)),
      ),
      tool(
        "rule_drop",
        "Drop a rule you no longer hold to, with why. Undo what carries it out too (the module, the fact override, the note). It stays in rules.json as history.",
        { id: z.number().int(), why: z.string().min(1) },
        async ({ id, why }) => text(notes.dropRule(id, why)),
      ),
    ],
  });
}
