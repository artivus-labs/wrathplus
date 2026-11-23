//! `Agent.ask`: a messy moment handed up to the planner with a digest of what led to it, answered
//! by `Agent.answer`, or given its default when a timeout passes.

use wrathplus_ui::script::{ScriptValue, UiScript};

fn agent(tag: &str) -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-ask-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    // The signals as a list of their names and arguments (tables as their keys, sorted).
    script
        .run(
            r##"
            signals = {}
            local function show(v)
              if type(v) ~= "table" then return tostring(v) end
              local keys = {}
              for k, x in pairs(v) do keys[#keys + 1] = tostring(k) .. "=" .. tostring(x) end
              table.sort(keys)
              return "{" .. table.concat(keys, ",") .. "}"
            end
            AgentSignal = function(name, ...)
              local parts = { name }
              for i = 1, select("#", ...) do parts[#parts + 1] = show((select(i, ...))) end
              signals[#signals + 1] = table.concat(parts, " ")
            end
            answers = {}
            "##,
        )
        .unwrap();
    script
}

#[test]
fn an_ask_signals_its_question_context_and_digest_and_the_answer_calls_back() {
    let mut script = agent("answer");
    script.fire_event(
        "CHAT_MSG_SAY",
        vec![
            ScriptValue::Str("follow me into the cave".into()),
            ScriptValue::Str("Brann".into()),
        ],
    );
    script.fire_event("COMBAT_LOG_EVENT_UNFILTERED", vec![ScriptValue::Int(1)]);
    let (id, signal): (i64, String) = script
        .eval(
            r#"
            local id = Agent.ask("Brann asks us to follow him. Go?", { who = "Brann" },
              function(answer, how) answers[#answers + 1] = tostring(answer) .. "/" .. how return "ran" end)
            for _, s in ipairs(signals) do if s:match("^ASK ") then return id, s end end
            return id, "no ASK signal"
            "#,
        )
        .unwrap();
    // The digest: what was said (not the combat log), then the ask's own note.
    assert!(
        signal.starts_with(&format!(
            "ASK {id} Brann asks us to follow him. Go? {{who=Brann}} {{1="
        )),
        "{signal}"
    );
    assert!(
        signal.contains("CHAT_MSG_SAY follow me into the cave Brann"),
        "{signal}"
    );
    assert!(!signal.contains("COMBAT_LOG"), "{signal}");
    assert!(
        signal.contains(&format!("ask {id}: Brann asks")),
        "{signal}"
    );
    let open: String = script
        .eval("local a = Agent.open_asks() return #a .. ' ' .. a[1].question")
        .unwrap();
    assert_eq!(open, "1 Brann asks us to follow him. Go?");
    // The planner answers: the callback runs once, and its result comes back.
    let (ok, ran): (bool, String) = script
        .eval(&format!("return Agent.answer({id}, 'yes')"))
        .unwrap();
    assert!(ok);
    assert_eq!(ran, "ran");
    let (again, why): (bool, String) = script
        .eval(&format!("return Agent.answer({id}, 'no')"))
        .unwrap();
    assert!(!again);
    assert_eq!(why, format!("no open ask {id}"));
    let answers: String = script
        .eval("return table.concat(answers, ' ') .. ' ' .. #Agent.open_asks()")
        .unwrap();
    assert_eq!(answers, "yes/answered 0");
}

#[test]
fn an_unanswered_ask_takes_its_default_at_the_timeout_and_one_without_waits() {
    let mut script = agent("timeout");
    script
        .run(
            r#"
            timed = Agent.ask("Odd mechanic: the totem shields the boss. Keep at it?", nil,
              function(answer, how) answers[#answers + 1] = tostring(answer) .. "/" .. how end,
              { timeout = 1, default = "keep" })
            patient = Agent.ask("Who is this whisperer?")
            "#,
        )
        .unwrap();
    for _ in 0..3 {
        script.tick(0.3);
    }
    let answers: String = script.eval("return table.concat(answers, ' ')").unwrap();
    assert_eq!(answers, "", "not before the timeout");
    for _ in 0..2 {
        script.tick(0.3);
    }
    let (answers, last, open): (String, String, i64) = script
        .eval("return table.concat(answers, ' '), signals[#signals], #Agent.open_asks()")
        .unwrap();
    assert_eq!(answers, "keep/timeout");
    let timed: i64 = script.eval("return timed").unwrap();
    assert_eq!(last, format!("ASK_EXPIRED {timed}"));
    // The ask without a timeout is still open, and answering it needs no callback.
    assert_eq!(open, 1);
    let ok: bool = script
        .eval("return Agent.answer(patient, 'a trader')")
        .unwrap();
    assert!(ok);
    assert!(script
        .eval::<bool>("return Agent.answer(timed, 'late') == false")
        .unwrap());
}
