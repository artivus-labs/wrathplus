# AzerothBench fork of benilla

Fork-specific scope: read `https://github.com/artivus-labs/wrathplus/wiki/AZEROTHBENCH` first. This fork targets Wrath
3.3.5a/build 12340; upstream's vanilla-only feature scope below describes the
baseline, not the fork's destination. Preserve upstream engineering and asset
provenance rules. The owner authorized current-realm testing and service restarts
on 2026-10-02. Use dedicated test credentials, preserve player state and back up
before migrations; no destructive resets without specific authorization. External
harnesses own policies/training; the client provides ordinary human play and
low-level, declared observation/control interfaces.

A from-scratch World of Warcraft 1.12.1 client in Rust and Bevy. The reference client is the
spec: benilla is a faithful, modern implementation of it, not a place to get creative.

Read, in this order:

1. `docs/METHOD.md` — the rules and how we work. Binding for every session.
2. `docs/MAP.md` — what is built right now, generated from the code. Orient here before
   touching anything.
3. `docs/CONTRIBUTING.md` — what gets in and how a change is judged.

The rest of the root is code and data: `crates/` the workspace, `reference/` the 1.12 name
catalogues the tests check against, `scripts/` the gates and instruments, `third_party/`
vendored code.

@docs/METHOD.md
