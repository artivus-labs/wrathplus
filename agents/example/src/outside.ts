// Outside notes: rare words from a person, never promised and never waited for. A running agent
// takes them from its terminal (stdin) and from a small file in its notes folder, so one started
// where its stdin is not a terminal (by the Manager, in a detached pane) can still be reached:
//
//   bun run agent note <name> "text"   appends { at, text } to <notes>/outside.jsonl
//
// The running agent tails that file from where it was when it started and hands each new note to
// the planner, worded as the stdin path words it.

import { appendFileSync, closeSync, existsSync, openSync, readSync, statSync } from "node:fs";
import { join } from "node:path";

export const OUTSIDE_FILE = "outside.jsonl";

/** An outside note as the planner reads it. */
export const outsideNote = (text: string) =>
  `Outside note (nobody is waiting for a reply, and no further note is promised): ${text.trim()}`;

/** Leave a note for the agent whose notes are in `notesHome`. */
export function sendNote(notesHome: string, text: string): void {
  if (!text.trim()) throw new Error("An outside note needs some text.");
  appendFileSync(join(notesHome, OUTSIDE_FILE), `${JSON.stringify({ at: new Date().toISOString(), text: text.trim() })}\n`);
}

/** Hand every note appended from now on to `deliver`, checking every `everyMs` until aborted. */
export function tailNotes(notesHome: string, deliver: (text: string) => void, signal: AbortSignal, everyMs = 1000): void {
  const path = join(notesHome, OUTSIDE_FILE);
  let offset = existsSync(path) ? statSync(path).size : 0;
  let partial = "";
  const check = () => {
    if (!existsSync(path)) return;
    const size = statSync(path).size;
    // Truncated or replaced: read it again from the start.
    if (size < offset) offset = 0;
    if (size === offset) return;
    const buffer = Buffer.alloc(size - offset);
    const fd = openSync(path, "r");
    try {
      readSync(fd, buffer, 0, buffer.length, offset);
    } finally {
      closeSync(fd);
    }
    offset = size;
    const lines = (partial + buffer.toString("utf8")).split("\n");
    partial = lines.pop() ?? "";
    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const note = JSON.parse(line) as { text?: unknown };
        if (typeof note.text === "string" && note.text.trim()) deliver(outsideNote(note.text));
      } catch {
        // A line that is not a note (a hand edit) is skipped.
      }
    }
  };
  const timer = setInterval(check, everyMs);
  signal.addEventListener("abort", () => clearInterval(timer));
}
