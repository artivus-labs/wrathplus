//! The producer walk for `every_event_a_chain_file_registers_has_a_producer`: every event name
//! production code can fire, read off the source tree. Test code is no producer. Files mounted
//! only for tests (`tests.rs`, `*_tests.rs`, `tests/`, examples, benches and `#[cfg(test)]`
//! `#[path]` children) and inline `#[cfg(test)] mod … { … }` blocks are skipped, so a test that
//! fires an event cannot stand in for a missing production fire (`QUEST_ACCEPTED`, 2026-10-24).
//!
//! A fire is recognised in these shapes:
//! - a literal at the call: `fire_event("X", …)`, `fire_event_into(lua, "X", …)`,
//!   `queue_event("X", …)`, `pending_events.push(("X", …))`;
//! - a helper whose `&str` parameter it fires (`unit_cast::fire`): every caps literal in that
//!   helper's call sites;
//! - a name the fire reads indirectly: a `const X: &str`, or a fn of that name's arms;
//! - in a file that fires indirectly: a tuple `("X", …)` however wrapped, a match arm `=> "X"`,
//!   or a block's lone tail value;
//! - anywhere in production: a queued `.push("X")` or `.push(("X", …))`, or a returned
//!   `Some("X")`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const CALLS: [&str; 4] = [
    concat!("fire_event", "("),
    concat!("fire_event_into", "("),
    concat!("queue_event", "("),
    concat!("pending_events.push", "(("),
];

/// Every event name the production sources under `root` fire, plus `constructed`.
pub(super) fn produced(root: &Path, constructed: &[&str]) -> HashSet<String> {
    let mut fired: HashSet<String> = constructed.iter().map(|s| (*s).to_string()).collect();
    let texts = production_texts(root);
    let mut indirect: HashSet<String> = HashSet::new();
    let mut helpers: HashSet<String> = HashSet::new();
    for text in &texts {
        let mut fires_indirectly = false;
        for call in CALLS {
            let mut from = 0;
            while let Some(i) = text[from..].find(call) {
                let at = from + i + call.len();
                from = at;
                let rest = text[at..].trim_start();
                let rest = rest.strip_prefix("lua,").map_or(rest, str::trim_start);
                if let Some(lit) = leading_literal(rest) {
                    fired.insert(lit.to_string());
                    continue;
                }
                fires_indirectly = true;
                let ident: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident.is_empty() {
                    continue;
                }
                if let Some(helper) = enclosing_fn_taking(&text[..at], &ident) {
                    helpers.insert(helper);
                }
                indirect.insert(ident);
            }
        }
        if fires_indirectly {
            scan_tables(text, &mut fired);
        }
        scan_queued_and_returned(text, &mut fired);
    }
    for text in &texts {
        for ident in &indirect {
            let decl = format!("const {ident}: &str = \"");
            if let Some(k) = text.find(&decl) {
                let body = &text[k + decl.len()..];
                if let Some(end) = body.find('"') {
                    fired.insert(body[..end].to_string());
                }
            }
            if text.contains(&format!("fn {ident}(")) {
                scan_tables(text, &mut fired);
            }
        }
        for helper in &helpers {
            scan_helper_calls(text, helper, &mut fired);
        }
    }
    fired
}

/// Each production `.rs` under `root`, test-only files and inline test modules removed, with
/// chains rustfmt split across lines rejoined ([`glue_chains`]).
fn production_texts(root: &Path) -> Vec<String> {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if !matches!(name, "tests" | "examples" | "benches" | "target") {
                    stack.push(p);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                files.push((p.clone(), std::fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    let test_children: HashSet<PathBuf> = files.iter().flat_map(test_path_children).collect();
    files
        .into_iter()
        .filter(|(p, _)| !test_children.contains(p))
        .map(|(_, text)| glue_chains(&strip_test_mods(&text)))
        .collect()
}

/// The files `text` mounts with `#[cfg(test)]` and `#[path = "…"]` on one declaration.
fn test_path_children((path, text): &(PathBuf, String)) -> Vec<PathBuf> {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("#[path = \"") else {
            continue;
        };
        let near = lines[i.saturating_sub(2)..(i + 3).min(lines.len())].join(" ");
        if let (Some(file), true) = (rest.split('"').next(), near.contains("#[cfg(test)]")) {
            out.push(dir.join(file));
        }
    }
    out
}

/// `text` without its `#[cfg(test)]` inline modules: the attribute, `mod name {` and the matched
/// braces, string and char literals aside.
fn strip_test_mods(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        let after = &rest[at + "#[cfg(test)]".len()..];
        let head = after.trim_start();
        let is_inline_mod = (head.starts_with("mod ") || head.starts_with("pub(crate) mod "))
            && head.find('{').is_some_and(|b| !head[..b].contains(';'));
        if !is_inline_mod {
            out.push_str(&rest[..at + "#[cfg(test)]".len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..at]);
        let open = rest.len() - head.len() + head.find('{').unwrap_or(0);
        rest = &rest[matching_brace(rest, open)..];
    }
    out.push_str(rest);
    out
}

/// The byte after the brace that closes the one at `open`, skipping string contents.
fn matching_brace(text: &str, open: usize) -> usize {
    let bytes = text.as_bytes();
    let (mut depth, mut i, mut in_str) = (0usize, open, false);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// `text` with every whitespace run before a `.` removed, so a chain rustfmt split across lines
/// reads as one call.
fn glue_chains(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            pending.push(c);
        } else {
            if c != '.' {
                out.push_str(&pending);
            }
            pending.clear();
            out.push(c);
        }
    }
    out.push_str(&pending);
    out
}

/// An event name's shape: upper-case words joined by `_`.
fn caps(lit: &str) -> bool {
    !lit.is_empty()
        && lit
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
}

/// The literal `rest` opens with, `"X"` → `X`.
fn leading_literal(rest: &str) -> Option<&str> {
    let body = rest.strip_prefix('"')?;
    body.find('"').map(|end| &body[..end])
}

/// The innermost `fn` before the end of `before`, when one of its parameters is `ident: &…`: a
/// helper that fires the name its caller passes.
fn enclosing_fn_taking(before: &str, ident: &str) -> Option<String> {
    let at = before.rfind("fn ")?;
    let sig = &before[at + 3..];
    let name: String = sig
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let params = &sig[sig.find('(')?..sig.find('{').unwrap_or(sig.len())];
    params.contains(&format!("{ident}: &")).then_some(name)
}

/// The caps literals in every call of `helper` in `text` (its definition aside).
fn scan_helper_calls(text: &str, helper: &str, fired: &mut HashSet<String>) {
    let call = format!("{helper}(");
    let mut from = 0;
    while let Some(i) = text[from..].find(&call) {
        let at = from + i;
        from = at + call.len();
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') || text[..at].ends_with("fn ") {
            continue;
        }
        let args_end = matching_paren(text, at + call.len() - 1);
        for (k, _) in text[at..args_end].match_indices('"') {
            if let Some(lit) = leading_literal(&text[at + k..]) {
                if caps(lit) {
                    fired.insert(lit.to_string());
                }
            }
        }
    }
}

/// The byte after the paren that closes the one at `open`.
fn matching_paren(text: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return open + i + 1;
                }
            }
            _ => {}
        }
    }
    text.len()
}

/// A name queued to fire later (`events.push("X")`, a pushed `("X", args)`) or handed back to the
/// firing caller (`return Some("X")`), however rustfmt wraps it.
fn scan_queued_and_returned(text: &str, fired: &mut HashSet<String>) {
    for (q, _) in text.match_indices('"') {
        let Some(lit) = leading_literal(&text[q..]).filter(|l| caps(l)) else {
            continue;
        };
        let before = text[..q].trim_end();
        if [".push(", ".push((", "Some("]
            .iter()
            .any(|s| before.ends_with(s))
        {
            fired.insert(lit.to_string());
        }
    }
}

/// The event-name table an indirect fire reads: a tuple `("X", …)` however rustfmt wraps it, a
/// match arm `=> "X"`, or a lone `"X"` as a block's tail value (an `if`/`else` choosing the name).
fn scan_tables(text: &str, fired: &mut HashSet<String>) {
    for (q, _) in text.match_indices('"') {
        let Some(lit) = leading_literal(&text[q..]).filter(|l| caps(l)) else {
            continue;
        };
        let before = text[..q].trim_end().chars().next_back();
        let after = text[q + lit.len() + 2..].trim_start().chars().next();
        if before == Some('(') && after == Some(',') {
            fired.insert(lit.to_string());
        }
    }
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    for (i, t) in lines.iter().enumerate() {
        let (t, arm_value) = match t.find("=> \"") {
            Some(k) => (t[k + 3..].trim_end_matches(','), true),
            None => (*t, false),
        };
        let Some(lit) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) else {
            continue;
        };
        let tail = lines.get(i + 1).is_some_and(|n| n.starts_with('}'));
        if (arm_value || tail) && caps(lit) {
            fired.insert(lit.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    /// The one event the host invents, with why it may: our own glue `ServerField.xml` (the custom
    /// server-address field) is its only listener, so no stock handler can hear it.
    const HOST_EVENTS: &[&str] = &["WRATHPLUS_REALMLIST_CHANGED"];

    /// Every event name production code fires by literal is one 12340 has: the in-game table (its
    /// init writes each name into its slot, `0xc24ee0`.. plus the low block at `0x6088xx`) or the
    /// glue table (`0x4dbc70`..), listed in `events_12340.txt`. A host-invented event
    /// (`FRIENDLIST_SHOW`, Wire B's contact-list storm) reaches stock handlers registered for
    /// nothing and can loop a request against its own reply. The chat feed's built names are
    /// guarded by `ui_chat`'s token test.
    #[test]
    fn every_fired_event_is_a_12340_event() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates");
        let known: std::collections::HashSet<&str> =
            include_str!("events_12340.txt").lines().collect();
        let mut alien = std::collections::BTreeSet::new();
        for text in super::production_texts(&root) {
            for call in super::CALLS {
                for (i, _) in text.match_indices(call) {
                    let rest = text[i + call.len()..].trim_start();
                    let rest = rest.strip_prefix("lua,").map_or(rest, str::trim_start);
                    if let Some(name) = super::leading_literal(rest) {
                        if !known.contains(name) && !HOST_EVENTS.contains(&name) {
                            alien.insert(name.to_string());
                        }
                    }
                }
            }
        }
        assert!(alien.is_empty(), "fired but not 12340 events: {alien:?}");
    }
}
