import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { AgentClient } from "../src/client.ts";
import { configFromEnv } from "../src/index.ts";
import { createServer, HELPERS, MAX_WAIT_MS } from "../src/tools.ts";

const TOKEN = "test-token-0123456789";
const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

interface Seen {
  method: string;
  path: string;
  query: Record<string, string>;
  auth: string | null;
  body: string;
}

/** A stand-in control port: records each request; `reply` decides the answer. */
let seen: Seen[] = [];
let reply: (req: Seen) => Response = () => Response.json({});
let port: ReturnType<typeof Bun.serve>;

beforeAll(() => {
  port = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    async fetch(request) {
      const url = new URL(request.url);
      const req: Seen = {
        method: request.method,
        path: url.pathname,
        query: Object.fromEntries(url.searchParams),
        auth: request.headers.get("authorization"),
        body: await request.text(),
      };
      seen.push(req);
      if (req.auth !== `Bearer ${TOKEN}`) {
        return Response.json({ error: "missing or wrong bearer token" }, { status: 401 });
      }
      return reply(req);
    },
  });
});

afterAll(() => port.stop(true));

afterEach(() => {
  seen = [];
  reply = () => Response.json({});
});

async function connect(baseUrl = port.url.href, token = TOKEN, longPoll = true): Promise<Client> {
  const server = createServer(new AgentClient(baseUrl, token), { longPoll });
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  await server.connect(serverSide);
  const client = new Client({ name: "test", version: "0" });
  await client.connect(clientSide);
  return client;
}

async function call(name: string, args: Record<string, unknown> = {}, client?: Client) {
  const mcp = client ?? (await connect());
  return (await mcp.callTool({ name, arguments: args })) as CallToolResult;
}

function textOf(result: CallToolResult): string {
  const first = result.content[0];
  if (first?.type !== "text") throw new Error(`expected text content, got ${first?.type}`);
  return first.text;
}

describe("tools", () => {
  test("lists the tools", async () => {
    const { tools } = await (await connect()).listTools();
    expect(tools.map((t) => t.name).sort()).toEqual([
      "events",
      "lua",
      "modules",
      "save_module",
      "screenshot",
      "status",
    ]);
    expect(tools.find((t) => t.name === "lua")?.description).toContain("wp.engine");
    expect(tools.find((t) => t.name === "lua")?.description).toContain("3.3.5");
    expect(tools.find((t) => t.name === "events")?.description).toContain("next");
  });

  test("without long-polling, events offers no wait_ms and never waits", async () => {
    const client = await connect(port.url.href, TOKEN, false);
    const events = (await client.listTools()).tools.find((t) => t.name === "events");
    expect(Object.keys(events?.inputSchema.properties ?? {})).not.toContain("wait_ms");
    expect(events?.description).not.toContain("wait_ms");
    reply = () => Response.json({ events: [], next: 0, missed: 0 });
    await call("events", { since: 3, wait_ms: 30_000 }, client);
    expect(seen[0]?.query.wait_ms).toBeUndefined();
  });

  test("lua posts the chunk as the raw body with its budget and the bearer token", async () => {
    reply = (req) => Response.json({ ok: true, values: [req.body], output: ["hi"] });
    const result = await call("lua", { code: 'print("hi") return UnitName("player")', budget: 5000 });
    expect(seen).toHaveLength(1);
    expect(seen[0]).toEqual({
      method: "POST",
      path: "/lua",
      query: { budget: "5000" },
      auth: `Bearer ${TOKEN}`,
      body: 'print("hi") return UnitName("player")',
    });
    expect(result.isError).toBeFalsy();
    expect(JSON.parse(textOf(result))).toEqual({
      ok: true,
      values: ['print("hi") return UnitName("player")'],
      output: ["hi"],
    });
  });

  test("lua without a budget sends no query, and a Lua error is a tool error", async () => {
    reply = () => Response.json({ ok: false, error: "attempt to call a nil value", output: [] });
    const result = await call("lua", { code: "nope()" });
    expect(seen[0]?.query).toEqual({});
    expect(result.isError).toBe(true);
    expect(textOf(result)).toContain("attempt to call a nil value");
  });

  test("screenshot returns PNG image content, default scale 0.5", async () => {
    reply = () => new Response(PNG, { headers: { "Content-Type": "image/png" } });
    const result = await call("screenshot");
    expect(seen[0]).toMatchObject({ method: "GET", path: "/screenshot", query: { scale: "0.5" } });
    expect(seen[0]?.auth).toBe(`Bearer ${TOKEN}`);
    expect(result.isError).toBeFalsy();
    expect(result.content).toEqual([
      { type: "image", mimeType: "image/png", data: Buffer.from(PNG).toString("base64") },
    ]);
  });

  test("screenshot passes an explicit scale and maps a JSON error", async () => {
    reply = () => Response.json({ error: "frame conversion: bad" }, { status: 500 });
    const result = await call("screenshot", { scale: 1 });
    expect(seen[0]?.query).toEqual({ scale: "1" });
    expect(result.isError).toBe(true);
    expect(textOf(result)).toContain("HTTP 500: frame conversion: bad");
  });

  test("events sends since/wait_ms/limit and returns the JSON", async () => {
    const body = { events: [{ seq: 4, time: 1.5, event: "PLAYER_LOGIN", args: [] }], next: 5, missed: 0 };
    reply = () => Response.json(body);
    const result = await call("events", { since: 3, wait_ms: 2000, limit: 10 });
    expect(seen[0]).toMatchObject({
      method: "GET",
      path: "/events",
      query: { since: "3", wait_ms: "2000", limit: "10" },
      auth: `Bearer ${TOKEN}`,
    });
    expect(JSON.parse(textOf(result))).toEqual(body);
  });

  test("events without since sends none: the port reads from now", async () => {
    reply = () => Response.json({ events: [], next: 7801, missed: 0 });
    const result = await call("events");
    expect(seen[0]?.query).toEqual({});
    expect(JSON.parse(textOf(result))).toEqual({ events: [], next: 7801, missed: 0 });
    // The whole buffer is asked for by name: since 0.
    await call("events", { since: 0 });
    expect(seen[1]?.query).toEqual({ since: "0" });
  });

  test("events passes names and exclude as comma-separated lists", async () => {
    reply = () => Response.json({ events: [], next: 9, missed: 0 });
    await call("events", { since: 4, names: ["AGENT_", "CHAT_MSG_", "PLAYER_DEAD"], exclude: ["AGENT_LUA"], limit: 20 });
    expect(seen[0]?.query).toEqual({
      since: "4",
      names: "AGENT_,CHAT_MSG_,PLAYER_DEAD",
      exclude: "AGENT_LUA",
      limit: "20",
    });
    // On the wire the commas are escaped, which the port undoes.
    await call("events", { names: [], exclude: [] });
    expect(seen[1]?.query).toEqual({});
    const refused = await call("events", { names: [""] });
    expect(refused.isError).toBe(true);
    expect(seen).toHaveLength(2);
  });

  test("events caps wait_ms under a host's 30 s tool timeout, and says so", async () => {
    expect(MAX_WAIT_MS).toBe(25_000);
    reply = () => Response.json({ events: [], next: 3, missed: 0 });
    await call("events", { since: 3, wait_ms: 30_000, names: ["AGENT_"] });
    expect(seen[0]?.query).toEqual({ since: "3", wait_ms: "25000", names: "AGENT_" });
    await call("events", { since: 3, wait_ms: 1_000 });
    expect(seen[1]?.query.wait_ms).toBe("1000");
    const events = (await (await connect()).listTools()).tools.find((t) => t.name === "events");
    expect(events?.description).toContain("at most 25000 ms");
    expect(events?.description).toContain("default 100");
    expect(events?.description).toContain("since=0");
  });

  test("every description opens with a sentence that stands alone inside 100 characters", async () => {
    const { tools } = await (await connect()).listTools();
    expect(tools).toHaveLength(6);
    for (const tool of tools) {
      const opening = (tool.description ?? "").split(/(?<=\.)\s/)[0] ?? "";
      expect(opening.length, `${tool.name}: ${opening}`).toBeGreaterThan(30);
      expect(opening.length, `${tool.name}: ${opening}`).toBeLessThanOrEqual(100);
      expect(opening.endsWith("."), `${tool.name}: ${opening}`).toBe(true);
    }
  });

  test("the lua tool carries the helper reference, compact", async () => {
    const lua = (await (await connect()).listTools()).tools.find((t) => t.name === "lua");
    expect(lua?.description).toContain(HELPERS);
    expect(HELPERS.length).toBeLessThanOrEqual(1500);
    for (const helper of [
      "AgentNearby(",
      "AgentPosition()",
      "AgentMoveTo(",
      "AgentWalk()",
      "AGENT_WALK_DONE",
      "AgentInteract(",
      "AgentTarget(",
      "Combat.engage(",
      "AGENT_COMBAT_DONE",
      "Combat.engaged(",
      "Quests.pick_up()",
      "Quests.hand_in()",
      "AGENT_QUESTS_DONE(kind, picked, handed)",
      "Quests.list()",
      "Move.to(",
      "Move.follow(",
      "Goals.on",
      "Goals.list()",
      "Goals.facts()",
      "Goals.push{",
      "Goals.hold{",
      "Agent.activity()",
      "StopCinematic()",
    ]) {
      expect(HELPERS, helper).toContain(helper);
    }
    // The walk's statuses and the fight's outcomes, as the client's Lua spells them.
    expect(HELPERS).toContain("idle|routing|walking|arrived|stopped|failed|stuck");
    expect(HELPERS).toContain("killed|lost|stopped|died");
  });

  test("status says what a null player and cinematic mean", async () => {
    const status = (await (await connect()).listTools()).tools.find((t) => t.name === "status");
    expect(status?.description).toContain("player is null");
    expect(status?.description).toContain("cinematic is true");
    expect(status?.description).toContain("StopCinematic()");
  });

  test("status gets /status", async () => {
    const body = { state: "InWorld", in_world: true, ui_ready: true, cinematic: false, time: 12.5, player: { name: "Jev" } };
    reply = () => Response.json(body);
    const result = await call("status");
    expect(seen[0]).toMatchObject({ method: "GET", path: "/status", query: {}, auth: `Bearer ${TOKEN}`, body: "" });
    expect(JSON.parse(textOf(result))).toEqual(body);
  });
});

describe("errors", () => {
  test("401 is a tool error with the server's text", async () => {
    const result = await call("status", {}, await connect(port.url.href, "wrong-token-0123456789"));
    expect(result.isError).toBe(true);
    expect(textOf(result)).toContain("HTTP 401: missing or wrong bearer token");
    expect(textOf(result)).not.toContain("wrong-token-0123456789");
  });

  test("504 is a tool error with the server's text", async () => {
    reply = () => Response.json({ error: "the game did not answer in time" }, { status: 504 });
    const result = await call("lua", { code: "return 1" });
    expect(result.isError).toBe(true);
    expect(textOf(result)).toContain("HTTP 504: the game did not answer in time");
  });

  test("connection refused says the client may not be running", async () => {
    const closed = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => new Response() });
    const url = closed.url.href;
    await closed.stop(true);
    const result = await call("status", {}, await connect(url));
    expect(result.isError).toBe(true);
    expect(textOf(result)).toContain("could not reach the WrathPlus client");
    expect(textOf(result)).toContain("may not be running");
  });
});

describe("configFromEnv", () => {
  test("accepts a URL and token", () => {
    expect(configFromEnv({ WRATHPLUS_AGENT_URL: "http://127.0.0.1:17950", WRATHPLUS_AGENT_TOKEN: TOKEN })).toEqual({
      url: "http://127.0.0.1:17950",
      token: TOKEN,
    });
  });

  test("names every missing variable", () => {
    expect(() => configFromEnv({})).toThrow(/WRATHPLUS_AGENT_URL.*WRATHPLUS_AGENT_TOKEN/);
  });

  test("rejects a non-http URL without echoing the token", () => {
    let message = "";
    try {
      configFromEnv({ WRATHPLUS_AGENT_URL: "ftp://x", WRATHPLUS_AGENT_TOKEN: TOKEN });
    } catch (error) {
      message = (error as Error).message;
    }
    expect(message).toContain("not an http(s) URL");
    expect(message).not.toContain(TOKEN);
  });
});

describe("modules", () => {
  test("save_module puts the source and modules lists or reads", async () => {
    reply = (req) =>
      req.method === "GET" && req.path === "/files/grind.loop"
        ? new Response("return {}", { headers: { "content-type": "text/plain" } })
        : Response.json(req.path === "/files" ? { modules: ["grind.loop"] } : { saved: "grind.loop" });
    const client = await connect();
    const saved = await call("save_module", { name: "grind.loop", source: "return {}" }, client);
    expect(saved.isError).toBeUndefined();
    expect(seen[0]).toMatchObject({ method: "PUT", path: "/files/grind.loop", body: "return {}" });
    expect(textOf(await call("modules", {}, client))).toContain("grind.loop");
    expect(textOf(await call("modules", { name: "grind.loop" }, client))).toBe("return {}");
    expect(seen.map((s) => `${s.method} ${s.path}`)).toEqual([
      "PUT /files/grind.loop",
      "GET /files",
      "GET /files/grind.loop",
    ]);
  });

  test("save_module refuses names that are not dotted words", async () => {
    const result = await call("save_module", { name: "../etc", source: "x" });
    expect(result.isError).toBe(true);
    expect(seen).toEqual([]);
  });
});
