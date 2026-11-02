import { describe as group, expect, test } from "bun:test";
import { Inbox, summarize, TOOLS } from "../src/brain.ts";
import { configFromEnv } from "../src/index.ts";
import { describe, PLANNER_EVENTS, watch, type GameEvent } from "../src/watch.ts";

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
    ], "Agentone");
    expect(lines).toEqual([
      "You died.",
      "Dongle whispers: need help?",
      "Wifey invites you to a party (AcceptGroup() or DeclineGroup()).",
      "Your Lua failed: module x not found (in: require('x'))",
      "Game error: Out of range.",
      "Your walk ended stuck: AgentWalk().note says why; route around it or pick another way.",
    ]);
  });

  test("the planner hears /say only when it names the character", () => {
    const lines = describe([
      event(1, "CHAT_MSG_SAY", ["anyone selling bread?", "Botty"]),
      event(2, "CHAT_MSG_SAY", ["agenttwo, follow me", "Shannon"]),
      event(3, "CHAT_MSG_PARTY", ["pull the next one", "Shannon"]),
    ], "Agenttwo", PLANNER_EVENTS);
    expect(lines).toEqual(["Shannon says: agenttwo, follow me", "[Party] Shannon: pull the next one"]);
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
          return Response.json({ events: [event(10, "PLAYER_DEAD"), event(11, "CHAT_MSG_WHISPER", ["hi", "Dongle"])], next: 12, missed: 0 });
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
    expect(messages).toEqual(["Game events:\n- You died.\n- Dongle whispers: hi"]);
  });
});

group("planner plumbing", () => {
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
    expect(config).toEqual({ url: "http://127.0.0.1:17950", token: "t", goal: "Reach level 5", model: "claude-sonnet-5-5", checkInSeconds: 0 });
  });
});
