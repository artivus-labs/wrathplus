# Example agent

A WrathPlus agent in three tiers: **Claude plans, Jev decides, Lua plays.** It runs on the
[Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk), so it uses this machine's Claude
Code login (a Claude subscription works; no API key), with **Claude Haiku 5.5** by default.

- **The planner** has exactly the six tools of the client's MCP server (`../mcp`): `lua`,
  `screenshot`, `events`, `status`, `modules`, `save_module`. It has no files, shell or web.
  Its system prompt (`src/prompt.ts`) steers it to put anything routine or fast into its own Lua
  modules built on `wp.engine` and `wp.values`, which run every frame in the client, and to keep
  its own calls for decisions and fixes.
- **Event-driven tiers**, to keep cost and context down, since the agent's own Lua carries the
  work between choices:
  - **The event watcher** (`src/watch.ts`) long-polls the client and hands every event to the fast
    tier. The planner only hears what needs it: death, whispers, party chat, invites, trades and
    duels, plus operator notes (each stdin line).
  - **The fast tier** (`src/policy.ts`, on when `TYPESAFE_API_KEY` is set) asks Jev, TypeSafe's
    System One decision model, only when woken: an activity ended (`AGENT_INTENT_DONE`), the
    planner sent a plan, a decision opened, combat started or ended, death, loot, gossip or quest
    windows, a walk ended, a level-up or an invite. While an activity runs it sleeps; only an
    interrupting event (combat, death, a new plan) makes it reconsider. A 20 s heartbeat
    (`WRATHPLUS_JEV_HEARTBEAT_MS`) is the safety net. Each call reads a compact state (one Lua
    call), chooses among the registered intents and `wait`, answers open decisions, and judges
    whether the plan has stalled or is done. `src/jev.ts` is a small client for
    `POST /v1/systemone`.
  - **The planner** (adaptive thinking, high effort) sets the plan with `set_plan`: each intent is
    an activity with the Lua that starts it and optionally `until` (Lua true once finished) and a
    `timeout`, registered in the client as `Agent.intent`s alongside any its own modules register.
    It is woken by the fast tier when the plan stalls, looks complete, or an intent fails three
    times running, and reads `fast_loop_status`. An idle check-in is off by default
    (`WRATHPLUS_AGENT_CHECKIN_SECONDS`).

## Run it

Each agent is one of the Manager's agents: its own realm account, headless client and control
port, shared with the Manager's **Agents** page. The managed realm must be running.

```sh
cd agents/mcp && bun install && cd ../example && bun install
cp ../.env.example ../.env          # then fill in TYPESAFE_API_KEY (for Jev)

bun run agent add Scout             # a realm account (AGENTSCOUT), control port and token
bun run agent run Scout --character Scoutly "Reach level 5 in Northshire."
bun run agent list                  # every agent, its client and whether a planner drives it
bun run agent dashboard             # http://127.0.0.1:17900: every running agent on one page
```

- **`run`** starts the agent's headless client (logged in, `--character` entered; without it the
  planner picks or creates one at character select), waits for it, runs the planner and Jev, and
  stops the client when you stop the agent (Ctrl-C). A client already started from the Manager is
  driven as it is. Lines typed into the terminal reach the planner as operator notes. A client `run`
  started is restarted into the same character if it exits or is disconnected (back at the login
  screen), so the planner carries on.
- **Several at once:** one `run` per terminal; each agent has its own account and port.
- **Watching:** each client serves a live view (`run` prints its address); the dashboard shows
  all of them side by side, with what each planner last said and did.
- **The planner model:** `--model haiku` (Claude Haiku 5.5, the default) or `--model sonnet` or
  any model id. It runs on this machine's Claude Code login (a Claude subscription) and ignores an
  `ANTHROPIC_API_KEY` in the shell; put `WRATHPLUS_ANTHROPIC_API_KEY` in `agents/.env` to bill an
  API key instead. It is isolated from your own Claude Code setup (no CLAUDE.md, settings or MCP
  servers) and works in its own notes folder.
- **State** lives with the Manager's (`<checkout>/wrathplus-config/agents/<ACCOUNT>/`): the
  secrets (0600), the client's home and log, and the agent's notes (memory, to-dos, sessions).

| Env (shell or `agents/.env`) | Default | |
|---|---|---|
| `TYPESAFE_API_KEY` | none | turns on the fast tier (Jev) |
| `WRATHPLUS_ANTHROPIC_API_KEY` | none | bill this key instead of the Claude Code login |
| `WRATHPLUS_CLIENT` | the Manager's build | the client binary |
| `WOW_NAV_MMAPS` | the managed server's `mmaps` | the navmesh for `AgentMoveTo` |
| `WRATHPLUS_AGENT_MODEL` | `claude-haiku-5-5` | when `--model` is not given |
| `WRATHPLUS_AGENT_CHECKIN_SECONDS` | `0` (off) | idle check-in |
| `WRATHPLUS_JEV_MODEL`, `WRATHPLUS_JEV_HEARTBEAT_MS` | `jev-latest`, `20000` | the fast tier's model and safety heartbeat |

Against a client you started some other way, `src/index.ts` takes the control port from
`WRATHPLUS_AGENT_URL` and `WRATHPLUS_AGENT_TOKEN` (and `WRATHPLUS_AGENT_MAX_TURNS` for smoke runs).

## Not yet

- **Other providers** (OpenAI-compatible, pi agent, Prime Intellect) after this works well.
- **WrathBench run records** (#66).
