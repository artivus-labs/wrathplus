<div align="center">
  <h1>WrathPlus</h1>
  <p><b>A World of Warcraft 3.3.5a (build 12340) client, written from scratch in Rust and <a href="https://bevy.org">Bevy</a></b></p>
</div>

[WrathPlus](https://github.com/artivus-labs/wrathplus) is the AzerothBench fork of
[benilla](https://github.com/samwhosung/benilla), porting its from-scratch 1.12.1 client to
Wrath of the Lich King 3.3.5a against an AzerothCore server. The launcher and workspace crates
use the `wrathplus` name; remaining 1.12 client paths are being removed
([`docs/VANILLA-REMOVAL-PLAN.md`](docs/VANILLA-REMOVAL-PLAN.md)).
Every file format, the network protocol and the interface engine are written from scratch, with
no original client code and no bundled game assets. The fork's scope and its evaluation contract
are in [`docs/AZEROTHBENCH.md`](docs/AZEROTHBENCH.md).

## Getting started

Build the native Manager first, then use its **Client** setup page to validate your own legally
obtained 3.3.5a/build 12340 assets, compile the game and launch it:

```sh
git clone https://github.com/artivus-labs/wrathplus.git
cd wrathplus
make launcher
make manager
```

For complete non-interactive client setup, use `make setup DATA="/path/to/Data" LOCALE=enUS`
(or `make all DATA="/path/to/Data"`). This does not download game assets or provision a server.
See [Client setup](docs/CLIENT-SETUP.md) for prerequisites, server selection and CLI arguments.

## Status

The native 3.3.5 refactor is in progress. [`docs/WRATH-NATIVE-PLAN.md`](docs/WRATH-NATIVE-PLAN.md)
holds the workstreams and what is done in each, [`docs/WRATH-OPCODE-COVERAGE.md`](docs/WRATH-OPCODE-COVERAGE.md)
the wire coverage, and [`docs/WRATH-BUG-BACKLOG.md`](docs/WRATH-BUG-BACKLOG.md) what play-testing
has found. A feature counts as working only with tests and live evidence on 3.3.5 data
([`docs/AZEROTHBENCH.md`](docs/AZEROTHBENCH.md), "Port gates"); this README makes no capability
claims beyond those documents.

Two experience presets exist ([`docs/EXPERIENCE.md`](docs/EXPERIENCE.md)): **Classic**, the
default, matches the stock 3.3.5 client; **Plus** is an optional modern preset.

## What's inside

- **Formats:** readers for the 3.3.5 asset stack (the MPQ patch chain and its locale archives, BLP,
  DBC with the build-12340 layouts, ADT with MH2O liquid, WDT, WDL, M2 version 264 with its
  external skins and `.anim` files, and WMO), wired into Bevy as an asset source.
- **World:** streamed terrain, portal-culled buildings, doodads and ground clutter, liquid, sky,
  weather and the client's lighting.
- **Networking:** the build-12340 authentication and world session, and the object mirror into
  the ECS.
- **Interface:** a FrameXML and Lua engine that runs the stock 3.3.5 interface from your install's
  archives.
- **Audio:** music, ambience and effects.

The format readers and the UI engine core are plain Rust with no Bevy in them, and the world
renderer runs with no game attached. [`docs/MAP.md`](docs/MAP.md) maps every crate and subsystem,
generated from the code.

## Server requirements and modules

WrathPlus is a client, not a server distribution. Run a separate **AzerothCore WotLK / 3.3.5a
(build 12340)** realm; other expansions and server implementations are not qualified here.
Follow the [AzerothCore installation guide](https://www.azerothcore.org/wiki/installation) for
its platform dependencies, database initialization and updates, and extraction of the server's
DBC, maps, vmaps and mmaps from your own compatible install. Those files and databases are not
included in this repository.

For a basic connection you need:

- Running `authserver` and `worldserver` processes with their database connections configured.
- An auth endpoint reachable by the client (normally TCP **3724**) and the realm's advertised
  world endpoint reachable too (normally TCP **8085**). Setting `--server` selects auth; it
  does not override an incorrect address in the server's realm list.
- A regular player account created on that realm. No GM privileges, bot account or LLM API key
  is needed for ordinary play. Keep account passwords, database credentials and API keys out of
  this repository.

### Playerbots development realm

Our development realm uses the **[`Playerbot` branch of mod-playerbots/azerothcore-wotlk](https://github.com/mod-playerbots/azerothcore-wotlk/tree/Playerbot)**
with **[mod-playerbots](https://github.com/mod-playerbots/mod-playerbots)**. The module requires
that server fork; do not add it to stock AzerothCore and expect it to build. Follow the module's
installation guide for a mutually compatible server/module pair and its additional database
and configuration setup. The local source revisions used for this setup are core
`7f12e89ee5f467a50e62eba1d525eac7dc953d03` and mod-playerbots
`7bae1b5c58c76a0aa20381155edc08096d1485b2`; these identify the development sources, not a promise
that arbitrary newer module combinations work.

The realm's generated module loader includes the following modules. They are **server-side
additions, not mandatory WrathPlus client dependencies**:

| Module | Purpose and local configuration |
| --- | --- |
| [mod-playerbots](https://github.com/mod-playerbots/mod-playerbots) | Player-like companions and world population; enabled with `AiPlayerbot.Enabled = 1`. |
| [mod-ah-bot](https://github.com/azerothcore/mod-ah-bot) | Auction-house population; seller enabled, buyer disabled in our configuration. |
| [mod-guildhouse](https://github.com/azerothcore/mod-guildhouse) | Phased guild housing and its NPC services. |
| [mod-player-bot-guildhouse](https://github.com/DustinHendrickson/mod-player-bot-guildhouse) | Connects Playerbots to guild housing; compiled in, but **disabled** locally (`PlayerbotGuildhouse.Enable = false`). Requires mod-playerbots and mod-guildhouse. |
| [mod-llm-chatter](https://github.com/Hokken/mod-llm-chatter) | LLM-backed bot dialogue; enabled locally. Requires Playerbots plus a separately configured model service; not needed for client login or normal gameplay. |
| [mod-multibot-bridge](https://github.com/Wishmaster117/mod-multibot-bridge) | Structured addon-message bridge for [MultiBot-Chatless](https://github.com/Wishmaster117/MultiBot-Chatless). Its UI is a separate client addon, not supplied by the server module or bundled with WrathPlus. |

To reproduce this realm, place compatible module checkouts under the server's `modules/`
directory, rerun CMake, rebuild/install the server, apply each module's documented SQL updates
to the appropriate databases, and configure its installed `.conf` files. Back up existing
databases before updates. Follow each module's own dependency and configuration instructions;
simply copying a configuration file does not install its schema or scripts. LLM Chatter and
MultiBot's presence on the realm does not establish complete addon compatibility in WrathPlus.
The client still has work in progress; see the coverage and bug documents above.

## Running it

You need:

- **A 3.3.5a client (build 12340)** for the game data. The client only reads it. `enUS` is the
  default locale; `WOW_LOCALE` selects another.
- **An AzerothCore 3.3.5a server**, with auth/world services, initialized databases, extracted
  server data and a player account. See [Server requirements and modules](#server-requirements-and-modules).
- **Rust 1.98.1** (pinned in `rust-toolchain.toml`, installed by rustup) **and a C compiler**,
  because the client's Lua is built from source: on macOS the
  Xcode command line tools, on Linux the ALSA and udev development packages and pkg-config, on
  Windows the separately installed Visual Studio C++ Build Tools and Windows SDK (rustup alone
  does not install these). See [Client setup](docs/CLIENT-SETUP.md) for Arch, Debian/Ubuntu,
  macOS, native Windows and WSL instructions, plus the Manager prerequisite checker.

```sh
WOW_DATA=/path/to/WoW/Data cargo run --release -p wrathplus
```

`WOW_DATA` names the install's `Data` folder; a link to the install named `WoW` at the repo root
does the same. The server defaults to `127.0.0.1:3724`, or what the install's
`Data/<locale>/realmlist.wtf` sets; change it from the Realmlist button on the login screen, or for
one session with `--server <host[:port]>` (`WOW_HOST`). Credentials go in at the login screen.
Flags skip steps, each only its own: `--account <name>` with `--password-env <VAR>` or
`--password-stdin` logs in (the password is never an argument), `--realm <name>`,
`--character <name>` and `--enter-world`; `WOW_USER`, `WOW_PASS`, `WOW_REALM` and `WOW_CHAR`
(pick and enter) map onto the same steps. See `crates/wrathplus-app/src/launch_plan.rs`.

Settings, screenshots and addons live in `wrathplus-config/` at the repo root; addons go in
`wrathplus-config/AddOns/`. Existing `benilla-config/` homes remain in use without moving
state; see [local-state compatibility](docs/CLIENT-SETUP.md#local-state-directory-compatibility).
[`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) has the rest, from the player build to the tests.

---

benilla, by its author and contributors, is the upstream this fork builds on. Early inspiration
and file format guidance came from the [wowemulation-dev](https://github.com/wowemulation-dev)
community, and [warcraft-rs](https://github.com/wowemulation-dev/warcraft-rs) in particular.

This is an independent fan project, not affiliated with or endorsed by Blizzard Entertainment. It
ships **no Blizzard content**: no art, models, sounds, maps, MPQ contents or FrameXML. You provide
your own legally obtained 3.3.5a client, and the stock interface runs off its FrameXML at runtime.
The few files under `crates/wrathplus-app/assets/ui/` are our own, not copies of it: adapters over
stock files, and the settings windows and script error log the client draws itself.

World of Warcraft is a trademark of Blizzard Entertainment, Inc. Our own code is licensed under
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. The two vendored components
under `third_party/`, the kira audio engine and a patched Lua 5.1, keep their own upstream
licenses, alongside each; their `BENILLA.md` files record what was changed and why.
