import { describe, expect, test } from "bun:test";
import { classify, describe as line, type GameEvent, Inbox, statusLine, wakeMessage } from "../src/wake.ts";

const ev = (event: string, ...args: unknown[]): GameEvent => ({ seq: 1, time: 0, event, args });

describe("classify", () => {
  test("asks, deaths, whispers and invites are urgent", () => {
    for (const e of ["AGENT_ASK", "PLAYER_DEAD", "CHAT_MSG_WHISPER", "PARTY_INVITE_REQUEST", "CHAT_MSG_PARTY"]) {
      expect(classify(ev(e, "hi", "Someone"), "Sixer")).toBe("urgent");
    }
  });
  test("our own chat is not news", () => {
    expect(classify(ev("CHAT_MSG_WHISPER", "hello", "Sixer"), "sixer")).toBeNull();
  });
  test("/say wakes only when it names us", () => {
    expect(classify(ev("CHAT_MSG_SAY", "anyone selling bread?", "Bob"), "Sixer")).toBeNull();
    expect(classify(ev("CHAT_MSG_SAY", "hey sixer, want to group?", "Bob"), "Sixer")).toBe("notable");
    expect(classify(ev("CHAT_MSG_SAY", "hey sixer", "Bob"))).toBeNull();
  });
  test("level-ups and goal news are notable, failed actions minor", () => {
    expect(classify(ev("PLAYER_LEVEL_UP", 5))).toBe("notable");
    expect(classify(ev("AGENT_GOAL_BLOCKED", "progress", "no plan"))).toBe("notable");
    expect(classify(ev("AGENT_ACTION_FAILED", "progress", "grind", "no mobs"))).toBe("minor");
    expect(classify(ev("UNIT_HEALTH", "player"))).toBeNull();
  });
});

describe("describe", () => {
  test("an ask says how to answer it, with its context and digest", () => {
    const text = line(ev("AGENT_ASK", 7, "Bob asks to group", { who: "Bob" }, ["Bob says: group?", "You killed a wolf"]));
    expect(text).toContain("ask 7");
    expect(text).toContain("Agent.answer(7, value)");
    expect(text).toContain('"who":"Bob"');
    expect(text).toContain("You killed a wolf");
  });
  test("chat lines are clipped", () => {
    expect(line(ev("CHAT_MSG_WHISPER", "x".repeat(1000), "Bob")).length).toBeLessThan(330);
  });
});

describe("Inbox", () => {
  test("repeats collapse, urgent lines come first, take empties it", () => {
    const inbox = new Inbox();
    inbox.add(ev("AGENT_ACTION_FAILED", "progress", "grind", "no mobs"));
    inbox.add(ev("AGENT_ACTION_FAILED", "progress", "grind", "no mobs"));
    inbox.add(ev("CHAT_MSG_WHISPER", "hi", "Bob"), "Sixer");
    inbox.add(ev("CHAT_MSG_SAY", "nobody", "Bob"), "Sixer");
    expect(inbox.size).toBe(2);
    expect(inbox.has("urgent")).toBe(true);
    const lines = inbox.take();
    expect(lines[0]).toBe("Bob whispers: hi");
    expect(lines[1]).toEndWith("(x2)");
    expect(inbox.size).toBe(0);
  });
});

describe("messages", () => {
  test("status line from /status", () => {
    const status = {
      in_world: true,
      player: { name: "Sixer", level: 3, class: "MAGE", class_name: "Mage", zone: "Dun Morogh", subzone: "Coldridge Valley", health: 80, health_max: 100, power: 50, power_max: 120, dead: false, combat: true },
    };
    const s = statusLine(status);
    expect(s).toContain("Sixer, level 3 Mage, Dun Morogh / Coldridge Valley.");
    expect(s).toContain("Health 80/100");
    expect(s).toContain("(in combat)");
    expect(statusLine({ in_world: false, state: "CharSelect", player: null })).toContain("CharSelect");
    expect(statusLine(undefined)).toContain("not in the world");
  });
  test("a wake message carries its reason, status and events", () => {
    const m = wakeMessage("events", ["Bob whispers: hi"], "Sixer, level 3 Mage.");
    expect(m).toContain("Something happened");
    expect(m).toContain("- Bob whispers: hi");
    expect(wakeMessage("checkin", [], "s", 20)).toContain("20 minutes");
    expect(wakeMessage("reflect", [], "s")).toContain("reflector");
  });
});
