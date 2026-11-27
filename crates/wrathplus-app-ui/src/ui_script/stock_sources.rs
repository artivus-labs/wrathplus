//! The stock interface's own text, as the player's install holds it, read-only: the files
//! `FrameXML.toc` lists and the `Blizzard_*` addons' (each `.toc` row, and every `.lua` and `.xml`
//! an `<Include file=>` or `<Script file=>` names), for an agent to search for how the stock UI
//! calls the API. Read off the chain on demand; nothing is copied anywhere, and without an install
//! the list is empty.

use std::collections::HashSet;

use wrathplus_ui::toc::Toc;

use super::addons::WRATH_BLIZZARD_ADDONS;
use super::reference_ui;

/// One stock UI file: its install path (`Interface/FrameXML/QuestLogFrame.lua`) and its text.
pub struct StockFile {
    pub path: String,
    pub text: String,
}

/// Every stock UI file off the player's chain: the core's in toc order, then each Blizzard addon's.
pub fn stock_ui_files() -> Vec<StockFile> {
    stock_ui_files_from(&|path| reference_ui::read(path))
}

/// [`stock_ui_files`] over any reader, by `\`-separated install path.
pub(crate) fn stock_ui_files_from(read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<StockFile> {
    let mut roots: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(toc) = read(reference_ui::TOC) {
        let files = Toc::parse(&wrathplus_ui::source::decode(&toc)).files;
        roots.push(("Interface/FrameXML".to_string(), files));
    }
    for name in WRATH_BLIZZARD_ADDONS {
        if let Some(toc) = read(&format!(r"Interface\AddOns\{name}\{name}.toc")) {
            let files = Toc::parse(&wrathplus_ui::source::decode(&toc)).files;
            roots.push((format!("Interface/AddOns/{name}"), files));
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (base, files) in roots {
        for file in files {
            add(
                read,
                &wrathplus_ui::loader::join_ref(&base, &file),
                &mut seen,
                &mut out,
            );
        }
    }
    out
}

/// One file, then (for an `.xml`) the files it includes, relative to its own directory.
fn add(
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    path: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<StockFile>,
) {
    if !seen.insert(path.to_lowercase()) {
        return;
    }
    let Some(bytes) = read(&path.replace('/', "\\")) else {
        return;
    };
    let text = wrathplus_ui::source::decode(&bytes).into_owned();
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    let includes = if path.to_lowercase().ends_with(".xml") {
        referenced_files(&text)
    } else {
        Vec::new()
    };
    out.push(StockFile {
        path: path.to_string(),
        text,
    });
    for file in includes {
        add(
            read,
            &wrathplus_ui::loader::join_ref(&dir, &file),
            seen,
            out,
        );
    }
}

/// The `file="…"` of each `<Script>` and `<Include>` tag in a FrameXML document.
fn referenced_files(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in xml.match_indices('<') {
        let tag = &xml[i + 1..];
        let is_ref = ["Script", "Include"].iter().any(|t| {
            tag.get(..t.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(t))
                && tag[t.len()..].starts_with(|c: char| c.is_whitespace())
        });
        if !is_ref {
            continue;
        }
        let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
        if let Some(at) = tag.find("file=\"") {
            let rest = &tag[at + 6..];
            if let Some(end) = rest.find('"') {
                out.push(rest[..end].to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic files only: a toc, an XML that includes a script, and the script.
    #[test]
    fn the_toc_rows_and_the_files_their_documents_name_are_read_once() {
        let files: &[(&str, &str)] = &[
            (
                r"Interface\FrameXML\FrameXML.toc",
                "## Interface: 30300\nA.lua\nB.xml\n",
            ),
            (r"Interface\FrameXML\A.lua", "function A_Fn() end\n"),
            (
                r"Interface\FrameXML\B.xml",
                "<Ui><Script file=\"B.lua\"/><Include file=\"A.lua\"/><Frame name=\"X\"/></Ui>",
            ),
            (
                r"Interface\FrameXML\B.lua",
                "function B_Fn() return A_Fn() end\n",
            ),
        ];
        let read = |p: &str| -> Option<Vec<u8>> {
            files
                .iter()
                .find(|(path, _)| *path == p)
                .map(|(_, t)| t.as_bytes().to_vec())
        };
        let got = stock_ui_files_from(&read);
        let paths: Vec<&str> = got.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "Interface/FrameXML/A.lua",
                "Interface/FrameXML/B.xml",
                "Interface/FrameXML/B.lua"
            ]
        );
        assert!(got[2].text.contains("B_Fn"));
        // No install, no files.
        assert!(stock_ui_files_from(&|_| None).is_empty());
    }
}
