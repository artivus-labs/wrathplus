// The proxy: POST /v1/messages to the API with the real key, metered; GET /healthz and /usage.

import { nextMonthStart, type Meter } from "./meter.ts";
import { PRICES, type Price, type Usage } from "./pricing.ts";
import { meteredStream, usageFromJson } from "./usage.ts";

export interface GuardOptions {
  meter: Meter;
  apiKey: string;
  /** Models callers may use; each needs a price in PRICES. */
  models: string[];
  upstream?: string;
  fetch?: (input: string, init: RequestInit) => Promise<Response>;
  now?: () => Date;
  log?: (line: string) => void;
  /** Upper bound for one upstream call. */
  timeoutMs?: number;
}

// Request headers forwarded to the API; everything else (the caller's key included) is dropped.
const FORWARD = ["content-type", "accept", "anthropic-version", "anthropic-beta"];
// Response headers passed back; the body is re-framed, so length/encoding are not.
const PASS_BACK = ["content-type", "request-id", "retry-after", "x-should-retry"];

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

/** An error in the Messages API's own shape, so SDK clients surface the message. */
function apiError(status: number, type: string, message: string, headers: Record<string, string> = {}) {
  return json(status, { type: "error", error: { type, message } }, headers);
}

export function createGuard(options: GuardOptions): (request: Request) => Promise<Response> {
  const { meter, apiKey } = options;
  const upstream = (options.upstream ?? "https://api.anthropic.com").replace(/\/+$/, "");
  const doFetch = options.fetch ?? ((input, init) => fetch(input, init));
  const now = options.now ?? (() => new Date());
  const log = options.log ?? ((line: string) => console.log(line));
  const timeoutMs = options.timeoutMs ?? 600_000;
  if (!apiKey) throw new Error("no API key");
  const prices = new Map<string, Price>();
  for (const model of options.models) {
    const price = PRICES[model];
    if (!price) throw new Error(`no price known for model ${model}; it cannot be allowed`);
    prices.set(model, price);
  }

  function record(model: string, price: Price, usage: Usage | null) {
    if (!usage) {
      log(`llm-guard: ${model} response carried no usage; not metered`);
      return;
    }
    meter.record(usage, price);
  }

  async function messages(request: Request): Promise<Response> {
    if (!meter.allow()) {
      meter.refuse();
      const totals = meter.snapshot();
      const resets = nextMonthStart(now());
      const wait = Math.max(1, Math.ceil((resets.getTime() - now().getTime()) / 1000));
      return apiError(
        429,
        "rate_limit_error",
        `llm-guard: the monthly LLM budget of $${meter.capUsd} for ${totals.month} is spent ` +
          `($${totals.usd.toFixed(2)}); calls resume at ${resets.toISOString()}`,
        // The Anthropic SDKs honour x-should-retry: no pointless retries until next month.
        { "retry-after": String(wait), "x-should-retry": "false" },
      );
    }
    const text = await request.text();
    let model: unknown;
    try {
      model = (JSON.parse(text) as { model?: unknown }).model;
    } catch {
      return apiError(400, "invalid_request_error", "llm-guard: request body is not JSON");
    }
    const price = typeof model === "string" ? prices.get(model) : undefined;
    if (!price || typeof model !== "string") {
      return apiError(
        400,
        "invalid_request_error",
        `llm-guard: model ${JSON.stringify(model)} is not allowed; allowed: ${[...prices.keys()].join(", ")}`,
      );
    }

    const headers = new Headers();
    for (const name of FORWARD) {
      const value = request.headers.get(name);
      if (value) headers.set(name, value);
    }
    headers.set("x-api-key", apiKey);
    let response: Response;
    try {
      response = await doFetch(`${upstream}/v1/messages`, {
        method: "POST",
        headers,
        body: text,
        signal: AbortSignal.timeout(timeoutMs),
      });
    } catch (error) {
      meter.error();
      log(`llm-guard: upstream unreachable: ${error instanceof Error ? error.message : String(error)}`);
      return apiError(502, "api_error", "llm-guard: the Anthropic API could not be reached");
    }

    const back = new Headers();
    for (const name of PASS_BACK) {
      const value = response.headers.get(name);
      if (value) back.set(name, value);
    }
    if (!response.ok) {
      meter.error();
      return new Response(await response.text(), { status: response.status, headers: back });
    }
    const contentType = response.headers.get("content-type") ?? "";
    if (contentType.includes("text/event-stream") && response.body) {
      const body = meteredStream(response.body, (usage) => record(model, price, usage));
      return new Response(body, { status: response.status, headers: back });
    }
    const body = await response.text();
    let parsed: unknown = null;
    try {
      parsed = JSON.parse(body);
    } catch {
      // Passed back as is; nothing to meter.
    }
    record(model, price, usageFromJson(parsed));
    return new Response(body, { status: response.status, headers: back });
  }

  return async (request: Request) => {
    const { pathname } = new URL(request.url);
    if (request.method === "GET" && pathname === "/healthz") {
      const usage = meter.snapshot();
      return json(200, { ok: true, month: usage.month, over_budget: usage.over_budget });
    }
    if (request.method === "GET" && pathname === "/usage") return json(200, meter.snapshot());
    if (pathname === "/v1/messages") {
      if (request.method !== "POST") return apiError(405, "invalid_request_error", "llm-guard: POST only");
      return messages(request);
    }
    return apiError(404, "not_found_error", `llm-guard: ${request.method} ${pathname} is not proxied`);
  };
}
