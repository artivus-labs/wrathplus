import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createGuard } from "../src/guard.ts";
import { Meter, monthOf, nextMonthStart } from "../src/meter.ts";
import { costUsd, PRICES } from "../src/pricing.ts";
import { StreamUsage } from "../src/usage.ts";

const HAIKU = PRICES["claude-haiku-5-5"]!;
const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function stateFile() {
  const dir = mkdtempSync(join(tmpdir(), "llm-guard-"));
  dirs.push(dir);
  return join(dir, "usage.json");
}

/** A clock the test can move. */
function clock(iso: string) {
  let now = new Date(iso);
  return { now: () => now, set: (next: string) => (now = new Date(next)) };
}

function setup(options: { cap?: number; at?: string; reply?: (init: RequestInit) => Response } = {}) {
  const time = clock(options.at ?? "2026-11-16T12:00:00Z");
  const logs: string[] = [];
  const path = stateFile();
  const meter = new Meter({ path, capUsd: options.cap ?? 100, now: time.now, log: (l) => logs.push(l) });
  const sent: { url: string; init: RequestInit }[] = [];
  const reply =
    options.reply ??
    (() =>
      Response.json(
        { type: "message", content: [{ type: "text", text: "Well met." }], usage: { input_tokens: 2000, output_tokens: 100 } },
        { headers: { "request-id": "req_1" } },
      ));
  const guard = createGuard({
    meter,
    apiKey: "real-key",
    models: ["claude-haiku-5-5"],
    now: time.now,
    log: (l) => logs.push(l),
    fetch: async (url, init) => {
      sent.push({ url, init });
      return reply(init);
    },
  });
  return { guard, meter, logs, sent, time, path };
}

function call(guard: (r: Request) => Promise<Response>, body: unknown = { model: "claude-haiku-5-5", max_tokens: 64, messages: [] }) {
  return guard(
    new Request("http://guard/v1/messages", {
      method: "POST",
      headers: { "content-type": "application/json", "x-api-key": "placeholder", "anthropic-version": "2023-06-01" },
      body: JSON.stringify(body),
    }),
  );
}

describe("pricing (claude-haiku-5-5)", () => {
  test("input and output at the short-prompt rates", () => {
    expect(costUsd({ input_tokens: 50_000, output_tokens: 50_000 }, HAIKU)).toBeCloseTo(0.03, 10);
  });

  test("cache writes at 1.25x input, reads at 0.1x input", () => {
    expect(costUsd({ cache_creation_input_tokens: 80_000 }, HAIKU)).toBeCloseTo(0.01, 10);
    expect(costUsd({ cache_read_input_tokens: 100_000 }, HAIKU)).toBeCloseTo(0.001, 10);
  });

  test("1-hour cache writes at 2x input when the breakdown says so", () => {
    const usage = { cache_creation_input_tokens: 100_000, cache_creation: { ephemeral_1h_input_tokens: 50_000 } };
    expect(costUsd(usage, HAIKU)).toBeCloseTo((50_000 * 0.125 + 50_000 * 0.2) / 1e6, 12);
  });

  test("a prompt over 100K tokens (cache included) prices the whole request at the long rates", () => {
    expect(costUsd({ input_tokens: 100_000, output_tokens: 1000 }, HAIKU)).toBeCloseTo((100_000 * 0.1 + 1000 * 0.5) / 1e6, 12);
    const long = { input_tokens: 60_000, cache_read_input_tokens: 50_000, output_tokens: 1000 };
    expect(costUsd(long, HAIKU)).toBeCloseTo((60_000 * 0.5 + 50_000 * 0.05 + 1000 * 2.5) / 1e6, 12);
  });
});

describe("meter", () => {
  test("persists the month's totals across restarts", () => {
    const time = clock("2026-11-16T12:00:00Z");
    const path = stateFile();
    const first = new Meter({ path, capUsd: 100, now: time.now, log: () => {} });
    first.record({ input_tokens: 2000, output_tokens: 100 }, HAIKU);
    expect(statSync(path).mode & 0o777).toBe(0o600);
    const second = new Meter({ path, capUsd: 100, now: time.now, log: () => {} });
    expect(second.snapshot()).toMatchObject({ month: "2026-11", calls: 1, input_tokens: 2000, output_tokens: 100, usd: 0.00025 });
  });

  test("logs 50, 80 and 100 percent once each", () => {
    const { meter, logs } = setup({ cap: 1 });
    meter.record({ output_tokens: 1_000_000 }, HAIKU); // $0.50
    meter.record({ output_tokens: 200_000 }, HAIKU); // $0.60
    meter.record({ output_tokens: 400_000 }, HAIKU); // $0.80
    meter.record({ output_tokens: 400_000 }, HAIKU); // $1.00
    meter.record({ output_tokens: 1 }, HAIKU);
    const alerts = logs.filter((l) => l.includes("% of the $1 budget"));
    expect(alerts.map((l) => l.match(/reached (\d+)%/)![1])).toEqual(["50", "80", "100"]);
    expect(alerts[2]).toContain("refusing calls until 2026-12-01T00:00:00.000Z");
  });

  test("a new UTC month starts at zero and keeps the old month in history", () => {
    const { meter, time } = setup({ cap: 1, at: "2026-11-30T23:59:59Z" });
    meter.record({ output_tokens: 3_000_000 }, HAIKU);
    expect(meter.allow()).toBe(false);
    time.set("2026-12-01T00:00:00Z");
    expect(meter.allow()).toBe(true);
    const snapshot = meter.snapshot();
    expect(snapshot).toMatchObject({ month: "2026-12", usd: 0, calls: 0 });
    expect(snapshot.history).toEqual([{ month: "2026-11", usd: 1.5, calls: 1, refused: 0 }]);
  });

  test("months and their boundaries are UTC", () => {
    expect(monthOf(new Date("2026-11-30T23:30:00-05:00"))).toBe("2026-12");
    expect(nextMonthStart(new Date("2026-12-31T10:00:00Z")).toISOString()).toBe("2027-01-01T00:00:00.000Z");
  });
});

describe("proxy", () => {
  test("forwards with the real key, never the caller's, and meters the response", async () => {
    const { guard, sent, meter } = setup();
    const response = await call(guard);
    expect(response.status).toBe(200);
    expect(response.headers.get("request-id")).toBe("req_1");
    expect(await response.json()).toMatchObject({ content: [{ text: "Well met." }] });
    expect(sent).toHaveLength(1);
    expect(sent[0]!.url).toBe("https://api.anthropic.com/v1/messages");
    const headers = sent[0]!.init.headers as Headers;
    expect(headers.get("x-api-key")).toBe("real-key");
    expect(headers.get("anthropic-version")).toBe("2023-06-01");
    expect(meter.snapshot()).toMatchObject({ calls: 1, input_tokens: 2000, output_tokens: 100, usd: 0.00025 });
  });

  test("refuses with a 429 once the budget is spent, until next month", async () => {
    const big = () => Response.json({ usage: { input_tokens: 0, output_tokens: 2_000_000 } }); // $1.00
    const { guard, sent, meter, time } = setup({ cap: 1, reply: big });
    expect((await call(guard)).status).toBe(200);
    const refused = await call(guard);
    expect(refused.status).toBe(429);
    expect(refused.headers.get("x-should-retry")).toBe("false");
    expect(Number(refused.headers.get("retry-after"))).toBe(14 * 86400 + 12 * 3600);
    const body = (await refused.json()) as { error: { type: string; message: string } };
    expect(body.error.type).toBe("rate_limit_error");
    expect(body.error.message).toContain("monthly LLM budget of $1 for 2026-11 is spent");
    expect(body.error.message).toContain("2026-12-01T00:00:00.000Z");
    expect(sent).toHaveLength(1);
    expect(meter.snapshot()).toMatchObject({ refused: 1, over_budget: true });

    time.set("2026-12-01T00:00:01Z");
    expect((await call(guard)).status).toBe(200);
    expect(sent).toHaveLength(2);
  });

  test("refuses a model it has no price for, without calling the API", async () => {
    const { guard, sent } = setup();
    const response = await call(guard, { model: "claude-haiku-4-5-20251001", max_tokens: 64, messages: [] });
    expect(response.status).toBe(400);
    expect(((await response.json()) as { error: { message: string } }).error.message).toContain("is not allowed");
    expect(sent).toHaveLength(0);
  });

  test("API errors pass through and cost nothing", async () => {
    const reply = () =>
      Response.json({ type: "error", error: { type: "invalid_request_error", message: "temperature" } }, { status: 400 });
    const { guard, meter } = setup({ reply });
    const response = await call(guard);
    expect(response.status).toBe(400);
    expect(meter.snapshot()).toMatchObject({ calls: 0, errors: 1, usd: 0 });
  });

  test("meters a streamed response from message_start and the final message_delta", async () => {
    const events = [
      { type: "message_start", message: { usage: { input_tokens: 3000, cache_read_input_tokens: 1000, output_tokens: 1 } } },
      { type: "content_block_delta", delta: { type: "text_delta", text: "Hail" } },
      { type: "message_delta", usage: { output_tokens: 40 } },
      { type: "message_delta", usage: { output_tokens: 120 } },
      { type: "message_stop" },
    ];
    const sse = events.map((e) => `event: ${e.type}\ndata: ${JSON.stringify(e)}\n\n`).join("");
    // Split mid-line to exercise buffering.
    const bytes = new TextEncoder().encode(sse);
    const reply = () =>
      new Response(
        new ReadableStream({
          start(controller) {
            for (let i = 0; i < bytes.length; i += 37) controller.enqueue(bytes.slice(i, i + 37));
            controller.close();
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      );
    const { guard, meter } = setup({ reply });
    const response = await call(guard, { model: "claude-haiku-5-5", max_tokens: 64, stream: true, messages: [] });
    expect(await response.text()).toBe(sse);
    expect(meter.snapshot()).toMatchObject({ calls: 1, input_tokens: 3000, cache_read_input_tokens: 1000, output_tokens: 120 });
    expect(meter.snapshot().usd).toBeCloseTo((3000 * 0.1 + 1000 * 0.01 + 120 * 0.5) / 1e6, 12);
  });

  test("healthz, usage, and nothing else", async () => {
    const { guard } = setup();
    expect(await (await guard(new Request("http://guard/healthz"))).json()).toEqual({ ok: true, month: "2026-11", over_budget: false });
    const usage = (await (await guard(new Request("http://guard/usage"))).json()) as Record<string, unknown>;
    expect(usage).toMatchObject({ month: "2026-11", usd: 0, calls: 0, cap_usd: 100, resets_at: "2026-12-01T00:00:00.000Z" });
    expect((await guard(new Request("http://guard/v1/models"))).status).toBe(404);
    expect((await guard(new Request("http://guard/v1/messages"))).status).toBe(405);
  });
});

test("stream usage ignores comments and partial lines", () => {
  const parser = new StreamUsage();
  parser.push(new TextEncoder().encode(': ping\n\ndata: {"type":"message_start","message":{"usage":{"input_tokens":5'));
  parser.push(new TextEncoder().encode('}}}\n\ndata: {"type":"message_delta","usage":{"output_tokens":7}}'));
  expect(parser.result()).toEqual({ input_tokens: 5, output_tokens: 7 });
});
