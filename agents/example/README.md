# Example agent

A WrathPlus agent in three tiers: **Claude plans, Jev decides, Lua plays.** It runs on the
[Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk), so it uses this machine's Claude
Code login (a Claude subscription works; no API key), with **Claude Sonnet 5.5** by default.

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

Start an agent client from the launcher's **Agents** page (or by hand with `--agent-port` and
`WOW_AGENT_TOKEN`), then:

```sh
cd agents/mcp && bun install && cd ../example && bun install
WRATHPLUS_AGENT_URL=http://127.0.0.1:17950 WRATHPLUS_AGENT_TOKEN=… \
  bun run src/index.ts "Reach level 5 in Northshire, doing the quests as you go."
```

| Env | Default | |
|---|---|---|
| `WRATHPLUS_AGENT_URL`, `WRATHPLUS_AGENT_TOKEN` | required | the client's control port and token |
| `WRATHPLUS_AGENT_MODEL` | `claude-sonnet-5-5` | any Claude model id |
| `WRATHPLUS_AGENT_CHECKIN_SECONDS` | `0` (off) | idle check-in |
| `TYPESAFE_API_KEY` | none | turns on the fast tier (Jev) |
| `WRATHPLUS_JEV_MODEL`, `WRATHPLUS_JEV_HEARTBEAT_MS` | `jev-latest`, `20000` | the fast tier's model and safety heartbeat |
| `WRATHPLUS_AGENT_MAX_TURNS` | none | smoke runs only: stop after the first turn, at most this many tool turns |

Watch it at the client's web view (`/view?token=…`, or **Watch** on the Agents page).

## Not yet

- **Pathfinding** (#68): click-to-move walks in a straight line, so indoor targets defeat it.
- **Other providers** (OpenAI-compatible, pi agent, Prime Intellect) after this works well.
- **WrathBench run records** (#66).
