# @wrathplus/agent-mcp

A small [MCP](https://modelcontextprotocol.io) server (stdio) that lets an LLM agent play WrathPlus
through the client's agent control port (`crates/wrathplus-agent`). It adds no game logic: each
tool is one HTTP call to the port.

| Tool | Port route | Returns |
|---|---|---|
| `lua(code, budget?)` | `POST /lua[?budget=N]` | `{ok, values, output}` or `{ok:false, error, output}` (a tool error) |
| `screenshot(scale?)` | `GET /screenshot?scale=S` | PNG image content, scale 0.5 by default |
| `events(since?, wait_ms?, limit?)` | `GET /events` | `{events, next, missed}`; pass `next` as the next `since` |
| `status()` | `GET /status` | `{state, in_world, ui_ready, time, player}` |

Lua runs in the UI's VM against the stock WoW 3.3.5a API. With the port open, the movement
functions (`MoveForwardStart`, `TurnLeftStart`, ...) work from any Lua. HTTP failures (401, 504,
...) and unreachable ports come back as MCP tool errors carrying the port's error text.

## Environment

| Variable | Meaning |
|---|---|
| `WRATHPLUS_AGENT_URL` | the port's base URL, e.g. `http://127.0.0.1:17950` |
| `WRATHPLUS_AGENT_TOKEN` | the bearer token: the same value as the client's `WOW_AGENT_TOKEN` |

The server exits at startup if either is missing. It never prints the token.

## Start the client with the port open

The port listens on 127.0.0.1 only, and only when the launch asks for it. The token (16+
characters) comes from the environment, never the command line:

```sh
export WOW_AGENT_TOKEN="$(openssl rand -hex 24)"
wrathplus --agent-port 17950            # or WOW_AGENT_PORT=17950
```

`--agent-port 0` picks a free port; the client logs `agent port: listening on http://...`.

## Register with Claude Code

```sh
cd agents/mcp && bun install
claude mcp add wrathplus \
  -e WRATHPLUS_AGENT_URL=http://127.0.0.1:17950 \
  -e WRATHPLUS_AGENT_TOKEN="$WOW_AGENT_TOKEN" \
  -- bun run /path/to/wrathplus/agents/mcp/src/index.ts
```

## Development

```sh
bun install --frozen-lockfile
bun run typecheck   # tsc --noEmit
bun test            # against a fake control port (Bun.serve on port 0)
```
