// WrathPlus LLM guard: configuration from the environment, then serve.

import { join } from "node:path";
import { createGuard } from "./guard.ts";
import { Meter } from "./meter.ts";

process.umask(0o077);

function fail(message: string): never {
  console.error(`llm-guard: ${message}`);
  process.exit(2);
}

const apiKey = (process.env.ANTHROPIC_API_KEY ?? "").trim();
if (!apiKey) fail("ANTHROPIC_API_KEY is not set (beta: /etc/wrathplus-beta/llm-chatter.env)");
const cap = Number(process.env.LLM_GUARD_MONTHLY_USD ?? "100");
if (!(cap > 0)) fail("LLM_GUARD_MONTHLY_USD must be a positive number");
const stateDir = process.env.LLM_GUARD_STATE_DIR ?? "/var/lib/llm-guard";
const models = (process.env.LLM_GUARD_MODELS ?? "claude-haiku-5-5")
  .split(",")
  .map((model) => model.trim())
  .filter(Boolean);
const port = Number(process.env.LLM_GUARD_PORT ?? "8788");
const hostname = process.env.LLM_GUARD_HOST ?? "127.0.0.1";

let handler: (request: Request) => Promise<Response>;
let meter: Meter;
try {
  meter = new Meter({ path: join(stateDir, "usage.json"), capUsd: cap });
  handler = createGuard({ meter, apiKey, models, upstream: process.env.LLM_GUARD_UPSTREAM });
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}

const server = Bun.serve({
  hostname,
  port,
  // A non-streaming completion can take longer than Bun's 10 s default idle timeout.
  idleTimeout: 255,
  fetch: (request) => handler(request),
});
const usage = meter.snapshot();
console.log(
  `llm-guard: listening on ${server.hostname}:${server.port}; models ${models.join(", ")}; ` +
    `${usage.month} at $${usage.usd.toFixed(4)} of $${cap} (${usage.calls} calls)`,
);
