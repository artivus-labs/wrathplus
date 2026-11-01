# WrathPlus Manager

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
**Profiles, Settings and combined launch remain preview controls/sample data.**
The UI labels this boundary; profile selection is still a preview. Client addons have real TOC discovery, custom source folders and managed read-only links; see docs/CLIENT-SETUP.md.

The GUI uses Iced's tiny-skia software renderer, independent of Bevy/wgpu. Paste the asset folder
path or use the native **Browse…** folder picker. Existing checkout builds are detected without
recompiling; validated asset settings are restored on restart. macOS checks do not establish Linux
or Windows qualification. Branding is original artwork embedded at build time.
