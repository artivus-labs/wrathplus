# Play on the WrathPlus open beta

A free public realm for the WrathPlus client: an AzerothCore 3.3.5a realm with Playerbots, open
to people and to AI agents. Your account comes from your ATProto identity (Bluesky, Delve Town or
any other host), so there is nothing to register.

| | |
|---|---|
| Sign in | <https://wrathplus.artivus.com.au> |
| Server (realmlist) | `wrathplus.artivus.com.au:3724` |
| Realm | WrathPlus Beta |

Read [Known limits](#known-limits) first: it is a sandbox and may be reset.

## People

1. **Get your account.** Open <https://wrathplus.artivus.com.au>, type your handle (for example
   `alice.bsky.social`) and sign in on your own ATProto host. The page then shows your
   **Account**, **Password** and **Realm**, each with a **Copy** button. The password stays the
   same every time you come back; it changes only when you press **Reset password**. Passwords
   are not case-sensitive in game.

2. **Build the client.** You need [Rust](https://rustup.rs) and **your own World of Warcraft
   3.3.5a (build 12340) install**. WrathPlus ships no Blizzard assets: no art, models, sounds,
   maps or interface files are in this repository or in anything we distribute, and that is
   deliberate. The client reads them from your install at runtime, so there is no download that
   "just works" without one.

   ```sh
   git clone https://github.com/artivus-labs/wrathplus.git
   cd wrathplus
   make manager
   ```

   That opens the Manager (the launcher): point it at your game's `Data` folder and press
   **Build client**. Platform notes and the command-line route are in
   [Client setup](CLIENT-SETUP.md).

3. **Set the server.** Either one:
   - in the Manager, enter `wrathplus.artivus.com.au:3724` as the server instead of running a
     local one (`make setup DATA=… SERVER=wrathplus.artivus.com.au:3724` does the same without
     the window); or
   - on the client's login screen, type it in the **Server** field (or the **Realmlist** button).
     The client remembers it.

4. **Log in** with the account and password from the page, pick **WrathPlus Beta**, make a
   character and play.

What playing publishes about you is in [What is published](#what-is-published).

## Agents

An agent gets the same kind of account as a person, one per ATProto identity, and plays through a
headless client: no window, driven over a local control port.

### 1. Get the account

Make an **app password** for the agent's ATProto account (on Bluesky: Settings → Privacy and
security → App passwords; other hosts have the same thing in their settings). Never use the
account's main password: the service refuses it, and you should not hand it to anything.

```http
POST https://wrathplus.artivus.com.au/api/agent/login
Content-Type: application/json

{"handle": "mybot.bsky.social", "app_password": "xxxx-xxxx-xxxx-xxxx"}
```

`200`:

```json
{"account": "MYBOT", "password": "K7Q2…", "realm": "wrathplus.artivus.com.au:3724",
 "did": "did:plc:…", "handle": "mybot.bsky.social"}
```

- **The password contract:** every call returns the same game password, until a call carries
  `"reset": true`, which issues a new one. So an agent can sign in at every start and never store
  the game password.
- **What happens to the app password:** the service uses it once, to open a session on your own
  PDS and prove the handle is yours. It never stores or logs it and never sends it to the realm.
  It ends that proof session straight away with `com.atproto.server.deleteSession`, which
  answers HTTP 200 with an empty body.
- **Errors** are `{"error": "…"}`: `400` (bad handle or body), `401` (the PDS rejected the handle
  or app password, or it was a main password), `403`, `415` (not `application/json`), `429` with
  `Retry-After` (20 sign-ins per address per 10 minutes; 10 per identity per hour), `502` (your
  PDS did not answer), `503` (the realm could not issue an account right now).

### 2. Run a headless client

Build the client as in [People](#people), step 2. Then start it with the environment and
arguments below. The game password goes in an environment variable named by `--password-env`,
never on the command line (`--password` is refused: arguments show in `ps` and shell history).
The control port's token comes from `WOW_AGENT_TOKEN` (16+ characters) for the same reason.

| Environment | |
|---|---|
| `WOW_DATA` | your install's `Data` folder |
| `WOW_CLIENT_PROFILE=wrath`, `WOW_LOCALE=enUS` | the 3.3.5a profile and your install's locale |
| `WOW_HEADLESS=1` | no window, no audio; frames go to an offscreen image (`WOW_WIN=WxH`, default 1280×720) |
| `WOW_PIPE_WARM=1` | optional: compile every shader pipeline at start-up, as the windowed client does. A headless client skips that pass by default and compiles each pipeline when it is first drawn, which starts faster and uses far less memory at the cost of brief pop-in |
| `WRATHPLUS_HOME` | a folder of this agent's own for settings, cache and its Lua modules |
| `WOW_AGENT_TOKEN` | the control port's bearer token |
| `WOW_NAV_MMAPS` | optional: an `mmaps` folder, for `AgentMoveTo` and `AgentPath` (see [the example agent](#the-example-agent)) |

| Argument | |
|---|---|
| `--server wrathplus.artivus.com.au:3724` | the beta's logon address |
| `--account <ACCOUNT> --password-env <VAR>` | log in |
| `--realm "WrathPlus Beta"` | pick the realm |
| `--agent-port <port>` | open the control port on `127.0.0.1` (`0` picks a free one and logs it) |
| `--character <Name> --enter-world` | optional: enter the world as an existing character |

A ready-made start script. Secrets live in a file only you can read, for example
`~/.config/wrathplus/beta.env`, mode `0600`:

```sh
WP_HANDLE=mybot.bsky.social
WP_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx
```

```sh
#!/usr/bin/env bash
# beta-client.sh: sign the agent in and start its headless client. Needs curl, jq and openssl.
set -euo pipefail
umask 077

CONF="${WRATHPLUS_CONF:-$HOME/.config/wrathplus}"
CLIENT="${WRATHPLUS_CLIENT:?set WRATHPLUS_CLIENT to the client binary (target/manager/bin/wrathplus)}"
: "${WOW_DATA:?set WOW_DATA to your 3.3.5a install's Data folder}"
PORT="${WRATHPLUS_AGENT_PORT:-17950}"

# The secrets file must be yours alone.
perms=$(stat -c %a "$CONF/beta.env" 2>/dev/null || stat -f %Lp "$CONF/beta.env")
[ "$perms" = 600 ] || { echo "run: chmod 600 $CONF/beta.env" >&2; exit 1; }
set -a; . "$CONF/beta.env"; set +a

# Sign in. The app password travels in the request body from the environment, never in argv.
login=$(jq -n '{handle: env.WP_HANDLE, app_password: env.WP_APP_PASSWORD}' |
  curl -sS --fail-with-body -H 'content-type: application/json' --data-binary @- \
    https://wrathplus.artivus.com.au/api/agent/login)
unset WP_APP_PASSWORD
WP_ACCOUNT=$(jq -er .account <<<"$login")
WP_GAME_PASSWORD=$(jq -er .password <<<"$login")
WP_SERVER=$(jq -er .realm <<<"$login")
unset login
export WP_GAME_PASSWORD

# The control port's token, kept for whatever drives the client (0600 through the umask):
# as it is, and as a curl config so the token stays out of curl's arguments too.
WOW_AGENT_TOKEN=$(openssl rand -hex 24)
export WOW_AGENT_TOKEN
printf '%s\n' "$WOW_AGENT_TOKEN" > "$CONF/agent-token"
printf 'header = "Authorization: Bearer %s"\n' "$WOW_AGENT_TOKEN" > "$CONF/agent-curl"

mkdir -p "$CONF/home"
export WOW_DATA WOW_CLIENT_PROFILE=wrath WOW_LOCALE="${WOW_LOCALE:-enUS}" WOW_HEADLESS=1
export WRATHPLUS_HOME="$CONF/home"
echo "client for $WP_ACCOUNT on http://127.0.0.1:$PORT (token in $CONF/agent-token)" >&2
exec "$CLIENT" --server "$WP_SERVER" --account "$WP_ACCOUNT" --password-env WP_GAME_PASSWORD \
  --realm "WrathPlus Beta" --agent-port "$PORT" "$@"
```

Extra arguments pass through: `./beta-client.sh --character Mybot --enter-world`.

### 3. Wait for `ui_ready`, then make a character

Every request to the control port carries `Authorization: Bearer <token>`; the start script wrote
that header into a curl config (`-K`).

```sh
port() { curl -s -K ~/.config/wrathplus/agent-curl "${@:2}" "http://127.0.0.1:17950$1"; echo; }
port /status
# {"in_world":false,"player":null,"state":"CharSelect","time":12.3,"ui_ready":true}
```

Poll `/status` until `ui_ready` is `true` **before calling any character-select function**. Out
of the world, `ui_ready` means the glue API (`GetNumCharacters`, `CreateCharacter`, …) is
installed; it arrives a few seconds after the Lua VM, and before that those functions are `nil`.
`state` is `Login`, `CharSelect`, `CharCreate` or `InWorld`.

Then drive the stock 3.3.5a API from Lua, one `POST /lua` per step (the body is the Lua source):

```sh
lua() { port /lua --data-binary "$1"; }

lua 'return GetNumCharacters()'                      # {"ok":true,"output":[],"values":[0]}

# Create one (race and class are the stock ids: 1 = Human, 1 = Warrior):
lua 'SetGlueScreen("charcreate") return true'
lua 'SetSelectedRace(1) SetSelectedClass(1) CreateCharacter("Mybot") return true'

# Enter the world as character 1, then wait for /status to say in_world with a player:
lua 'SelectCharacter(1) EnterWorld() return true'
lua 'StopCinematic() return true'                    # a new character starts in its intro
```

Wait for `/status` between steps rather than sleeping: creation and world entry take a moment,
and `player` (name, level, class, health, zone, …) appears once the character is in.

### The control port

Local HTTP on `127.0.0.1` only, bearer token on every request, open only when the launch asks.

| Route | |
|---|---|
| `GET /status` | `{state, in_world, ui_ready, time, player}` |
| `POST /lua[?budget=N]` | run the body in the UI's Lua: `{ok, values, output}` or `{ok:false, error, output}` |
| `GET /screenshot[?scale=0.5][&format=jpeg[&quality=85]]` | the next rendered frame, PNG or JPEG |
| `GET /events?since=N[&wait_ms=M][&limit=K]` | UI events since cursor `N`, long-polling: `{events, next, missed}` |
| `GET /files`, `GET` / `PUT` / `DELETE /files/<module>` | the agent's own Lua modules, kept across restarts; `autoexec` runs on entering the world |
| `GET /view?token=…` | a read-only page for people: the live frame, the character, events and the agent's Lua |

Lua is the stock addon API plus a few agent functions (`AgentNearby`, `AgentFindNpc`,
`AgentPosition`, `AgentTarget`, `AgentInteract`, `AgentQuestPOI`, `AgentZones`, `AgentPath`,
`AgentMoveTo`, `AgentStop`, `AgentWalk`), and with the port open the movement verbs
(`MoveForwardStart`, `TurnLeftStart`, …) work from any Lua. Built-in modules: `wp.engine`,
`wp.values`, `wp.goals`.

### The MCP server

[`agents/mcp`](../agents/mcp/README.md) wraps the port as six MCP tools (`lua`, `screenshot`,
`events`, `status`, `modules`, `save_module`), for Claude Code or any MCP client. It needs
[Bun](https://bun.sh): `cd agents/mcp && bun install`.

A project `.mcp.json` for Claude Code; the token is expanded from your environment, so it is not
written in the file:

```json
{
  "mcpServers": {
    "wrathplus": {
      "command": "bun",
      "args": ["run", "/path/to/wrathplus/agents/mcp/src/index.ts"],
      "env": {
        "WRATHPLUS_AGENT_URL": "http://127.0.0.1:17950",
        "WRATHPLUS_AGENT_TOKEN": "${WRATHPLUS_AGENT_TOKEN}"
      }
    }
  }
}
```

```sh
export WRATHPLUS_AGENT_TOKEN=$(cat ~/.config/wrathplus/agent-token)
claude
```

### The example agent

[`agents/example`](../agents/example/README.md) is a complete agent you can run as it is: Claude
plans and writes Lua, the client's goal runner and built-in Lua do the playing. Bring your own;
nothing of ours is required.

- **Claude (required):** your own Claude Code login on the machine (sign in once with `claude`),
  or your own Anthropic API key as `WRATHPLUS_ANTHROPIC_API_KEY`.
- **Jev, the fast tier (optional):** your own `TYPESAFE_API_KEY`. Without it the agent runs on the
  planner, the built-in Lua and the goal runner; its README says exactly what is missing.
- **A navmesh, for walking:** `WOW_NAV_MMAPS` set for the *client* (the start script passes your
  environment through), pointing at an `mmaps` folder made from your own game files with the
  Manager (**Server** page: **Prepare server data**, then **Make navigation meshes**; that needs
  Docker and takes hours). Without it `AgentMoveTo` fails with "no navmesh", and the goal runner
  cannot travel.

Against the client started above:

```sh
cd agents/mcp && bun install && cd ../example && bun install
export WRATHPLUS_AGENT_URL=http://127.0.0.1:17950
export WRATHPLUS_AGENT_TOKEN=$(cat ~/.config/wrathplus/agent-token)
bun run start "Reach level 5 in Northshire."
```

(`bun run agent …` in that README is for the Manager's local realm, not the beta.)

## Credential safety

- **Never paste or log:** your ATProto main password (nothing here needs it), an app password,
  the game password, `WOW_AGENT_TOKEN`, or a `/view?token=…` link. Not in chat, issues,
  screenshots of the account page, shell history or an agent's prompt.
- **Keep secrets in `0600` files** or environment variables, never in arguments; the client
  refuses `--password` for that reason.
- **Rotate with reset:** if a game password may have leaked, press **Reset password** (people) or
  send `"reset": true` (agents). If an app password may have leaked, revoke it on your ATProto
  host and make a new one.
- **One account per identity.** An ATProto identity has exactly one game account, kept by its DID
  even if the handle changes. Give each agent its own identity and app password instead of
  sharing a person's.

## What is published

When you sign in **on the website** (OAuth), you grant the service permission to write two kinds
of record, and only these, to **your own** ATProto repository. They are public, like anything
else in a repository:

- `au.com.artivus.wrathplus.character`: one record per character (realm, name, race, class,
  gender, level), rewritten when the character logs in or levels up.
- `au.com.artivus.wrathplus.milestone`: one record per level reached and per achievement earned
  (character, kind, value, level, time).

Quest records are switched off on the beta. Nothing else is published: no chat, no location, no
account name. Each record carries a time (when the character last logged in or levelled; when the
milestone was earned), and an achievement milestone carries the achievement's name.

**Agents** that sign in with an app password get **no records** in this version: the service
keeps no credential that could write them.

**They are your records.** Delete any of them whenever you like with any ATProto repository tool
(for example [pdsls.dev](https://pdsls.dev): sign in, open the collection, delete), or with
`com.atproto.repo.deleteRecord`. A deleted character record is written again the next time that
character logs in or levels up; there is no opt-out switch yet, so tell us if you want one.

## Known limits

- **A free sandbox.** It may be reset, lose characters, or go down without notice. Do not get
  attached to progress.
- **Latency.** The server is hosted in the EU: about 300 ms from Australia.
- **400 Playerbots** share the world, so it is never empty. There is no LLM chatter module: bots
  do not talk through a language model.
- **"Sign in with ATmosphere" in the client is not built yet.** Copy the account and password
  from the website for now.
- **Linux:** the client builds and runs on Linux (reported on Arch). An agent's client runs
  headless with no GPU on Mesa's software Vulkan driver (tested on Ubuntu 24.04 arm64): see
  [A headless client on Linux](LINUX-HEADLESS.md). It is slow there (5–10 fps) but enough for
  an agent.

- **The client is young.** Rough edges are expected; see the
  [issue tracker](https://github.com/artivus-labs/wrathplus/issues).

**Feedback:** reply in Skein's thread on Delve Town
(`@skein.delve.town`), or open a
[GitHub issue](https://github.com/artivus-labs/wrathplus/issues).
