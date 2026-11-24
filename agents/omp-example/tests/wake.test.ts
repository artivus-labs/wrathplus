import { describe, expect, test } from "bun:test";
import { classify, describe as line, Evidence, type GameEvent, Inbox, statusLine, wakeMessage } from "../src/wake.ts";

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
  test("level-ups and goal news are notable; failures only feed the evidence", () => {
    expect(classify(ev("PLAYER_LEVEL_UP", 5))).toBe("notable");
    expect(classify(ev("AGENT_GOAL_BLOCKED", "progress", "no plan"))).toBe("notable");
    expect(classify(ev("AGENT_ACTION_FAILED", "progress", "grind", "no mobs"))).toBeNull();
    expect(classify(ev("UI_ERROR_MESSAGE", "Can't do that while moving"))).toBeNull();
    expect(classify(ev("UNIT_HEALTH", "player"))).toBeNull();
  });
  test("a fight left without progress is urgent; other fight ends only feed the evidence", () => {
    const stuck = ev("AGENT_COMBAT_DONE", "0xG", "no_progress", "Leper Gnome's health has not dropped in 12s");
    expect(classify(stuck)).toBe("urgent");
    expect(line(stuck)).toBe("Combat left 0xG without progress: Leper Gnome's health has not dropped in 12s");
    expect(classify(ev("AGENT_COMBAT_DONE", "0xG", "killed"))).toBeNull();
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
    inbox.add(ev("AGENT_GOAL_BLOCKED", "progress", "no plan"));
    inbox.add(ev("AGENT_GOAL_BLOCKED", "progress", "no plan"));
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
    const checkin = wakeMessage("checkin", [], "s", 20, ["3x game error"]);
    expect(checkin).toContain("every 20 minutes");
    expect(checkin).toContain("Evidence since the last check-in:\n- 3x game error");
    expect(wakeMessage("checkin", [], "s", 20)).toContain("nothing failed");
    expect(m).not.toContain("Evidence");
    expect(wakeMessage("reflect", [], "s")).toContain("reflector");
  });
});

test("evidence: failures counted, most frequent first, fights and the latest review", () => {
  const e = new Evidence();
  for (let i = 0; i < 3; i++) e.add(ev("UI_ERROR_MESSAGE", "Can't do that while moving"));
  e.add(ev("AGENT_ACTION_FAILED", "progress", "train", "Mage Trainer is not where they stood"));
  e.add(ev("AGENT_PULL_REFUSED", "Rockjaw Trogg", "an elite"));
  e.add(ev("AGENT_COMBAT_DONE", "0x1", "killed"));
  e.add(ev("AGENT_COMBAT_DONE", "0x2", "killed"));
  e.add(ev("PLAYER_DEAD"));
  e.add(ev("AGENT_COMBAT_REVIEW", "Fight review. Fireball 55 cast, 16 failed."));
  e.add(ev("CHAT_MSG_SAY", "hi", "Bob"));
  const lines = e.take();
  expect(lines[0]).toBe("fights: 2 killed, 1 you died");
  expect(lines[1]).toBe('3x game error "Can\'t do that while moving"');
  expect(lines).toContain("1x goal action train failed: Mage Trainer is not where they stood");
  expect(lines).toContain("1x pull refused: an elite");
  expect(lines.at(-1)).toContain("Fireball 55 cast, 16 failed");
  expect(e.take()).toEqual([]);
});
