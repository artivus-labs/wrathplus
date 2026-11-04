//! The structural check on `crate::app_core::vm_memo`'s rule, from `app_core/vm_memo.rs`: a `Local`
//! in a system that holds the VM is a `VmMemo`. It scans the whole source tree, so it lives in the
//! root layer.

use crate::test_support::fn_parameter_lists;

/// A `Local` in a system that takes the VM must be a [`VmMemo`](crate::app_core::vm_memo::VmMemo) or be listed in [`EXEMPT`]: a
/// stale memo fails silently, so the check is structural.
#[test]
fn a_local_in_a_system_that_holds_the_vm_is_keyed_on_the_session() {
    let mut offenders = Vec::new();
    // The whole app: every layer crate after the split.
    for (rel, file) in crate::app_core::source_tree::rust_files() {
        let text = std::fs::read_to_string(&file).expect("readable source");
        if !text.contains("UiScript") {
            continue; // no VM in this file, so no memory about one
        }
        for params in fn_parameter_lists(&text) {
            if !params.contains("UiScript") {
                continue;
            }
            for (name, ty) in locals_in(params) {
                if ty.contains("VmMemo") || EXEMPT.contains(&(rel.as_str(), name.as_str())) {
                    continue;
                }
                offenders.push(format!("{rel}: `{name}: Local<{ty}>`"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these memos outlive the VM they are about — wrap them in \
         `crate::ui_script::VmMemo<…>` and read them through `.get(&script)`, or add them to \
         `EXEMPT` with the reason they are not memory about the VM:\n  {}",
        offenders.join("\n  ")
    );
}

/// `Local`s in VM-holding systems that are not memory about the VM, keyed
/// `(path under src/, parameter name)`.
const EXEMPT: &[(&str, &str)] = &[
    // Immutable archive-source catalog, not VM memory: keyed by weak source Arc identity.
    // The separate `seeded` VmMemo still pushes cached icons into every replacement VM.
    ("ui_macro/mod.rs", "catalog"),
    // Its build-12340 twin (both chooser lists), keyed the same way.
    ("ui_macro/mod.rs", "wrath"),
    // A raster fact; a fresh VM re-seats its measurer on `!has_text_measurer()` anyway.
    ("ui_script/extract/mod.rs", "last_seam"),
    // The plate driver's anti-overlap scratch, cleared at the top of every run.
    ("vplates.rs", "bucket"),
    // The window's scale factor, which gates the same re-seat as `last_seam`.
    ("ui_script/extract/mod.rs", "last_dpi"),
    // GetFramerate's frame-time ring: engine state in the reference (`0xcd76b0`), kept across reloads.
    ("ui_script/mod.rs", "ring"),
    // A fact about the chat roster (a defense channel joined); its push has its own `VmMemo`.
    ("world_state_ui.rs", "defense_channel"),
    // The cursor systems on both platforms track OS cursor state, which outlives the VM.
    ("cursor.rs", "was_looking"),
    ("cursor.rs", "rects_disabled"),
    ("cursor.rs", "decode_failed"),
    ("cursor.rs", "last_set"),
    ("cursor.rs", "last_ptr"),
    // The wheel's fraction toward its next notch, a fact about the device, not the VM.
    ("bindings.rs", "notches"),
];

/// Every `name: Local<Ty>` in a parameter list as `(name, Ty)`, nested generics whole.
fn locals_in(params: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, _) in params.match_indices("Local<") {
        let open = i + "Local<".len();
        let mut depth = 1usize;
        let mut close = None;
        for (j, c) in params[open..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        // Walk back over `: ` and `mut ` to the parameter's own name.
        let head = params[..i].trim_end().trim_end_matches(':').trim_end();
        let name = head
            .rsplit(|c: char| c == ',' || c == '(' || c.is_whitespace())
            .find(|s| !s.is_empty() && *s != "mut")
            .unwrap_or("<unnamed>");
        out.push((name.to_string(), params[open..close].trim().to_string()));
    }
    out
}
