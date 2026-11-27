# OMP example agent

A WrathPlus agent that plays purely through the client's MCP server ([`../mcp`](../mcp/README.md))
inside **[OMP](https://github.com/can1357/oh-my-pi)** (oh-my-pi, MIT), a coding-agent harness,
with **Claude Sonnet 5.5**. It treats the game like a codebase: the LLM drives, and it builds its
own real-time controller as Lua modules in the client (`lua_search`, `lua_doc`, `save_module`,
`propose_module`), writes its own skills and notes, and a scheduled reflection reviews the
scorecard and the judge's ledger and proposes module changes. Deterministic code executes; no
tactics are hard-coded; many goals in tension are measured (the scorecard) rather than one reward.

[`AGENTS.md`](AGENTS.md) is the agent's system prompt: what WrathPlus is, the three speeds, the
tools and Lua APIs, the judge, the goals, how to behave on a public realm, and cost discipline.

## How it differs from `agents/example`

| | [`agents/example`](../example/README.md) | this one |
|---|---|---|
| Harness | Claude Agent SDK, our own planner loop | OMP, a general coding agent, unmodified |
| Planner | Claude Haiku 5.5 | Claude Sonnet 5.5 |
| Fast tier | Jev (optional) picks intents between planner calls | none: the agent writes the real-time logic itself as Lua modules, and `Agent.ask` hands messy moments up |
| Tools | the MCP tools plus harness tools (plans, goals, holds, notes, wake-ups) | the MCP tools only, plus OMP's file tools for its own notes and skills |
| Improving itself | reflection digests from the harness's own events | `propose_module` and the judge: changes are trialled against the scorecard before they are kept |

## What it needs

- A WrathPlus client with the agent control port open ([BETA.md](../../docs/BETA.md#agents)
  shows how, for the open beta).
- **An Anthropic API key** as `ANTHROPIC_API_KEY` (the documented default). Locally you can
  instead sign OMP in with a Claude Pro/Max plan: run `node_modules/.bin/omp` once in this folder
  with the same `PI_CODING_AGENT_DIR` the launcher uses (`<state>/omp-agent`), type
  `/login anthropic`, and leave `ANTHROPIC_API_KEY` unset.
- [Bun](https://bun.sh). OMP itself needs Bun 1.4.4 or later; `bun install` here brings a pinned
  Bun (1.4.5) and OMP (`@oh-my-pi/pi-coding-agent` 19.1.0) into `node_modules`, and `run.sh` uses
  them, so the Bun on your PATH may be older.

```sh
cd agents/mcp && bun install && cd ../omp-example && bun install
export WRATHPLUS_AGENT_URL=http://127.0.0.1:17950
export WRATHPLUS_AGENT_TOKEN=$(cat ~/.config/wrathplus/agent-token)
export ANTHROPIC_API_KEY=...           # from a 0600 file, never typed into a shared shell
./run.sh
```

The client may be at character select: the agent picks or creates its character, enters the
world and skips the intro itself. Stop it with Ctrl-C (it closes OMP, which ends the MCP server).

## How it runs

`run.sh` starts `src/player.ts`, the launcher, which runs **one long-lived OMP process in RPC
mode** (`omp --mode rpc --no-extensions --no-title`) and feeds it prompts.

**Why one long-lived process, not a loop of `omp -p` turns:** the MCP server is not only a set of
tools. Its scorecard sampler and the judge run inside the server process, sampling every 30 s and
swapping a module under trial between windows of 12 minutes. OMP starts a stdio MCP server per
session and stops it when the session ends, so a fresh `omp -p` per turn would restart the judge
every turn (abandoning any trial), and a second OMP process beside the player would run a second
judge on the same folder. So the player's process stays up, and reflection runs inside it as a
**subagent** (OMP's `task` tool with the `reflector` agent in `.omp/agents/`): a fresh context that
borrows the parent's MCP connection, so the same judge sees its proposals.

The launcher:

- **Wakes the planner on the game's events.** It long-polls the client's event stream for what a
  player would notice: `AGENT_ASK` (the Lua handing a moment up), a death, a whisper, party chat,
  an invite, a trade or duel, a level-up, a blocked or finished goal, a hold that ran out, and
  `/say` that names the character. Asks, deaths, whispers and invites are urgent: they steer a
  turn that is running. A notable event that repeats one that woke the planner within
  `WRATHPLUS_OMP_REPEAT_MINUTES` (the same goal blocked for the same reason, the same hold run out)
  does not wake it again: the repeats are counted and shown in the next check-in's evidence.
  Urgent events always wake it. Combat, loot, walks and failures never wake it: the Lua plays, and the
  failures are counted for the check-ins.
- **Batches what the character passes.** `AGENT_NOTICED` (wp.notice: a quest giver, trainer, vendor,
  node, subzone ... new to the character) is collected for `WRATHPLUS_OMP_NOTICE_SECONDS` into one
  wake, nearest first; it rides along sooner with anything else that wakes the planner. Each
  check-in's evidence adds what was passed since the last, muted or not (the client's
  `Notice.passed`), counted by kind with names.
- **Checks in** every `WRATHPLUS_OMP_CHECKIN_MINUTES`, whatever else woke it meanwhile: the turn for improving the controller. The check-in carries **evidence** the launcher counted since the last one (game errors from `UI_ERROR_MESSAGE`, failed goal actions and their reasons, refused pulls, how fights ended, the latest fight review), and asks the agent to take the most costly recurring failure or the weakest scorecard objective, study its cause in the Lua, and change the controller or write down why not (the `improve-the-controller` skill). The launcher only counts; it never decides what a failure means.
- **Reflects** every `WRATHPLUS_OMP_REFLECT_HOURS`: it asks the planner to dispatch the
  `reflector` agent (the `reflect` skill: scorecard, needs, ledger, at most two proposals).
- **Starts a fresh session** every `WRATHPLUS_OMP_SESSION_HOURS`, but never while the judge has a
  trial running (it reads the ledger on disk), so the context stays small and finished sessions
  become memory.
- **Restarts OMP** if it exits, continuing the same session (`--continue`), and aborts a turn
  that runs longer than `WRATHPLUS_OMP_TURN_MAX_MINUTES` (a stuck loop).
- **Logs usage**: one line per model call in `<state>/usage.jsonl` (`kind` `assistant` or
  `subagent`: model, input, output, cache read and write tokens, cost in USD as OMP prices it),
  and after each turn OMP's own running totals for the session (`kind: "session_stats"`). What the
  planner said and which tools it called go to `<state>/launcher.log`.

### Where things live

Everything of one agent's is under its state folder, `WRATHPLUS_OMP_STATE`:

| Path | |
|---|---|
| `work/` | OMP's working directory. `AGENTS.md`, `.omp/config.yml` and `.omp/mcp.json` are copied from this example at every start; the seed skills and the reflector only when missing, since the agent edits and adds to them. `notes/` is the agent's. |
| `omp-agent/` | OMP's agent directory (`PI_CODING_AGENT_DIR`): sessions, memory, model catalog. Nothing from your own `~/.omp` is read. |
| `omp-root/` | OMP's config root (`PI_CONFIG_DIR`, when the state folder is under your home): its logs. |
| `usage.jsonl`, `launcher.log` | spend and what happened. |

OMP runs outside any checkout on purpose: its context discovery walks up to the repository root,
and this repository's own `AGENTS.md` and `CLAUDE.md` are instructions for people working on the
code, not for a player. `.omp/config.yml` also turns off other tools' context and config sources
(Claude Code, Codex, Cursor...), so a `~/.codex/AGENTS.md` or `~/.claude/CLAUDE.md` of yours is
never read.

### What the agent can do on this machine

Its tools are the WrathPlus MCP tools, OMP's file tools (`read`, `write`, `edit`, `find`, `grep`)
for its notes and skills, `task` for the reflector, `todo`, `web_search` and `wait`. `bash`,
code evaluation (`eval`), the browser and desktop control are off and denied in `.omp/config.yml`;
`../secrets` beside the working directory is a restricted folder, which a headless session cannot
open.

## Memory

OMP's `local` memory backend is on: when a session has been quiet for an hour, the next start
summarises it into `MEMORY.md` and a short summary that is put in front of the next session, kept
in `omp-agent/` (not the repository). There are no embeddings and no memory service. **What it
sends to the provider:** the extraction pass sends each finished session's transcript (cut to
about 4,000 tokens) to the `default` model (Sonnet 5.5); the consolidation pass sends the
extractions to the `smol` model (Haiku 5.5). These calls go to Anthropic like the planner's own and
are not in `usage.jsonl`. Lessons are not captured automatically (`autolearn` is off): the agent
writes skills and notes deliberately.

## Privacy and telemetry

The launcher sets `PI_AUTO_QA=0` (OMP's automatic tool-issue reports, which can be pushed to
`qa.omp.sh` after the user consents, are off), `OTEL_SDK_DISABLED=true` and
`telemetry.otlpExportEnabled: false` (OpenTelemetry export only happens when `OTEL_*` endpoints are
set; these make sure it does not). OMP has no analytics SDK. It does fetch its model catalog
(provider model lists) when its cache is empty or stale, and probes local model servers (Ollama,
LM Studio, llama.cpp) on `127.0.0.1`.

## Cost

Sonnet 5.5 is priced (in OMP's catalog) at $2 per million input tokens, $10 per million output,
$0.10 per million cached reads. OMP's own system prompt plus `AGENTS.md` and the tool schemas come
to about 22,000 tokens before any play, so every model call costs at least that much in (mostly
cached) input. The settings that keep it down:

- the Lua plays; the planner is woken by events, not polling, and ends its turns;
- `providers.cacheRetention: long` (one-hour prompt cache on API keys) and `cacheWarming: off`,
  since turns are minutes apart;
- the session is replaced every few hours, and OMP compacts a long one;
- optional `WRATHPLUS_OMP_MAX_COST` (USD per session) is passed to OMP as `--max-cost`.

Read `usage.jsonl` for the real numbers.

## Settings

| Env | Default | |
|---|---|---|
| `WRATHPLUS_AGENT_URL`, `WRATHPLUS_AGENT_TOKEN` | required | the client's control port and token |
| `ANTHROPIC_API_KEY` | none | the planner's key (or an OMP login, above) |
| `WRATHPLUS_OMP_STATE` | `~/.local/share/wrathplus-omp-agent` | this agent's folder |
| `WRATHPLUS_OMP_MODEL` | `anthropic/claude-sonnet-5-5` | OMP's model selector (`omp models find sonnet`) |
| `WRATHPLUS_OMP_THINKING` | `medium` | `off`, `low`, `medium`, `high`, ... |
| `WRATHPLUS_OMP_CHECKIN_MINUTES` | `20` | between check-ins (`0`: never) |
| `WRATHPLUS_OMP_REFLECT_HOURS` | `3` | between reflections (`0`: off) |
| `WRATHPLUS_OMP_SESSION_HOURS` | `6` | before a fresh session (`0`: never) |
| `WRATHPLUS_OMP_TURN_MAX_MINUTES` | `30` | a longer turn is aborted (`0`: never) |
| `WRATHPLUS_OMP_REPEAT_MINUTES` | `15` | a notable event repeating one that woke the planner this recently (the same goal blocked for the same reason, the same hold run out) does not wake it again; repeats are counted in the next check-in (`0`: every one wakes it) |
| `WRATHPLUS_OMP_NOTICE_SECONDS` | `15` | `AGENT_NOTICED` events are collected this long into one wake (`0`: each wakes at once) |
| `WRATHPLUS_OMP_MAX_COST` | none | USD per session, passed as `--max-cost` |
| `WRATHPLUS_OMP_FRESH` | off | `1`: start a new session instead of continuing the last one |
| `WRATHPLUS_JUDGE_DIR`, `WRATHPLUS_SCORECARD_SECONDS` | the MCP server's | passed through to it |

## Development

```sh
bun install --frozen-lockfile
bun run typecheck
bun test            # wake rules and evidence, usage logging, the working dir, the RPC decoder
```
