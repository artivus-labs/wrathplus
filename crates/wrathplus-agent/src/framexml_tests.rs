//! `framexml`: search and doc over synthetic stock-UI text (never the install's).

use super::*;

fn files() -> Vec<StockFile> {
    vec![
        StockFile {
            path: "Interface/FrameXML/LogFrame.lua".into(),
            text: "-- a synthetic log window\n\
                   function LogFrame_Update()\n\
                   \tlocal n = GetNumLogEntries()\n\
                   \tfor i = 1, n do\n\
                   \t\tlocal title, level, isHeader = GetLogTitle(i)\n\
                   \tend\n\
                   end\n\
                   LogFrame_Hide = function(self) self:Hide() end\n"
                .into(),
        },
        StockFile {
            path: "Interface/AddOns/Blizzard_Synth/Synth.xml".into(),
            text: "<Ui>\n<Frame name=\"Synth\">\n<Scripts><OnShow>GetLogTitle(1)</OnShow></Scripts>\n</Frame>\n</Ui>\n".into(),
        },
    ]
}

#[test]
fn a_search_finds_each_line_with_its_place_and_context_definitions_first() {
    let files = files();
    let found = search(&files, "getlogtitle");
    assert_eq!(found.len(), 2);
    let (score, e) = &found[0];
    assert_eq!(*score, 7);
    assert_eq!(e.kind, "framexml");
    assert_eq!(e.name, "LogFrame.lua");
    assert_eq!(e.at, "Interface/FrameXML/LogFrame.lua:5");
    assert_eq!(e.doc, "local title, level, isHeader = GetLogTitle(i)");
    let context = e.source.as_ref().unwrap();
    assert!(context.starts_with("3: \tlocal n"), "{context}");
    assert!(context.ends_with("7: end"), "{context}");
    assert_eq!(found[1].1.at, "Interface/AddOns/Blizzard_Synth/Synth.xml:3");
    // A definition names its function and ranks above a use.
    let defs = search(&files, "logframe_");
    assert_eq!(
        defs.iter()
            .map(|(s, e)| (*s, e.name.as_str()))
            .collect::<Vec<_>>(),
        [(6, "LogFrame_Update"), (6, "LogFrame_Hide")]
    );
    assert!(search(&files, "").is_empty());
}

#[test]
fn doc_reads_a_function_a_file_or_a_line() {
    let files = files();
    let f = doc(&files, "LogFrame_Update").unwrap();
    assert_eq!(f["at"], "Interface/FrameXML/LogFrame.lua:2");
    let src = f["source"].as_str().unwrap();
    assert!(
        src.starts_with("function LogFrame_Update()") && src.ends_with("\nend"),
        "{src}"
    );
    let file = doc(&files, "logframe.lua").unwrap();
    assert_eq!(file["lines"], 8);
    assert_eq!(
        file["functions"],
        json!(["LogFrame_Update :2", "LogFrame_Hide :8"])
    );
    let line = doc(&files, "Synth.xml:3").unwrap();
    assert!(line["source"].as_str().unwrap().contains("3: <Scripts>"));
    assert!(doc(&files, "Nothing").is_none());
}

/// The `framexml` kind through `search_in` and `doc_in`: after every other kind's hits, with
/// context; a stock global the UI defines reads as its FrameXML function.
#[test]
fn the_framexml_kind_ranks_after_the_others_and_doc_falls_back_to_it() {
    let dir = std::env::temp_dir().join(format!("wp-agent-fx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files = files();
    let api = || vec!["GetLogTitle".to_string()];
    let found = crate::search::search_in(
        "getlogtitle",
        &crate::search::KINDS,
        &dir,
        api,
        || &files[..],
        10,
    );
    let kinds: Vec<&str> = found["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["api", "framexml", "framexml"]);
    assert!(found["results"][1]["source"].is_string());
    assert!(found["results"][0].get("source").is_none());
    let d = crate::search::doc_in("LogFrame_Update", &dir, |_| true, || &files[..]).unwrap();
    assert_eq!(d["kind"], "framexml");
    let _ = std::fs::remove_dir_all(&dir);
}
