# Contributing

This fork is a faithful 3.3.5a (build 12340) client: in its default **Classic** preset a change
is accepted when it makes the client more like the stock 3.3.5a client or fixes a bug, with
evidence from the reference, in one small piece, with the gates green. The optional **Plus**
preset ([`https://github.com/artivus-labs/wrathplus/wiki/EXPERIENCE`](https://github.com/artivus-labs/wrathplus/wiki/EXPERIENCE)) is the one in-tree exception, and every Plus feature is
off under Classic. Everything else lives on top, in a crate of its own or a fork. The 1.12 paths
still in the tree are being removed ([`https://github.com/artivus-labs/wrathplus/wiki/VANILLA-REMOVAL-PLAN`](https://github.com/artivus-labs/wrathplus/wiki/VANILLA-REMOVAL-PLAN)) and are
not a reference for new work.

## Where to start

Open work is planned in [`https://github.com/artivus-labs/wrathplus/wiki/WRATH-NATIVE-PLAN`](https://github.com/artivus-labs/wrathplus/wiki/WRATH-NATIVE-PLAN), its gaps in
[`WRATH-OPCODE-COVERAGE.md`](WRATH-OPCODE-COVERAGE.md), and what play-testing found in
[`https://github.com/artivus-labs/wrathplus/issues`](https://github.com/artivus-labs/wrathplus/issues). Each entry says what 3.3.5a does, what the client
does instead, and where. Work happens on a branch and lands by review.

## What gets in

- A fix for a bug, with how to see it before and after.
- A step closer to 3.3.5a: a missing packet, verb, window, effect or behaviour, done the way
  the real client does it.
- A correction where the client and the reference disagree, with the reference fact stated.
- Whatever the stock 3.3.5a client loads from its archives, the client loads: models, textures or
  data rows that 3.3.5a shows and the client does not are a bug.

## What does not

- Features 3.3.5a does not have, and behaviour changed because it seems better, outside the Plus
  preset. They live on top ("Building on top"), so that what Classic does is what 3.3.5a does,
  for everyone who builds on it. A deviation from the reference is the maintainer's call and is
  recorded where it lives; a pull request is not the place to propose one.
- Anything from a WoW install: art, models, sounds, maps, data, and interface code (FrameXML and
  GlueXML), which runs off the player's own install (`docs/METHOD.md`).
- Big or mixed changes. One change per pull request, small enough to read in one sitting.

## Building on top

A feature 3.3.5a does not have lives in a crate of its own that adds its Bevy plugins through
`wrathplus_app::run_with`, as `crates/wrathplus-app/examples/extended_launcher.rs` shows, or in a
fork. A client started through `run_with` says `extended` in its build line, so its reports read
apart from stock WrathPlus's. The crate keeps its own settings, never rows in WrathPlus's CVar table,
which every addon reads.

WrathPlus opens a piece of itself to such a crate only when its own 3.3.5a work creates that piece,
never for the crate alone, and promises no stable API: a crate pins the WrathPlus revision it
builds on. What a crate cannot reach stays a fork's.

## How a change is judged

1. `scripts/gates.sh` is green: fmt, clippy with warnings denied, the workspace tests (once
   with the client data, once without, so a test that reads the install has to declare it), the
   doc-link and render-pass lints, the player build with its own tests, and the engine boot
   checks. A pull request from a fork runs the ones that need neither the install nor a display
   in CI, on Linux; the rest run on the maintainer's machine before it lands.
2. The reference fact is stated: what 3.3.5a does, and where that is known from (the build-12340
   `Wow.exe`, a DBC field, a FrameXML line, a packet capture, AzerothCore source for a wire
   layout). The 3.3.5 interface surface is the installed-FrameXML census
   (`wrathplus-app`'s `wrath_framexml_tests`); the 1.12 tables under `reference/` serve only the
   1.12 gates that removal step 9 deletes.
3. A comment where it matters, one line, saying what the code does and the 3.3.5a fact behind it.
   No history.
4. The commit message says what changed, for a player or a developer, in one line.

## After you open it

A maintainer reads it, checks it against the reference, runs it and finishes it here: a rebase
onto main, a fix, a test or a comment, added as commits on top of yours on your branch, so leave
"Allow edits by maintainers" ticked. It lands as one squash-merged commit with you as its author.
When what would land is mostly ours, we land our own version with you as a co-author and close
yours with a note, and when main already has the fix, we close yours and say where. One that is
out of scope is closed with the reason.

## Setting up

- **The toolchain.** Stable Rust (`rust-toolchain.toml` adds clippy and rustfmt) and a C
  compiler, because the client's Lua is vendored and built from source. On macOS the Xcode
  command line tools, which also supply libclang for the audio bindings; on Linux the ALSA and
  udev development packages and pkg-config. `python3` runs two of the gates.
- **A 3.3.5a install of your own.** `WOW_DATA=<its Data folder>`, or a `WoW` link at the repo
  root, which only a dev build sees: the player build looks for `Data/` or `WoW/Data/` beside
  the binary. The client reads the install and never writes into it. `WOW_DATA=` (set, empty)
  means "no install", which is how the no-install boot is tested on a machine that has one.
  `WOW_CLIENT_PROFILE=wrath` (the only profile) and `WOW_LOCALE` (default `enUS`) select the
  archive dialect; the hermetic gates unset both.
- **Without the install, green is hollow.** Well over a thousand tests read the install and skip
  when it is absent, and a few dozen more read a corpus of 1.12 addons for the 1.12 VM tests
  (`WRATHPLUS_ADDON_CORPUS=<a folder of addons>`, or a `wow-addons-vanilla` link at the root),
  third-party content that is not in this repo. `scripts/gates.sh` and `scripts/check.sh` print
  how many tests skipped and why. Where the data is, `WRATHPLUS_REQUIRE_DATA=1` turns a skip into a
  failure, and the gates set it themselves when the install and the corpus both resolve.
- **A server to test against.** An AzerothCore 3.3.5a server; `--server` or `WOW_HOST` names it
  for the session (default `127.0.0.1:3724`, or the install's `realmlist.wtf`; the launch flags
  are in `crates/wrathplus-app/src/launch_plan.rs`). A scripted run has no default account: `WOW_USER`, `WOW_PASS` and
  `WOW_CHAR` name a test account on your server whose login kicks nobody, all three, either in
  the environment or in a `.probe-identity` file at the repo root (one per line, never
  committed), and `scripts/smoke.sh` (the live login gate) refuses without them. `WOW_CHAR`
  must already be on the account, since the client creates a character only when asked:
  `WOW_PROBE_CHARCREATE=<name> WOW_PROBE_CHARCREATE_KEEP=1`, run without `WOW_CHAR`, makes it.
  The probes drive the body with GM commands, so give that account the top GM level.
- **Running it unattended.** The rules are `docs/METHOD.md`, "The local server"; these are the
  switches.
  - `WOW_UNATTENDED=1` reconnects instead of waiting at a dialog, and exits non-zero on a login
    it cannot pass. `WOW_NOSOUND=1` runs silent. A capture (`WOW_CAPTURE`) is both, and a rig
    (`WOW_RIG`) is unattended. `WOW_ALLOW_ACCOUNT=1` overrides the account guard.
  - `WOW_RIG="tauren druid 60 gear:heal-preraid-bis spec:heal-preraid-bis at:ThunderBluff"`
    finds or creates that character on the account and applies the server's premade sets
    (`WOW_RIG="gear:?"` lists them). It was built on vmangos's `.character premade` commands and
    is not ported to AzerothCore. `WOW_PROBE_CHAT="<command>"` sends one GM
    command and logs the reply as `net: server says`. An account named `probe` and digits is
    shielded: every world entry sends `.cheat god on`, `.die` kills through it for a death test,
    and `WOW_GOD=off` leaves it off.
  - A trace is `WOW_MOVE_TRACE=<file>`, filtered with `WOW_MOVE_TRACE_TAGS` (`"move,snd,in"`) and
    ended by `WOW_PROBE_EXIT_AT=<seconds>`; grep it for the `rly` and `snd` lines.
    Add `hit` for movement obstruction diagnostics: when a frame retains less than half its
    intended horizontal travel, it replays that horizontal capsule sweep and logs the entity,
    source kind (WMO, M2, terrain, or runtime/unknown), contact normal/point and nearest triangle.
    This replay is not the resolver's exact contact history; `nearest_face` is a geometric match,
    not an exact sweep feature ID. Positions and normals use Bevy world coordinates (+Y up).
    A replay miss is logged explicitly. With `hit` filtered out, no diagnostic cast, triangle
    search or formatting runs; movement resolution is unchanged.
  - A capture runs through `scripts/visual.sh`, whose recipe is the header of
    `crates/wrathplus-app/src/capture/mod.rs`. Pixel questions go through `wrathplus-visual crop`,
    `series` and `hotspot`.
- **The loop.** `cargo play` builds and runs the play profile. `scripts/check.sh` verifies a
  round of work; `scripts/gates.sh` is the full chain, and it opens a window for the engine boot
  checks, so it needs a display. `WOW_STOCK_UI=1` boots a dev build on the stock interface
  alone, without WrathPlus's layer. Work on a branch.

## Reporting a bug

Open a [GitHub issue](https://github.com/artivus-labs/wrathplus/issues): what you did, what you
saw and what 3.3.5a does instead, with the `wrathplus build` line from the start of the terminal
output, your platform and the server you ran on.

## Where notes go

- **Open work** (bugs, playtest findings, ideas, plans, spikes) goes in
  [issues](https://github.com/artivus-labs/wrathplus/issues), tracked on the
  [WrathPlus project board](https://github.com/users/artivus-labs/projects/4). Opt-in classic+
  ideas take the `type:plus` label; stock 3.3.5 stays the default.
- **Reference** (how the stock client behaves, audits, qualification evidence, past plans) goes in
  the [developer wiki](https://github.com/artivus-labs/wrathplus/wiki).
- The repo keeps only docs the code or a new contributor needs: this file, `CLIENT-SETUP.md`,
  `METHOD.md`, the generated `MAP.md` and `WRATH-OPCODE-COVERAGE.md` (read by the replay tool).
  Don't add loose markdown notes or backlogs to `docs/`.

Working with an AI agent is expected. The agent reads `AGENTS.md`, and the same rules bind it.

### Abbey dry-room liquid regression

NSABBEY's installed 3.3.5 groups have raw MOGP `groupLiquid = 0` and no MLIQ;
these must not receive a whole-room liquid override. The native group query
`0x7c8360` rejects resolved LiquidType ID zero before testing the grid. The bounded
fix originally treated raw zero without MLIQ as dry, unconditionally. The full
Wrath decoder below now supersedes that temporary mapping for Wrath assets.
Gridless nonzero resolved IDs remain supported: MOGP `0x1000` is not required
for a legitimate whole-room flood.
Headless regressions query the player's actual indoor claim at the Abbey stuck
anchor in groups 3/5, then check the Wrath ground-controller limit; an explicit
nonzero gridless flood remains a positive control. Live traversal is manager-only.

### Wrath WMO liquid identity and query state

The build-12340 loader resolves MOGP liquid fields with retained MOHD flags before
queries consume them. MOHD `0x4` chooses direct input; otherwise raw `15` becomes
zero and other inputs increment with wrapping. Inputs 1–20 resolve by `(n-1)&3`
to IDs 13/14/19/20 (group `0x80000` selects 14 rather than 13 for class zero).
Flagged MLIQ supplies the first non-hole tile class only when this resolves to zero.
Evidence: native loader `0x7d82e0`, resolver `0x7d7310`, query `0x7c8360`.

The asset retains actual tile dimensions independently of the legacy render mesh:
all-hole grids, missing geometry, and malformed data must not turn a bounded pool
into an infinite-height room flood. A valid nonzero resolved ID with either tile
dimension zero retains the native whole-group `FLT_MAX` query. Invalid groups and
unknown catalog categories warn and do not synthesize water.

LiquidType word 3 supplies the semantic category, not its ID nibble or the
LiquidMaterial render selector. Schema/consumer references: AzerothCore
`shared/DataStores/DBCStructure.h` (`LiquidTypeEntry::Type`), `DBCfmt.h`
(`LiquidTypefmt`), `game/DataStores/DBCStores.cpp::GetLiquidFlags` (`1 << Type`),
and `game/Grids/GridTerrainData.h` water/ocean/magma/slime masks 1/2/4/8.
Catalogs load once from the same explicit boot override chain as the asset reader.
The grid is built with the resolved category (`wmo_group_liquid_mesh_of_kind`): under MOHD
`0x4` `groupLiquid` is the resolver input, not a nibble, and the nibble reading dropped every
Stormwind canal and Ironforge pool. The speed bits come from the input (`n − 1`), which reproduce
the 1.12 nibbles over the shipped cities; texture/UV/opacity/scroll/sound policy is otherwise the
legacy one, and this does not certify native liquid visual parity. Camera
submersion's existing treatment of whole-room floods is not broadened here.
