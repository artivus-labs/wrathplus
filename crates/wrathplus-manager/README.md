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

**Server, Profiles, Settings and combined launch remain preview controls/sample data.**
They do not query Docker, fetch modules, manage services, migrate databases or alter the realm.
The UI labels this boundary; profile selection is still a preview. Client addons have real TOC discovery, custom source folders and managed read-only links; see docs/CLIENT-SETUP.md.

The GUI uses Iced's tiny-skia software renderer, independent of Bevy/wgpu. Paste the asset folder
path or use the native **Browse…** folder picker. Existing checkout builds are detected without
recompiling; validated asset settings are restored on restart. macOS checks do not establish Linux
or Windows qualification. Branding is original artwork embedded at build time.
