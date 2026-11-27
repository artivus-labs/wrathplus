//! `stock_api`: the generated table against a fresh UI VM's globals, and the extractor on
//! synthetic source.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::stock_extract::{census_notes, is_test_path, lua_defs, scan_lua, scan_rust};

/// The globals a fresh in-game VM registers, and those the character-select (glue) VM adds.
fn registered() -> BTreeSet<String> {
    let s = wrathplus_ui::script::UiScript::new().unwrap();
    let mut out: BTreeSet<String> = crate::lua::globals(&s).into_iter().collect();
    let glue = wrathplus_ui::script::UiScript::new().unwrap();
    glue.install_glue_api().unwrap();
    glue.install_glue_create_api().unwrap();
    out.extend(crate::lua::globals(&glue));
    out
}

#[test]
fn get_quest_log_title_carries_its_returns_place_and_census_note() {
    let e = entry("GetQuestLogTitle").expect("GetQuestLogTitle");
    assert_eq!(e.kind, "api");
    assert!(e.doc.contains("isHeader"), "{}", e.doc);
    assert!(e.doc.contains("Arity (census): 0x5e5cc0"), "{}", e.doc);
    assert!(
        e.at.starts_with("crates/wrathplus-ui/src/script/quest_log.rs:"),
        "{}",
        e.at
    );
    assert!(e.source.unwrap().contains("\"GetQuestLogTitle\""));
    // Not a binding of ours: no entry.
    assert!(entry("NoSuchStockFunction").is_none());
}

/// Every global a fresh UI VM registers (but Lua's own) has a row, and every row is at a line
/// naming it; `--nocapture` prints the coverage, with and without a comment. Rows for names no
/// VM registers (a widget method, a row of a constants table) are never served: the port looks
/// rows up by the VM's own names.
#[test]
fn every_registered_global_has_a_row_at_a_line_naming_it() {
    let globals = registered();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let mut texts: BTreeMap<&str, String> = BTreeMap::new();
    for (name, at, ..) in STOCK_API {
        let (file, line) = at.rsplit_once(':').unwrap();
        let text = texts
            .entry(file)
            .or_insert_with(|| std::fs::read_to_string(format!("{root}{file}")).unwrap());
        let line = text
            .lines()
            .nth(line.parse::<usize>().unwrap() - 1)
            .unwrap();
        // A Rust registration quotes it; a Lua one (`wrath_compat.lua`, a chunk in a Rust string)
        // defines it.
        let names_it = line.contains(&format!("\"{name}\"")) || lua_defs(line).contains(name);
        assert!(names_it, "{name}: {at} reads {line:?}");
    }
    let ours: Vec<&String> = globals
        .iter()
        .filter(|g| !LUA_OWN.contains(&g.as_str()))
        .collect();
    let rows: Vec<_> = ours.iter().filter_map(|g| row(g)).collect();
    let documented = rows.iter().filter(|r| !r.2.is_empty()).count();
    let missing: Vec<&&String> = ours.iter().filter(|g| row(g).is_none()).collect();
    eprintln!(
        "stock API docs: {} registered globals, {} with a row ({documented} with a comment, {} \
         without); no row: {missing:?}; rows never served: {}",
        ours.len(),
        rows.len(),
        rows.len() - documented,
        STOCK_API.len() - rows.len(),
    );
    assert!(missing.is_empty(), "{missing:?}");
}

/// Lua 5.1's own base library, which no binding of ours registers.
const LUA_OWN: &[&str] = &[
    "assert",
    "collectgarbage",
    "dofile",
    "error",
    "gcinfo",
    "getfenv",
    "getmetatable",
    "ipairs",
    "load",
    "loadfile",
    "loadstring",
    "module",
    "newproxy",
    "next",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawset",
    "require",
    "select",
    "setfenv",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
];

#[test]
fn the_extractor_reads_the_comment_above_each_registration_shape() {
    let src = r#"
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // `0x1`: one, two.
    g.set("One", lua.create_function(|_, ()| Ok(1))?)?;

    // `0x2`: the multi-line form,
    // on two lines.
    g.set(
        "Two",
        lua.create_function(|_, unit: String| Ok(unit == "player"))?,
    )?;
    g.set("Bare", lua.create_function(|_, ()| Ok(()))?)?;

    // Each of these answers nil.
    for name in [
        "StubA",
        "StubB",
    ] {
        g.set(name, lua.create_function(|_, ()| Ok(()))?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn t() { g.set("TestOnly", lua.create_function(|_, ()| Ok(()))?)?; }
}
"#;
    let mut out = BTreeMap::new();
    scan_rust("x.rs", src, &mut out);
    let names: Vec<&str> = out.keys().map(String::as_str).collect();
    // "player" is an argument, not a name; test code is skipped.
    assert_eq!(names, ["Bare", "One", "StubA", "StubB", "Two"]);
    assert_eq!(out["One"].doc, "`0x1`: one, two.");
    assert_eq!(out["One"].at, "x.rs:6");
    assert_eq!(out["Two"].doc, "`0x2`: the multi-line form, on two lines.");
    assert_eq!(out["Two"].at, "x.rs:11");
    assert!(out["Two"].source.starts_with("g.set(\n    \"Two\","));
    assert!(out["Two"].source.ends_with(")?;"));
    // No comment directly above: an empty doc, not a neighbour's.
    assert_eq!(out["Bare"].doc, "");
    assert_eq!(out["StubB"].doc, "Each of these answers nil.");

    let lua = "-- Answers the build.\nfunction GetBuildInfo()\n  return 1\nend\nlocal function hidden() end\nSetX = function(v) end\n";
    let mut out = BTreeMap::new();
    scan_lua("c.lua", lua, &mut out);
    assert_eq!(out.keys().collect::<Vec<_>>(), ["GetBuildInfo", "SetX"]);
    assert_eq!(out["GetBuildInfo"].doc, "Answers the build.");
    assert_eq!(
        out["GetBuildInfo"].source,
        "function GetBuildInfo()\n  return 1\nend"
    );

    let census = "const SHORT_REVIEWED: &[(&str, &str)] = &[\n    (\n        \"F\",\n        \"0x1: two\",\n    ),\n];\nconst OTHER: &[&str] = &[\"G\"];\n";
    let notes = census_notes(census);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes["F"], "0x1: two");
    assert!(is_test_path("crates/wrathplus-ui/src/script/tests/a.rs"));
    assert!(is_test_path(
        "crates/wrathplus-ui/src/script/guild_tests.rs"
    ));
    assert!(!is_test_path("crates/wrathplus-ui/src/script/guild.rs"));
}

#[test]
fn the_extractor_follows_helpers_tables_lets_and_embedded_lua() {
    let src = r##"
fn install(lua: &Lua) -> mlua::Result<()> {
    // `0x3`: queued for the app.
    set(lua, "Helped", |lua, _| {
        push(lua);
        none()
    })?;
    // Lower case, plain bytes.
    let lower = lua.create_function(|_, s: String| Ok(s))?;
    other(lua)?;
    other(lua)?;
    other(lua)?;
    other(lua)?;
    g.set("lower", lower)?;
    Ok(())
}

// Rows of a verb table.
const VERBS: &[(&str, Verb)] = &[
    ("RowA", |s| {
        s.x();
    }),
    ("RowB", |s| s.y()),
    ("PLAYER_EVENT", |s| s.z()),
];

const CHUNK: &str = r#"
-- Splits a string.
function strsplit(d, s)
    return s
end
tinsert, tremove = table.insert, table.remove
"#;
"##;
    let mut out = BTreeMap::new();
    scan_rust("y.rs", src, &mut out);
    let names: Vec<&str> = out.keys().map(String::as_str).collect();
    // An event-shaped table row is not a function.
    assert_eq!(
        names,
        ["Helped", "RowA", "RowB", "lower", "strsplit", "tinsert", "tremove"]
    );
    assert_eq!(out["Helped"].doc, "`0x3`: queued for the app.");
    assert!(out["Helped"].strong);
    assert_eq!(out["lower"].doc, "Lower case, plain bytes.");
    assert!(out["lower"].source.starts_with("let lower ="));
    assert_eq!(out["RowA"].doc, "Rows of a verb table.");
    assert_eq!(out["RowB"].doc, "Rows of a verb table.");
    assert_eq!(out["RowA"].source, "(\"RowA\", |s| {\n    s.x();\n}),");
    assert!(!out["RowB"].strong);
    assert_eq!(out["strsplit"].doc, "Splits a string.");
    assert_eq!(out["tinsert"].at, "y.rs:32");
}
