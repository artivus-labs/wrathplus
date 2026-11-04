# WrathPlus Manager

## First run

1. **Install.** Build and open the Manager: `make launcher`, then `make run`. The platform
   prerequisites (Rust, Git, a C toolchain; Docker for a local server) are in
   [`docs/CLIENT-SETUP.md`](../../docs/CLIENT-SETUP.md).
2. **Client assets.** On **Client**, choose the **Data** folder of your own WoW 3.3.5a
   (build 12340) install and validate it, then **Build client**. No game files are supplied.
3. **Local server** (optional; skip it to play on an existing realm). Start Docker, open
   **Server** and follow *Set up your own server* from the top:
   - **Get server source** downloads AzerothCore with Playerbots (about 1 GB, a minute or two).
   - Choose a **server type** (*Playerbots* is recommended) and **Build**; the first build takes
     about an hour.
   - **Prepare server data** reads maps from your client (a few minutes, about 1 GB).
   - **Start**, then **Use this server for the client**.
   - Optionally **Make navigation meshes** so bots and agents find their way (several hours;
     you can play meanwhile, then restart the server).
4. **Play.** On **Play**, press **Launch game** and log in with an account on that realm. (The
   Manager cannot yet create a player account on the local realm.)

The rest of this page is detail for each part.

## Detail

Native Rust/Iced desktop Manager. **Client setup is operational**: validate a user-provided,
legally obtained 3.3.5a/build 12340 Data folder, compile the player client, persist setup and launch
it on an explicit click. The GUI and `--setup` CLI use the same Rust setup operations.

```sh
make launcher
make run # alias: make manager
make doctor # build-prerequisite checklist
# Or complete client setup without opening a window or launching the game:
make setup DATA="/path/to/Data" LOCALE=enUS SERVER=127.0.0.1:3724
```

Full instructions and boundaries: [`../../docs/CLIENT-SETUP.md`](../../docs/CLIENT-SETUP.md).

No original game files are supplied or downloaded. Assets remain read-only. Builds run off the UI
thread and retain compiler logs; failed builds preserve the previous managed executable. Launch
uses explicit data/locale/server settings, not stored account credentials or unattended probes.

**Server** manages only the isolated `wrathplus-managed-local` Compose project using
`packaging/server`: build image, bootstrap/start, stop auth/world (retain DB), status
and retained logs. Select extracted server data (read-only) and a clean core checkout.
Auth/world use loopback ports 13724/18085; adopting or resetting an existing realm is not
supported. Play → Start starts the managed server directly without navigating away and
switches to Stop while running. Setup opens the configuration page with separate server
controls, file settings and build controls. Activity holds Manager operation history,
client build output and server logs; other pages keep contextual progress/errors and
links to Activity rather than a global activity footer. Both pages share
running/starting/stopped/error status.
Health is checked on Manager startup and every ten seconds across pages, including servers
started outside Manager. Workers keep Iced responsive; readiness requires Compose healthy services.
The single build action shows **Build** when the local image is missing and **Rebuild**
when present; Docker lookup failures stay unknown, not "missing". Successful builds record
image ID, source path and sorted module directory names in `build-receipt.json`. Polling
warns on added/removed modules or an unverified existing image; it never retroactively
marks an old image current. This initial inventory tracks module presence, not edits within
modules, CMake enablement or running-server activation. Rebuild, then restart to apply new
modules. The legacy `reuse_image` JSON field remains readable but no longer skips compilation.
Server settings live in `manager-server.json`, separate from `manager.json`, in the selected
local-state home; full private logs live under `managed-server/wrathplus-managed-local/manager-logs`.
The animated activity bar is not a completion estimate; Ninja counts are shown only when reported.
Use the explicit connection button to change the client endpoint; it never launches the game.
**Addons** now separates Client addons and Server modules (no coupled-package view).
Both accept local folders or explicit public GitHub `owner/repo` / HTTPS imports. Git is
required for imports; network work is off-thread, bounded and credential-free, with resolved
revision retained. Imports never update existing checkouts or activate running code.
Server module selection persists separately. Playerbots is selected by default but optional;
all modules can be included/excluded for the next build. Build/Rebuild is available directly
on the module page. Modules and Server setup show build progress and elapsed time, then a
clear success/failure result with next steps. The last build result and its optional technical
details remain available through automatic status checks and other server actions in the current
Manager session; they are replaced by the next build result. No restart is automatic.
Config files are detected beneath each module automatically, including disabled modules.
**Edit config** opens a modal, preferring the existing managed `.conf` over its `.conf.dist`
template. Opening or cancelling never creates a file; **Save** creates the local copy when
needed, leaving the original template untouched. Dirty edits require explicit discard before
closing. Saves are atomic, private (0600 on Unix), size-limited and reject externally changed files. Config contents are
not included in Activity. These overrides persist under the managed instance's `module-config/`
directory and are mounted read-only, then privately copied into the container at startup.
Restart to apply config edits; no rebuild is needed for config-only changes. Playerbots database
credentials and bounded population settings remain Manager-owned. When mod-llm-chatter is selected,
Start also builds and starts its bundled Python bridge as a companion Compose service. The
**Start / check LLM bridge** action provisions it on an already healthy realm without restarting
MySQL/auth/world. Stop includes the bridge; deselecting Chatter stops its bridge on the next Start.
The bridge gets a private runtime config with instance DB credentials and internal `mysql` routing;
provider/model/key settings remain in the user's managed config. Startup probes the configured
provider, and recurring health checks verify DB/schema without paid LLM calls. Activity includes
bridge logs and Server setup shows service status. Config edits apply to the bridge on the next
Start/check; worldserver config changes still require an explicit realm restart. Manager stages only selected module directories into
an isolated Docker additional context; it never edits the chosen core or module sources.
The build uses a fresh CMake directory so deselection cannot reuse stale configuration.
Rebuild and restart are explicit; disabling does not roll back database migrations.
Playerbots plus `Wishmaster117/mod-multibot-bridge` are the standard single-player foundation;
the module page provides a named bridge source shortcut. Playerbots is discovered from the core;
external modules still require explicit import, selection and rebuild. Existing bridge selections
are preserved. Neither a shortcut nor a selected module proves running-image inclusion.
LLM chatter and guildhouse shortcuts are separate integrations. Review code before building.

Chatter Companion's **Known bots** roster is not a nearby-unit or party list. Its `.llmc roster`
command selects bots with `llm_bot_memories` rows for the current player. Grouping and interactions
processed by the separate LLM generation bridge create persistent identities/memories; merely
seeing or whispering a bot does not guarantee roster membership. The server module only queues
work and delivers results; it does not call the LLM itself. An empty roster with queued events,
no generated messages and no identities points to an absent or failing generation bridge.
`mod-multibot-bridge` serves MultiBot controls and cannot replace that service. Standard Playerbots
chat can still respond while LLM generation is offline. Check the separate bridge service health
and startup provider probe; do not interpret a healthy worldserver as healthy LLM generation.
**Profiles, Settings and combined launch remain preview controls/sample data.**
The UI labels this boundary; profile selection is still a preview. Client addons have real TOC discovery, custom source folders and managed read-only links; see docs/CLIENT-SETUP.md.

The GUI uses Iced's tiny-skia software renderer, independent of Bevy/wgpu. Paste the asset folder
path or use the native **Browse…** folder picker. Existing checkout builds are detected without
recompiling; validated asset settings are restored on restart. macOS checks do not establish Linux
or Windows qualification. Branding is original artwork embedded at build time.
