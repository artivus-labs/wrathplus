//! The `framexml` kind: the stock interface's own Lua and XML, as the player's install holds it,
//! searched for how the stock UI calls the API (`QuestLogFrame.lua` reading `GetQuestLogTitle`).
//! Read once off the chain on the first search ([`wrathplus_app_ui::ui_script::stock_ui_files`])
//! and kept in memory; read-only, never written anywhere. Without an install there is nothing.

use std::sync::OnceLock;

use serde_json::{json, Value};
use wrathplus_app_ui::ui_script::StockFile;

use crate::search::Entry;

/// Lines of context around a hit, each side.
const CONTEXT: usize = 2;

/// The player's stock UI files, read on first use. Tests never read the install: they pass their
/// own synthetic files to [`search`] and [`doc`].
pub(crate) fn installed() -> &'static [StockFile] {
    if cfg!(test) {
        return &[];
    }
    static FILES: OnceLock<Vec<StockFile>> = OnceLock::new();
    FILES.get_or_init(wrathplus_app_ui::ui_script::stock_ui_files)
}

/// `function Name(` (or `Name = function(`) at the start of a trimmed line: the name.
fn defines(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let name = if let Some(rest) = t.strip_prefix("function ") {
        rest.split_once('(')?.0.trim()
    } else {
        let (lhs, rhs) = t.split_once('=')?;
        rhs.trim_start().strip_prefix("function")?;
        lhs.trim()
    };
    (!name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b':'))
    .then_some(name)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The lines `from..to` (0-based, clamped) of `lines`, each `N: text`.
fn window(lines: &[&str], from: usize, to: usize) -> String {
    let to = to.min(lines.len());
    (from.min(to)..to)
        .map(|i| format!("{}: {}", i + 1, lines[i]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every line of `files` holding `q` (lower case): a definition scores 6, any other line 7, after
/// every other kind's hits. Each entry carries the line as its doc and a few lines around it as
/// its source.
pub(crate) fn search(files: &[StockFile], q: &str) -> Vec<(u8, Entry)> {
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    for file in files {
        let lines: Vec<&str> = file.text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.to_lowercase().contains(q) {
                continue;
            }
            let def = defines(line);
            out.push((
                if def.is_some() { 6 } else { 7 },
                Entry {
                    kind: "framexml",
                    name: def.unwrap_or(basename(&file.path)).to_string(),
                    signature: String::new(),
                    doc: line.trim().chars().take(200).collect(),
                    at: format!("{}:{}", file.path, i + 1),
                    source: Some(window(&lines, i.saturating_sub(CONTEXT), i + CONTEXT + 1)),
                },
            ));
        }
    }
    out
}

/// What `name` is in the stock UI: a function it defines (its text), a file (`QuestLogFrame.lua`:
/// its functions), or a line of one (`QuestLogFrame.lua:195`: the lines around it).
pub(crate) fn doc(files: &[StockFile], name: &str) -> Option<Value> {
    let name = name.trim();
    let (file_part, line) = match name.rsplit_once(':') {
        Some((f, l)) if l.parse::<usize>().is_ok() => (f, l.parse::<usize>().ok()),
        _ => (name, None),
    };
    let lower = file_part.to_lowercase();
    let file = files.iter().find(|f| {
        let p = f.path.to_lowercase();
        p == lower || basename(&p) == lower
    });
    if let Some(file) = file {
        let lines: Vec<&str> = file.text.lines().collect();
        return Some(match line {
            Some(n) => json!({"kind": "framexml", "name": basename(&file.path),
                "at": format!("{}:{n}", file.path),
                "source": window(&lines, n.saturating_sub(11), n + 20)}),
            None => {
                let functions: Vec<String> = lines
                    .iter()
                    .enumerate()
                    .filter_map(|(i, l)| defines(l).map(|d| format!("{d} :{}", i + 1)))
                    .collect();
                json!({"kind": "framexml", "name": basename(&file.path), "at": format!("{}:1", file.path),
                    "lines": lines.len(), "functions": functions})
            }
        });
    }
    for file in files {
        let lines: Vec<&str> = file.text.lines().collect();
        if let Some(i) = lines.iter().position(|l| defines(l) == Some(name)) {
            return Some(json!({"kind": "framexml", "name": name,
                "at": format!("{}:{}", file.path, i + 1),
                "source": crate::search::body(&lines, i)}));
        }
    }
    None
}

#[cfg(test)]
#[path = "framexml_tests.rs"]
mod tests;
