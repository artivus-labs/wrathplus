# A headless client on Linux without a GPU

An agent's client needs no desktop and no GPU: `WOW_HEADLESS=1` renders offscreen, and Mesa's
software Vulkan driver (llvmpipe/lavapipe) draws the frames on the CPU. This is the recipe for a
server or container. It was run on Ubuntu 24.04 (arm64, in Docker) against the beta realm; other
distributions need the equivalent packages (on Arch: `vulkan-icd-loader` and `vulkan-swrast`,
not tried).

You still need your own 3.3.5a (12340) `Data` folder on the machine, as in
[Client setup](CLIENT-SETUP.md).

## Packages

To build (the list in Client setup, plus what a bare server image lacks):

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config libasound2-dev libudev-dev libxkbcommon-dev \
  libwayland-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev \
  libvulkan1 mesa-vulkan-drivers curl ca-certificates git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
. "$HOME/.cargo/env"
```

`mesa-vulkan-drivers` is the software renderer. `libvulkan1` only recommends it, so an install
with `--no-install-recommends` (most container images) leaves it out, and the client then stops
with `Unable to find a GPU!`.

To run a client built elsewhere, a machine needs only
`libasound2t64 libvulkan1 mesa-vulkan-drivers`.

## Build

```sh
git clone https://github.com/artivus-labs/wrathplus.git && cd wrathplus
cargo build --locked -p wrathplus --bin wrathplus --no-default-features --profile play
```

The first `cargo` call installs the pinned toolchain. With 4 jobs (`CARGO_BUILD_JOBS=4`) the
build took 7.5 minutes and stayed under 10 GiB of memory (the container's peak, file cache
included); the target folder is 4.6 GB. The
binary is `target/play/wrathplus`.

## Run

```sh
export WOW_HEADLESS=1 WOW_DATA=/path/to/Data WOW_CLIENT_PROFILE=wrath WOW_LOCALE=enUS
export WRATHPLUS_HOME="$HOME/wrathplus-agent"        # the client's own state; any empty folder
export WOW_AGENT_TOKEN="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
export WOW_HEADLESS_FPS=5                            # see Resources
export WRATHPLUS_AGENT_PASSWORD='...'                # your game account's password
mkdir -p "$WRATHPLUS_HOME"
target/play/wrathplus --server wrathplus.artivus.com.au:3724 \
  --account MYACCOUNT --password-env WRATHPLUS_AGENT_PASSWORD --realm "WrathPlus Beta" \
  --character Myname --enter-world --agent-port 17980
```

The log should show, in order:

```
AdapterInfo { name: "llvmpipe (LLVM ...)", ... device_type: Cpu, ... backend: Vulkan }
headless: no window; rendering 1280x720 offscreen at 5 fps
agent port: listening on http://127.0.0.1:17980
loading screen: cleared (...)
```

Bevy's warning that the adapter "only supports software rendering" is expected. Then, from the
same machine:

```sh
auth="Authorization: Bearer $WOW_AGENT_TOKEN"
curl -s -H "$auth" http://127.0.0.1:17980/status
# {"in_world":true,"player":{...,"name":"Myname",...},"state":"InWorld","ui_ready":true}
curl -s -H "$auth" -X POST --data-binary 'return UnitName("player"), UnitLevel("player"), GetZoneText()' \
  http://127.0.0.1:17980/lua
# {"ok":true,"output":[],"values":["Myname",1,"Elwynn Forest"]}
curl -s -H "$auth" -o frame.jpg 'http://127.0.0.1:17980/screenshot?format=jpeg'
```

`/status` says `in_world` a few seconds before the loading screen lifts; wait until
`GetZoneText()` is not empty before acting on a screenshot.

**No character yet:** leave out `--character` and `--enter-world`, wait for
`"state":"CharSelect","ui_ready":true`, then `POST /lua` `SetGlueScreen("charcreate")`, then
`SetSelectedRace(1) SetSelectedClass(1) CreateCharacter("Myname")`, then `EnterWorld()`. A new
character starts in its intro cinematic; `StopCinematic()` ends it.

**In a container:** the port listens on 127.0.0.1 only, so run the agent (or the MCP server) in
the same container, or give the container the host's network.

**The MCP server** ([`agents/mcp`](../agents/mcp/README.md)) works unchanged against this port
with Bun on Linux: `WRATHPLUS_AGENT_URL=http://127.0.0.1:17980` and
`WRATHPLUS_AGENT_TOKEN="$WOW_AGENT_TOKEN"`.

## Resources

Measured on 16 shared arm64 cores, in Northshire, 1280×720 unless noted, before headless clients
skipped the start-up shader pass (#149). Since then a headless client skips it by default
(`WOW_PIPE_WARM=1` brings it back), so the memory and start-up figures below are the upper bound;
new figures for the default are still to be measured on Linux. A Mac with a GPU runs
the same headless client in about 2 GiB and 0.6 of a core.

| Setting | Frame rate reached | CPU | Memory |
|---|---|---|---|
| default (30 fps asked) | 7–10 fps | 5 cores | 10 GiB |
| `WOW_HEADLESS_FPS=5` | 5 fps | 3 cores | 10 GiB |
| `WOW_HEADLESS_FPS=10 WOW_WIN=640x360` | 10 fps | 2.2 cores | 10 GiB |
| `WOW_HEADLESS_FPS=5`, limited to 2 cores | 4.2–4.5 fps | 2 cores | 7–10 GiB |
| `WOW_HEADLESS_FPS=5 WOW_WIN=640x360`, limited to 2 cores | 5 fps | 1 core | 7–10 GiB |

- The software renderer cannot reach the default 30 fps. Set `WOW_HEADLESS_FPS=5` (or 10 at
  `WOW_WIN=640x360`); Lua, screenshots and an `AgentMoveTo` walk were checked at 5 fps on 2 cores.
- With `WOW_PIPE_WARM=1`, plan for **up to 10 GiB of memory per client**. Nearly all of it is the
  start-up pass that compiles every shader variant behind the loading screen, which the software
  renderer keeps in memory, and start to in the world takes 40–75 seconds, most of it that pass.
  The default skips the pass and compiles each pipeline when it is first drawn.
- A `/lua` call answers in 0.1–0.2 s and a screenshot in 0.3–0.9 s.

## Walking routes (`AgentMoveTo`)

`AgentMoveTo` and `AgentPath` need the realm's navigation mesh: start the client with
`WOW_NAV_MMAPS=/path/to/mmaps`. Without it they return
`no navmesh: start the client with WOW_NAV_MMAPS=<server data>/mmaps`; the stock movement
functions (`MoveForwardStart`...) and `AgentInteract`'s straight-line walk still work.

The mesh is built from your own game data and is not distributed. The Manager builds it with the
local server's tools (`wrathplus-manager --make-navmesh`, see
[Client setup](CLIENT-SETUP.md#4-a-local-realm-optional)); that needs Docker and the server image
and takes hours. The whole folder is 5.8 GB; the two starting continents are maps `000` (0.9 GB)
and `001` (1.4 GB), and the client reads only the files of the map it is on, so copying
`000*` and `001*` to the agent's machine is enough to begin with.
