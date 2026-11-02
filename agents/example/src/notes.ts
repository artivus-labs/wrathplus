// The agent's own notes, kept in its home folder between sessions: a to-do list, a MEMORY.md for
// whatever it wants to remember, and a JSONL log of every session it can search. Plain files,
// nothing clever: the point is a consistent agent that keeps its bearings across restarts and
// compactions.
//
//   <home>/MEMORY.md            free-form notes
//   <home>/todo.json            [{ id, text, status: pending | in_progress | done, note? }]
//   <home>/sessions/*.jsonl     one line per message: { at, session, kind, text }

import { createSdkMcpServer, tool } from "@anthropic-ai/claude-agent-sdk";
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";

export interface Todo {
  id: number;
  text: string;
  status: "pending" | "in_progress" | "done";
  note?: string;
}

const MEMORY_LIMIT = 20_000;

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
    return [
      memory ? `Your MEMORY.md:\n${memory}` : "Your MEMORY.md is empty.",
      open.length
        ? `Your open to-dos:\n${open.map((t) => `${t.id}. [${t.status}] ${t.text}${t.note ? ` (${t.note})` : ""}`).join("\n")}`
        : "You have no open to-dos.",
      last ? `The end of your last session:\n${last}` : "",
    ]
      .filter(Boolean)
      .join("\n\n");
  }
}

export const NOTE_TOOLS = ["todo_read", "todo_write", "memory_read", "memory_append", "memory_write", "session_search"].map(
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
    ],
  });
}
