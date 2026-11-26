# WrathPlus's `lua-src` fork: what differs from upstream, and how to check

Upstream: [`mlua-rs/lua-src-rs`](https://github.com/mlua-rs/lua-src-rs), MIT, version `550.0.0`,
the version mlua 0.11 resolves to. Wired in through `[patch.crates-io]` in the workspace root.

## What the client's Lua is

Build 12340 (3.3.5a) ships stock Lua 5.1. Its `.rdata` carries 5.1's own messages in 5.1's
spelling: `bad argument #%d to '%s' (%s)`, `calling '%s' on bad self (%s)`,
`cannot use '...' outside a vararg function`, and `nesting of [[...]] is deprecated`
(`LUA_COMPAT_LSTR` 1). The grammar, the VM and `luaconf.h` are therefore upstream's, unchanged.

## The delta

Three differences remain:

* `src/lib.rs` has Lua 5.2/5.3/5.4/5.5 stripped from the `Version` enum (with their source trees
  deleted): WrathPlus builds 5.1 and only 5.1, and a fork that still offered the other four would
  answer a request for one with a missing directory at build time instead of a compile error.
* `lua-5.1.5/lstrlib.c` limits each native `string.find`, `string.match`, `gmatch` iterator call
  or `string.gsub` call to 100,000,000 deterministic matcher steps and 200 recursive matcher
  frames. Plain-search byte comparisons, search-position iterations and recursive/backtrack
  matcher entries consume the same per-call budget. Exhaustion raises `pattern matching work limit
  exceeded` or `pattern matching recursion limit exceeded`. This is a host safety bound, not
  client behaviour: it bounds the pathological backtracking CPU that runs below Lua's VM hook,
  which the instruction budget cannot interrupt. It is not wall-clock enforcement; `string.rep`
  is not covered, and the embedding's allocator limit handles its output growth.
* `lua-5.1.5/lbaselib.c`: `pcall` and `xpcall` put their status under the function before the
  call. Upstream pushes it afterwards with no slot reserved: a `LUA_MULTRET` call may leave the
  results filling the frame (`adjustresults`), and under `LUA_USE_APICHECK`, which `src/lib.rs`
  defines for a debug build, that push aborts the process (`lapi.c`, `api_incr_top`) once the
  results reach the argument count plus `LUA_MINSTACK` (20): 21 for `pcall(f)`. Without the check
  the push lands in stack the callee's frame had already claimed, so a release build was never at
  risk, and what Lua sees is the same in both.

The 1.12-era Lua 5.0 compatibility hunks (the table generic-for, 5.0's constructor
compat-semicolon and table-size hint, nesting long strings, backquote `LUA_QL`, and the deleted
`...`/`#`/`%` parser arms) and their `wrathplus.wrath51` runtime switch are gone with Vanilla.

To verify the Lua sources against upstream at any time:

```sh
# 550.0.0 is the pinned version; adjust the path if cargo's registry hash differs.
diff -r third_party/lua-src/lua-5.1.5 \
  ~/.cargo/registry/src/*/lua-src-550.0.0/lua-5.1.5
```

The diff is `lstrlib.c` and `lbaselib.c`.
