# Example agent

A deliberately plain WrathPlus agent: **Claude plans, Lua plays.** It runs on the
[Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk), so it uses this machine's Claude
Code login (a Claude subscription works; no API key), with **Claude Sonnet 5.5** by default.

- **The planner** has exactly the six tools of the client's MCP server (`../mcp`): `lua`,
  `screenshot`, `events`, `status`, `modules`, `save_module`. It has no files, shell or web.
  Its system prompt (`src/prompt.ts`) steers it to put anything routine or fast into its own Lua
  modules built on `wp.engine` and `wp.values`, which run every frame in the client, and to keep
  its own calls for decisions and fixes.
- **The event watcher** (`src/watch.ts`) long-polls the client. Things a player would notice
  (death, whispers, party chat, invites, trades, duels, level-ups, game errors, its own failed Lua)
  arrive as `Game events:` messages while the planner works, batched over 3 seconds.
- **Operator notes:** each line typed on stdin goes to the planner.
- After a finished turn with no news it checks in every 2 minutes
  (`WRATHPLUS_AGENT_CHECKIN_SECONDS`, `0` to switch off).

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
| `WRATHPLUS_AGENT_CHECKIN_SECONDS` | `120` | idle check-in |
| `WRATHPLUS_AGENT_MAX_TURNS` | none | smoke runs only: stop after the first turn, at most this many tool turns |

Watch it at the client's web view (`/view?token=…`, or **Watch** on the Agents page).

## Not yet

- **The fast loop (Jev).** Realtime choices currently come from the agent's own Lua (`wp.engine`).
  A fast model on the event stream is the next step.
- **Other providers** (OpenAI-compatible, pi agent, Prime Intellect) after this works well.
- **WrathBench run records** (#66).
