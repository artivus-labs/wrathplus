import { expect, test } from "bun:test";
import { Jev } from "../src/jev.ts";
import { Triage } from "../src/triage.ts";
import { requestOf } from "../src/watch.ts";

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
