//! `search`: the built-in index, ranking, the agent's own modules and `doc`.

use super::*;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wp-agent-search-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(found: &Value) -> Vec<String> {
    found["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

/// Every `Agent*` function registered from Rust has an entry, and every entry is registered, at
/// the line it names.
#[test]
fn the_agent_functions_and_their_registrations_are_in_step() {
    let mut registered: Vec<String> = Vec::new();
    for (_, _, _, _, text) in AGENT_FNS {
        for line in text.lines() {
            let mut rest = line;
            while let Some(i) = rest.find("\"Agent") {
                let tail = &rest[i + 1..];
                let end = tail.find('"').unwrap_or(0);
                let name = &tail[..end];
                // `wp!(…, "Agent")` names a module's global table, not a function.
                if ident(name)
                    && !line.trim_start().starts_with("//")
                    && !line.contains("format!")
                    && !line.trim_start().starts_with("wp!(")
                {
                    registered.push(name.to_string());
                }
                rest = &tail[end..];
            }
        }
    }
    registered.sort();
    registered.dedup();
    let mut listed: Vec<String> = AGENT_FNS.iter().map(|f| f.0.to_string()).collect();
    listed.sort();
    assert_eq!(registered, listed);
    for e in builtins().iter().filter(|e| e.name.starts_with("Agent")) {
        let (file, line) = e.at.split_once(':').unwrap();
        let text = AGENT_FNS.iter().find(|f| f.3 == file).unwrap().4;
        let at: usize = line.parse().unwrap();
        assert!(
            text.lines()
                .nth(at - 1)
                .unwrap()
                .contains(&format!("\"{}\"", e.name)),
            "{}",
            e.at
        );
    }
}

#[test]
fn a_module_function_has_its_signature_doc_place_and_source() {
    let all = builtins();
    let pick_up = all
        .iter()
        .find(|e| e.name == "Quests.pick_up")
        .expect("Quests.pick_up");
    assert_eq!(pick_up.signature, "Quests.pick_up()");
    // No comment above it: the module header's entry is its doc.
    assert!(
        pick_up.doc.starts_with("Quests.pick_up()"),
        "{}",
        pick_up.doc
    );
    // A header entry runs on through its continuation lines (the shared note after hand_in).
    let hand_in = all.iter().find(|e| e.name == "Quests.hand_in").unwrap();
    assert!(hand_in.doc.contains("AGENT_QUESTS_DONE"), "{}", hand_in.doc);
    let (file, line) = pick_up.at.split_once(':').unwrap();
    assert_eq!(file, "wp/quest.lua");
    let src = crate::env::built_ins()
        .iter()
        .find(|b| b.0 == "wp.quest")
        .unwrap()
        .2;
    assert!(src
        .lines()
        .nth(line.parse::<usize>().unwrap() - 1)
        .unwrap()
        .starts_with("function Q.pick_up("));
    assert!(pick_up
        .source
        .as_ref()
        .unwrap()
        .starts_with("function Q.pick_up("));
    // A module with no global is named by the module.
    assert!(all.iter().any(|e| e.name.starts_with("wp.values.")));
    // A comment above the function is its doc; the body runs to its own `end`.
    let src = "-- Pick a target.\n-- The nearest.\nfunction M.pick(a, b)\n  if a then\n    return a\n  end\n  return b\nend\nlocal x = 1\nreturn M\n";
    let found = lua_functions("builtin", "wp.test", Some("Test"), "wp/test.lua", src);
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].name.as_str(), found[0].signature.as_str()),
        ("Test.pick", "Test.pick(a, b)")
    );
    assert_eq!(found[0].doc, "Pick a target. The nearest.");
    assert_eq!(found[0].at, "wp/test.lua:3");
    assert!(found[0]
        .source
        .as_ref()
        .unwrap()
        .ends_with("  return b\nend"));
}

#[test]
fn names_come_first_then_docs_then_source() {
    let dir = scratch("rank");
    let found = search("pick_up", &["builtin"], &dir, Vec::new, 50);
    let got = names(&found);
    assert_eq!(got[0], "Quests.pick_up", "{got:?}");
    // Every name hit comes before every doc-only hit.
    let first_doc_only = found["results"].as_array().unwrap().iter().position(|e| {
        !e["name"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("pick_up")
    });
    if let Some(i) = first_doc_only {
        assert!(
            got[i..]
                .iter()
                .all(|n| !n.to_lowercase().contains("pick_up")),
            "{got:?}"
        );
    }
    // Case does not matter, and the result is the same every time.
    assert_eq!(search("PICK_UP", &["builtin"], &dir, Vec::new, 50), {
        let mut again = search("pick_up", &["builtin"], &dir, Vec::new, 50);
        again["query"] = json!("pick_up");
        again
    });
    // An Agent* function by its name.
    assert_eq!(
        names(&search("agentservices", &["builtin"], &dir, Vec::new, 3))[0],
        "AgentServices"
    );
}

#[test]
fn own_modules_are_found_by_name_and_line_and_the_api_by_name() {
    let dir = scratch("own");
    std::fs::create_dir_all(dir.join("grind")).unwrap();
    std::fs::write(
        dir.join("grind/loop.lua"),
        "local M = {}\nfunction M.start()\n  Goals.on = true\nend\nreturn M\n",
    )
    .unwrap();
    let found = search("goals.on", &["own"], &dir, Vec::new, 10);
    assert_eq!(found["results"][0]["name"], "grind.loop");
    assert_eq!(found["results"][0]["at"], "grind/loop.lua:3");
    assert_eq!(found["results"][0]["doc"], "Goals.on = true");
    // The module by its name: its first line.
    let by_name = search("grind", &["own"], &dir, Vec::new, 10);
    assert_eq!(by_name["results"][0]["at"], "grind/loop.lua:1");
    // The VM's globals: names only, ranked, the Agent* ones left to the built-ins.
    let api = || {
        vec![
            "UnitName".to_string(),
            "UnitNameUnmodified".to_string(),
            "GetUnitName".to_string(),
            "AgentNearby".to_string(),
        ]
    };
    let found = search("unitname", &["api"], &dir, api, 10);
    assert_eq!(
        names(&found),
        ["UnitName", "UnitNameUnmodified", "GetUnitName"]
    );
    assert_eq!(found["results"][0]["kind"], "api");
}

#[test]
fn doc_reads_a_builtin_an_own_module_or_an_api_name() {
    let dir = scratch("doc");
    std::fs::write(dir.join("autoexec.lua"), "Goals.on = true\n").unwrap();
    let pick_up = doc("Quests.pick_up", &dir, |_| false).unwrap();
    assert_eq!(pick_up["kind"], "builtin");
    assert!(pick_up["source"]
        .as_str()
        .unwrap()
        .starts_with("function Q.pick_up("));
    // The bare name when it is one function; several, the candidates.
    assert_eq!(
        doc("pick_up", &dir, |_| false).unwrap()["name"],
        "Quests.pick_up"
    );
    assert_eq!(
        doc("agentwalk", &dir, |_| false).unwrap()["name"],
        "AgentWalk"
    );
    let own = doc("autoexec", &dir, |_| false).unwrap();
    assert_eq!(
        (own["kind"].as_str(), own["source"].as_str()),
        (Some("own"), Some("Goals.on = true\n"))
    );
    let api = doc("UnitName", &dir, |n| n == "UnitName").unwrap();
    assert_eq!(api["kind"], "api");
    let missing = doc("NoSuchThing", &dir, |_| false).unwrap_err();
    assert!(missing["error"].as_str().unwrap().contains("NoSuchThing"));
    assert!(missing["suggestions"].is_array());
}

#[test]
fn every_builtin_span_runs_from_its_definition_to_its_own_end() {
    for e in builtins().iter().filter(|e| e.source.is_some()) {
        let src = e.source.as_deref().unwrap();
        let first = src.lines().next().unwrap();
        let last = src.lines().last().unwrap();
        let indent = |l: &str| l.len() - l.trim_start().len();
        assert!(first.contains("function"), "{}: {first}", e.name);
        assert!(
            last.trim_end().ends_with("end") && indent(last) == indent(first)
                || src.lines().count() == 1,
            "{} at {} ends {last:?}",
            e.name,
            e.at
        );
    }
    // A one-line function is its own line, not the next function's body.
    let src = "local M = {}\nfunction M.one() return 'x' end\nfunction M.two()\n  return 2\nend\nreturn M\n";
    let found = lua_functions("builtin", "t", None, "t.lua", src);
    assert_eq!(
        found[0].source.as_deref(),
        Some("function M.one() return 'x' end")
    );
    assert_eq!(
        found[1].source.as_deref(),
        Some("function M.two()\n  return 2\nend")
    );
}

#[test]
fn exact_own_module_precedes_builtin_shorthand_and_only_registered_agents_are_filtered() {
    let dir = scratch("precedence");
    std::fs::write(dir.join("pick_up.lua"), "return 'own'").unwrap();
    assert_eq!(
        doc("pick_up", &dir, |_| panic!("no VM needed")).unwrap()["kind"],
        "own"
    );
    assert_eq!(
        doc("Quests.pick_up", &dir, |_| false).unwrap()["kind"],
        "builtin"
    );
    let found = search(
        "agent",
        &["api"],
        &dir,
        || vec!["AgentNearby".into(), "AgentAddonHelper".into()],
        20,
    );
    assert_eq!(names(&found), ["AgentAddonHelper"]);
    std::fs::remove_dir_all(dir).unwrap();
}
