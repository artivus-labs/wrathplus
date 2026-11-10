# Example agent

A WrathPlus agent in three tiers: **Claude plans, Jev decides, Lua plays.** It runs on the
[Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk), with **Claude Haiku 5.5** by default.

## What you need: bring your own

Nothing of ours is required: no key, account or path of the project's.

| Tier | Needs | |
|---|---|---|
| Planner (Claude) | **your own** Claude Code login on this machine (sign in once with `claude`; a Claude subscription works, no API key), **or** your own Anthropic API key as `WRATHPLUS_ANTHROPIC_API_KEY` | required |
| Fast tier (Jev) | your own `TYPESAFE_API_KEY` ([typesafe.ai](https://typesafe.ai)) | optional |
| Lua and the goal runner | a WrathPlus client with the agent port open; `wp.engine`, `wp.values` and `wp.goals` are built into it | required, no key |

**Without `TYPESAFE_API_KEY`** the agent starts and says `Fast tier: off`. What still runs: the
planner with the client, notes, wake-up, goal and module tools; the goal runner, which the agent
switches on and which plans and runs actions toward the top goal by itself (`progress` is always
underneath); the built-in Lua; the event watcher (deaths, whispers, party chat, invites, trades);
and reflection. What is missing: `set_plan` / `fast_loop_status`, answers to `Agent.decide` (its
default applies at its timeout), extending a hold that ran out (it ends and the runner carries
on), and party-request triage (requests go to the planner). The planner is told so
(`NO_FAST_TIER` in `src/prompt.ts`). Expect more planner turns, so more Claude usage, than with
Jev.

Walking (`AgentMoveTo`, and the goal runner's travel) needs the realm's navmesh: `WOW_NAV_MMAPS`
pointing at an `mmaps` folder made from your own game files (Manager, **Server** page: **Prepare
server data**, then **Make navigation meshes**; or `wrathplus-manager --make-navmesh`).

On the public beta realm, see [Play on the WrathPlus open beta](../../docs/BETA.md#the-example-agent).

- **The planner** has exactly the six tools of the client's MCP server (`../mcp`): `lua`,
  `screenshot`, `events`, `status`, `modules`, `save_module`. It has no files, shell or web.
  Its system prompt (`src/prompt.ts`) steers it to put anything routine or fast into its own Lua
  modules built on `wp.engine` and `wp.values`, which run every frame in the client, and to keep
  its own calls for decisions and fixes.
- **Event-driven tiers**, to keep cost and context down, since the agent's own Lua carries the
  work between choices:
  - **The event watcher** (`src/watch.ts`) long-polls the client and hands every event to the fast
    tier. The planner only hears what needs it: death, whispers, party chat, invites, trades and
    duels. Small talk (an NPC naming it, a stranger's or a bot's `/say` with its name, an emote
    at it) does not wake it line by line: with the fast tier Jev waves, nods, says hello, ignores
    it or passes on what needs thought (`SmallTalk` in `src/triage.ts`); without it the lines ride
    along with the next wake-up and wake the planner by themselves at most once in ten minutes.
    When Jev judges that a remark or greeting wants a spoken reply in character rather than an
    emote, one short Claude Haiku 5.5 call (`src/say.ts`, the same login as the planner, no
    tools, no session) writes the line from the character (name, race, class, level, zone), who
    spoke and what, the last few lines of chat and the last few hundred characters of MEMORY.md;
    it is cut to one plain line of at most 120 characters and said in `/say`. At most one such
    line every 30 s and 20 an hour, one answer per speaker per 2 minutes; a failure or a line
    not written within 10 s becomes a wave or a nod. Each appears under "Said to you nearby" as
    `said "<line>" to <speaker>`. `WRATHPLUS_AGENT_SAY=0` keeps small talk to emotes.
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
- **Goals** (`src/goals.ts`, the client's `Goals` API in `wp.goals`): the planner sets goals
  (`push_goal`, `drop_goal`, `goals`) and teaches actions and facts in its modules; the client's
  GOAP runner plans and runs actions toward the top goal, with the standing goal `progress` always
  underneath, so the agent is never idle. Waits are bounded: `hold` takes a reason and seconds,
  and when one runs out (`AGENT_HOLD_EXPIRED`) Jev decides `keep_waiting` (for a minute, a few, or
  ten) or `stop_waiting`, from the reason, the minutes waited, who in the party is actually here,
  and the goals. Idle with no hold, Jev is offered `wait` for a minute, then not at all; after three
  minutes the planner is told, and every ten minutes after that while it lasts, with each goal's
  next actions or why it is blocked (`idleGraceMs`, `idleNotifyMs`, `idleRepeatMs`).
- **Nobody is coming** (`src/waits.ts`, `src/runner.ts`): there is no operator or user in the
  loop, and the harness does not let the planner wait for one. The prompt says so first. A `hold`
  is classified before it is placed (by Jev; without the fast tier by a word pattern): a wait for
  something in the game that happens by itself is placed, a wait for a person, an answer, an
  instruction or a permission is refused with the goals and facts to decide from. After Jev ends
  a wait (`stop_waiting`), the same wait in similar words is refused for 30 minutes. The tool
  result shows the time held in all. `wake_me` with no time limit, or longer than 300 s, is
  refused while nothing runs in the game. `Goals.on` belongs to the harness: checked every 15 s
  and turned back on (the planner is told once); `WRATHPLUS_AGENT_GOALS=0` leaves the runner off.
  A reflection window that was 80% idle or on hold leads with that and asks for its cause to be
  fixed.
- **Reflection** (`src/reflect.ts`): after a death, a level-up, three failed actions in ten
  minutes, fifteen minutes without progress, or a quiet moment every half hour
  (`WRATHPLUS_AGENT_REFLECT_MINUTES`; `WRATHPLUS_AGENT_REFLECT=0` turns it off), the planner gets
  a digest of the window (deaths and causes, fight reviews, goals, failed actions, Jev's choices
  and holds, idle time, Lua and game errors) with a scoreboard (XP/hour, deaths, quests, idle %),
  and works on its harness like a coding agent: small change, test, deploy, watch, roll back.
  A built-in that keeps ending as a no-op (a quest giver visit that changes nothing) is named
  with the agent's own loaded modules as the first suspects, and a MEMORY.md line that says an
  area is exhausted or unreachable is set against the live goal facts that say otherwise.
  Scores go to `<notes>/reflections.jsonl`. Every `save_module` is kept as a version in
  `<notes>/modules/<name>/` with `changelog.jsonl` (`src/modules.ts`); `module_versions`,
  `note_change` and `rollback_module` are its tools.

## Run it

Each agent is one of the Manager's agents: its own realm account, headless client and control
port, shared with the Manager's **Agents** page. The managed realm must be running.

```sh
cd agents/mcp && bun install && cd ../example && bun install
cp ../.env.example ../.env          # optional: your own keys (see above); it may stay empty

bun run agent add Scout             # a realm account (AGENTSCOUT), control port and token
bun run agent run Scout --character Scoutly "Reach level 5 in Northshire."
bun run agent list                  # every agent, its client and whether a planner drives it
bun run agent note Scout "text"     # an outside note for a running agent
bun run agent dashboard             # http://127.0.0.1:17900: every running agent on one page
```

- **`run`** starts the agent's headless client (logged in, `--character` entered; without it the
  planner picks or creates one at character select), waits for it, runs the planner and Jev, and
  stops the client when you stop the agent (Ctrl-C). A client already started from the Manager is
  driven as it is. A line typed into the terminal reaches the planner as an outside note, which
  it is told is rare and never to wait for. A client `run`
  started is restarted into the same character if it exits or is disconnected (back at the login
  screen), so the planner carries on.
- **Outside notes without a terminal:** `bun run agent note Scout "The Westfall road is open."`
  reaches a running agent the same way, worded so that nobody is waiting for a reply. It appends to
  `notes/outside.jsonl` in the agent's state folder, which the running agent reads every second
  from where it was when it started; a note for an agent nobody drives is refused, since it would
  never be read. Use it when the agent's stdin is not yours to type into (started by the Manager,
  a detached pane).
- **Several at once:** one `run` per terminal; each agent has its own account and port.
- **Watching:** each client serves a live view (`run` prints its address); the dashboard shows
  all of them side by side, with what each planner last said and did.
- **The planner model:** `--model haiku` (Claude Haiku 5.5, the default) or `--model sonnet` or
  any model id. It runs on this machine's Claude Code login (yours: a Claude subscription) and ignores an
  `ANTHROPIC_API_KEY` in the shell; put `WRATHPLUS_ANTHROPIC_API_KEY` in `agents/.env` to bill an
  API key instead. It is isolated from your own Claude Code setup (no CLAUDE.md, settings or MCP
  servers) and works in its own notes folder.
- **State** lives with the Manager's (`<checkout>/wrathplus-config/agents/<ACCOUNT>/`): the
  secrets (0600), the client's home and log, and the agent's notes (memory, to-dos, sessions).

| Env (shell or `agents/.env`) | Default | |
|---|---|---|
| `TYPESAFE_API_KEY` | none | optional: your own key turns on the fast tier (Jev) |
| `WRATHPLUS_ANTHROPIC_API_KEY` | none | optional: bill your own API key instead of your Claude Code login |
| `WRATHPLUS_CLIENT` | the Manager's build | the client binary |
| `WOW_NAV_MMAPS` | the managed server's `mmaps` | the navmesh for `AgentMoveTo` |
| `WRATHPLUS_AGENT_MODEL` | `claude-haiku-5-5` | when `--model` is not given |
| `WRATHPLUS_AGENT_CHECKIN_SECONDS` | `0` (off) | idle check-in |
| `WRATHPLUS_AGENT_SAY` | on | `0`: small talk gets emotes only, no line from Claude Haiku |
| `WRATHPLUS_JEV_MODEL`, `WRATHPLUS_JEV_HEARTBEAT_MS` | `jev-latest`, `20000` | the fast tier's model and safety heartbeat |

Against a client you started some other way, `src/index.ts` takes the control port from
`WRATHPLUS_AGENT_URL` and `WRATHPLUS_AGENT_TOKEN` (and `WRATHPLUS_AGENT_MAX_TURNS` for smoke runs);
its notes go to `WRATHPLUS_AGENT_HOME`, else `~/.local/share/wrathplus-agents/agent-<port>`. This
is the way to run it on a realm other than the Manager's, such as the open beta: `bun run agent`
only knows the managed local realm.

## Not yet

- **Other providers** (OpenAI-compatible, pi agent, Prime Intellect) after this works well.
- **WrathBench run records** (#66).
