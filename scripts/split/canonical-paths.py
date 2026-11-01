#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Rewrite `crate::` paths that go through a re-export to the item's canonical path.

The crate split (https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN) moves items down a layer and leaves a
`pub(crate) use crate::<home>::X;` at the old path, so callers keep compiling. A layer that
moves into its own crate can no longer reach the upper module that re-exports, so its
references must name the home. This script finds every re-export inside one crate's `src/`,
follows chains to the defining path, and rewrites references that go through them. A chain may
start with relative re-exports (`pub(crate) use child::X`, `self::`, `super::`) that lead into a
`crate::` one; past its first `crate::` hop only those are followed, as a relative re-export can
name a module private to its parent. It rewrites:

- `crate::old::X` in a path or type becomes `crate::home::X`;
- `use crate::old::{A, B};` becomes one `use` per item that moved, beside the rest.

  uv run scripts/split/canonical-paths.py                  # dry run: the rewrites, per file
  uv run scripts/split/canonical-paths.py --apply          # write them
  uv run scripts/split/canonical-paths.py --via net        # only re-exports declared in `net` (and below)
  uv run scripts/split/canonical-paths.py --home app_core  # only those whose home is under `app_core`
  uv run scripts/split/canonical-paths.py --drop-reexports # with --via: delete the re-export lines too

Python on this machine runs only through `uv run` (stdlib only; no environment to install).

It never touches comments, string literals or the re-export lines themselves (unless
--drop-reexports), and never commits. Run `cargo check` afterwards; an item the re-export renamed
(`use x::A as B`) is rewritten to its home name `A`.
"""
import argparse
import os
import re
import sys

CRATES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates")
# The root crate: `wrathplus-app` once scripts/rename-wrathplus.sh has run.
ROOT = os.path.join(
    CRATES,
    "wrathplus-app" if os.path.isdir(os.path.join(CRATES, "wrathplus-app")) else "wrathplus-app",
    "src",
)

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
USE_RE = re.compile(
    r"^(?P<indent>[ \t]*)pub(?:\(crate\))? use crate::(?P<path>(?:" + IDENT + r"::)*)"
    r"(?:\{(?P<list>[^}]*)\}|(?P<one>" + IDENT + r"(?: as " + IDENT + r")?));[ \t]*\n",
    re.M,
)
# A re-export by a relative path (`self::`, `super::`, or a child module), which only the table
# reads: the rewrite never touches these lines, but a chain through one must still resolve.
REL_USE_RE = re.compile(
    r"^[ \t]*pub(?:\([^)]*\))? use (?P<base>self::|(?:super::)+)?(?P<path>(?:" + IDENT + r"::)+)"
    r"(?:\{(?P<list>[^}]*)\}|(?P<one>" + IDENT + r"(?: as " + IDENT + r")?));",
    re.M,
)


def module_of(rel):
    """`net/objects.rs` -> `net::objects`; `cvars/mod.rs` -> `cvars`; `lib.rs` -> ``."""
    p = rel[:-3]
    parts = p.split(os.sep)
    if parts[-1] in ("mod", "lib"):
        parts = parts[:-1]
    return "::".join(parts)


def rust_files(root):
    for dp, _, fs in os.walk(root):
        for f in sorted(fs):
            if f.endswith(".rs"):
                full = os.path.join(dp, f)
                yield os.path.relpath(full, root), full


def split_items(text):
    """`A, B as C, d::E` -> [("A","A"), ("C","B"), ...]: (visible name, home name)."""
    out = []
    for raw in text.split(","):
        item = " ".join(raw.split())
        if not item or "::" in item or item == "self" or "{" in item:
            continue
        if " as " in item:
            home, name = item.split(" as ")
            out.append((name.strip(), home.strip()))
        else:
            out.append((item, item))
    return out


def relative_home(root, rel, mod, base, path):
    """The absolute module a relative `use` names, or None when its first segment is no child
    module of the declaring one (an external crate)."""
    parts = mod.split("::") if mod else []
    if base.startswith("super::"):
        ups = base.count("super::")
        if ups > len(parts):
            return None
        parts = parts[: len(parts) - ups]
    elif base != "self::":
        # A bare path: only a child module's, found on disk beside the declaring file.
        stem = rel[:-3]
        here = os.path.dirname(stem) if os.path.basename(stem) in ("mod", "lib") else stem
        first = path.split("::")[0]
        if not (
            os.path.isfile(os.path.join(root, here, first + ".rs"))
            or os.path.isfile(os.path.join(root, here, first, "mod.rs"))
        ):
            return None
    return "::".join(parts + path.split("::"))


def reexports(root):
    """`old::path::Name` -> `home::path::Name` for every `pub(crate) use crate::…` in `src/`, and for a
    relative re-export only when its target is itself a re-export (a hop in a chain): one of an
    item the child module defines is left alone, as that module may be private."""
    table, relative = {}, {}
    for rel, full in rust_files(root):
        mod = module_of(rel)
        src = open(full).read()
        for m in USE_RE.finditer(src):
            home_mod = m.group("path").rstrip(":")
            items = split_items(m.group("list")) if m.group("list") is not None else split_items(m.group("one"))
            for name, home in items:
                old = f"{mod}::{name}" if mod else name
                table[old] = (f"{home_mod}::{home}", rel)
        for m in REL_USE_RE.finditer(src):
            home_mod = relative_home(root, rel, mod, m.group("base") or "", m.group("path").rstrip(":"))
            if home_mod is None:
                continue  # an external crate (`bevy::…`), not a module of ours
            items = split_items(m.group("list")) if m.group("list") is not None else split_items(m.group("one"))
            for name, home in items:
                old = f"{mod}::{name}" if mod else name
                relative.setdefault(old, (f"{home_mod}::{home}", rel))
    # Follow chains: a home may itself be a re-export.
    # Relative hops only lead into a chain: once a `crate::` hop is taken only those follow, as a
    # relative re-export past it can name a module private to its parent. A chain with no
    # `crate::` hop resolves to nothing.
    def resolve(key, absolute=False, seen=()):
        if key in seen:
            return key if absolute else None
        if key in table:
            return resolve(table[key][0], True, seen + (key,))
        if not absolute and key in relative:
            return resolve(relative[key][0], False, seen + (key,))
        return key if absolute else None

    out = {old: (resolve(old), rel) for old, (_, rel) in table.items()}
    for old, (_, rel) in relative.items():
        home = None if old in table else resolve(old)
        if home is not None:
            out[old] = (home, rel)
    return out


def strip_noncode(src):
    """The source with comments and string literals blanked to spaces, offsets kept."""
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif src[i] == '"' and not src.startswith("'\"'", i - 1):
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            for k in range(i + 1, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out)


def rewrite(src, table, keep):
    """The rewritten source and the list of (old, new) path changes."""
    code = strip_noncode(src)
    changes = []
    edits = []  # (start, end, replacement)
    # 1. `use crate::old::{...};` lists: peel the moved items into their own `use` lines.
    list_re = re.compile(
        r"^(?P<indent>[ \t]*)(?P<vis>(?:pub(?:\([^)]*\))? )?)use crate::(?P<path>(?:"
        + IDENT + r"::)*)\{(?P<list>[^}]*)\};[ \t]*$",
        re.M,
    )
    for m in list_re.finditer(code):
        if m.group("vis"):  # a re-export line; left to --drop-reexports
            continue
        path = m.group("path").rstrip(":")
        stay, moved = [], []
        for raw in m.group("list").split(","):
            item = " ".join(raw.split())
            if not item:
                continue
            name = item.split(" as ")[0]
            key = f"{path}::{name}" if path else name
            hit = table.get(key)
            if hit and keep(key, hit[0]) and "::" not in name and name != "self":
                moved.append((item, hit[0].rsplit("::", 1)[0], name))
            else:
                stay.append(item)
        if not moved:
            continue
        ind = m.group("indent")
        lines = []
        if stay:
            inner = ", ".join(stay)
            lines.append(f"{ind}use crate::{path}::{{{inner}}};" if len(stay) > 1 or "::" in inner
                         else f"{ind}use crate::{path}::{inner};")
        for item, home_mod, name in moved:
            alias = item[len(name):]
            lines.append(f"{ind}use crate::{home_mod}::{table[f'{path}::{name}'][0].rsplit('::', 1)[1]}{alias};")
            changes.append((f"crate::{path}::{name}", f"crate::{table[f'{path}::{name}'][0]}"))
        edits.append((m.start(), m.end(), "\n".join(lines)))
    # 2. Inline paths `crate::old::Name` outside those lines.
    path_re = re.compile(r"\bcrate::((?:" + IDENT + r"::)*" + IDENT + r")")
    taken = [(a, b) for a, b, _ in edits]
    for m in path_re.finditer(code):
        if any(a <= m.start() < b for a, b in taken):
            continue
        line_start = code.rfind("\n", 0, m.start()) + 1
        line = code[line_start:code.find("\n", m.start())]
        if re.match(r"\s*pub(\([^)]*\))? use ", line):
            continue  # a re-export itself
        segs = m.group(1).split("::")
        # The longest prefix that is a re-export key, so `crate::net::Spline::new` still matches.
        for cut in range(len(segs), 0, -1):
            key = "::".join(segs[:cut])
            hit = table.get(key)
            if hit and keep(key, hit[0]):
                new = "crate::" + "::".join([hit[0]] + segs[cut:])
                edits.append((m.start(), m.end(), new))
                changes.append(("crate::" + m.group(1), new))
                break
    if not edits:
        return src, []
    out = src
    for a, b, rep in sorted(edits, reverse=True):
        out = out[:a] + rep + out[b:]
    return out, changes


def drop_reexports(src, mod, via, table, keep):
    """Delete `pub(crate) use crate::…` lines (or items) declared in a `via` module."""
    if not (mod == via or mod.startswith(via + "::")):
        return src, 0
    dropped = 0
    def sub(m):
        nonlocal dropped
        items = split_items(m.group("list")) if m.group("list") is not None else split_items(m.group("one"))
        keep_items = []
        for name, home in items:
            key = f"{mod}::{name}" if mod else name
            if key in table and keep(key, table[key][0]):
                dropped += 1
            else:
                keep_items.append(home if name == home else f"{home} as {name}")
        if not keep_items:
            return "\x00DROP\x00"
        if len(keep_items) == len(items):
            return m.group(0)
        path = m.group("path")
        inner = ", ".join(keep_items)
        body = f"{{{inner}}}" if len(keep_items) > 1 else inner
        return f"{m.group('indent')}pub(crate) use crate::{path}{body};\n"
    out = USE_RE.sub(sub, src)
    # A dropped line takes the comments and attributes directly above it.
    out = re.sub(r"(?:^[ \t]*(?://[^\n]*|#\[[^\n]*\])\n)*\x00DROP\x00", "", out, flags=re.M)
    return out, dropped


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--apply", action="store_true", help="write the rewrites (default: dry run)")
    ap.add_argument("--via", help="only re-exports declared in this module path and below")
    ap.add_argument("--home", help="only re-exports whose canonical home is under this module path")
    ap.add_argument("--drop-reexports", action="store_true", help="with --via: delete those re-exports")
    ap.add_argument("--root", default=ROOT, help="the crate's src/ (default: wrathplus-app)")
    args = ap.parse_args()
    if args.drop_reexports and not args.via:
        ap.error("--drop-reexports needs --via")
    root = os.path.normpath(args.root)
    table = reexports(root)

    def under(path, prefix):
        return path == prefix or path.startswith(prefix + "::")

    def keep(old, home):
        if args.via and not under(old.rsplit("::", 1)[0], args.via):
            return False
        if args.home and not under(home, args.home):
            return False
        return True

    total = files = dropped = 0
    for rel, full in rust_files(root):
        src = open(full).read()
        new, changes = rewrite(src, table, keep)
        n_drop = 0
        if args.drop_reexports:
            new, n_drop = drop_reexports(new, module_of(rel), args.via, table, keep)
        if new == src:
            continue
        files += 1
        total += len(changes)
        dropped += n_drop
        if not args.apply:
            for old, canon in changes:
                print(f"{rel}: {old} -> {canon}")
            if n_drop:
                print(f"{rel}: would drop {n_drop} re-exported item(s)")
        else:
            with open(full, "w") as h:
                h.write(new)
    verb = "rewrote" if args.apply else "would rewrite"
    print(f"{verb} {total} path(s) in {files} file(s)" + (f", dropping {dropped} re-export(s)" if args.drop_reexports else ""),
          file=sys.stderr)


if __name__ == "__main__":
    main()
