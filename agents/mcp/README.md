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
| `lua_search(query, kinds?, limit?)` | `GET /lua/search?q=…&kind=…&limit=…` | what exists to call: built-in helpers (signature, doc, file:line), your modules' matching lines, the API's names |
| `lua_doc(name)` | `GET /lua/doc?name=…` | one function's signature, doc and source, or a module's source |
| `modules(name?)` | `GET /files[/<name>]` | the saved module list, or one module's source |
| `save_module(name, source)` | `PUT /files/<name>` | `{saved, bytes}`; `require(name)` or `AgentReload(name)` in Lua loads it |
| `scorecard(sample?)` | (the judge's samples) | current values, needs (gaps) and trends; see [Scorecard and judge](#scorecard-and-judge) |
| `propose_module(name, source, claim)` | (the judge) | `{trial, version, ahead}`: a candidate, trialled before it is kept |
| `ledger(module?, limit?)` | (the judge) | the running trial, the queue, and every trial with its windows and verdict |

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

`Agent.ask(question, context, on[, {timeout, default}])` in Lua hands a messy moment (someone
talking, an odd mechanic, a hard choice) to the planner: it signals `AGENT_ASK(id, question,
context, digest)`, the digest being recent events as lines, and the planner answers with
`Agent.answer(id, value)`, which calls `on(value, "answered")`; a timeout calls `on(default,
"timeout")` and signals `AGENT_ASK_EXPIRED(id)`. A planner waits on it with
`events {names: ["AGENT_ASK"]}`; `Agent.open_asks()` lists those not yet answered.

Modules are the agent's own Lua, kept in the client's `Agent/scripts` folder across reloads and
restarts; a module named `autoexec` runs whenever the client enters the world. Two are built in:
`wp.engine`, a Playerbots-style decision engine, and `wp.values`, client-visible world state.

## Scorecard and judge

The server scores the character on many goals at once, so no single number can be gamed (endless
safe grinding, never dying): every 30 s in the world it reads, through the stock API only, level
and experience, money, the quest log, equipped item levels (`GetItemInfo` on each slot's link),
profession skill lines, the spellbook's size, zone and subzone, and whether the character is idle
(alive, out of combat, still, not casting, health and mana full). Deaths come from `PLAYER_DEAD`
events. Each sample is a line in `<judge dir>/characters/<realm>-<name>/samples.jsonl`. A quest
that leaves the log complete counts as handed in. If the agent's Lua replaces one of the stock
functions read, the sample says so (`tampered`) and the stretch it is in is not trusted.

**Needs** (`src/needs.ts`, one table) are targets that rise with level: each known profession
within 25 of five skill a level, the equipped average item level within 5 of the level, and a gold
reserve of 25 x level² copper. Each is a gap (0 = met) and a share of its target; the shares sum
to one needs number whose change per hour is an objective.

**The judge** (`src/judge.ts`) decides which module changes are kept. `propose_module` keeps the
candidate as a version in the judge's folder and queues a trial. The champion is the module as
the judge last kept it, or as the client holds it at the first trial (no file: the champion is the
module's absence). With the character in the world the judge runs the two in alternating windows,
pairs in ABBA order so drift cancels, loading one or the other between windows (`stop()` on a
loaded module that has one, then the file and `AgentReload`). The first seconds of a window
settle and are not scored. Each window is scored on these objectives:

| Objective | Better | Tolerance (larger of) |
|---|---|---|
| `xp_per_hour` | higher | 10% of the champion |
| `gold_per_hour` | higher | 1000 copper/h, 15% |
| `quests_per_hour` | higher | 1/h, 15% |
| `exploration_per_hour` (new zones and subzones) | higher | 2/h, 20% |
| `skill_per_hour` (profession skill-ups) | higher | 2/h, 15% |
| `deaths_per_hour` | lower | 1/h, and 2 deaths apart |
| `idle_share` | lower | 0.05 |
| `needs_gap_per_hour` | lower | 0.05/h |

The verdict is Pareto-style: the candidate is promoted if at least one objective improves and none
regresses, each beyond its tolerance and with the mean pair difference at least one standard error
from zero (`min_t`); otherwise the champion is put back. With fewer valid pairs than
`min_valid_pairs` the trial is inconclusive and the champion stays. The ledger keeps, per
objective, both means, the mean difference, its share of the champion, the tolerance, `n`, `sd`,
`t` and `d` (Cohen's d of the pair differences).

A window is void when the client restarts, the UI reloads, the samples stop for long, another
character plays or a stock function is replaced: what can't be measured. A void pair is replaced
by a spare (`extra_pairs`). A death does not void its window: its deaths count in deaths/h and the
time it cost (dying, the corpse run) in every rate. (`void_on_death: true` in `judge.json` voids
such windows for the rates, still counting their deaths.) A module saved by hand during a
trial stops it, leaving the module as saved. If this server restarts mid-trial the trial is
abandoned and the champion put back; queued proposals resume.

Defaults (window 12 min, settle 30 s, 3 pairs, 2 spare, 2 needed, `min_t` 1, tolerances above)
are overridden by `<judge dir>/judge.json`, read at each trial's start and recorded with it:

```json
{ "window_minutes": 15, "pairs": 4, "objectives": { "xp_per_hour": { "rel": 0.05 } } }
```

The judge's folder is out of the agent's reach: no tool writes it but `propose_module`, which only
appends, Lua has no file access, and the server refuses a folder inside the modules folder. Every
proposal, window and verdict is a line in `<judge dir>/ledger.jsonl`, read with `ledger`.

## Environment

| Variable | Meaning |
|---|---|
| `WRATHPLUS_AGENT_URL` | the port's base URL, e.g. `http://127.0.0.1:17950` |
| `WRATHPLUS_AGENT_TOKEN` | the bearer token: the same value as the client's `WOW_AGENT_TOKEN` |
| `WRATHPLUS_AGENT_TOKEN_FILE` | instead of `WRATHPLUS_AGENT_TOKEN`: a file holding the token, for hosts that put a server's environment on a command line (the Claude Agent SDK does), where `ps` shows it to every local user |
| `WRATHPLUS_SCORECARD_SECONDS` | how often the scorecard samples (default 30; `0` turns the scorecard and judge off) |
| `WRATHPLUS_JUDGE_DIR` | the judge's folder (default `judge` beside the client's modules folder: `<client home>/Agent/judge`) |

The server exits at startup if the URL or the token is missing. It never prints the token.

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
