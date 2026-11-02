import { expect, test } from "bun:test";
import { Waker } from "../src/wake.ts";

const event = (name: string, args: unknown[] = []) => ({ seq: 1, time: 0, event: name, args });

test("a wake-up fires once, on the first awaited event or when its time is up", async () => {
  const woken: string[] = [];
  const waker = new Waker((text) => woken.push(text));
  expect(waker.set(undefined, ["AGENT_WALK_DONE"], "check the walk")).toContain("on AGENT_WALK_DONE");
  waker.notice([event("UNIT_HEALTH", ["player"])]);
  expect(woken).toEqual([]);
  waker.notice([event("AGENT_WALK_DONE", ["arrived"]), event("AGENT_WALK_DONE", ["stuck"])]);
  expect(woken).toEqual(['Wake-up you asked for (check the walk): AGENT_WALK_DONE ["arrived"].']);
  expect(waker.pending).toBe(false);

  waker.set(1, ["QUEST_LOG_UPDATE"], "look at the quest");
  await Bun.sleep(1100);
  expect(woken[1]).toBe("Wake-up you asked for (look at the quest): 1s passed.");
  waker.notice([event("QUEST_LOG_UPDATE")]);
  expect(woken).toHaveLength(2);
});

test("setting a wake-up replaces the one before", () => {
  const woken: string[] = [];
  const waker = new Waker((text) => woken.push(text));
  waker.set(undefined, ["PLAYER_DEAD"], "first");
  waker.set(undefined, ["PLAYER_LEVEL_UP"], "second");
  waker.notice([event("PLAYER_DEAD")]);
  expect(woken).toEqual([]);
  waker.notice([event("PLAYER_LEVEL_UP", [4])]);
  expect(woken).toEqual(["Wake-up you asked for (second): PLAYER_LEVEL_UP [4]."]);
});
