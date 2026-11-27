//! The stock API's docs from our own bindings, read off their source: `build.rs` runs it over
//! `wrathplus-ui/src/script` and the arity census, and `stock_api` serves the table it writes.
//!
//! A binding's doc is the comment written above its registration, in our words from the 12340
//! decompile (`0x5e5cc0: title, level, tag, ...`); none is invented. A registration is a quoted
//! name where one is registered: next to a `create_function` (`g.set("Name",
//! lua.create_function(...))`, its multi-line form, a `for name in ["A", "B"]` loop over one), a
//! helper's `set(lua, "Name", …)`, or a global function of `wrath_compat.lua` ([`Found::strong`]);
//! or, weaker, a function-shaped name leading a table row (`("GetArmorPenetration", |s| …)`) that
//! a helper loops over. The table is an index by name: the port serves a row only for a name the
//! VM actually registers, and `stock_api_tests` checks every registered name has one. Test code
//! (`*tests*.rs`, `tests/`, an inline `#[cfg(test)] mod`) is skipped.

use std::collections::BTreeMap;

/// One registered stock function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// `crates/wrathplus-ui/src/script/quest_log.rs:221`.
    pub at: String,
    /// The comment above the registration, without its slashes; empty when it has none.
    pub doc: String,
    /// The registration's text, from its statement's first line, at most [`SNIPPET_LINES`].
    pub source: String,
    /// Found where a registration is sure (next to a `create_function`, a helper's
    /// `(lua, "Name"`, a Lua definition), not only shaped like one (`("Name", …)` in a table).
    pub strong: bool,
}

/// The longest registration snippet kept.
pub const SNIPPET_LINES: usize = 24;

/// How far around a quoted name a `create_function` makes it a registration.
const NEAR: usize = 3;

fn ident(s: &str) -> bool {
    s.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// Every `"Ident"` literal on a code line (not in a comment) placed where a registration puts its
/// name: first in a call or tuple (`set("Name", …)`, `("Name", f)`, a line of its own), after a
/// helper's `lua,` (`set(lua, "Name", …)`: `true` with the name), or, in a `for` list
/// (`in_list`), after a comma. Literals deeper in a call (`UnitIs(unit, "player")`) are
/// arguments, not names.
fn literals(line: &str, in_list: bool) -> Vec<(&str, bool)> {
    let code = match line.find("//") {
        // A `//` inside a string ("http://") is rare in bindings; cut at the first.
        Some(i) => &line[..i],
        None => line,
    };
    let mut out = Vec::new();
    let mut at = 0;
    let pieces: Vec<&str> = code.split('"').collect();
    // Odd pieces are inside quotes (no escaped quotes in a bare identifier).
    for (k, piece) in pieces.iter().enumerate() {
        if k % 2 == 1 && k + 1 < pieces.len() && ident(piece) {
            let before = code[..at - 1].trim_end();
            let after = pieces[k + 1].trim_start();
            // A helper's `set(lua, "Name", …)`, or the globals table's own `g.set("Name", f)`.
            let helper = [
                "(lua,",
                "(&lua,",
                "g.set(",
                "globals.set(",
                "globals().set(",
            ]
            .iter()
            .any(|h| before.ends_with(h));
            let leads = before.is_empty()
                || helper
                // A struct field (`get: "GetBaseMip"`) or a tuple's later name
                // (`("TurnLeftStart", "TurnLeftStop", …)`).
                || before.ends_with(':')
                || before.ends_with("\",")
                || before.ends_with('(')
                || before.ends_with('[')
                || (in_list && before.ends_with(','));
            let closes = after.is_empty()
                || after.starts_with(',')
                || after.starts_with(')')
                || after.starts_with(']');
            if leads && closes {
                out.push((*piece, helper));
            }
        }
        at += piece.len() + 1;
    }
    out
}

/// The lines that are not test code: `None` for those inside an inline `#[cfg(test)] mod … {}`.
fn live_lines(text: &str) -> Vec<Option<&str>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<Option<&str>> = lines.iter().map(|l| Some(*l)).collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" {
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim_start().starts_with("#[") {
                j += 1;
            }
            let head = lines.get(j).map_or("", |l| l.trim());
            if (head.starts_with("mod ") || head.starts_with("pub mod ")) && head.ends_with('{') {
                let mut depth = 0i32;
                let mut k = j;
                while k < lines.len() {
                    depth += lines[k].matches('{').count() as i32;
                    depth -= lines[k].matches('}').count() as i32;
                    k += 1;
                    if depth <= 0 {
                        break;
                    }
                }
                for slot in &mut out[i..k] {
                    *slot = None;
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The first line of the statement holding line `i`: up through the lines that carry it on (a
/// `g.set(` opening, the names of a `for` list), stopping at a comment, a blank line or a line
/// that ends a statement.
fn statement_start(lines: &[Option<&str>], i: usize) -> usize {
    let mut start = i;
    while start > 0 && i - start < 8 {
        let Some(prev) = lines[start - 1] else { break };
        let t = prev.trim();
        let carries_on = !t.is_empty()
            && !is_comment(prev)
            && (t.ends_with('(')
                || t.ends_with('[')
                || t.starts_with("#[")
                || matches!(t, "lua," | "&lua,")
                // A sibling name of a list, not the end of a tuple or call before it.
                || (t.starts_with('"') && t.ends_with("\",")));
        if !carries_on {
            break;
        }
        start -= 1;
    }
    start
}

/// The `//` comment block directly above line `start`, joined, without slashes.
fn comment_above(lines: &[Option<&str>], start: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut j = start;
    while j > 0 {
        match lines[j - 1] {
            Some(l) if is_comment(l) => {
                out.push(l.trim_start().trim_start_matches('/').trim());
                j -= 1;
            }
            _ => break,
        }
    }
    out.reverse();
    out.retain(|l| !l.is_empty());
    out.join(" ")
}

/// The statement from `start` until its brackets close on a line ending it, at most
/// [`SNIPPET_LINES`].
fn snippet(lines: &[Option<&str>], start: usize) -> String {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for l in lines[start..].iter().take(SNIPPET_LINES) {
        let Some(l) = l else { break };
        out.push(*l);
        depth += l.matches(['(', '[', '{']).count() as i32;
        depth -= l.matches([')', ']', '}']).count() as i32;
        let t = l.trim_end();
        if depth <= 0 && (t.ends_with(';') || t.ends_with('}') || t.ends_with("),")) {
            break;
        }
    }
    let indent = out
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    out.iter()
        .map(|l| l.get(indent..).unwrap_or(l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The registrations in one Rust file (`file` is its repo path), into `out` (the first one of a
/// name wins: files are scanned in path order).
pub fn scan_rust(file: &str, text: &str, out: &mut BTreeMap<String, Found>) {
    let lines = live_lines(text);
    // Lua chunks in Rust strings (`stdlib.rs`, `wrath_surface.rs`) define globals too.
    scan_lua_lines(file, &lines, true, out);
    for (i, line) in lines.iter().enumerate() {
        let Some(line) = line else { continue };
        if is_comment(line) || !line.contains('"') {
            continue;
        }
        let start = statement_start(&lines, i);
        let in_loop = lines[start].is_some_and(|l| l.trim_start().starts_with("for "));
        let names = literals(line, in_loop);
        if names.is_empty() {
            continue;
        }
        let window = |from: usize, to: usize| {
            lines[from..to.min(lines.len())]
                .iter()
                .flatten()
                .any(|l| l.contains("create_function"))
        };
        // Next to a `create_function`, or a `for` list one registers below.
        let near =
            window(i.saturating_sub(NEAR), i + NEAR + 1) || (in_loop && window(start, start + 40));
        // `set(\n    lua,\n    "Name",`: a helper's multi-line call.
        let after_lua = i > 0 && lines[i - 1].is_some_and(|l| matches!(l.trim(), "lua," | "&lua,"));
        for (name, helper) in names {
            let strong = near || helper || after_lua;
            if !strong && !function_like(name) {
                continue;
            }
            let (doc, source) = match bound_above(&lines, i, line, name) {
                // `g.set("strreplace", strreplace)`: the doc is the `let`'s, above.
                Some(at) if comment_above(&lines, start).is_empty() => {
                    let s = statement_start(&lines, at);
                    (comment_above(&lines, s), snippet(&lines, s))
                }
                // A table row's text is the row, not its table from the top.
                _ if !in_loop && lines[start].is_some_and(|l| l.trim_end().ends_with('[')) => {
                    (comment_above(&lines, start), snippet(&lines, i))
                }
                _ => (comment_above(&lines, start), snippet(&lines, start)),
            };
            // A row of a list (`for (name, …) in [ … ]`, `const VERBS: &[…] = &[ … ]`) with no comment
            // of its own shares the list's.
            let doc = match enclosing_list(&lines, i) {
                Some(j) if doc.is_empty() => comment_above(&lines, statement_start(&lines, j)),
                _ => doc,
            };
            let found = Found {
                name: name.to_string(),
                at: format!("{file}:{}", i + 1),
                doc,
                source,
                strong,
            };
            match out.get(name) {
                Some(have) if have.strong || !strong => {}
                _ => {
                    out.insert(name.to_string(), found);
                }
            }
        }
    }
}

/// The line opening the list that line `i` sits in (one ending in `[`, as `for name in [` or
/// `const VERBS: &[…] = &[`), by its brackets, within [`LIST_REACH`] lines and inside one item.
fn enclosing_list(lines: &[Option<&str>], i: usize) -> Option<usize> {
    let mut open = 0i32;
    for j in (i.saturating_sub(LIST_REACH)..i).rev() {
        let l = lines[j]?;
        if is_comment(l) {
            continue;
        }
        open += l.matches('[').count() as i32 - l.matches(']').count() as i32;
        if open > 0 && l.trim_end().ends_with('[') {
            return Some(j);
        }
        // An item's first line (`fn`, `const` …) without the list: not in one.
        if !l.is_empty() && !l.starts_with(char::is_whitespace) {
            return None;
        }
    }
    None
}

/// How far above a row its list may open.
const LIST_REACH: usize = 120;

/// For `g.set("Name", value)` with `value` a local: the line of its `let value =` above (within
/// [`LET_REACH`] lines), whose comment documents the binding.
fn bound_above(lines: &[Option<&str>], i: usize, line: &str, name: &str) -> Option<usize> {
    let rest = line.split_once(&format!("\"{name}\","))?.1.trim_start();
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if value.is_empty() || !value.starts_with(|c: char| c.is_ascii_lowercase()) {
        return None;
    }
    let lets = [format!("let {value} ="), format!("let {value}:")];
    (i.saturating_sub(LET_REACH)..i).rev().find(|&j| {
        lines[j].is_some_and(|l| lets.iter().any(|p| l.trim_start().starts_with(p.as_str())))
    })
}

/// How far above a `g.set` its value's `let` may be.
const LET_REACH: usize = 200;

/// A name shaped like a stock function: a capital first, and a small letter somewhere (not an
/// event's `PLAYER_LOGIN`).
fn function_like(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.bytes().any(|b| b.is_ascii_lowercase())
}

/// The globals one Lua line defines: `function Name(`, or the plain names an assignment sets
/// (`tinsert, tremove = table.insert, table.remove`, `X = X or none`); none for a `local`.
pub fn lua_defs(line: &str) -> Vec<&str> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("function ") {
        return rest
            .split_once('(')
            .map(|(n, _)| n.trim())
            .filter(|n| ident(n))
            .into_iter()
            .collect();
    }
    let Some((lhs, _)) = t.split_once(" = ") else {
        return Vec::new();
    };
    let names: Vec<&str> = lhs.split(',').map(str::trim).collect();
    if lhs.starts_with("local ") || !names.iter().all(|n| ident(n)) {
        return Vec::new();
    }
    names
}

/// The global functions a Lua chunk defines (`lua_defs`), with the `--` comment above each: a
/// `.lua` file, or (`embedded`) the Lua inside a Rust string, whose lines end with no `;`.
fn scan_lua_lines(
    file: &str,
    lines: &[Option<&str>],
    embedded: bool,
    out: &mut BTreeMap<String, Found>,
) {
    for (i, line) in lines.iter().enumerate() {
        let Some(line) = line else { continue };
        let t = line.trim_end();
        if embedded && (t.ends_with(';') || t.trim_start().starts_with("let ") || t.contains("::"))
        {
            continue;
        }
        let names = lua_defs(line);
        if names.is_empty() {
            continue;
        }
        let mut doc: Vec<&str> = lines[..i]
            .iter()
            .rev()
            .map_while(|l| l.filter(|l| l.trim_start().starts_with("--")))
            .map(|l| l.trim_start().trim_start_matches('-').trim())
            .collect();
        doc.reverse();
        doc.retain(|l| !l.is_empty());
        let end = if t.trim_start().starts_with("function ") {
            lines[i..]
                .iter()
                .take(SNIPPET_LINES)
                .position(|l| {
                    l.is_some_and(|l| l.trim() == "end" || l.trim_end().ends_with(" end"))
                })
                .map_or(SNIPPET_LINES, |j| j + 1)
        } else {
            1
        };
        let source: Vec<&str> = lines[i..(i + end).min(lines.len())]
            .iter()
            .flatten()
            .copied()
            .collect();
        for name in names {
            if out.get(name).is_some_and(|f| f.strong) {
                continue;
            }
            out.insert(
                name.to_string(),
                Found {
                    name: name.to_string(),
                    at: format!("{file}:{}", i + 1),
                    doc: doc.join(" "),
                    source: source.join("\n"),
                    strong: true,
                },
            );
        }
    }
}

/// The global functions a `.lua` file (`wrath_compat.lua`) defines.
pub fn scan_lua(file: &str, text: &str, out: &mut BTreeMap<String, Found>) {
    let lines: Vec<Option<&str>> = text.lines().map(Some).collect();
    scan_lua_lines(file, &lines, false, out);
}

/// The arity census's reviewed SHORT notes (`("GetQuestLogTitle", "0x5e5cc0: the tenth ...")`),
/// name to note: why a binding answers fewer values than the stock Lua unpacks.
pub fn census_notes(text: &str) -> BTreeMap<String, String> {
    let Some(from) = text.find("const SHORT_REVIEWED") else {
        return BTreeMap::new();
    };
    let block = &text[from..];
    let block = &block[..block.find("\n];").unwrap_or(block.len())];
    let strings: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
    strings
        .chunks(2)
        .filter(|p| p.len() == 2 && ident(p[0]))
        .map(|p| (p[0].to_string(), p[1].to_string()))
        .collect()
}

/// Whether a source path is test code, skipped whole.
pub fn is_test_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.split('/').any(|p| p == "tests")
        || path.rsplit('/').next().is_some_and(|f| f.contains("tests"))
}
