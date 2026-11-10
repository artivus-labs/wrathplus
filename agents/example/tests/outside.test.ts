import { expect, test } from "bun:test";
import { appendFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { note } from "../src/cli.ts";
import { OUTSIDE_FILE, outsideNote, sendNote, tailNotes } from "../src/outside.ts";

test("a running agent hears notes left after it started, worded as a terminal line is", async () => {
  const home = mkdtempSync(join(tmpdir(), "wp-outside-"));
  sendNote(home, "an old note, from before it started");
  const heard: string[] = [];
  const abort = new AbortController();
  tailNotes(home, (text) => heard.push(text), abort.signal, 10);
  sendNote(home, "  The Westfall road is open. ");
  // Half a line now, the rest later: delivered once whole.
  appendFileSync(join(home, OUTSIDE_FILE), '{"text":"Pallytest is ');
  await Bun.sleep(40);
  appendFileSync(join(home, OUTSIDE_FILE), 'back"}\nnot a note\n');
  await Bun.sleep(40);
  abort.abort();
  expect(heard).toEqual([
    "Outside note (nobody is waiting for a reply, and no further note is promised): The Westfall road is open.",
    outsideNote("Pallytest is back"),
  ]);
  expect(() => sendNote(home, "  ")).toThrow("needs some text");
  rmSync(home, { recursive: true });
});

test("bun run agent note leaves the note in a running agent's notes folder, and refuses one nobody reads", () => {
  const home = mkdtempSync(join(tmpdir(), "wp-note-"));
  mkdirSync(join(home, "agents", "AGENTFOUR", "notes"), { recursive: true });
  writeFileSync(join(home, "agents", "agents.json"), JSON.stringify([{ name: "Agentfour", account: "AGENTFOUR", port: 17953 }]));
  expect(() => note(["Agentfour", "hello"], home)).toThrow("is not being driven by the example agent");
  writeFileSync(join(home, "agents", "AGENTFOUR", "running.json"), JSON.stringify({ pid: process.pid, port: 17953, model: "m", goal: "g", started: "" }));
  expect(note(["agentfour", "Duskwood", "is", "19-28."], home)).toBe("Note left for Agentfour. Nobody is waiting for a reply.");
  const line = JSON.parse(readFileSync(join(home, "agents", "AGENTFOUR", "notes", OUTSIDE_FILE), "utf8").trim());
  expect(line.text).toBe("Duskwood is 19-28.");
  expect(() => note(["Nobody", "hi"], home)).toThrow('No agent "Nobody"');
  expect(() => note(["Agentfour"], home)).toThrow("Usage");
  rmSync(home, { recursive: true });
});
