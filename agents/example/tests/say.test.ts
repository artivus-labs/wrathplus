import { expect, test } from "bun:test";
import type { Jev } from "../src/jev.ts";
import { memorySlice, SAY_LIMIT, sayLine, type Speak } from "../src/say.ts";
import { SAY_WHEN, SmallTalk, type Chatter, type SmallTalkOptions } from "../src/triage.ts";

const remy: Chatter = { speaker: "Remy", text: "Fine day for a walk to Goldshire, Agentfour!", kind: "npc" };
const MEMORY = [
  "- Elwynn: Goldshire is exhausted for level 11.",
  "- Lyria trains warriors in Goldshire.",
  "- Nov 10: Westfall south road is safe at full health; Sentinel Hill has the hand-ins.",
].join("\n");

/** A Jev that picks `choice` for small talk, a client that answers the character Lua and records
 *  the rest, and a stubbed writer. */
function speaker(choice: string, speak: Speak, options: Partial<SmallTalkOptions> = {}) {
  let now = 0;
  const criteria: string[][] = [];
  const ran: string[] = [];
  const prompts: string[] = [];
  const jev = {
    ask: async (_state: never, questions: { small_talk: { criteria: Record<string, string> } }) => {
      criteria.push(Object.keys(questions.small_talk.criteria));
      return { model: "jev-test", answers: { small_talk: { type: "choice", choice, confidence: 0.9, probabilities: {} } } };
    },
  } as unknown as Jev;
  const lua = async (code: string) => {
    if (code.includes("UnitRace")) return { ok: true, values: [{ name: "Agentfour", race: "Human", class: "Warrior", level: 11, zone: "Elwynn Forest", subzone: "Goldshire" }] };
    ran.push(code);
    return { ok: true, values: [true] };
  };
  const talk = new SmallTalk({
    jev,
    lua,
    now: () => now,
    memory: () => MEMORY,
    speak: async (prompt, signal) => (prompts.push(prompt), speak(prompt, signal)),
    ...options,
  });
  return { talk, criteria, ran, prompts, at: (ms: number) => (now = ms) };
}

test("a remark Jev judges wants words gets one line in character, from the character, the place, the chat and the memory", async () => {
  const s = speaker("say", async () => '"Aye, the road\'s clear today. Mind the wolves past the bridge!"');
  // Earlier chat nearby (Jev's say spent here keeps the gap; the next one is a minute on).
  await s.talk.handle([{ speaker: "Marshal Dughan", text: "Stay out of the mine.", kind: "npc" }]);
  s.at(60_000);
  const sorted = await s.talk.handle([remy]);
  expect(s.criteria.at(-1)).toContain("say");
  expect(sorted).toEqual({
    wake: [],
    later: [`Remy says to you: ${remy.text} (an NPC) (the fast loop said "Aye, the road's clear today. Mind the wolves past the bridge!" to Remy)`],
  });
  expect(s.ran.at(-1)).toBe(`SendChatMessage("Aye, the road's clear today. Mind the wolves past the bridge!", "SAY") return true`);
  const prompt = s.prompts.at(-1)!;
  expect(prompt).toContain("You are Agentfour, a level 11, Human, Warrior, in Goldshire, Elwynn Forest");
  expect(prompt).toContain(`Remy (an NPC) said to you: ${remy.text}`);
  expect(prompt).toContain("- Marshal Dughan says to you: Stay out of the mine. (an NPC)");
  expect(prompt).toContain("Sentinel Hill has the hand-ins.");
  expect(prompt).toContain(`at most ${SAY_LIMIT} characters`);
  expect(prompt).toContain("nothing about being an AI");
  expect(SAY_WHEN).toContain("spoken reply in character");
});

test("says are capped: one per 30 s, 20 an hour, and one reply per speaker per 2 minutes", async () => {
  const s = speaker("say", async () => "Well met.");
  const from = (name: string): Chatter => ({ speaker: name, text: "Hello there!", kind: "say" });
  await s.talk.handle([from("A")]);
  expect(s.ran).toEqual(['SendChatMessage("Well met.", "SAY") return true']);
  // 10 s later, someone else: no say on offer, so Jev picks among the emotes.
  s.at(10_000);
  await s.talk.handle([from("B")]);
  expect(s.criteria.at(-1)).not.toContain("say");
  // The same speaker within 2 minutes: no answer at all on offer.
  s.at(45_000);
  await s.talk.handle([from("A")]);
  expect(s.criteria.at(-1)).toEqual(["ignore", "needs_thought"]);
  // 20 in an hour, then no more until the oldest is an hour old.
  for (let i = 0; i < 19; i++) {
    s.at(60_000 + i * 31_000);
    await s.talk.handle([from(`C${i}`)]);
  }
  expect(s.ran.filter((c) => c.startsWith("SendChatMessage"))).toHaveLength(20);
  s.at(60_000 + 19 * 31_000);
  await s.talk.handle([from("D")]);
  expect(s.criteria.at(-1)).not.toContain("say");
  s.at(3_600_001);
  await s.talk.handle([from("E")]);
  expect(s.criteria.at(-1)).toContain("say");
});

test("a failed, empty or slow line falls back to the emote that fits", async () => {
  const failing = speaker("say", async () => {
    throw new Error("overloaded");
  });
  expect((await failing.talk.handle([{ speaker: "Tom", text: "Hello, friend!", kind: "say" }])).later).toEqual([
    "Tom says to you: Hello, friend! (the fast loop waved)",
  ]);
  expect(failing.ran).toEqual(['DoEmote("WAVE", "Tom") return true']);
  const empty = speaker("say", async () => "*waves*");
  await empty.talk.handle([remy]);
  expect(empty.ran).toEqual(['DoEmote("NOD", "Remy") return true']);
  const slow = speaker("say", (_p, signal) => new Promise((resolve) => signal.addEventListener("abort", () => resolve("too late"))), { sayTimeoutMs: 20 });
  await slow.talk.handle([remy]);
  expect(slow.ran).toEqual(['DoEmote("NOD", "Remy") return true']);
  // No writer (WRATHPLUS_AGENT_SAY=0): no say on offer.
  const off = speaker("nod", async () => "x", { speak: undefined });
  await off.talk.handle([remy]);
  expect(off.criteria[0]).not.toContain("say");
});

test("a written line is cleaned: one line, no quotes, no emotes or labels, cut to the limit", () => {
  expect(sayLine('  "Well met, traveller."  \nSecond line')).toBe("Well met, traveller.");
  expect(sayLine("“Light be with you.”")).toBe("Light be with you.");
  expect(sayLine("*bows* Good day to you. (smiles)")).toBe("Good day to you.");
  expect(sayLine("Agentfour: Aye, that it is.")).toBe("Aye, that it is.");
  const long = sayLine(`${"word ".repeat(40)}end`);
  expect(long.length).toBeLessThanOrEqual(SAY_LIMIT);
  expect(long.endsWith("word")).toBe(true);
  expect(sayLine("   ")).toBe("");
  expect(memorySlice(MEMORY, 90)).toBe("- Nov 10: Westfall south road is safe at full health; Sentinel Hill has the hand-ins.");
  expect(memorySlice(MEMORY, 400)).toBe(MEMORY);
});
