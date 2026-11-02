//! Vanilla-removal step 1's guard (`https://github.com/artivus-labs/wrathplus/wiki/VANILLA-REMOVAL-PLAN`): no build-12340 module decodes
//! through the 1.12 parser. Every `wrath_*` source names neither `parse_server*` nor the 1.12
//! `events::decode`, so deleting `messages/parse.rs` cannot break a Wrath arm.

use std::path::Path;

/// Every `.rs` file under `dir` whose path has a `wrath_` component, recursively.
fn wrath_sources(dir: &Path, inside: bool, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let wrath = inside || name.starts_with("wrath_");
        if path.is_dir() {
            wrath_sources(&path, wrath, out);
        } else if wrath && name.ends_with(".rs") && name != "wrath_native_codec_guard_tests.rs" {
            out.push(path);
        }
    }
}

#[test]
fn no_wrath_module_decodes_through_the_1_12_parser() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    wrath_sources(&src, false, &mut files);
    assert!(files.len() > 50, "the walk found the Wrath modules");
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| {
            let text = std::fs::read_to_string(f).expect("read");
            ["parse_server", "events::decode", "events::{decode"]
                .iter()
                .any(|needle| text.contains(needle))
        })
        .map(|f| f.strip_prefix(&src).unwrap_or(f).display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "these Wrath sources still decode through the 1.12 parser: {offenders:?}"
    );
}
