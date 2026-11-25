import { describe, expect, test } from "bun:test";
import { classify, describe as line, Evidence, type GameEvent, Inbox, Repeats, statusLine, wakeMessage } from "../src/wake.ts";

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
  test("a hold that ended with its walk says so", () => {
    expect(line(ev("AGENT_HOLD_EXPIRED", "to Thelsamar", 64, "walk arrived"))).toBe(
      'Your hold "to Thelsamar" ended with its walk (walk arrived) after 64 s: hold again or Goals.release().',
    );
    expect(line(ev("AGENT_HOLD_EXPIRED", "resting", 300))).toBe('Your hold "resting" ran out after 300 s: hold again or Goals.release().');
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

describe("Repeats", () => {
  const MIN = 60_000;
  const blocked = ev("AGENT_GOAL_BLOCKED", "progress", "no plan reaches progressed = true now");
  const held = (r: Repeats, e: GameEvent, at: number) => r.hold(e, classify(e, "Sixer"), at);

  test("the same goal blocked for the same reason wakes once per window; repeats go to the check-in", () => {
    const r = new Repeats(15 * MIN);
    expect(held(r, blocked, 0)).toBe(false);
    for (let i = 1; i <= 19; i++) expect(held(r, blocked, i * 30_000)).toBe(true);
    // Another reason, or another goal, is news.
    expect(held(r, ev("AGENT_GOAL_BLOCKED", "progress", "something else"), MIN)).toBe(false);
    expect(held(r, ev("AGENT_GOAL_BLOCKED", "reach_loch", "no plan reaches progressed = true now"), MIN)).toBe(false);
    expect(r.take()).toEqual([
      "AGENT_GOAL_BLOCKED progress: 'no plan reaches progressed = true now' ×19 (repeats within 15 min of a wake; not woken for)",
    ]);
    expect(r.take()).toEqual([]);
    // Once the window since the last wake is over, it wakes again.
    expect(held(r, blocked, 15 * MIN)).toBe(false);
    expect(held(r, blocked, 16 * MIN)).toBe(true);
  });

  test("a hold that runs out again is the same hold however long it waited", () => {
    const r = new Repeats(15 * MIN);
    expect(held(r, ev("AGENT_HOLD_EXPIRED", "watching the camp", 60), 0)).toBe(false);
    expect(held(r, ev("AGENT_HOLD_EXPIRED", "watching the camp", 125), MIN)).toBe(true);
    expect(held(r, ev("AGENT_HOLD_EXPIRED", "resting", 30), MIN)).toBe(false);
    expect(r.take()).toEqual(["AGENT_HOLD_EXPIRED 'watching the camp' ×1 (repeats within 15 min of a wake; not woken for)"]);
  });

  test("urgent events are never held back, and a window of 0 is off", () => {
    const r = new Repeats(15 * MIN);
    for (const e of [
      ev("CHAT_MSG_WHISPER", "hi", "Bob"),
      ev("AGENT_ASK", 1, "group?"),
      ev("PLAYER_DEAD"),
      ev("PARTY_INVITE_REQUEST", "Bob"),
      ev("AGENT_COMBAT_DONE", "0xG", "no_progress", "health has not dropped"),
    ]) {
      expect(held(r, e, 0)).toBe(false);
      expect(held(r, e, 1000)).toBe(false);
    }
    // Events that never wake are not counted as repeats either (the evidence counts them).
    expect(held(r, ev("AGENT_ACTION_FAILED", "progress", "grind", "x"), 0)).toBe(false);
    expect(held(r, ev("AGENT_ACTION_FAILED", "progress", "grind", "x"), 1)).toBe(false);
    expect(r.take()).toEqual([]);
    const off = new Repeats(0);
    expect(held(off, blocked, 0)).toBe(false);
    expect(held(off, blocked, 1)).toBe(false);
  });
});
