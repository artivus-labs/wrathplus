import { describe, expect, test } from "bun:test";
import { digestLines, flushNotices, NoticeBatch, parseNotice, passedLua } from "../src/notice.ts";
import { classify, type GameEvent, Inbox } from "../src/wake.ts";

const ev = (event: string, ...args: unknown[]): GameEvent => ({ seq: 1, time: 0, event, args });
const noticed = (kind: string, name: string, guid: string | null, distance: number | null, extra?: string) =>
  ev("AGENT_NOTICED", kind, name, guid, distance, 1, 2, extra ?? null);

describe("NoticeBatch", () => {
  test("several notices in one window make one line, nearest first, one entry per NPC", () => {
    const batch = new NoticeBatch(15_000);
    batch.add(noticed("vendor", "Smith Argus", "0x2", 40.4, "Weaponsmith"), 0);
    batch.add(noticed("repair", "Smith Argus", "0x2", 40.4, "Weaponsmith"), 0);
    batch.add(noticed("quest_available", "Mountaineer Kadrell", "0x1", 25), 1_000);
    batch.add(noticed("quest_available", "Old Timer", "0x3", 30, "trivial"), 2_000);
    batch.add(noticed("subzone", "Thelsamar", null, 0, "Loch Modan"), 3_000);
    expect(batch.due(14_999)).toBe(false);
    expect(batch.due(15_000)).toBe(true);
    const inbox = new Inbox();
    expect(flushNotices(batch, inbox, 15_000)).toBe(5);
    const lines = inbox.take();
    expect(lines).toHaveLength(1);
    expect(lines[0]).toStartWith(
      "Noticed: subzone Thelsamar (Loch Modan); quest giver Mountaineer Kadrell (!) 25 yd; quest giver Old Timer (grey !) 30 yd; vendor, repairs Smith Argus (Weaponsmith) 40 yd.",
    );
    // A new window starts with the next notice.
    expect(batch.size).toBe(0);
    expect(batch.due(100_000)).toBe(false);
  });

  test("a window not yet over holds the notices, unless something else wakes the planner", () => {
    const batch = new NoticeBatch(15_000);
    const inbox = new Inbox();
    batch.add(noticed("trainer", "Granis Swiftaxe", "0x5", 12, "class: Warrior Trainer"), 0);
    expect(flushNotices(batch, inbox, 5_000)).toBe(0);
    expect(inbox.size).toBe(0);
    // An urgent event goes in at once, never waiting for the batch; the notices ride along.
    expect(inbox.add(ev("CHAT_MSG_WHISPER", "hi", "Bob"), "Sixer")).toBe("urgent");
    expect(inbox.has("urgent")).toBe(true);
    expect(flushNotices(batch, inbox, 5_000)).toBe(1);
    const lines = inbox.take();
    expect(lines[0]).toBe("Bob whispers: hi");
    expect(lines[1]).toContain("trainer Granis Swiftaxe (class: Warrior Trainer) 12 yd");
  });

  test("a portal reads as where it leads, with what it asks", () => {
    const batch = new NoticeBatch(0);
    batch.add(noticed("portal", "Deeprun Tram", null, 40.2), 0);
    batch.add(noticed("portal", "Onyxia's Lair", null, 52, "level 80, raid"), 0);
    expect(batch.take()).toStartWith(
      "Noticed: portal to Deeprun Tram 40 yd; portal to Onyxia's Lair (level 80, raid) 52 yd.",
    );
  });

  test("a long batch is cut, the rest counted; a window of 0 hands over at once", () => {
    const batch = new NoticeBatch(0, 3);
    for (let i = 0; i < 5; i++) batch.add(noticed("player", `P${i}`, `0x${i}`, 10 + i), 0);
    expect(batch.due(0)).toBe(true);
    const line = batch.take() ?? "";
    expect(line).toContain("player P0 10 yd; player P1 11 yd; player P2 12 yd; and 2 more");
    expect(batch.take()).toBeNull();
  });

  test("notices are notable and parse their arguments", () => {
    expect(classify(noticed("vendor", "X", "0x1", 3))).toBe("notable");
    expect(parseNotice(noticed("node", "Copper Vein", "0x9", 7.26, "ore"))).toEqual({
      kind: "node",
      name: "Copper Vein",
      guid: "0x9",
      distance: 7.26,
      extra: "ore",
    });
    expect(parseNotice(noticed("subzone", "Goldshire", null, null)).distance).toBeUndefined();
  });
});

describe("digest", () => {
  test("Notice.passed reads as one evidence line, muted counted, and gives the next cursor", () => {
    const answer = {
      ok: true,
      values: [
        {
          next: 42,
          dropped: 0,
          kinds: [
            { kind: "player", count: 9, muted: 9, names: ["A", "B", "C", "D", "E", "F"] },
            { kind: "quest_available", count: 2, muted: 1, names: ["Marshal", "Old Timer"] },
            { kind: "subzone", count: 1, muted: 0, names: ["Goldshire"] },
          ],
        },
      ],
    };
    const { lines, next } = digestLines(answer);
    expect(next).toBe(42);
    expect(lines).toEqual([
      "passed since the last check-in (12 noticed): 9 player, 9 muted: A, B, C, D, E, F, …; 2 quest giver, 1 muted: Marshal, Old Timer; 1 subzone: Goldshire",
    ]);
  });

  test("nothing passed, a dropped count, and an answer that is not one", () => {
    expect(digestLines({ ok: true, values: [{ next: 3, dropped: 0, kinds: {} }] })).toEqual({ lines: [], next: 3 });
    const dropped = digestLines({ ok: true, values: [{ next: 9, dropped: 4, kinds: [{ kind: "node", count: 1, muted: 0, names: ["Silverleaf"] }] }] });
    expect(dropped.lines[0]).toContain("(1 noticed, 4 more no longer kept)");
    expect(digestLines({ ok: false, error: "attempt to index a nil value" })).toEqual({ lines: [] });
    expect(digestLines({ ok: true, values: [null] })).toEqual({ lines: [] });
    expect(passedLua(17)).toBe("return Notice and Notice.passed(17)");
  });
});
