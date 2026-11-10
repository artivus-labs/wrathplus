import { expect, test } from "bun:test";
import { Jev } from "../src/jev.ts";
import { chatterLine, COURTESIES, SmallTalk, Triage, type Chatter, type SmallTalkOptions } from "../src/triage.ts";
import { chatterOf, requestOf } from "../src/watch.ts";

/** A fake Jev answering what the test sets, and a fake client recording the Lua and chat. */
function fake(wants: string, confidence: number, reply: number, lua: { ok: boolean; values?: unknown[] } = { ok: true, values: [true] }) {
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    fetch: () =>
      Response.json({
        model: "jev-test",
        answers: {
          wants: { type: "choice", choice: wants, confidence, probabilities: {} },
          reply: { type: "noul", noul: reply },
        },
      }),
  });
  const ran: string[] = [];
  const said: string[] = [];
  const triage = new Triage(
    new Jev("k", "jev-test", server.url.href.replace(/\/$/, "")),
    async (code) => {
      ran.push(code);
      return lua;
    },
    async (text, channel, to) => {
      said.push(`${channel} ${to}: ${text}`);
    },
  );
  return { triage, ran, said, stop: () => server.stop(true) };
}

const follow = { sender: "Pallytest", text: "follow me", channel: "PARTY" as const };

test("a clear request is done at once, answered in a few words, and the planner left asleep", async () => {
  const f = fake("follow_me", 0.92, 0.1);
  const t = await f.triage.handle(follow, "level 5");
  f.stop();
  expect(t).toEqual({ handled: true, note: 'Pallytest said "follow me"; the fast loop followed them', reply: false });
  expect(f.ran[0]).toContain('Move.companion = "Pallytest"');
  expect(f.said).toEqual(["PARTY Pallytest: Following."]);
});

test("unsure, or something else, goes to the planner untouched", async () => {
  for (const [wants, confidence] of [["follow_me", 0.5], ["something_else", 0.99]] as const) {
    const f = fake(wants, confidence, 0.1);
    const t = await f.triage.handle(follow, null);
    f.stop();
    expect(t).toEqual({ handled: false, note: 'Pallytest said "follow me"', reply: true });
    expect(f.ran).toEqual([]);
  }
});

test("a request that also wants an answer is done, and the planner answers it", async () => {
  const f = fake("pick_up_quests", 0.9, 0.8);
  const t = await f.triage.handle({ sender: "Pallytest", text: "grab Bernice's quest, which one is it?", channel: "PARTY" }, null);
  f.stop();
  expect(t.handled).toBe(false);
  expect(t.note).toContain("the fast loop is picking up the quests on offer nearby");
  expect(f.said).toEqual([]);
});

test("a request it cannot carry out goes to the planner with the reason", async () => {
  const f = fake("pick_up_quests", 0.9, 0.1, { ok: true, values: [false, "no quest giver nearby has a quest for us"] });
  const t = await f.triage.handle({ sender: "Pallytest", text: "get the quest", channel: "PARTY" }, null);
  f.stop();
  expect(t).toEqual({
    handled: false,
    note: 'Pallytest said "get the quest"; the fast loop tried to (pick_up_quests) but no quest giver nearby has a quest for us',
    reply: true,
  });
});

test("requests are party chat, whispers, and /say from the party; never our own", () => {
  const ev = (event: string, args: unknown[]) => ({ seq: 1, time: 0, event, args });
  const party = new Set(["pallytest"]);
  expect(requestOf(ev("CHAT_MSG_PARTY_LEADER", ["follow me", "Pallytest"]), "Agenttwo", party)?.channel).toBe("PARTY");
  expect(requestOf(ev("CHAT_MSG_WHISPER", ["come here", "Stranger"]), "Agenttwo", party)?.channel).toBe("WHISPER");
  expect(requestOf(ev("CHAT_MSG_SAY", ["wait here", "Pallytest"]), "Agenttwo", party)?.sender).toBe("Pallytest");
  expect(requestOf(ev("CHAT_MSG_SAY", ["wait here", "Botty"]), "Agenttwo", party)).toBeNull();
  expect(requestOf(ev("CHAT_MSG_PARTY", ["Following.", "Agenttwo"]), "Agenttwo", party)).toBeNull();
});

const ma: Chatter = { speaker: "Ma Stonefield", text: "City folk never understand that, Agentfour.", kind: "npc" };

/** A Jev that answers small talk with `choice`, and a client recording the Lua. */
function talker(choice: string, confidence = 0.9, options: Partial<SmallTalkOptions> = {}) {
  const asked: { state: { said: unknown }; questions: { small_talk: { criteria: Record<string, string> } } }[] = [];
  const ran: string[] = [];
  const jev = {
    ask: async (state: never, questions: never) => {
      asked.push({ state, questions });
      if (choice === "error") throw new Error("Jev HTTP 500");
      return { model: "jev-test", answers: { small_talk: { type: "choice", choice, confidence, probabilities: {} } } };
    },
  } as unknown as Jev;
  const talk = new SmallTalk({ jev, lua: async (code) => (ran.push(code), { ok: true, values: [true] }), ...options });
  return { talk, asked, ran };
}

test("with the fast tier, small talk is answered with a courtesy and never wakes the planner", async () => {
  let now = 0;
  const t = talker("nod", 0.9, { now: () => now, perSpeakerMs: 120_000 });
  const first = await t.talk.handle([ma], "level 11, in Goldshire");
  expect(first).toEqual({ wake: [], later: ["Ma Stonefield says to you: City folk never understand that, Agentfour. (an NPC) (the fast loop nodded)"] });
  expect(t.ran).toEqual(['DoEmote("NOD", "Ma Stonefield") return true']);
  expect(t.asked[0]?.state.said).toEqual({ from: "Ma Stonefield", text: ma.text, kind: "npc" });
  expect(Object.keys(t.asked[0]?.questions.small_talk.criteria ?? {})).toEqual(["ignore", "needs_thought", ...Object.keys(COURTESIES)]);
  // The same speaker again a moment later: one courtesy was enough, so none is offered.
  now = 30_000;
  await t.talk.handle([ma]);
  expect(Object.keys(t.asked[1]?.questions.small_talk.criteria ?? {})).toEqual(["ignore", "needs_thought"]);
  expect(t.ran).toHaveLength(1);
  now = 200_000;
  await t.talk.handle([ma]);
  expect(t.ran).toHaveLength(2);
});

test("Jev may ignore it, or pass on what really needs the planner", async () => {
  const ignored = talker("ignore");
  expect(await ignored.talk.handle([ma])).toEqual({ wake: [], later: [`${chatterLine(ma)} (ignored)`] });
  expect(ignored.ran).toEqual([]);
  const invite: Chatter = { speaker: "Thrallson", text: "Agentfour, want to group for Hogger?", kind: "say" };
  const real = talker("needs_thought");
  expect(await real.talk.handle([invite])).toEqual({ wake: ["Thrallson says to you: Agentfour, want to group for Hogger? (the fast loop judged this needs you)"], later: [] });
  // Unsure: it rides along with the next wake-up rather than costing a turn or an odd emote.
  expect(await talker("needs_thought", 0.4).talk.handle([invite])).toEqual({ wake: [], later: [chatterLine(invite)] });
  expect(await talker("laugh", 0.3).talk.handle([ma])).toEqual({ wake: [], later: [`${chatterLine(ma)} (ignored)`] });
});

test("without the fast tier small talk is batched: one wake-up per period at most, the rest rides along", async () => {
  let now = 1_000_000;
  const talk = new SmallTalk({ now: () => now, wakeEveryMs: 600_000 });
  const wave: Chatter = { speaker: "Botty", text: "Botty waves at you.", kind: "emote" };
  expect(await talk.handle([ma, wave])).toEqual({
    wake: [
      "Ma Stonefield says to you: City folk never understand that, Agentfour. (an NPC) (small talk: a short reply or an emote is welcome, not required)",
      "Botty waves at you. (small talk: a short reply or an emote is welcome, not required)",
    ],
    later: [],
  });
  now += 60_000;
  expect(await talk.handle([ma])).toEqual({ wake: [], later: [chatterLine(ma)] });
  now += 300_000;
  expect((await talk.handle([wave])).wake).toEqual([]);
  now += 300_000;
  expect((await talk.handle([ma])).wake).toHaveLength(1);
  // Jev failing is the same as having none.
  const down = talker("error", 0.9, { now: () => now, wakeEveryMs: 600_000 });
  expect((await down.talk.handle([ma])).wake).toHaveLength(1);
  expect((await down.talk.handle([ma])).later).toEqual([chatterLine(ma)]);
});

test("small talk is NPCs naming us, strangers' /say with our name and emotes at us; never the party or whispers", () => {
  const e = (event: string, args: unknown[]) => ({ seq: 1, time: 0, event, args });
  const party = new Set(["pallytest"]);
  expect(chatterOf(e("CHAT_MSG_MONSTER_SAY", [ma.text, "Ma Stonefield"]), "Agentfour", party)).toEqual(ma);
  expect(chatterOf(e("CHAT_MSG_MONSTER_SAY", ["Quiet now, love.", "Ma Stonefield"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_MONSTER_SAY", [ma.text, "Ma Stonefield"]), "Agentfour", party, false)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_SAY", ["Nice axe, Agentfour!", "Botty"]), "Agentfour", party)).toEqual({ speaker: "Botty", text: "Nice axe, Agentfour!", kind: "say" });
  expect(chatterOf(e("CHAT_MSG_SAY", ["anyone selling linen?", "Botty"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_TEXT_EMOTE", ["Botty waves at you.", "Botty"]), "Agentfour", party)).toEqual({ speaker: "Botty", text: "Botty waves at you.", kind: "emote" });
  expect(chatterOf(e("CHAT_MSG_EMOTE", ["pokes Agentfour with a stick.", "Botty"]), "Agentfour", party)?.text).toBe("Botty pokes Agentfour with a stick.");
  expect(chatterOf(e("CHAT_MSG_TEXT_EMOTE", ["You wave at Botty.", "Agentfour"]), "Agentfour", party)).toBeNull();
  // The party and anything private still reach the planner (or triage) as before.
  expect(chatterOf(e("CHAT_MSG_SAY", ["Agentfour, over here", "Pallytest"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_TEXT_EMOTE", ["Pallytest waves at you.", "Pallytest"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_WHISPER", ["Agentfour, you there?", "Botty"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_PARTY", ["Agentfour, you there?", "Stranger"]), "Agentfour", party)).toBeNull();
  expect(chatterOf(e("CHAT_MSG_MONSTER_WHISPER", ["Agentfour, come closer.", "Spirit"]), "Agentfour", party)).toBeNull();
});
