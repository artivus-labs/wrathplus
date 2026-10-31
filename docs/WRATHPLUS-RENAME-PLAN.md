# Wrathplus rename: execution plan

Status: prepared 2026-10-20 on `native/experience`, not executed. The approved naming policy is
[WRATHPLUS-RENAME.md](WRATHPLUS-RENAME.md); this document is its inventory, its order against the
other structural moves, and the procedure around [`scripts/rename-wrathplus.sh`](../scripts/rename-wrathplus.sh).
It is step 10 of [VANILLA-REMOVAL-PLAN.md](VANILLA-REMOVAL-PLAN.md). The manager schedules the run;
no worker executes it on a shared branch.

## Order

1. **Vanilla removal steps 3–9 land first.** They delete most of what still says 1.12 (below);
   renaming code that is about to be deleted only enlarges the rename diff and every in-flight
   rebase. The script does not depend on them and can run at any point, but the review is smallest
   after them.
2. **The rename, as one commit, during a worker pause.** The script moves 22 paths and rewrites
   about 1,330 tracked files (2026-10-20 dry run). Every open branch touches some of those files, so
   all workers merge `wrath-hud`, stop, and resume on the renamed base.
3. **Then the app crate split** ([APP-CRATE-SPLIT.md](APP-CRATE-SPLIT.md)). Rename first because:
   - The split creates seven crates. Done first, they are born `benilla-*` and renamed a commit
     later, or born `wrathplus-*` beside `benilla-*` siblings; either way 26 names move instead of
     19, and the split's review (its resolved edge list) is keyed to names that change.
   - The split document already uses post-rename names and gates itself on the rename being green.
   - Git follows a directory rename only when its files stay put. The rename is pure path moves
     plus token edits, so `git log --follow`/blame survive it; the split moves files *between*
     crates. Folding both into one change would lose the similarity tracking of both.
   - The rename is mechanical and re-runnable; the split is design work with review rounds. The
     mechanical step should not wait on the judgement-heavy one.

   A split branch prepared before the rename reconciles by running the same script on it
   (idempotent: it only rewrites old tokens) and then merging the renamed base.

## Inventory (2026-10-20, wrath-hud `3e913bc1` plus `native/experience`)

### Packages: 19, all renamed

| Old | New |
|---|---|
| `benilla` (`crates/benilla`, the launcher) | `wrathplus` (`crates/wrathplus`) |
| `benilla-{adt, app, assets, blp, buildstamp, bytes, dbc, formats, m2, mpq, protocol, srp, ui, visual, wdt, wmo, world, worldview}` | `wrathplus-{…}`, same suffix |

Rust identifiers follow (`benilla_app` → `wrathplus_app`), as do `Cargo.lock`'s local entries
(209 lines), `.cargo/config.toml`'s `cargo play` alias, and the quality baselines' paths.

### Binaries

| Target | Old | New |
|---|---|---|
| launcher `[[bin]]` | `benilla` | `wrathplus` |
| worldview `[[bin]]` | `benilla-worldview` | `wrathplus-worldview` |
| visual harness (implicit `main.rs`) | `benilla-visual` | `wrathplus-visual` |
| formats `src/bin/benilla-extract/` | `benilla-extract` | `wrathplus-extract` |
| protocol `src/bin/benilla-auth.rs` | `benilla-auth` | `wrathplus-auth` |
| protocol `src/bin/benilla-world/` (test world server) | `benilla-world` | `wrathplus-world` (shares its name with the crate, as today) |
| `mpqx`, `blpx` | unchanged | neutral names |

No `benilla` wrapper is provided (policy). The launchers outside the repository call the binary by
path, so they change in the same window (below).

### Human-visible strings

The window titles (`"benilla"`, `"benilla worldview"`), the build banner (`"benilla build …"`),
the crash-report header and the `benilla:` stderr prefixes. Product-name prose in comments and
docs ("benilla implements …") is **not** mechanically renamed: the policy forbids a global
substitution, and that prose is tidied with the docs, by hand, after the rename.

### Stays as it is

- Every `WOW_*` variable and every uppercase `BENILLA_*` token. The 13 real environment consumers
  (`BENILLA_HOME`, `BENILLA_REQUIRE_DATA`, `BENILLA_SKIP_LOG`, `BENILLA_ADDON_CORPUS`,
  `BENILLA_AURA_DUMP`, `BENILLA_AURA_TRACE`, `BENILLA_REFSTRINGS_REPORT` and the six
  `BENILLA_GIT_*`/`BENILLA_PROFILE`/`BENILLA_PROJECT_DIR` build stamps) keep their names. The
  roughly 60 other `BENILLA_*` names are Lua globals in the UI and its fixtures, not environment.
  Moving the build stamps to `WRATHPLUS_*` with old-name aliases is a separate code change with a
  test, before or after the rename, never inside the script.
- Existing `benilla-config/` and everything under it (saved state) remain untouched; fresh
  homes use `wrathplus-config/` ([compatibility](CLIENT-SETUP.md#local-state-directory-compatibility)). `/games/benilla`-style test install
  paths, the Lua registry keys (`benilla_*_methods`, `benilla_zone_text`, …), settings keys
  (`benilla_seat`, `benilla_seat_options`), temp-file prefixes (`benilla-gates`, `benilla-sv-…`).
- `third_party/` (vendored history, `BENILLA.md` provenance), historical command records
  (`` `cmd`: exit 0 `` lines), commit ids, the repository and worktree paths, branch names and the
  heavy-build lock.

### Names that still say 1.12 or Vanilla

None of these is renamed by the script; each is deleted by its removal step, and its tests with it.

| Name | Sites | Goes with |
|---|---:|---|
| `FieldLayout::Vanilla5875`, `UpdateFields_1_12_1`, `VanillaObjectType` | 636 / 21 / 10 | step 5 |
| `SessionDialect::Vanilla5875` | 3 | step 4 (the baseline warning App deletes) |
| `Opcodes_1_12_1`, `benilla-srp`'s `vanilla_header.rs` (the 1.12 header cipher) | 9 / 8 | step 3 |
| protocol `messages` (1.12 name for the shared types) | — | step 3's separate `messages` → `types` commit |
| `ScriptDialect::Vanilla`, `UiScript::new_vanilla`, test helpers `vanilla_script`/`bare_vanilla_script` | 17 / 188 | step 9 |
| `wrath_compat.lua` | 2 | step 8 |

After steps 1–9 the `wrath_` prefixes (`wrath_layouts`, `wrath_liquid`, `wrath_m2`, …) no longer
distinguish anything. Dropping them is ordinary per-owner refactoring, not part of the one-shot.

### Outside the repository

`~/Workspace/artivus/azerothbench-client/target/azb-live/`: `launch.sh` and `probe.sh` run
`target/azb-live/bin/benilla` and set `RUST_LOG=…benilla_app::…`; `cap.sh` likewise. The script
rewrites them with `--live-dir`, and the play binary must be rebuilt and copied as
`bin/wrathplus` in the same window, or `launch.sh` points at nothing. Old `RUST_LOG` filters on
`benilla_*` targets stop matching after the rename; the in-repo recipes are rewritten, private
ones are not.

CI (`.github/workflows/ci.yml`), `scripts/{gates,check,smoke,visual,cine,summon-live,soundprobe,
showcase-pair,genmap}.sh` and `scripts/quality/*` carry `-p benilla*` package lists;
the script rewrites them with everything else.

## The script

`scripts/rename-wrathplus.sh` (dry run by default; `--apply`; `--live-dir <dir>`):

1. Refuses `--apply` on a dirty tree. Never commits.
2. `git mv`s each `crates/benilla*` directory and the product-named `src/bin` targets, skipping
   any already moved, and fails if both names exist.
3. Rewrites tracked text files with an allowlist of exact tokens: the 19 package names in
   kebab and snake case, the two tool names, `crates/benilla` paths, `-p`/`--bin`/`bin/`/`debug/`/
   `release/`/`play/` binary contexts, the launcher's `name = "benilla"`, and the human-visible
   strings above. Lookaheads keep `benilla-world` off `benilla-worldview` and `benilla_ui` off
   `benilla_ui_doc`.
4. Prints the follow-up checks.

A second run reports `0 path move(s), 0 file(s)`; that is the idempotence check.

## Procedure

Before:

- [ ] Steps 3–9 merged, or the manager accepts renaming what remains.
- [ ] All workers paused with their branches merged into `wrath-hud`.
- [ ] `cargo test -p benilla-app --lib` (hermetic) green on that base, as the policy requires.
- [ ] Free disk checked: renamed crates rebuild every workspace artifact in each target directory.

Run, on the base:

```sh
scripts/rename-wrathplus.sh                       # dry run: read the list
scripts/rename-wrathplus.sh --apply --live-dir ~/Workspace/artivus/azerothbench-client/target/azb-live
scripts/rename-wrathplus.sh                       # expect 0 and 0
cargo metadata --offline --format-version 1 --filter-platform aarch64-apple-darwin >/dev/null
```

`cargo metadata --offline` without `--filter-platform` fails on this machine before and after the
rename (a Windows-only dependency is not in the local registry cache); that is not a rename fault.

After the script:

- [ ] `cargo check --workspace --tests --examples` and the hermetic
      `cargo test -p wrathplus-app --lib`, then `scripts/gates.sh`.
- [ ] `scripts/quality/run.sh clippy` workspace-wide: the clippy and LoC baselines map by path
      (rewritten); CRAP source hashes change with the import edits, so re-audit and say so rather
      than resetting the baseline.
- [ ] `scripts/doc-links.py` for links into renamed paths.
- [ ] Rebuild the play binary as `target/azb-live/bin/wrathplus`; the manager runs the live checks.
- [ ] One commit (`chore: rename benilla-* to wrathplus-*`), merged before any worker resumes.
      Workers then merge it, update their gate scripts' `-p` lists and `RUST_LOG` filters.
- [ ] The product-name prose pass over docs and comments, by hand, as a follow-up commit.

Trial on a detached worktree of `native/experience` (2026-10-20): 22 moves, 1,331 files, second
run 0 and 0, filtered `cargo metadata` clean, leftover `benilla` tokens only those listed under
"Stays" (the two recorded command lines in `AZEROTHBENCH.md` among them). On the renamed tree,
`cargo check --workspace --tests --examples` exits 0 with only the baseline `Vanilla5875`
warning, and the hermetic `cargo test -p wrathplus-app --lib` reports 4833 passed, 0 failed, 13
ignored, the count of the same run before the rename.
