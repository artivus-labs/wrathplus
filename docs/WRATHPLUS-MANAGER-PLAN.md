# WrathPlus Manager — proposed design

Architecture plan: the native Iced shell and client setup slice are implemented separately in `crates/wrathplus-manager`; see [Client setup](CLIENT-SETUP.md) for the supported workflow. Server/container bootstrap, addon/module management and multi-profile operations remain proposed, not implemented. This plan describes the wider destination, not a claim that those controls are operational.

## 1. Product boundary

A desktop control centre for playing and operating a local WrathPlus realm. The client remains independently launchable. Installing/running a server is optional: users can connect to an existing realm without Docker. External agents use the same documented CLI as humans, not a privileged hidden gameplay interface.

First supported workflow: install/select WrathPlus → select the user's own build-12340 assets → choose an existing realm or create a local realm → select a profile → Play. Never download proprietary client assets, write to the user's install, commandeer the existing development realm or silently migrate its databases.

Recommended name: **WrathPlus Manager**. Keep `wrathplus` as the game binary; `wrathplus-manager` is the desktop app and `wrathplusctl` the CLI.

## 2. GUI and technology

### Recommendation

Use **Rust + Iced** for the desktop GUI, subject to a small cross-platform UX/packaging spike. Iced's MIT licence fits the existing permissive workspace and keeps runtime/domain logic in Rust without Qt/C++ deployment. Use Tokio for asynchronous orchestration, Clap for CLI, Serde/TOML for user documents, SQLite for operation/event state, OS credential storage for secrets. Select exact dependency versions during implementation, not from this plan.

Alternatives:

| Toolkit | Why consider it | Cost / decision gate |
| --- | --- | --- |
| Iced | Rust-native, branded retained application state, permissive licensing | Prove large-list virtualization, keyboard/accessibility, native dialogs, HiDPI, IME and resource use on macOS/Windows/Linux. Do not assume accessibility completeness. |
| Qt Quick/QML + CXX-Qt | Mature QML controls/models, strong desktop tooling; Rust domain layer is feasible | Qt runtime/C++ bridge/bundling and LGPL/commercial compliance; CXX-Qt documents x86_64 CI on three desktop OSes, not proof of Apple Silicon qualification. |
| Slint | Declarative QML-like UI with Rust integration and live preview | Review exact royalty-free/GPL/commercial terms for distribution; not simply an MIT dependency. Prove complex manager tables/accessibility and packaging. |
| egui/eframe | Fast Rust implementation and already familiar in the ecosystem | Strong tools UI, but not the first choice for a polished branded consumer manager. |

No webview/backend stack and no Bevy game runtime in the manager. The manager must build and run without compiling the renderer merely to parse launch configuration. Extract lightweight shared contracts where needed rather than importing Bevy-heavy app crates.

### Visual direction

Reuse original project art:
- `packaging/icon/wrathplus-icon-1024.png`, platform icon variants.
- `measure/logo/wrathplus-logo.svg` / PNG; promote a canonical distributable wordmark into packaging during implementation rather than shipping a measurement directory.

Night-blue/slate surfaces, icy blue active states, restrained gold primary actions, clear typography and subtly framed panels. Echo the client title screen, not the density or small text of an in-game HUD. Use only original/appropriately licensed artwork. Reduced motion, scalable text, keyboard navigation, visible focus and status conveyed in text as well as colour.

Navigation:
- **Play**: logo, active profile, selected client/assets/realm, readiness, prominent Play button. First-run cards replace an empty dashboard.
- **Profiles**: create, duplicate, inspect effective settings, compare, activate, export/import.
- **Client**: builds/releases, asset/locale validation, launch options and graphics/CVars.
- **Addons & Mods**: separate client addons, asset packs and extended packages; provenance, dependencies, enable state, compatibility and safe updates.
- **Server**: overview, setup, modules, builds, players, accounts, configuration, backups, logs.
- **Activity**: persistent jobs, progress, logs, failures, recovery and next steps.
- **Settings**: storage/runtime selection, credentials, update policy and diagnostics.

Persistent header shows profile and realm; persistent activity area shows long operations without blocking navigation. Play is disabled only by actual blockers and explains how to fix them. Every change shows whether it is immediate, next-client-launch, server restart or rebuild+migration.

## 3. Shared architecture

Proposed independent Rust packages (exact split can remain small initially):

```text
wrathplus-manager-core    profiles, validation, planning, state machines, operation journal
wrathplus-manager-io      filesystem/download/git/process/keychain and Compose adapters
wrathplus-manager        Iced view models and native GUI
wrathplusctl             Clap commands and JSON/JSONL presentation
```

Separate pure domain policy from side effects. GUI and CLI call the same typed service API. No shell-string interpolation: execute fixed programs with argument arrays; validate URLs/paths/options. Do not allow manifests to add arbitrary local executable hooks by default.

No mandatory always-running daemon in the first release. Per-installation/profile locks serialize mutating jobs across GUI and CLI; SQLite leases/heartbeats plus reconciliation distinguish crashed operations from healthy running ones. Downloads use partial files and verification; builds are staged and atomically promoted. Persist job identity, planned resources, step results, cancellation points and logs. Cancellation during migration is not equivalent to cancelling a download; display an uncancellable critical section where necessary.

CLI can run synchronously, or explicitly submit a durable operation to a locally spawned worker. If detached mode is implemented, the worker owns the journal/lease; GUI exit must not silently terminate a database upgrade. Reconcile Docker/build state after a manager crash rather than trusting the old PID alone.

## 4. Installation, storage and launch contract

Use platform application-data directories, with an explicit portable root option:

```text
manager.toml                 schema version, registries, runtime, local paths
profiles/<id>/profile.toml   desired config and references
profiles/<id>/profile.lock   exact resolved component identities/hashes
profiles/<id>/client-home/   config.toml, AddOns, mods, saved state
clients/<version-platform>/  immutable versioned client installations
packages/<digest>/          verified download/source cache
servers/<id>/               recipe, source lock, build/config/log/backup metadata
state.sqlite                operations, events, installation records
```

Existing client seam: `WOW_DATA` selects Data; `WOW_LOCALE` selects locale; `BENILLA_HOME` isolates config/addons; normal login flags include `--server`, `--account`, `--realm`, `--character`, `--enter-world`, `--fill-only`, and stdin-based password input. Preserve these names for compatibility; any future `WRATHPLUS_HOME` alias needs an explicit client migration, not a manager-only assumption.

Validate assets through lightweight existing format probes (build/profile, locale and required archive/DBC/model/UI sentinels), never a path-exists check alone. Do not copy or modify the installed Data tree. Distinguish client assets from server-derived DBC/maps/vmaps/mmaps; they are not interchangeable.

Typed launch settings cover server/account/realm selection, Classic/Plus, supported graphics/CVars, locale, audio and window options. Reuse authoritative client defaults and validators via a small serializable settings contract; add schema/version handshake where needed. Do not expose every developer `WOW_*` probe or privileged harness flag as a consumer setting. Provide a redacted launch preview and an explicit advanced allowlist. Sanitize inherited environment so stale capture/autologin/probe overrides cannot defeat the selected profile.

Generate a profile's client home atomically before launch; preserve unknown settings and game-written CVar changes by importing/reconciling them, not overwriting the whole file on every Play. Separate user intent, resolved launch settings and mutable saved variables. Prevent two clients writing the same home. Do not replace a running client executable or mutate its mounted addon/pack tree.

Client downloads: use project release metadata with platform/architecture identity and pinned digest; add signature verification when publishing infrastructure supports it, not an unimplemented guarantee. Side-by-side install/update/rollback; offline launch of already verified versions. Until release artifacts are available, support an explicitly selected local binary and an opt-in developer source-build path. No surprise Rust toolchain installation for ordinary players.

Credentials live in OS credential storage; exported profiles contain references/placeholders only. Passwords never in argv, Git URLs, logs or exported Compose files. Prefer the client's existing `--password-stdin` seam for optional remembered login; default remains player login.

## 5. AzerothCore via Compose

### Supported recipes, not arbitrary combinations

First recipes: **stock AzerothCore** and **Playerbots-compatible AzerothCore**. Playerbots requires the `mod-playerbots/azerothcore-wotlk` Playerbot branch; it is not a module that can be added blindly to upstream. Resolve recipe refs to exact commits and record validated core/module/config/database-data versions. Labels must distinguish tested, declared-compatible, unverified and known-incompatible.

Use Docker Engine + Compose v2 as the first backend. Detect Docker Desktop on macOS/Windows or Docker Engine on Linux; guide installation/startup rather than silently installing a privileged runtime. Alternative engines and remote Docker are later adapters. Confirm runtime terms/licensing separately from the manager.

Generate a readable, exportable Compose project and override/config files derived from pinned upstream templates. Avoid fixed global container names; allocate instance-specific project names, volumes, networks and ports. A project-name change alone does not fix upstream fixed `container_name` collisions. Export includes lock/build inputs and secret placeholders; show equivalent commands. Clearly mark an exported project as externally managed unless explicitly adopted/locked again.

Services/steps:
1. Preflight runtime/Compose, host+engine architecture, memory/disk, ports, sources and data.
2. Resolve/fetch core/modules and construct a real self-contained build context; do not depend on symlinks outside it.
3. Build or acquire a recipe-qualified image with explicit provenance, never confuse an upstream `:master` image with Playerbots-enabled binaries.
4. Initialize MySQL and wait for database readiness.
5. Run pinned core/module migration/import jobs.
6. Prepare server data through a validated extractor/import path.
7. Start authserver/worldserver with required runtime SQL/config/data available.
8. Verify application readiness, advertised realm address and a protocol/client connection; container running alone is insufficient.

Data policy: baseline is extraction from the user's compatible install to a manager-owned output tree, keeping input read-only. Existing upstream Compose downloads pre-extracted data; do not silently adopt that path. Any optional external data provider needs explicit provenance/licensing review, user opt-in, version/digest verification and recipe compatibility. Until extraction/import is qualified, server setup is not advertised as one-click complete.

Important observed gaps in the local fork: external module symlinks are outside Docker's build context; the worldserver image does not include module SQL although Playerbots updates its fourth database at runtime. Include exact module SQL snapshots in the runtime image/read-only mount and validate all four DBs. The stock db-import step covers auth/characters/world, not proof of Playerbots bootstrap. One-shot jobs exiting successfully are healthy, not dead servers.

Security defaults: management and database access private/host-loopback; DB need not publish a host port; auth/world loopback by default with explicit LAN exposure flow and correct realm advertisement. Unique generated credentials with restricted files/runtime secret injection; no default root password for normal operation. Separate migration and runtime privileges. Treat Docker access and third-party CMake/SQL as privileged code execution, not a sandboxed addon.

Apple Silicon: do not claim native arm64 container support until built/bootstrap/login/bot-load qualified. Detect host and engine architectures; test linux/arm64 and, if necessary, offer explicit linux/amd64 emulation with cost warning. Bound build jobs by configured CPU/memory, not upstream nproc+1 alone. Windows support includes Docker/WSL prerequisites and path/filesystem tests.

## 6. Module installation and safe rebuilds

GitHub URL or curated recipe → fetch to staging → inspect expected package type/root → resolve core fork/dependencies → show permissions/build/config/SQL impact → pin commits → trust approval → build new image → back up affected DBs/config → stop/drain world → migrate → start → readiness → activate new deployment record.

Do source compilation while the old realm is running where resources allow; only the final cutover needs downtime. Module installation and migrations are not a hot reload. Block concurrent mutations to one instance; distinguish queued, building, backup, migration, starting, ready and failed. Failed build leaves the old server untouched.

Back up **auth, characters, world and Playerbots when present**, plus installed config, module/source lock and data version. Define consistency: pause world/other writers and use a database-version-appropriate dump/snapshot. Record completion and test restoration. An old image alone is not a rollback after schema migration. Only offer automatic rollback for a verified compatible schema; otherwise guide explicit snapshot restoration with impact confirmation. Never delete volumes via ordinary Stop/Uninstall; destructive purge is separate and confirmed.

Unknown module manifests/scripts are untrusted claims. Never automatically execute arbitrary SQL found anywhere in a repository. Use supported pinned migration layouts and planned, reviewable migration actions. Detect config-template changes via three-way merge and validate options rather than replacing user .conf files wholesale.

## 7. Client addons, packs and extended packages

One package inventory, distinct package kinds:
- Stock 3.3.5a Lua/XML addon (`.toc`, interface 30300, dependencies, optional deps, LoadOnDemand, saved variables).
- WrathPlus asset pack (existing loose asset overlay semantics).
- Extended WrathPlus addon (future versioned capability/API contract; do not invent a working ABI now).
- AzerothCore source module (native build, config and potentially database effects).
- Bundle tying client/server components together, e.g. a bridge addon and matching module.

Start with GitHub releases/repository refs and local folders/archives. Prefer published release assets when repositories contain development-only trees or packaging substitutions. Let users choose package roots when an archive contains several addons; preserve embedded libraries and prevent duplicate folder ownership. Check safe extraction (traversal, symlinks, case collisions, file count/size limits), HTTPS source/redirect policy, revisions/hashes and update diffs. Never run addon packaging scripts automatically. Private tokens belong in credential storage. Cache by digest and pin every resolved version; updates are explicit transactions.

CurseForge and other catalogues are later source adapters: verify current API access, game/version filters, distribution permissions and download restrictions. No scraping or redistribution workaround. A source being listed is not proof that its latest addon supports 3.3.5a.

### Compatibility and ordering

Hard checks: duplicate IDs/folder ownership, missing required dependencies, declared version/API/core-fork mismatches, asset path collisions, impossible ordering, platform/architecture and cross-client/server requirements. Warnings: outdated interface declarations, bundled library versions, unverified API use and overlapping Lua globals/hooks. Arbitrary Lua/C++ interactions cannot be proven safe statically; retain “unknown” and runtime diagnostic evidence rather than a false green certification.

Do not conflate three different orders:
1. Stock addon dependencies/optional dependencies/LoadOnDemand and current reference-compatible loader semantics. Manager can explain resolved order now; arbitrary user priority must not silently rewrite manifests or violate stock behavior. A later explicit client capability can expose supported soft precedence within dependency constraints, separately from Classic semantics. Current loader cycle behavior is not identical to a generic DAG: report cycles accurately instead of declaring all existing stock cycles universally invalid.
2. Asset pack precedence: already defined as ascending list, last matching path wins. Expose drag-to-prioritize with a per-file winner/provenance preview; reject unsafe/ambiguous paths. Preserve Classic opt-in/masking policy.
3. Native server modules: no universal load-order slider. Build dependencies, link/registration and SQL ordering are module-specific; only offer declared constraints backed by a recipe. Conflicting schema/API changes may require incompatibility, not a different order.

Install/activate client packages only with that profile's client closed. Snapshot addon sets; preserve saved variables on update/uninstall unless separately requested. Runtime diagnostics can link Lua errors to package provenance without claiming all conflicts are detected.

## 8. Profiles and environments

A profile couples client version, asset reference/locale, launch/CVar/Classic-Plus settings, addon/pack selections+priority, server recipe/module lock/config and credential references. Local paths remain machine-specific bindings; export portable IDs and prompt remapping on import.

**Server data identity is separate from a profile.** Switching profiles cannot restore or overwrite a realm database implicitly. Offer: compatible reuse of an existing server instance, attach another instance, or explicit clone from a named backup. Changing native modules/core may require rebuild+migration before activation. Preview downtime, disk cost, preserved saved state and whether online sessions block the switch.

Store desired `profile.toml` plus resolved `profile.lock`: exact client build/digest, core/module commits, container/base image digests, package hashes, schema/config version and data provenance. Secrets excluded. Export excludes proprietary assets, DB dumps and private package contents by default. Reproducibility is bounded by available artifacts and user assets; do not promise portable worlds from a small profile file.

## 9. Server operations and agent interface

Initial: start, graceful stop/restart, readiness/status, logs, resources, backup/restore, config validation, updates, rebuild history and port/realm diagnostics. Later: account creation/password reset, online/offline player search, bot population controls, scheduled shutdowns, world announcements and allowlisted administrative actions.

Use Compose for lifecycle. Use AzerothCore SOAP as an optional authenticated administrative command adapter; it is not a typed bot API or generic RCON. The fork also has RA/telnet, which should remain disabled by default. SOAP requires secure host/container bind configuration and can log command arguments; redact secrets and avoid unsafe account-command logging. Commands unavailable to console/SOAP remain unsupported, not emulated with DB writes.

Player search can combine supported commands/live telemetry with a versioned SELECT-only DB adapter for persistent/offline data. Mark snapshot time and schema compatibility. Humans/bots/agents need evidenced classifications: module-owned bot identities and an explicit agent registration/lease table or external harness metadata. Never guess agents from names or treat every playerbot as an external agent. Full bot control may require a future narrow server module/service with its own versioned contract.

CLI examples (proposed):

```sh
wrathplusctl doctor --json
wrathplusctl profile create local-plus
wrathplusctl client configure --profile local-plus --assets /path/to/Data
wrathplusctl server install --recipe playerbots --instance local --plan --json
wrathplusctl package add github:owner/repo --kind server-module --ref <commit> --profile local-plus --plan
wrathplusctl profile apply local-plus --yes --json
wrathplusctl server start local --wait --json
wrathplusctl server players local --query Bench --json
wrathplusctl launch --profile local-plus
wrathplusctl job events <id> --follow --jsonl
```

Stable versioned JSON schemas, structured error codes, deterministic exit statuses, noninteractive flags, dry-run plans and separate stdout/stderr. Long jobs stream JSONL events with job/step IDs, timestamps, progress and outcomes. Commands fail on unmet preconditions rather than prompting forever in automation. Mutating/destructive actions have explicit scope and confirmation tokens; a plan can be applied with its hash to reject stale approvals. No agent-specific backdoor, public management listener or mandatory MCP service; a later thin MCP adapter can wrap the CLI.

## 10. Delivery milestones and acceptance

| Milestone | Deliverable | Exit gate |
| --- | --- | --- |
| 0: contracts + GUI spike | Small Iced branded Play/Profile/Activity prototype; shared headless command model; platform packaging proof; document exact schema and kit decision | macOS arm64, Windows x64 and Linux smoke, file picker/HiDPI/keyboard/accessibility assessment, headless CLI build, no renderer dependency; review visual direction before expanding |
| 1: client manager | Existing binary selection plus verified release installs when available, asset preflight, typed config, isolated profiles, Play, activity/doctor | Fresh setup and existing-home import; assets remain unchanged; no secret argv/logs; game writes preserved; running binary/addon locks; update/rollback and offline launch tests |
| 2: managed local realm | Compose recipe/build/bootstrap, server data extraction/import, start/stop/readiness/logs, backup/restore | Disposable fresh stock realm and separate Playerbots realm; all required DBs incl fourth DB; protocol/login; restart preserves characters; restore drill; multi-instance naming/ports; architecture qualification before platform claims |
| 3: package + rebuild | GitHub/local addon/pack/module sources, locks/compatibility diagnostics, build/SQL plan, staged rebuild/cutover | Dependency/path/security fixtures; bridge pair; incompatible fork rejected; failed build leaves old world healthy; migration failure recovery with verified DB snapshot; saved variables preserved |
| 4: complete profiles + operations | Profile diff/apply/export/import, supported player/bot queries, account ops, schedule/status, agent-friendly jobs | Cross-machine remap; no silent world cloning; concurrent GUI/CLI mutation tests; classification provenance; crash reconciliation and cancellation tests |
| 5: ecosystem | CurseForge/other source adapters, extended addon capabilities, curated recipe catalogue, optional remote server/agent adapters | API/distribution approval, explicit capability compatibility, signed metadata where supported, remote auth/security and platform matrix |

Profiles and CLI exist in milestone 1; milestone 4 completes coupled server/module transitions rather than postponing those foundations. GitHub package management can proceed in parallel with server bootstrap once contracts are stable. Keep one reviewable PR per vertical slice, not one large launcher PR.

Testing: pure planner/manifest tests, fake process/download adapters, archive adversarial fixtures, deterministic job recovery tests, disposable Docker integration tests, platform GUI/package smoke and explicitly attended live login. Never run qualification against the current development realm by default. No generated MAP edits during planning. Existing CI repair remains separate.

## 11. References and inspected seams

Local client: `crates/wrathplus-app-core/src/launch_plan.rs`, `local_state.rs`, `data_config.rs`; `crates/wrathplus-ui/src/script/addon.rs`, `addon_gate.rs`; `docs/EXPERIENCE.md`; `README.md` server/module section; `packaging/README.md`.

Local server evidence (read-only): sibling `azerothcore-playerbots/core/docker-compose.yml`, `apps/docker/{README.md,Dockerfile,entrypoint.sh}`, `modules/mod-playerbots/{README.md,src/Script/Playerbots.cpp}`, `src/tools/dbimport`, `src/server/apps/worldserver/{ACSoap,RemoteAccess}`. External module symlinks and default image layout need correction in generated build recipes, not edits to the running development server.

Fetched primary references:
- https://www.azerothcore.org/wiki/install-with-docker
- https://www.azerothcore.org/wiki/installing-a-module
- https://github.com/mod-playerbots/mod-playerbots/wiki/Installation-Guide
- https://github.com/mod-playerbots/azerothcore-wotlk/tree/Playerbot
- https://iced.rs/ and https://raw.githubusercontent.com/iced-rs/iced/master/LICENSE
- https://kdab.github.io/cxx-qt/book/
- https://slint.dev and https://slint.dev/pricing

## 12. Recommended next action

Review this architecture and visual direction, then implement milestone 0 as a separate feature branch/PR. Default recommendation is Iced; switch to Qt Quick/QML if the spike demonstrates a material accessibility/table/desktop-integration gap. Do not begin server installation or migrate the current realm as part of the visual prototype.
