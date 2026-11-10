// The say step of small talk (`SmallTalk`, triage.ts): when Jev judges that a remark or a greeting
// wants a spoken reply in character rather than an emote, one short Claude Haiku call writes the
// line. No tools, no planner session: the character, the place, who spoke and what, the last few
// lines of nearby chat and a small slice of the agent's memory for flavour. The line is cleaned up
// (one line, no quotes, no emotes, cut to `SAY_LIMIT`) before it is said.

import { query } from "@anthropic-ai/claude-agent-sdk";
import { tmpdir } from "node:os";
import { claudeEnv } from "./env.ts";

/** The model that writes the line: Claude Haiku 5.5, fast and cheap. */
export const SAY_MODEL = "claude-haiku-5-5";
/** The longest line said, in characters. */
export const SAY_LIMIT = 120;
/** How much of MEMORY.md rides along, from its end (characters). */
export const MEMORY_SLICE = 400;

/** The character as the say prompt describes it, read in one Lua call. */
export const CHARACTER_LUA = `return { name = UnitName("player"), race = UnitRace("player"), class = UnitClass("player"),
  level = UnitLevel("player"), zone = GetRealZoneText(), subzone = GetSubZoneText() }`;

export interface Character {
  name?: string;
  race?: string;
  class?: string;
  level?: number;
  zone?: string;
  subzone?: string;
}

export interface SayContext {
  character: Character;
  speaker: string;
  text: string;
  /** "npc", "say", "emote" or "whisper". */
  kind: string;
  /** The last few lines of chat nearby, oldest first. */
  chat: string[];
  /** The tail of MEMORY.md. */
  memory: string;
}

/** The writer of one line: the prompt in, the model's raw text out. */
export type Speak = (prompt: string, signal: AbortSignal) => Promise<string>;

/** The end of MEMORY.md, whole lines, at most `limit` characters. */
export function memorySlice(memory: string, limit = MEMORY_SLICE): string {
  const lines = memory.trim().split("\n");
  const out: string[] = [];
  let size = 0;
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i]!.trim();
    if (!line) continue;
    if (size + line.length + 1 > limit) {
      if (!out.length) out.unshift(line.slice(-limit));
      break;
    }
    out.unshift(line);
    size += line.length + 1;
  }
  return out.join("\n");
}

export function sayPrompt(c: SayContext): string {
  const me = c.character;
  const who = [me.name ?? "a traveller", me.level ? `a level ${me.level}` : "", me.race ?? "", me.class ?? ""].filter(Boolean).join(", ");
  const where = [me.subzone, me.zone].filter(Boolean).join(", ") || "somewhere in Azeroth";
  return [
    `You are ${who}, in ${where}, in World of Warcraft: Wrath of the Lich King.`,
    `${c.speaker}${c.kind === "npc" ? " (an NPC)" : ""} ${c.kind === "emote" ? "did this" : "said to you"}: ${c.text}`,
    c.chat.length ? `Said nearby just before:\n${c.chat.map((l) => `- ${l}`).join("\n")}` : "",
    c.memory ? `From your own recent notes, for flavour only:\n${c.memory}` : "",
    `Answer ${c.speaker} with one short line of speech in character, as a player would say it aloud: at most ${SAY_LIMIT} characters, plain words, no emotes or actions, no quotation marks, nothing about being an AI, an agent or a program. Reply with the line only.`,
  ]
    .filter(Boolean)
    .join("\n\n");
}

/** One clean line to say, or "" when nothing usable came back. */
export function sayLine(raw: string, limit = SAY_LIMIT): string {
  let line = raw.split("\n").map((l) => l.trim()).find(Boolean) ?? "";
  // Emotes and stage directions (*waves*, (smiles)) are not speech.
  line = line.replace(/\*[^*]*\*/g, " ").replace(/\([^)]*\)/g, " ");
  // A speaker label, then surrounding quotes of any kind.
  line = line.replace(/^[A-Z][\w' -]{0,24}:\s+/, "");
  line = line.replace(/\s+/g, " ").trim().replace(/^["'“”‘’«»]+|["'“”‘’«»]+$/g, "").trim();
  if (line.length > limit) {
    const cut = line.slice(0, limit);
    const space = cut.lastIndexOf(" ");
    line = (space > limit / 2 ? cut.slice(0, space) : cut).replace(/[\s,;:-]+$/, "");
  }
  return line;
}

/** The real writer: one Claude Haiku turn through the Agent SDK on the planner's login, with no
 *  tools, no settings and no session kept. */
export function haikuSpeak(model = SAY_MODEL): Speak {
  return async (prompt, signal) => {
    const abortController = new AbortController();
    signal.addEventListener("abort", () => abortController.abort());
    let text = "";
    for await (const message of query({
      prompt,
      options: {
        model,
        effort: "low",
        systemPrompt: "You write one line of in-character speech for a World of Warcraft player character.",
        tools: [],
        allowedTools: [],
        permissionMode: "dontAsk",
        settingSources: [],
        env: claudeEnv(),
        cwd: tmpdir(),
        persistSession: false,
        maxTurns: 1,
        abortController,
      },
    })) {
      if (message.type === "result") {
        if (message.subtype !== "success") throw new Error(`say: ${message.subtype}`);
        text = message.result;
      }
    }
    return text;
  };
}
