import { describe as group, expect, test } from "bun:test";
import { Inbox, summarize, TOOLS } from "../src/brain.ts";
import { configFromEnv } from "../src/index.ts";
import { SYSTEM_PROMPT, systemPrompt } from "../src/prompt.ts";
import { collapse, describe, overheard, PLANNER_EVENTS, RepeatedErrors, watch, type GameEvent } from "../src/watch.ts";

const event = (seq: number, name: string, args: unknown[] = []): GameEvent => ({ seq, time: seq, event: name, args });

group("events a player would notice", () => {
  test("deaths, whispers, invites and failed agent Lua become lines; noise does not", () => {
    const lines = describe([
      event(1, "UNIT_HEALTH", ["player"]),
      event(2, "PLAYER_DEAD"),
      event(3, "CHAT_MSG_WHISPER", ["need help?", "Dongle"]),
      event(4, "PARTY_INVITE_REQUEST", ["Wifey"]),
      event(5, "AGENT_LUA", ["require('x')", false, "module x not found"]),
      event(6, "AGENT_LUA", ["return 1", true, null]),
      event(7, "CHAT_MSG_SAY", ["hello", "Agentone"]),
      event(8, "UI_ERROR_MESSAGE", ["Out of range."]),
      event(9, "UI_ERROR_MESSAGE", ["Out of range."]),
      event(10, "AGENT_WALK_DONE", ["arrived"]),
      event(11, "AGENT_WALK_DONE", ["stuck"]),
      event(12, "AGENT_WALK_DONE", ["stalled", "Quartermaster Hicks"]),
      event(13, "AGENT_RESTED", [200, null]),
    ], "Agentone");
    expect(lines).toEqual([
      "You died.",
      "Dongle whispers: need help?",
      "Wifey invites you to a party (AcceptGroup() or DeclineGroup()).",
      "Your Lua failed: module x not found (in: require('x'))",
      "Game error: Out of range. ×2",
      "Your walk ended stuck: AgentWalk().note says why; route around it or pick another way.",
      "A walk to Quartermaster Hicks stalled: no ground gained for a while, even after sending it again. Look at where you are (a wall, a ledge, a door) and route around it.",
      "Rested: health and mana are full.",
    ]);
  });

  test("/say wakes the planner from the party or by name; the rest is overheard", () => {
    const events = [
      event(1, "CHAT_MSG_SAY", ["anyone selling bread?", "Botty"]),
      event(2, "CHAT_MSG_SAY", ["agenttwo, follow me", "Shannon"]),
      event(3, "CHAT_MSG_SAY", ["over here", "Pallytest"]),
      event(4, "CHAT_MSG_PARTY", ["pull the next one", "Pallytest"]),
      event(5, "CHAT_MSG_MONSTER_SAY", ["Fresh bread, baked this very morning!", "Kira Songshine"]),
      event(6, "CHAT_MSG_SAY", ["hello", "Agenttwo"]),
    ];
    const party = new Set(["pallytest"]);
    expect(describe(events, "Agenttwo", PLANNER_EVENTS, party)).toEqual([
      "Shannon says: agenttwo, follow me",
      "Pallytest says: over here",
      "[Party] Pallytest: pull the next one",
    ]);
    expect(overheard(events, "Agenttwo", party)).toEqual([
      "Botty says: anyone selling bread?",
      "Kira Songshine says: Fresh bread, baked this very morning!",
    ]);
  });

  test("the planner hears emotes aimed at it, not other people's or its own", () => {
    const lines = describe([
      event(1, "CHAT_MSG_TEXT_EMOTE", ["Kobold Worker ponders Pallytest's actions.", "Kobold Worker"]),
      event(2, "CHAT_MSG_TEXT_EMOTE", ["Pallytest waves at you.", "Pallytest"]),
      event(3, "CHAT_MSG_TEXT_EMOTE", ["You wave at Pallytest.", "Agenttwo"]),
      event(4, "CHAT_MSG_EMOTE", ["pokes Agenttwo with a stick.", "Pallytest"]),
    ], "Agenttwo", PLANNER_EVENTS);
    expect(lines).toEqual([
      'Pallytest waves at you. (an emote; answer with DoEmote("wave", "Pallytest") or in chat)',
      "Pallytest pokes Agenttwo with a stick. (an emote)",
    ]);
  });

  test("an NPC naming the character wakes the planner (unless switched off)", () => {
    const events = [
      event(1, "CHAT_MSG_MONSTER_SAY", ["Deep night, and the cows still want their milking. City folk never understand that, Agenttwo.", "Ma Stonefield"]),
      event(2, "CHAT_MSG_MONSTER_SAY", ["Quiet now, love.", "Ma Stonefield"]),
    ];
    expect(describe(events, "Agenttwo", PLANNER_EVENTS)).toEqual([
      "Ma Stonefield says to you: Deep night, and the cows still want their milking. City folk never understand that, Agenttwo. (an NPC; a reply in /say is welcome, not required)",
    ]);
    expect(overheard(events, "Agenttwo")).toEqual(["Ma Stonefield says: Quiet now, love."]);
    expect(describe(events, "Agenttwo", PLANNER_EVENTS, undefined, false)).toEqual([]);
    expect(overheard(events, "Agenttwo", undefined, false)).toHaveLength(2);
  });

  test("a game error that keeps coming back reaches the planner once, with its count", () => {
    const errors = new RepeatedErrors(3, 15_000, 60_000);
    const far = (seq: number) => event(seq, "UI_ERROR_MESSAGE", ["You are too far away!"]);
    expect(errors.notice([far(1), far(2)], 0)).toEqual([]);
    expect(errors.notice([far(3)], 5_000)).toEqual([
      'Game error, 3 times in 15s: "You are too far away!". Your Lua keeps doing something the game refuses; fix it.',
    ]);
    expect(errors.notice([far(4), far(5)], 6_000)).toEqual([]);
    expect(errors.notice([far(6), far(7), far(8)], 70_000)).toHaveLength(1);
    expect(errors.notice([event(9, "UI_ERROR_MESSAGE", ["Not enough rage"])], 70_000)).toEqual([]);
  });

  test("the watcher batches a burst into one message", async () => {
    let served = false;
    const port = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch(request) {
        const since = Number(new URL(request.url).searchParams.get("since"));
        if (since >= 1_000_000) return Response.json({ events: [], next: 10, missed: 0 });
        if (!served && since === 10) {
          served = true;
          // A walk that fails in a hot loop around the death: one line with its count.
          const walks = Array.from({ length: 40 }, (_, i) => event(12 + i, "AGENT_WALK_DONE", ["failed"]));
          return Response.json({ events: [walks[0], event(10, "PLAYER_DEAD"), ...walks.slice(1), event(11, "CHAT_MSG_WHISPER", ["hi", "Dongle"])], next: 52, missed: 0 });
        }
        await Bun.sleep(50);
        return Response.json({ events: [], next: since, missed: 0 });
      },
    });
    const abort = new AbortController();
    const messages: string[] = [];
    const done = watch({ url: port.url.href.replace(/\/$/, ""), token: "t", batchMs: 100, signal: abort.signal, onMessage: (m) => messages.push(m) });
    while (!messages.length) await Bun.sleep(20);
    abort.abort();
    await done;
    port.stop(true);
    expect(messages).toEqual([
      "Game events:\n- Your walk ended failed: AgentWalk().note says why; route around it or pick another way. ×40\n- You died.\n- Dongle whispers: hi",
    ]);
  });

  test("a request triage carried out reaches neither the planner nor the fast tier's wake-ups", async () => {
    let served = false;
    const port = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch(request) {
        const since = Number(new URL(request.url).searchParams.get("since"));
        if (since >= 1_000_000) return Response.json({ events: [], next: 10, missed: 0 });
        if (!served && since === 10) {
          served = true;
          return Response.json({
            events: [event(10, "CHAT_MSG_PARTY", ["follow me", "Pallytest"]), event(11, "PLAYER_LEVEL_UP", [8])],
            next: 12,
            missed: 0,
          });
        }
        await Bun.sleep(50);
        return Response.json({ events: [], next: since, missed: 0 });
      },
    });
    const abort = new AbortController();
    const messages: string[] = [];
    const seen: string[] = [];
    const handled: string[] = [];
    const done = watch({
      url: port.url.href.replace(/\/$/, ""),
      token: "t",
      batchMs: 50,
      signal: abort.signal,
      onMessage: (m) => messages.push(m),
      onEvents: (events) => seen.push(...events.map((e) => e.event)),
      triage: async (r) => ({ handled: true, note: `${r.sender} said "${r.text}"; the fast loop followed them`, reply: false }),
      onHandled: (note) => handled.push(note),
    });
    while (!messages.length) await Bun.sleep(20);
    abort.abort();
    await done;
    port.stop(true);
    expect(seen).toEqual(["PLAYER_LEVEL_UP"]);
    expect(handled).toEqual(['Pallytest said "follow me"; the fast loop followed them']);
    expect(messages[0]).not.toContain("follow me");
  });
});

group("planner plumbing", () => {
  test("small talk is sorted apart from the news: it wakes the planner only when the sorter says so", async () => {
    let served = false;
    const port = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch(request) {
        const since = Number(new URL(request.url).searchParams.get("since"));
        if (since >= 1_000_000) return Response.json({ events: [], next: 10, missed: 0 });
        if (!served && since === 10) {
          served = true;
          return Response.json({
            events: [
              event(10, "CHAT_MSG_MONSTER_SAY", ["Mind the wolves, Agenttwo.", "Ma Stonefield"]),
              event(11, "CHAT_MSG_SAY", ["Agenttwo, group for Hogger?", "Thrallson"]),
              event(12, "CHAT_MSG_WHISPER", ["hi", "Dongle"]),
              event(13, "CHAT_MSG_SAY", ["Agenttwo, this way", "Pallytest"]),
            ],
            next: 14,
            missed: 0,
          });
        }
        await Bun.sleep(50);
        return Response.json({ events: [], next: since, missed: 0 });
      },
    });
    const abort = new AbortController();
    const messages: string[] = [];
    const sorted: string[] = [];
    const done = watch({
      url: port.url.href.replace(/\/$/, ""),
      token: "t",
      batchMs: 100,
      signal: abort.signal,
      self: () => "Agenttwo",
      party: () => new Set(["pallytest"]),
      smallTalk: async (items) => {
        sorted.push(...items.map((i) => `${i.kind} ${i.speaker}`));
        return items.filter((i) => i.kind === "say").map((i) => `${i.speaker} says to you: ${i.text} (the fast loop judged this needs you)`);
      },
      onMessage: (m) => messages.push(m),
    });
    while (!messages.length) await Bun.sleep(20);
    abort.abort();
    await done;
    port.stop(true);
    expect(sorted).toEqual(["npc Ma Stonefield", "say Thrallson"]);
    // The whisper and the party's /say wake it as ever; the NPC's line does not appear at all.
    expect(messages).toEqual([
      "Game events:\n- Dongle whispers: hi\n- Pallytest says: Agenttwo, this way\n- Thrallson says to you: Agenttwo, group for Hogger? (the fast loop judged this needs you)",
    ]);
  });

  test("the prompt says early that nobody is there, and promises no person anywhere", () => {
    const own = SYSTEM_PROMPT.indexOf("You are on your own");
    expect(own).toBeGreaterThan(0);
    expect(own).toBeLessThan(SYSTEM_PROMPT.indexOf("You act only through the client"));
    expect(SYSTEM_PROMPT).toContain("There is no operator and no user.");
    expect(SYSTEM_PROMPT).toContain("nobody will answer a question, approve a plan, give an instruction or log in");
    expect(SYSTEM_PROMPT).toContain("is a decision you must take now with what you know: decide, note why in MEMORY.md, and act");
    expect(SYSTEM_PROMPT).toContain('marked "Outside note". Such notes are rare and may never come');
    expect(SYSTEM_PROMPT).not.toMatch(/operator notes|a person reading along/);
    expect(SYSTEM_PROMPT).toContain("The goal runner is always on while you play");
    // Shannon's lines stay.
    expect(SYSTEM_PROMPT).toContain("Reflect regularly on your performance");
    expect(SYSTEM_PROMPT).toContain("Work on your professions, skills, and abilities");
    expect(SYSTEM_PROMPT).toContain("Keep your gear maintained");
    expect(SYSTEM_PROMPT).toContain("Keep your spells and talents up to date");
    expect(SYSTEM_PROMPT).toContain("Personality traits and behavior guidelines for your character");
    expect(systemPrompt(false)).toContain("You are on your own");
  });

  test("the prompt takes zone levels from Goals.zones() and routes travel through data.zone", () => {
    expect(SYSTEM_PROMPT).toContain("a zone's levels come from Goals.zones()");
    expect(SYSTEM_PROMPT).toContain("never from your memory of the game");
    expect(SYSTEM_PROMPT).toContain("push_goal with zone set (the goal's data.zone)");
    expect(SYSTEM_PROMPT).toContain("Never teach an action of your own that walks to another zone");
    expect(SYSTEM_PROMPT).toContain("When a built-in keeps doing nothing, suspect your own modules first");
  });

  test("the prompt says any vendor buys anything, and that a walk by hand holds the runner", () => {
    expect(SYSTEM_PROMPT).toContain("every vendor buys any item, quartermasters and smiths included");
    expect(SYSTEM_PROMPT).toContain("puts it on hold until the walk ends");
    expect(SYSTEM_PROMPT).toContain("wake_me on AGENT_RESTED");
  });

  test("the inbox yields messages in order and ends when closed", async () => {
    const inbox = new Inbox();
    inbox.push("first");
    setTimeout(() => {
      inbox.push("second");
      inbox.close();
    }, 10);
    const seen: unknown[] = [];
    for await (const m of inbox) seen.push(m.message.content);
    expect(seen).toEqual(["first", "second"]);
  });

  test("only the WrathPlus tools are allowed", () => {
    expect(TOOLS).toEqual([
      "mcp__wrathplus__lua",
      "mcp__wrathplus__screenshot",
      "mcp__wrathplus__events",
      "mcp__wrathplus__status",
      "mcp__wrathplus__modules",
      "mcp__wrathplus__save_module",
    ]);
  });

  test("tool calls are shown short", () => {
    const line = summarize({
      type: "assistant",
      message: { content: [{ type: "text", text: "Checking." }, { type: "tool_use", id: "1", name: "mcp__wrathplus__lua", input: { code: "return 1" } }] },
    } as never);
    expect(line).toBe('Checking.\n→ lua {"code":"return 1"}');
  });

  test("config needs the port and token; Sonnet 5.5 by default", () => {
    expect(() => configFromEnv([], {})).toThrow("WRATHPLUS_AGENT_URL");
    const config = configFromEnv(["Reach", "level", "5"], { WRATHPLUS_AGENT_URL: "http://127.0.0.1:17950/", WRATHPLUS_AGENT_TOKEN: "t" });
    expect(config).toEqual({ url: "http://127.0.0.1:17950", token: "t", goal: "Reach level 5", model: "claude-haiku-5-5", checkInSeconds: 0 });
  });
});

test("lines repeated across batches add up to one line with their count", () => {
  expect(collapse(["Your walk ended failed ×3", "You died.", "Your walk ended failed ×5", "Your walk ended failed"])).toEqual([
    "Your walk ended failed ×9",
    "You died.",
  ]);
});
