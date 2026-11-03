# Managed Playerbots server (arm64 qualification)

A clean, pinned Playerbot fork + mod-playerbots build, isolated from existing
realms. This recipe does not import old characters, custom modules or credentials.
The native Manager Server page invokes this recipe asynchronously and retains
full operation logs, elapsed time and a bounded live tail. Its activity indicator
is indeterminate; a Ninja count is build tasks only, never estimated time remaining.

## Inputs and lifecycle

Docker Desktop with Compose v2 is required. The current image targets linux/arm64.
Provide a **clean** checkout with the matching module already present:

- Core `93333d6a9eb00fdf0179adde30da3c17618207d6` (Playerbot branch).
- mod-playerbots `05a1f6fceb8a32aa879b13f5f48934c6e46a774c`.

The recipe-specific `Dockerfile.dockerignore` preserves tracked placeholders that
upstream's ignore file excludes; do not weaken the revision/cleanliness guard.
CMake uses two compiler jobs, Release, and `USE_COREPCH/USE_SCRIPTPCH=OFF`.
Compose requires BuildKit additional-context support. `MODULE_SOURCES` defaults to the
core's modules directory for manual use; Manager supplies an isolated directory containing
only selected module sources. Base revision/cleanliness checks run before this overlay.
Each image build uses a fresh CMake directory; ccache remains shared. Source selection is
not runtime activation: rebuild then restart, with backups before module migrations.

```sh
export ACORE_SOURCE=/absolute/path/to/clean/core
export SERVER_RECIPE="$PWD/packaging/server"
export SPIKE_DOCKERFILE="$SERVER_RECIPE/Dockerfile"
export SERVER_DATA=/absolute/path/to/extracted/server/data
export MODULE_CONFIG=/absolute/path/to/private/module-config
mkdir -p "$MODULE_CONFIG"
# Manager creates this persistent directory and supplies MODULE_CONFIG automatically.
# Optional .conf overrides are copied privately into etc/modules on each startup.
# Edits require restart, not rebuild. API keys are never printed by the recipe.
# SERVER_DATA contains dbc/, maps/, vmaps/, optionally mmaps/; NOT client MPQs.
docker compose -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" config --quiet
docker compose --progress plain -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" build playerbots-build
docker compose -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" up -d mysql
docker compose -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" run --rm bootstrap
docker compose -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" up -d --wait --wait-timeout 900 authserver worldserver
docker compose -p wrathplus-managed-local -f "$SERVER_RECIPE/compose.yaml" stop worldserver authserver
```

MySQL 8.4 is included (pinned multi-platform image digest). It has **no published
port**. Credentials are generated into instance-owned Docker volumes, never argv,
Manager JSON or checked-in env files. Core config files containing credentials
are private per-service tmpfs files. SQL initialization grants only the four
`acore_*` schemas to the application user. `dbimport` populates/updates auth,
characters and world, including module world/characters SQL. Upstream worldserver
owns the fourth **Playerbots** schema population and update; healthy world startup
must follow dbimport. The runtime includes the pinned SQL trees at their compiled
source paths and loads module config from `/opt/acore/etc/modules`.

When the image includes **mod-ah-bot** and no saved `mod_ahbot.conf` names its own account,
worldserver startup creates an `AHBOT` account (random salt and verifier, so nobody can log
in) and one level-1 character, `Ahbot`, then points `AuctionHouseBot.Account`/`GUID` at them.
Without a saved override the seller is also switched on. If a player already took the name
`Ahbot`, the bot stays disabled and the log says so.

Auth publishes only `127.0.0.1:13724`, world only `127.0.0.1:18085`; realmlist
advertises the latter. Extracted server data is mounted read-only. The managed qualification population defaults to five online random bots
(`MANAGED_BOT_COUNT=0..5`, zero disables autologin). Periodic population inflation,
AddClass pool creation and random guild creation remain disabled. Upstream may
create ten characters per bot account; online population is bounded to five. Health requires actual
TCP listeners, not just successful container spawn. Setup/start can take minutes.

Stopping auth/world preserves MySQL and named volumes. No reset/delete action is
provided. Do not reuse a project name belonging to another installation. Do not
mount an existing MySQL data directory or attach old realm credentials. Upstream
updates run on this managed instance only; image rollback does **not** undo DB
migrations. Back up and prove restore before any future upgrade/cutover. Docker
volume access is privileged; this is local development isolation, not a hostile
multi-user security boundary.

## Qualification evidence

Native compilation: 1,848 compile/link tasks, 1h 7m 25s, no compiler warning
diagnostics found in captured log. Loader smoke exposed missing Boost runtime
libraries; six explicit Boost packages corrected it and a cached rebuild passed.
Image `sha256:feb1e4b92b768a3a04fd814bdc56a7e0b0123864bda36cc53aceb1027d3ff4d3`
(`linux/arm64`): all executable ldd checks and auth/world --help passed as nonroot
with read-only rootfs, no network and dropped capabilities.

Disposable project `wrathplus-qualification-v2`: MySQL 8.4.13 healthy; dbimport
exit 0; Playerbots populated/updated itself; auth/world TCP health checks passed.
Observed tables: auth 22, characters 111, world 315, Playerbots 28. No accounts or
characters were created. Playerbots logged successful initialization. Evidence:
`/tmp/wrathplus-db-bootstrap.log`, `/tmp/wrathplus-server-up.log` and exit markers.
Full MySQL/auth/world stop and restart passed all health checks (exit 0).
Playerbots update rows (39) and world creature templates (29,945) matched before
and after restart. All three qualification services were then stopped; volumes
remain intact. Evidence: `/tmp/wrathplus-server-restart.log` and
`/tmp/wrathplus-db-{before,after}-restart.txt`. This proves schema/data persistence,
not character-progress persistence (no accounts/characters were created).

First disposable MySQL initialization attempt failed on a host-mounted shell
script's execution permissions. The recipe now generates private SQL into a
named initialization volume instead. The failed project/volume was retained,
not silently reset. The successful qualification uses a fresh project.

Managed-local follow-up: dedicated AZBENCH/Benchone created in the new realm.
SRP auth, realm endpoint, world authentication and initial world entry passed,
then passed again after full MySQL/auth/world restart with the same character.
Five random bots logged in; upstream created ten characters in one RNDBOT account.
No normal player account or old character data was used.

A quiesced four-schema dump (auth/world stopped) was restored into a separate,
network-disabled MySQL container. All 476 table logical exports matched byte for
byte using ordered, single-row SQL output. Physical CHECKSUM TABLE differed for
16 tables despite identical logical dumps, so it was not used as the restore
acceptance criterion. The dated backup and checksum are retained privately outside Git.
Restore-check container is stopped, its volume retained. This validates this
fresh instance's backup/restore, not migration or recovery of the old realm.

## Our own modules

`modules/` holds AzerothCore modules written for WrathPlus realms, kept here until they get
repositories of their own. They appear among the standard modules on the Manager's Server modules
page, and Add registers the folder from this checkout.

- `mod-quest-minimap-points` fills the quest points AzerothCore's world database lacks, so a
  3.3.5a client shows the quest marker and its arrow on the minimap again.

## Bundled Chatter companion

Selecting `mod-llm-chatter` also enables its bundled `tools/` generation service on
Manager Start. `chatter.Dockerfile` builds an isolated uv-managed Python environment;
no host Python packages are installed. The `chatter` Compose profile exposes the
service to status/log/stop commands without making it mandatory for Playerbots-only
setups. Manager passes a bounded, isolated snapshot of the selected module as
`CHATTER_SOURCE`, rejecting working `.conf`/`.env` files, links and special files
before any companion image build.

The bridge uses a read-only root filesystem and module-config mount. `chatter.sh`
copies the config into private tmpfs, replaces DB settings with generated instance
credentials/internal MySQL routing, and drops to UID/GID 10001. Provider settings
are preserved. Startup upstream preflight performs one real provider call;
periodic DB/schema checks do not make model calls. No MySQL port is exposed.
Start/check recreates only the bridge on an already healthy realm, applying saved
bridge config. Stop includes it. Disabling the module stops it on the next Start.
A healthy bridge proves preflight and process/DB health, not in-game delivery;
verify a new grouped-player interaction and persisted identity/memory separately.
Upstream clears session work when no real players are online. Dependency versions
and the uv base tag are not fully locked; record resolved digests for releases.

Still unqualified: attended gameplay, graphical controls/folder-picker interaction,
load beyond five bots, upgrades, amd64/Windows runtime, publication, migration
and old-realm cutover. No proprietary server/client data is bundled. Ubuntu and
apt package snapshots are not fully locked; record image digests for releases.
