# Build the Manager, then set up the client

WrathPlus supplies original client software, not World of Warcraft game files. You need a
legally obtained **Wrath of the Lich King 3.3.5a / build 12340** installation or backup that you
are entitled to use. Obtain it through a source you are legally authorized to use; this project
does not host game downloads or endorse third-party repacks. A current retail/Classic download
is not a compatible replacement. Validation checks compatibility, not ownership or licensing.

## 1. Clone and build the Manager

Install Rust with **rustup** (https://rustup.rs). The checkout's `rust-toolchain.toml` selects
the required version; a distro's unrelated `rustc` version may not match. Rustup may need internet
access on first use to install that toolchain, and Cargo needs access to uncached dependencies.
Install the native prerequisites for your platform below before building the Manager.

```sh
git clone https://github.com/artivus-labs/wrathplus.git
cd wrathplus
make launcher
make manager
```

`make` defaults to `launcher`. This builds **only the native Manager**, not the Bevy game or a
server; assets are not required yet. The Manager binary is `target/release/wrathplus-manager`
(`.exe` on Windows). `make run` is an alias for `make manager`. Keep the checkout: the Manager
builds its game from these sources, not a downloaded executable. A prebuilt Manager can open
without Cargo; **Build client** still requires the toolchain. Launching an already-built game
does not require Cargo.

### macOS

Install Xcode Command Line Tools with `xcode-select --install`, then rustup. The checker verifies
the selected developer directory and C compiler. Finder launch can have a minimal PATH; Manager
looks for Cargo in PATH, `CARGO_HOME/bin`, and the user's `.cargo/bin` as well.

### Linux: Arch

Install build dependencies (review and run these commands yourself):

```sh
sudo pacman -S --needed base-devel pkgconf alsa-lib systemd-libs libxkbcommon wayland libx11 libxcursor libxi libxrandr
```

Use rustup for the repository toolchain. If using Arch's `rustup` package, do not also install a
conflicting distro `rust` package. ALSA and libudev metadata must be discoverable by pkg-config.
A running X11 or Wayland desktop and working GPU drivers/Vulkan are needed to play. Install the
Vulkan loader (`vulkan-icd-loader`) and the driver appropriate to your GPU, not every vendor's
driver. For example Mesa systems commonly use `vulkan-radeon` or `vulkan-intel`; NVIDIA has its
own matching driver packages. The Manager's software renderer opening is not proof the game's
Vulkan renderer will work.

### Linux: Debian / Ubuntu

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev libvulkan1
```

Install rustup separately and use your distribution's GPU driver instructions. Other Linux
distributions need equivalent packages; Manager checks tools and pkg-config capabilities, not
distribution names. Display/GPU health and all transitive build dependencies still require an
actual build and run.

### Windows: native MSVC

Install rustup for Windows with an MSVC host toolchain, and **Visual Studio Build Tools** with
**Desktop development with C++**, the MSVC tools, and a Windows SDK. Rustup alone does not
install the complete C++ toolchain. Use a Developer PowerShell/Native Tools terminal if the
linker is not discoverable. The current native Windows preflight conservatively requires
`cl.exe`, `link.exe`, and a `WindowsSdkDir` containing Include/Lib directories; launch Manager
from a Visual Studio Developer PowerShell/Native Tools terminal. A normal desktop launch may
need that environment even when Visual Studio is installed. These checks are diagnostics, not
a successful compile/link proof. Make is optional; from PowerShell in the checkout use:

```powershell
.\scripts\manager.ps1 -Action run
.\scripts\manager.ps1 -Action doctor
.\scripts\manager.ps1 -Action setup -Data 'D:\Games\WoW\Data'
```

The script is not signed; follow your organization's script execution policy. Alternatively
run Cargo and the executable directly (no execution-policy change is required):

```powershell
cargo build --locked -p wrathplus-manager --release --jobs 2
.\target\release\wrathplus-manager.exe --repo (Get-Location).Path
```

Do not mix MSVC and MinGW artifacts in one target directory. Native Windows is a separate
qualification target, not something proven by Linux or WSL passing.

### Windows: WSL2 / WSLg (Linux path)

Install WSL2 and a Linux distribution, then follow that distribution's Linux instructions
**inside WSL**, including Linux rustup. Keep the checkout and build directory in the Linux
filesystem (for example `~/src/wrathplus`) for performance. Use Linux paths for assets, such as
`/mnt/d/Games/WoW/Data`; do not pass `D:\...` to the Linux Manager. Assets stay read-only.

Run `make run` inside WSL. GUI use needs WSLg (or a separately configured display server),
working host GPU drivers and usable accelerated graphics/Vulkan in WSL. A terminal-only WSL
installation can use `--doctor` or `--validate` but is not enough to play. WSL builds a **Linux**
Manager and game, not native Windows `.exe` files. If the server is hosted on Windows rather
than inside WSL, configure an address reachable from that WSL network; localhost behavior
depends on your WSL networking mode.

### Check before building

In Client setup choose **Check / Recheck**. Build is enabled only after prerequisite checks
pass; warnings (such as low or unmeasured disk space) remain visible. Build runs a fresh check
again, so a stale green checklist cannot bypass it. Checks do not install software or request
administrator access. Probes disable rustup automatic installation: a missing pinned toolchain
is reported as a blocker. Install it explicitly with `rustup toolchain install` from the checkout,
then Recheck. The actual Cargo build can still fetch uncached dependencies.

```sh
make doctor
# Or with an already-built Manager (no Manager rebuild):
target/release/wrathplus-manager --doctor --repo /path/to/wrathplus
```

`--doctor` exits nonzero for blockers, zero when only warnings/passes remain. Assets are checked
separately with `--validate`. Output-directory checks may create and remove unique probe files
in safe writable locations; no probe is allowed inside the selected game installation.

These are supported setup paths, not a claim of live qualification on every platform. Native
macOS checks are run locally. `.github/workflows/manager.yml` defines synthetic-asset tests,
strict Clippy and Manager builds on macOS, Ubuntu and Windows; adding the workflow is not
proof those remote jobs have passed. Arch and WSL still need native qualification, and all
platforms need their own graphical/game acceptance.

`JOBS=2` is the default. Override `TARGET_DIR` to reuse a build directory and `JOBS` to set a
machine-appropriate concurrency limit. A first game build needs substantial free disk space and
can take several minutes or longer; do not interpret compilation time as a frozen Manager.

## 2. Select and validate your game assets

Extract your own supported installation outside the checkout. In **Client**, paste its **Data**
folder path and select the matching locale (for example `enUS`). Do not select an installer,
a compressed download, or just `Wow.exe`.

Choosing a folder validates it automatically; for a typed path click **Validate assets**. Canceling
the dialog preserves the previous selection. Linux uses the desktop XDG file portal, so install
`xdg-desktop-portal` and the backend appropriate to your desktop if the chooser is unavailable.

Click **Validate assets**. Validation reads the archive chain, checks build 12340 metadata and
required client resources. It does not run the original executable or modify your installation.
Missing, unreadable, truncated or wrong-version data produces an actionable error rather than
silently marking the game ready. A successful validation is a compatibility smoke check, not a
checksum audit of every game file.

## 3. Build and launch

Set the authentication server (`host:port`; default `127.0.0.1:3724`), then **Build client**.
Cargo compiles the player client with development features disabled. Compiler output goes to the
Manager build log. A failed build must not replace the last working managed executable.

Manager also detects existing executables, in order: its managed build, the checkout's
`target/azb-live/bin/wrathplus`, then `play/wrathplus` and `release/wrathplus` in the configured
target directory (with `.exe` on Windows). The detected path appears on Play. Discovery never
runs a binary to identify it; presence is not proof of freshness or compatibility. Only use
trusted builds from this checkout. Empty files and non-executable Unix files are rejected.

Select and validate your assets to enable **Launch game** without recompiling an existing
build. Successful validation saves settings when a build exists; future Manager starts restore
them. Launch always validates again. To register an existing build from the terminal without
launching or copying it:

```sh
target/release/wrathplus-manager --use-existing --data /path/to/Data
```

After successful setup the Client row shows **Available** and enables **Launch game**. Launch
starts the managed executable with your chosen read-only assets, locale and server. Enter account
credentials in the game; Manager does not store passwords. The server must be running and
reachable: run a local realm from the **Server** page (see below) or enter an existing realm's
address. Starting server and client with one press is still a preview.

Close the game normally before rebuilding. The Manager tracks its launched process and disables
rebuild/duplicate launch while it is running. Keep the Manager open while playing in this initial
setup version; do not run concurrent Managers against the same checkout.

## 4. A local realm (optional)

The **Server** page builds and runs a private AzerothCore 3.3.5a realm with Playerbots, isolated
from any other server on the machine. It needs **Docker Desktop** (or Docker Engine with Compose
v2 on Linux); if Docker is missing or stopped the page says so, and **Start Docker** opens it.

Its *Set up your own server* card walks a fresh install through these steps in order:

1. **Get server source** downloads the Playerbot branch of AzerothCore and mod-playerbots, at the
   revisions the build checks, into the Manager's state folder and selects it.
2. Choose a **server type**: *Playerbots (recommended)*, *Playerbots + Chatter* (bots talk through
   a language model you configure under **Addons → Server modules**) or *Plain AzerothCore*.
   **Advanced** opens the full module list, where the standard modules are one click each.
3. **Build** the server image. The first build can take over an hour.
4. **Prepare server data** runs AzerothCore's own extractors from the built image on your client's
   Data folder (mounted read-only, no network) and selects the result (`dbc`, `maps`, `vmaps`).
   **Make navigation meshes** adds `mmaps` afterwards; it takes hours, the realm can run meanwhile,
   and a stopped run continues where it left off. Restart the server to use them.
5. **Start**. The realm listens on `127.0.0.1:13724`; **Use this server for the client** points
   the client at it. Stopping keeps characters, progress and the database.

Already have a clean checkout or extracted data? Enter the folders under **Server files
(advanced)** instead. Without the window, `wrathplus-manager --get-server-source`,
`--prepare-server-data` and `--make-navmesh [MAP[,X,Y]]` run the same steps.

The recipe behind the page, its isolation and credential handling are in
[`packaging/server/README.md`](../packaging/server/README.md).

## Non-interactive setup

For a machine with assets already present:

```sh
make setup DATA="/path/to/World of Warcraft/Data" LOCALE=enUS SERVER=realm.example:3724
# Equivalent full client setup alias (does not build/provision a server):
make all DATA="/path/to/World of Warcraft/Data"
```

Or use the same shared setup path directly:

```sh
target/release/wrathplus-manager --setup \
  --repo "$PWD" --data "/path/to/Data" --locale enUS \
  --server realm.example:3724 --target-dir "$PWD/target"
```

`--validate --data "/path/to/Data"` performs the read-only compatibility checks without building.
`--setup` validates, builds and records setup, then exits. It **never launches the game**.
Without `--setup` the binary opens the UI. `--help` lists supported arguments. Arguments are passed
to child processes as individual values, not interpreted as shell commands by the Manager.

Assets stay in their original directory; no MPQs, extracted data or original interface code are
copied into the repository or distributed by this workflow. Manager configuration and build logs
are local, ignored state, separate from the original installation.

### Choose the build / client directory

Client setup includes a **Build / client directory** text field and native **Browse…**
button. Select a Cargo target directory for new builds, or a directory containing an
existing trusted `wrathplus` executable (`wrathplus.exe` on Windows). Manager searches
`manager/bin/`, the selected directory itself, `bin/`, `play/`, then `release/` in that
order; the checkout's `target/azb-live/bin/` is a fallback. The actual detected executable
is shown below the field, so check it before launching.

New builds use the selected directory as Cargo's target and install to its
`manager/bin/` subdirectory. Changing it clears the prerequisite checklist; recheck before
building. **Validate / save settings** validates assets and saves the location when a
client exists; successful build and launch also persist it. A location with no build is
kept in the current session until setup succeeds. Original asset-install directories
cannot be used for output. Canceling Browse leaves the previous path unchanged.

### Private login defaults

Manager reads the checkout's optional, ignored `.azerothbench-test.env` at each launch,
matching the old attended launcher's account/password prefill. Supported assignments are
`AZEROTHBENCH_TEST_USER` and `AZEROTHBENCH_TEST_PASSWORD` (plain values or matching single/
double quotes; optional `export`). This is a literal data file, **not a shell script**:
no expansion, substitutions or commands are executed. Keep it private (0600 on Unix).
An absent file leaves the login fields empty; malformed values block launch with a
redacted error. Credentials are never saved in Manager JSON or added to process arguments.

Explicit inherited `WOW_USER` / `WOW_PASS` take precedence, followed by inherited
`AZEROTHBENCH_TEST_USER` / `AZEROTHBENCH_TEST_PASSWORD`, then the private file.
Manager always uses fill-only mode: you still press **Login** and choose a character.
Server, assets and locale come from Manager settings. GM/god mode are off, and the
window remains an ordinary attended window. Probe/unattended options and the old
launcher's temporary diagnostic capture suite are not imported. Manager continues
using its checkout-local home selected below; it does not migrate the old script's game settings.

## Local-state directory

WrathPlus uses `wrathplus-config/`, with addons in `wrathplus-config/AddOns/`.
The runtime selects the primary checkout for development builds (shared across linked
worktrees), or the executable directory for player builds. The entire home is used for reads
and writes: CVars, account/realmlist login state, macros, bindings, addon enable state,
SavedVariables, AddOns, screenshots/logs, `manager.json` and `manager-addons.json`.

Manager uses the same folder under its selected checkout, including when restoring
`manager.json` at startup, and explicitly passes that selected home as `WRATHPLUS_HOME` to the
game. Inherited `WRATHPLUS_HOME` is intentionally not imported by Manager. Direct client launches
honor an explicit `WRATHPLUS_HOME` exactly (including an empty value). `WOW_CAPTURE` disables all
client player-state access. Private `.azerothbench-test.env` login prefill lives at the checkout
root.

## Addons and custom source folders

The **Addons → Client only** page scans real TOCs from the local state's `AddOns`,
the selected asset installation's `Interface/AddOns`, the checkout's authored `addons`,
and custom sources. **Browse…** or **Add source** accepts either an individual addon
folder or an AddOns collection. A folder with a single differently named TOC is mapped
using that TOC's logical addon name (e.g. `MultiBot-Chatless/MultiBot.toc` → `MultiBot`).

The list shows version, Interface, startup/load-on-demand, declared required/optional
addon dependencies, saved-variable names and missing manifest entries. `.pub`-only
Blizzard directories are not third-party packages; their code comes from the archives.
Only Interface 30300 packages with complete manifest entries can be added here.

**Add to client** registers a managed directory link under `wrathplus-config/AddOns`
(or the existing legacy home described above).
It neither copies source files nor writes into the original installation. This preserves
one namespace for Lua/XML, sibling dependencies, textures, fonts and demand loading.
Custom source registrations live in ignored `<selected home>/manager-addons.json`, separate
from game settings; profile selection itself remains a preview. On Windows, directory
links require Developer Mode or symlink permission; failure is explicit, not a silent copy.

Apply changes while the managed client is stopped. Duplicate logical identities and
unmanaged destination folders are refused. Unchecking removes only the managed link;
source files and SavedVariables are preserved. Unavailable saved registrations remain
removable. Existing local addons remain in place. Per-character enable/disable remains
in the client's character-select **AddOns** screen; registration is not an override of
that state. Dependencies must also be registered; discovery does not install them.

Addons execute Lua code: select only trusted sources. **Discovered/registered does not
mean compatible or successfully executed.** Runtime load errors are in the client log.
Manager does not connect to the realm to certify server modules or exercise addon UI.

### Current local inventory and requirements

| Addon | Requirements |
| --- | --- |
| MultiBot 4.0 (installed Chatless variant) | Interface 30300; embedded Ace libraries; `mod-playerbots` for playerbot features and `mod-multibot-bridge` for bridge features. No external TOC addon dependencies. Account and character SavedVariables. Full installed UI/bridge acceptance remains a live check. |
| AzerothBenchObserver | Authored observation fixture; Interface 30300; account SavedVariables. |
| AzerothBenchObserverReport | Requires AzerothBenchObserver; load on demand; character SavedVariables. |
| AzerothBenchUnits | Authored observed-unit UI fixture; Interface 30300. |

These are discovered candidates, not automatically enabled benchmark instrumentation.
The installed MultiBot is not bundled or copied into this repository.

## Running the game directly

The launcher is optional once the client builds. With Cargo:

```sh
WOW_DATA=/path/to/WoW/Data cargo run --release -p wrathplus
```

You need **Rust** (the version pinned in `rust-toolchain.toml`, installed by rustup) **and a C
compiler**, because the client's Lua is built from source: see the platform sections above.

`WOW_DATA` names the install's `Data` folder; a link to the install named `WoW` at the repo root
does the same. `enUS` is the default locale; `WOW_LOCALE` selects another. The server defaults to
`127.0.0.1:3724`, or what the install's `Data/<locale>/realmlist.wtf` sets; change it from the
Realmlist button on the login screen, or for one session with `--server <host[:port]>`
(`WOW_HOST`). Credentials go in at the login screen.

Flags skip steps, each only its own: `--account <name>` with `--password-env <VAR>` or
`--password-stdin` logs in (the password is never an argument), `--realm <name>`,
`--character <name>` and `--enter-world`. `WOW_USER`, `WOW_PASS`, `WOW_REALM` and `WOW_CHAR`
(pick and enter) map onto the same steps. See `crates/wrathplus-app/src/launch_plan.rs`.

## Using your own AzerothCore server

Any **AzerothCore WotLK / 3.3.5a (build 12340)** realm works; other expansions and server
implementations are not tested. The client needs:

- `authserver` and `worldserver` running, with their databases set up
  ([AzerothCore installation guide](https://www.azerothcore.org/wiki/installation)).
- The auth port (normally TCP **3724**) reachable, and the world address the realm list
  advertises (normally TCP **8085**). `--server` selects auth only; it does not fix a wrong
  address in the server's realm list.
- An ordinary player account. No GM rights, bot account or LLM key is needed to play.

Playerbots needs the [`Playerbot` branch of mod-playerbots/azerothcore-wotlk](https://github.com/mod-playerbots/azerothcore-wotlk/tree/Playerbot)
with [mod-playerbots](https://github.com/mod-playerbots/mod-playerbots); it does not build on
stock AzerothCore. The modules the launcher offers (MultiBot bridge, LLM Chatter, guild house,
Playerbot guild house, auction house bot) are all optional server additions; on a server you run
yourself, follow each module's own install, SQL and config steps. Back up databases first.
