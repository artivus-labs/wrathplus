# Quality ratchets

**Enabled:** the clippy ratchet (2026-10-20) and the LoC ratchet (2026-10-22) are rungs of
`scripts/gates.sh`, `scripts/check.sh` and CI. CRAP, machete and typos stay opt-in until the
activation plan's later steps.

The Rust 1.98.1 formatting normalization migrated existing LoC allowances by exactly each
file's formatter-only line delta; it did not absorb pre-existing growth. Future allowances
continue to ratchet down, not up.

Nothing here installs a hook, modifies Rust, or rewrites a baseline automatically. Ratchets join
the existing `scripts/gates.sh` chain (fmt/clippy/test/doc-links/etc.), not a parallel gate system;
CI mirrors its bare-runner checks. Keep CRAP on demand/pre-merge and out of the fast hook.

After cutover/rename the manager can install `scripts/quality/pre-commit`; until then run checks
explicitly from the repository root. Python 3, Rust tools matching the project toolchain and
Git are required. Optional standalone tools: `cargo machete`, `typos`, `cargo llvm-cov`,
`cargo crap` (the latter two are used like Snapdragon's LCOV → cargo-crap workflow).

```sh
scripts/quality/run.sh loc                         # all tracked Rust, or explicit file paths
scripts/quality/run.sh fmt crates/wrathplus-bytes/src/lib.rs
scripts/quality/run.sh clippy -p wrathplus-bytes      # defaults to workspace/all targets
scripts/quality/run.sh machete                     # cargo-machete must be installed
scripts/quality/run.sh typos                       # typos must be installed
scripts/quality/run.sh crap -p wrathplus-bytes        # on-demand/pre-merge ONLY; real test coverage
python3 scripts/quality/test_gate.py               # cheap synthetic contract tests
```

- LoC counts physical lines in tracked `.rs` files (including tests/examples/vendor code).
  Limit 500. Generated sources listed in `gate.py`'s `GENERATED` are exempt by name. An existing offender can shrink or stay the same; new/grown violations fail.
  Deletions pass; moving an offender to a new path does not preserve its allowance.
- Formatting is `rustfmt --check`, edition 2021, `skip_children=true` for explicitly selected
  files; no recursive check of unstaged module children and no automatic formatting.
- Clippy runs `--all-targets` in warnings mode and fails on every warning the JSON allowlist
  does not name (`-D warnings` would stop each promoted crate's dependents and hide their
  diagnostics). Exact legacy diagnostics may be grandfathered by the allowlist: path, lint,
  message, source SHA-256 and multiplicity. Changing any byte
  in a file invalidates its legacy allowlist (intentionally stricter than changed functions).
  Compiler errors/tool failures cannot be grandfathered. The wrapper records JSON and filters
  promoted legacy warnings instead of globally disabling lints. No wildcard lint allowances.
- CRAP = CC² × (1 − line coverage)³ + CC, threshold 10. The initial baseline is explicitly a
  **static zero-coverage upper bound**, not a measured coverage result. It was made using
  `cargo crap --workspace --no-default-excludes --missing pessimistic` without LCOV, as
  approved while the shared build queue is busy. Once cutover completes, the manager should
  replace this with a real measured baseline. On-demand runs always generate fresh LCOV with
  `cargo llvm-cov --lib`; failed tests/export never reuse old data. The scorer includes examples
  and tests as well as production functions; code not executed by lib tests is pessimistic.
  Existing unchanged files can score at most their baseline; **any changed/new file's functions
  must all score ≤10**. File SHA-256 matching is a conservative alternative to incorrectly
  matching renamed/nested functions by a line number; changed files lose all exceptions.
  Same-named functions use a multiset of allowances. Scores are not truncated or filtered to
  hide violations. Full coverage is opt-in and can require a large fresh instrumented target.
- `machete` and `typos` are strict standalone tools, not silently skipped if absent. `machete`
  scans `crates/` unless given paths (vendored `third_party/` is not ours to prune); `typos`
  reads the repository's `_typos.toml` (Blizzard's own spellings, domain words, the verbatim
  `reference/*.tsv` excluded). CRAP refuses to start without `llvm-tools-preview`, which
  `cargo llvm-cov` would otherwise prompt to install, hanging the run. No blanket
  exemptions or implicit installs. They are not required for the tooling unit's Python tests.

The dormant hook exports the entire **index snapshot** to an ephemeral directory, then runs only
LoC, rustfmt and typos on staged added/modified/renamed files. It neither builds nor runs CRAP,
clippy, dependency analysis or client code. It never stages fixes, updates the index, or checks
unstaged fixes as if they were committed. Gate code/baselines are taken from the staged snapshot
also. Bootstrap: commit tooling before installing it. Deleted files are skipped. No hook is
installed by these scripts, and no hook runs merely by committing this unit.

Heavy standalone clippy/coverage commands take a slot through the machine's
`target/azb-live/heavy.sh` (two slots; override with `AZB_HEAVY`), else `lockf -k
/tmp/azb-heavy.lock`; a caller already holding one sets `AZB_QUALITY_LOCKED=1`. They use `nice -n 15`,
`CARGO_BUILD_JOBS=4`, `CARGO_INCREMENTAL=0`, and hermetic fixture environment. `CARGO_TARGET_DIR`
must be set to your own directory (`run.sh` refuses without it), **never another worker's**. Check the 40-GiB cap/free space before a run; coverage must wait for its turn. Artifacts
are left in a fresh `/tmp/azb-quality-report.*` directory for diagnosis, never committed.

Fresh baselines and the activation plan: `https://github.com/artivus-labs/wrathplus/wiki/QUALITY-GATES-ACTIVATION`.

## Initial inventory (base 68529487)

LoC: **1,766 tracked Rust files; 786,664 physical lines; 530 files >500 lines**.
Static CRAP: **13,933 reported functions; 5,198 upper-bound scores >10**. These are NOT measured
coverage violations. Baseline contains only offenders, with source hashes; changed files must
meet the threshold rather than getting a raised baseline. Clippy: **30 warning diagnostics, 12 distinct lint codes**, workspace/all-targets exit 0
(before promotion with `-D warnings`). Baselines are committed ratchet budgets and may only shrink;
do not regenerate them to accept a failure. After rename, make one explicit reviewed mechanical
path migration rather than silently resetting debt.
