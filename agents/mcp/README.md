# @wrathplus/agent-mcp

A small [MCP](https://modelcontextprotocol.io) server (stdio) that lets an LLM agent play WrathPlus
through the client's agent control port (`crates/wrathplus-agent`). It adds no game logic: each
tool is one HTTP call to the port.

| Tool | Port route | Returns |
|---|---|---|
| `lua(code, budget?)` | `POST /lua[?budget=N]` | `{ok, values, output}` or `{ok:false, error, output}` (a tool error) |
| `screenshot(scale?)` | `GET /screenshot?scale=S` | PNG image content, scale 0.5 by default |
| `events(since?, names?, exclude?, wait_ms?, limit?)` | `GET /events` | `{events, next, missed}`; pass `next` as the next `since` |
| `status()` | `GET /status` | `{state, in_world, ui_ready, cinematic, time, player, nav}` (`nav`: where walking routes come from) |
| `modules(name?)` | `GET /files[/<name>]` | the saved module list, or one module's source |
| `save_module(name, source)` | `PUT /files/<name>` | `{saved, bytes}`; `require(name)` or `AgentReload(name)` in Lua loads it |

Lua runs in the UI's VM against the stock WoW 3.3.5a API. With the port open, the movement
functions (`MoveForwardStart`, `TurnLeftStart`, ...) work from any Lua. HTTP failures (401, 504,
...) and unreachable ports come back as MCP tool errors carrying the port's error text.

`events` is built for a language model's context. Without `since` a read starts from now (it
returns only the cursor, or what arrives during `wait_ms`); `since: 0` asks for the whole retained
buffer, which is mostly other units' combat log and `UNIT_*` noise. `names` and `exclude` take
event names, or prefixes when they end in `_` or `*` (`["AGENT_", "CHAT_MSG_", "QUEST_"]`); with
them only a matching event ends a wait. `limit` defaults to 100. `wait_ms` is cut to 25000,
because hosts time a tool call out at about 30 s.

`status` reports `player: null` for about a second after entering the world, until the
character's record has arrived (wait for `in_world` with a `player`), and `cinematic: true` while
a cinematic holds the client: a new character starts in its intro, during which nothing is near
and walking is refused. `StopCinematic()` in Lua skips it; the tools never skip one themselves.

The `lua` tool's description carries a short reference of the agent helpers (`AgentNearby`,
`AgentMoveTo`, `Combat.engage`, `Quests`, `Move`, `Goals`...) and which of them finish in the
background as `AGENT_*` events.

Modules are the agent's own Lua, kept in the client's `Agent/scripts` folder across reloads and
restarts; a module named `autoexec` runs whenever the client enters the world. Two are built in:
`wp.engine`, a Playerbots-style decision engine, and `wp.values`, client-visible world state.

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
