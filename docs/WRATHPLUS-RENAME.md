# Wrathplus rename plan

Approved naming plan for the one-shot rename **after native Wrath cutover acceptance**.
This document is a plan, not activation of the rename. The rename must not change rendering,
protocol, assets, gameplay or configuration semantics. **Classic remains the evaluation default;
Plus remains optional.**

## Approved compatibility decisions

- `BENILLA_HOME` stays canonical. Do not add a `WRATHPLUS_HOME` alias.
- Do not provide a legacy `benilla` executable wrapper.
- Emit old `BENILLA_GIT_*` build-stamp names as aliases when introducing the new names.
- Do not move the repository checkout as part of this work.
- Preserve existing saved-state locations and runtime environment contracts.

## Rename together

The current workspace contains 19 packages. Rename `crates/benilla` and package `benilla` to
`crates/wrathplus` and `wrathplus`. Rename each `benilla-*` directory/package to `wrathplus-*`:

`adt`, `app`, `assets`, `blp`, `buildstamp`, `bytes`, `dbc`, `formats`, `m2`, `mpq`, `protocol`,
`srp`, `ui`, `visual`, `wdt`, `wmo`, `world`, `worldview`.

Update together:

- Manifest package names, dependency keys/paths, feature forwarding, and local package identities
  in `Cargo.lock`.
- Rust `benilla_*` imports, crate-qualified references and documentation links.
- Primary binary `benilla` to `wrathplus`; viewer `benilla-worldview` to `wrathplus-worldview`.
  Inventory auto-discovered `src/bin` tools before deciding their names; prose mentioning a tool
  is not evidence of its Cargo target name.
- `.cargo/config.toml`'s `cargo play` package selection.
- Examples, integration tests, architecture checks that parse package/module names, scripts and
  CI package/binary invocations. Include check/gates/visual/smoke/showcase/cine/summon-live callers.
- Window title, CLI help, human-readable build banners and current product documentation.

Rust package and import identity changes are breaking for downstream consumers. Publish the
mapping; do not create 19 duplicate compatibility shim crates. The old executable name is not
supported after the rename. Check callers for stale binaries rather than silently launching one.

## Environment and build stamps

All `WOW_*` runtime variables remain canonical and unchanged: data discovery, authentication,
locale, client dialect, experience, capture, showcase and debug controls. Preserve the explicit
`WOW_DATA=` no-install behavior and `WOW_CAPTURE` no-player-state behavior.

Keep `BENILLA_HOME` and externally used `BENILLA_*` runtime inputs, including installed-test and
corpus controls (`BENILLA_REQUIRE_DATA`, `BENILLA_SKIP_LOG`, `BENILLA_ADDON_CORPUS`) and diagnostic
inputs such as aura tracing and reference-string reports. Inventory actual environment consumers;
Lua UI and fixture globals also use the `BENILLA_*` prefix and must not be treated as environment
variables or mechanically renamed.

Internal build stamps may move to `WRATHPLUS_GIT_SHA`, `WRATHPLUS_GIT_SHORT`,
`WRATHPLUS_GIT_DATE`, `WRATHPLUS_GIT_DESCRIBE`, `WRATHPLUS_PROFILE` and `WRATHPLUS_PROJECT_DIR`.
Update the buildstamp producer and both launcher consumers atomically. **Emit old
`BENILLA_GIT_*` names as aliases** for downstream launcher compatibility. Inventory downstream
use of `BENILLA_PROFILE` and `BENILLA_PROJECT_DIR` before removing those names; retaining emitted
aliases is safer than silently breaking a launcher. These compile-time stamps are distinct from
runtime settings.

Crate renames change tracing targets. Update in-repository `RUST_LOG=benilla_app...` recipes to
new targets and document the mapping; old filters will not automatically match. Do not silently
broaden verbosity to compensate.

## Persistent paths and contracts that stay

Use `wrathplus-config/` for fresh homes; existing `benilla-config/` homes remain selected
without mutation, even if both names exist. See [local-state compatibility](CLIENT-SETUP.md#local-state-directory-compatibility).
Keep `config.toml`, macros, bindings, saved variables, mod-pack paths and schemas. `BENILLA_HOME` retains its exact current semantics, including an
explicit empty value. Preserve shared-primary-checkout state resolution for dev worktrees and
executable-adjacent state for player builds. No automatic copy, merge, deletion, or reset.

Keep `.azerothbench-test.env` loading and ignore rules; never commit its contents. Keep `mpq://`,
archive paths, WoW/Data discovery, configuration/CVar tables, addon identities, wire protocol and
build identities. Preserve PNG/manifest/result schemas, showcase route filenames, and historical
capture artifacts/provenance.

Most script filenames are neutral and should stay. Rename build outputs and their invocations,
not the active checkout, worker worktrees, branches, or shared heavy-build lock. Repository moves
are explicitly out of scope: they can lose the WoW symlink or change stamped project/state paths.

## Documentation and attribution

Update the README product name and current commands. Replace its vanilla-only capability promise
only with verified post-cutover Wrath capabilities, not aspirations. Keep Classic/Plus labels and
WoW/reference symbols intact. Do not globally substitute `benilla`, `vanilla` or `Azerothbench`.

Preserve licenses, upstream credits/links, historical milestone documents and commit IDs.
Document filenames need not change unless product-branded; if renamed, update all inbound links
and retain redirect stubs where external links require them. `third_party/*/BENILLA.md` records
patch provenance: preserve that history and attribution rather than rewriting vendored history.

## One-shot sequence after cutover

1. Obtain native Wrath cutover acceptance and freeze worker merges for the rename window.
2. Inventory exact package, binary, environment, path, tracing, documentation and baseline
   references. Add compatibility tests before the mechanical change where needed.
3. Apply the naming-only rename atomically, with reviewable manifest/source/tooling/docs groups.
   Do not mix AO, feature-default changes, state migrations or repository moves into it.
4. Map quality baseline paths/diagnostics explicitly from old to new names. Preserve LoC
   allowances and clippy warning counts. CRAP source hashes change with import edits: re-audit
   and explain naming-only changes, not silently reset the baseline or pretend coverage improved.
5. Run script syntax/tests and documentation-link checks. Run the required hermetic
   `cargo test -p benilla-app --lib` immediately before the package rename and its equivalent
   `cargo test -p wrathplus-app --lib` afterward. Run workspace and no-default-feature gates and
   build renamed launchers/tools. Queue installed-data/census gates when appropriate.
6. The manager alone runs live Classic pixel-equality/capture checks and packaging checks,
   including legacy environment and saved-state behavior. Workers must not launch the client.
7. Merge/release only after validation; update worker instructions, branch bases and Cargo
   commands afterward. Do not push from worker branches.

Builds continue to use each worker's own target directory below 40 GB, shared heavy-build locking,
`nice -n 15`, `CARGO_BUILD_JOBS=4`, `CARGO_INCREMENTAL=0`, and tests with four threads. Renamed local
crates can duplicate substantial artifacts; budget disk space before the rebuild. Use the existing
hermetic unit-test environment (`env -u WOW_CLIENT_PROFILE -u WOW_LOCALE WOW_DATA=`).

Rollback restores naming commits and commands without moving, deleting or rewriting saved state.
Principal risks are downstream import/launcher breaks, stale executable selection, tracing-filter
changes, baseline hash invalidation, and duplicated build artifacts—not a reason to change runtime
behavior during a rename.
