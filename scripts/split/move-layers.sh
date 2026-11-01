#!/usr/bin/env bash
# Move wrathplus-app's modules into one crate per layer (https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN), in the
# checkout $PWD is in. The layers and their modules are scripts/split/layers.txt; every ui_*
# module not listed elsewhere is ui; the rest stays in wrathplus-app, the root.
#
#   scripts/split/move-layers.sh [--layers "core net"] [--drop-reexports] [--widen] [--test-support]
#
# For each layer, in dependency order: crates/wrathplus-app-<layer> with wrathplus-app's package,
# dependencies and features, a path dependency on every lower layer crate (and a dev-dependency
# with `test-support`, so a lower crate's test helpers reach this crate's tests), its modules
# `git mv`'d with their `#[path]` siblings, and a lib.rs that declares them `pub mod` and
# re-exports every lower layer's modules at its root (`pub(crate) use benilla_app_core::net;`),
# so a moved `crate::net::…` path still resolves. The root declares `pub use benilla_app_x::m;`
# in place of each moved `mod m;` and depends on every layer crate, forwarding `dev` and
# `test-support`.
#
# First, `canonical-paths.py --apply` rewrites every path through a re-export to its home;
# `--drop-reexports` then deletes each module's re-exports (a glob `use super::*` that relied on
# one breaks; the dry run counts them). `--widen` turns every `pub(crate)` in the layer crates
# into `pub`, the blanket widening; without it the compiler names each item a crate boundary hides.
# `--test-support` makes each `test_support` module's items `pub`: they are the shared test
# helpers' home. Any other `#[cfg(test)]` helper a dependent crate's tests reach (a constructor, a
# probe method, `local_state::test_env`) is opened by `widen.py`, from the compiler's
# "configured out" note, to `#[cfg(any(test, feature = "test-support"))]`; the rest stay
# `cfg(test)`, so the lib build has no test-only dead code.
#
# It never commits and never builds. Run it on a throwaway worktree for a dry run, or on the move
# commit's tree, then `cargo check` bottom-up: `-p wrathplus-app-core`, `-net`, `-world`, `-ui`,
# `-tools`, then `-p wrathplus-app`, each `--lib --tests`. The crates take the root's prefix:
# `wrathplus-app-<layer>` once scripts/rename-wrathplus.sh has run, which goes first.
#
# It starts only from a committed tree with no merge in progress: a half-moved layer's crate would
# count as moved on a re-run. After a failed layer, `git reset --hard` to the last good commit and
# `git clean -fd crates/`, then run that layer again.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "$root"
P=benilla
[ -d crates/wrathplus-app ] && P=wrathplus
app=crates/$P-app
src=$app/src
layers="core net world ui tools"
drop=0
widen=0
test_support=0
while [ $# -gt 0 ]; do
    case "$1" in
        --layers) layers="$2"; shift 2 ;;
        --drop-reexports) drop=1; shift ;;
        --widen) widen=1; shift ;;
        --test-support) test_support=1; shift ;;
        *) echo "usage: $0 [--layers \"core net …\"] [--drop-reexports] [--widen] [--test-support]" >&2; exit 2 ;;
    esac
done
if [ -n "$(git status --porcelain)" ] || [ -e "$(git rev-parse --git-path MERGE_HEAD)" ]; then
    echo "move-layers: uncommitted or untracked changes, or a merge in progress; commit or restore first" >&2
    exit 1
fi

mods_of() { grep "^$1:" scripts/split/layers.txt | sed "s/^$1: //"; }
all_mods="$(grep -E '^(pub(\(crate\))? )?mod [a-z_0-9]+;' $src/lib.rs | sed -E 's/.*mod ([a-z_0-9]+);/\1/')"
listed=" $(for l in core net world ui tools; do mods_of $l; done | tr '\n' ' ') "
extra_ui=""
for m in $all_mods; do
    case $m in ui_*) [[ "$listed" == *" $m "* ]] || extra_ui+=" $m" ;; esac
done
# A layer's modules: its line, and for ui every unlisted ui_* module.
of() { mods_of "$1"; [ "$1" = ui ] && echo "$extra_ui"; true; }
# The `#[cfg(…)]` line above a module's declaration in the root lib.rs, if any.
cfg_of() { grep -B1 -E "^(pub(\(crate\))? )?mod $1;" $src/lib.rs | head -1 | grep '^#\[cfg' || true; }

uv run scripts/split/canonical-paths.py --apply
# A merge may have brought an import that crosses layers.txt upward: stop before moving anything.
uv run scripts/split/layer-edges.py || { echo "move-layers: upward edges (above); fix layers.txt or the import, then restore the tree" >&2; exit 1; }
if [ $drop = 1 ]; then
    for m in $all_mods; do
        uv run scripts/split/canonical-paths.py --via "$m" --drop-reexports --apply 2>/dev/null || true
    done
fi

# The layers already moved, lowest first: an earlier run's crates count, so each layer can be its
# own commit (`--layers core`, then `--layers net`, …).
lower=""
new=""   # the layers this run moves
for l in core net world ui tools; do
    [[ " $layers " == *" $l "* ]] && continue
    [ -f crates/$P-app-$l/Cargo.toml ] && lower+=" $l"
done
for l in $layers; do
    crate=$P-app-$l
    dst=crates/$crate
    mkdir -p "$dst/src"
    # Cargo.toml: wrathplus-app's, renamed, with the lower crates as dependencies.
    deps=""; devdeps=""; devf=""; tsf=""
    for b in $lower; do
        deps+="$P-app-$b = { path = \"../$P-app-$b\", default-features = false }"$'\n'
        devdeps+="$P-app-$b = { path = \"../$P-app-$b\", default-features = false, features = [\"test-support\"] }"$'\n'
        devf+=", \"$P-app-$b/dev\""
        tsf+="\"$P-app-$b/test-support\", "
    done
    ROOTPKG="$P-app" CRATE="$crate" DEVF="$devf" TSF="${tsf%, }" DEPS="$deps" DEVDEPS="$devdeps" perl -0pe '
        s/^name = "\Q$ENV{ROOTPKG}\E"/name = "$ENV{CRATE}"/m;
        s/^description = [^\n]*\n//m;
        # The root already depends on the layers an earlier run moved: start from its own list.
        s/^\Q$ENV{ROOTPKG}\E-\w+ = [^\n]*\n//mg;
        s/,?\s*"\Q$ENV{ROOTPKG}\E-\w+\/(?:dev|test-support)"//g;
        s/^(dev = \[[^\]]*)\]/$1$ENV{DEVF}]/m;
        s/^test-support = \[\]/test-support = [$ENV{TSF}]/m;
        s/^\[dependencies\]\n/[dependencies]\n$ENV{DEPS}/m;
        s/^\[dev-dependencies\]\n/[dev-dependencies]\n$ENV{DEVDEPS}/m' $app/Cargo.toml > "$dst/Cargo.toml"
    {
        echo "//! The $l layer of $P-app: see https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN."
        echo '#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]'
        echo
    } > "$dst/src/lib.rs"
    for m in $(of $l); do
        c="$(cfg_of "$m")"
        [ -n "$c" ] && echo "$c" >> "$dst/src/lib.rs"
        echo "pub mod $m;" >> "$dst/src/lib.rs"
        # The module's files, and the `#[path]` siblings at src/ they name, transitively: a
        # sibling may itself be a module root with test siblings (`ui_cast` -> `ui_spell_targeting`
        # -> its tests).
        sibs=""
        todo="$src/$m.rs $(find $src/$m -name '*.rs' 2>/dev/null || true)"
        while [ -n "${todo// /}" ]; do
            set -- $todo; f="$1"; shift; todo="$*"
            [ -f "$f" ] || continue
            [ "$(dirname "$f")" = "$src" ] || [ "$f" = "$src/$m.rs" ] || case "$f" in $src/$m/*) ;; *) continue ;; esac
            for p in $( { grep -oE '#\[path = "[^"/]+\.rs"\]' "$f" || true; } | sed -E 's/#\[path = "(.*)"\]/\1/'); do
                [ "$(dirname "$f")" = "$src" ] && [ -f "$src/$p" ] || continue
                case " $sibs " in *" $p "*) continue ;; esac
                sibs+=" $p"
                todo+=" $src/$p"
            done
        done
        for p in $sibs; do git mv "$src/$p" "$dst/src/$p"; done
        [ -f $src/$m.rs ] && git mv $src/$m.rs "$dst/src/$m.rs"
        [ -d $src/$m ] && git mv $src/$m "$dst/src/$m"
        perl -0pi -e "s/^(?:pub(?:\(crate\))? )?mod $m;\n/pub use ${P}_app_$l::$m;\n/m" $src/lib.rs
    done
    if [ -n "$lower" ]; then
        { echo; echo "// Every lower layer's modules at this crate's root, so \`crate::<module>::…\` resolves."; } >> "$dst/src/lib.rs"
        for b in $lower; do
            for m in $(of $b); do
                c="$(git show HEAD:$src/lib.rs | grep -B1 -E "^(pub(\(crate\))? )?mod $m;" | head -1 | grep '^#\[cfg' || true)"
                [ -n "$c" ] && echo "$c" >> "$dst/src/lib.rs"
                echo "#[allow(unused_imports)]" >> "$dst/src/lib.rs"
                echo "pub(crate) use ${P}_app_${b}::$m;" >> "$dst/src/lib.rs"
            done
        done
    fi
    git add "$dst"
    lower+=" $l"
    new+=" $l"
done

# The root depends on every layer crate and forwards its features; its own tests get each layer
# crate's test helpers through a dev-dependency with `test-support`, as the layer crates do.
deps=""; devdeps=""; devf=""; tsf=""
for b in $new; do
    deps+="$P-app-$b = { path = \"../$P-app-$b\", default-features = false }"$'\n'
    devdeps+="$P-app-$b = { path = \"../$P-app-$b\", default-features = false, features = [\"test-support\"] }"$'\n'
    devf+=", \"$P-app-$b/dev\""
    tsf+="\"$P-app-$b/test-support\", "
done
DEVF="$devf" TSF="${tsf%, }" DEPS="$deps" DEVDEPS="$devdeps" perl -0pi -e '
    s/^(dev = \[[^\]]*)\]/$1$ENV{DEVF}]/m;
    s/^test-support = \[([^\]]*)\]/"test-support = [" . join(", ", grep { length } $1, $ENV{TSF}) . "]"/me;
    s/^\[dependencies\]\n/[dependencies]\n$ENV{DEPS}/m;
    s/^\[dev-dependencies\]\n/[dev-dependencies]\n$ENV{DEVDEPS}/m' $app/Cargo.toml

# The shipped UI tree goes with `ui_script` (its `include_dir!` and source-tree reads name its own
# crate), and the dev resolver in `run_mode` (core) names that crate's folder.
case " $lower " in
    *" ui "*)
        if [ -d $app/assets ]; then
            git mv $app/assets crates/$P-app-ui/assets
            UI="$P-app-ui" perl -0pi -e 's/Some\(std::path::Path::new\(env!\("CARGO_MANIFEST_DIR"\)\)\)/Some(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "\/..\/$ENV{UI}")))/' \
                crates/$P-app-core/src/run_mode.rs
        fi
        ;;
esac

if [ $widen = 1 ]; then
    for b in $lower; do
        find crates/$P-app-$b/src -name '*.rs' -exec perl -pi -e 's/\bpub\(crate\)/pub/g' {} +
    done
fi
if [ $test_support = 1 ]; then
    # A `test_support` module is the shared helpers' home; its items are for other crates' tests.
    t=0
    for b in $lower; do
        for f in $(find crates/$P-app-$b/src -path '*test_support*' -name '*.rs'); do
            c=$(grep -c 'pub(crate)' "$f" || true)
            perl -pi -e 's/\bpub\(crate\)/pub/g' "$f"
            t=$((t + c))
        done
    done
    echo "test-support: $t test_support items made pub"
fi
echo "moved: $(for b in $lower; do echo -n "$b=$(echo $(of $b) | wc -w | tr -d ' ') "; done)"
