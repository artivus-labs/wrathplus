# llm-guard

A small metering proxy between the beta realm's LLM chatter bridge (mod-llm-chatter) and
`api.anthropic.com`. It holds the API key, prices each response, and refuses calls once the
calendar month's budget is spent. No runtime dependencies: Bun's server and `fetch`.

- `POST /v1/messages` — checks the budget and the model, forwards to the API with `x-api-key`
  from `ANTHROPIC_API_KEY` (the caller's key is dropped), reads `usage` from the response (JSON or
  a streamed response: `message_start` plus the last `message_delta`) and adds its cost to the
  month. API errors pass through and cost nothing.
- Over budget: `429` with an Anthropic-shaped `rate_limit_error` naming the budget and the reset
  time, `retry-after` until the 1st (UTC), and `x-should-retry: false` so SDK clients do not retry.
- A model without a known price (`src/pricing.ts`) is refused with a `400` before any call.
- `GET /healthz` — `{ ok, month, over_budget }`. `GET /usage` — the month so far: `usd`,
  `cap_usd`, `percent`, `calls`, `refused`, `errors`, token counts, `resets_at`, earlier months.
- The month's totals live in `$LLM_GUARD_STATE_DIR/usage.json` (0600, written atomically after
  each call), so a restart keeps them. A new UTC month starts at zero.
- Logs one line when the month reaches 50, 80 and 100 % of the budget.

Pricing for `claude-haiku-5-5`, USD per million tokens: input $0.10, output $0.50 for a prompt of
at most 100K tokens (uncached input + cache writes + cache reads); beyond that the whole request
at $0.50 / $2.50. Cache writes 1.25× the input rate (1-hour writes 2×, when the usage breakdown
says so), cache reads 0.1×. Thinking tokens are part of `output_tokens`.

Calls already in flight when the budget is reached still complete and are counted, so a month can
end slightly over the cap.

## Configuration (environment)

| Variable | Default | |
| --- | --- | --- |
| `ANTHROPIC_API_KEY` | — | required; the service exits without it |
| `LLM_GUARD_MONTHLY_USD` | `100` | the budget per calendar month (UTC) |
| `LLM_GUARD_MODELS` | `claude-haiku-5-5` | comma-separated; each needs a price |
| `LLM_GUARD_STATE_DIR` | `/var/lib/llm-guard` | where `usage.json` lives |
| `LLM_GUARD_HOST` / `LLM_GUARD_PORT` | `127.0.0.1` / `8788` | listen address |
| `LLM_GUARD_UPSTREAM` | `https://api.anthropic.com` | |

On the beta realm it runs as the `llm-guard` Compose service (`packaging/deploy/beta`), with the
key from `/etc/wrathplus-beta/llm-chatter.env` and `/healthz` and `/usage` published on the box's
loopback only. See that README's "LLM chatter" section.

```sh
bun install
bun test
bun run typecheck
```
