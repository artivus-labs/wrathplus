use crate::script::UiScript;

#[test]
fn get_chat_window_info_answers_build_12340s_ten_values() {
    let s = UiScript::new().unwrap();
    assert_eq!(s.arity("GetChatWindowInfo(1)").unwrap(), 10);
}

#[test]
fn every_window_name_is_the_empty_string_not_a_label() {
    let s = UiScript::new().unwrap();
    for id in 1..=7 {
        let name: String = s
            .eval(&format!("return (GetChatWindowInfo({id}))"))
            .unwrap();
        assert_eq!(name, "", "window {id} name");
    }
}

/// The stock records (`0x501f00`): windows 1 and 2 shown and docked at 1 and 2, windows 3 to 7
/// neither.
#[test]
fn hidden_and_undocked_windows_answer_nil_never_zero() {
    let s = UiScript::new().unwrap();
    let probe = |id: i32| -> (String, String) {
        let shown = s
            .eval::<String>(&format!(
                "local _,_,_,_,_,_,shown = GetChatWindowInfo({id}) return type(shown)"
            ))
            .unwrap();
        let docked = s
            .eval::<String>(&format!(
                "local _,_,_,_,_,_,_,_,docked = GetChatWindowInfo({id}) return type(docked)"
            ))
            .unwrap();
        (shown, docked)
    };
    assert_eq!(probe(1), ("number".into(), "number".into()));
    assert_eq!(probe(2), ("number".into(), "number".into()));
    for id in 3..=7 {
        assert_eq!(probe(id), ("nil".into(), "nil".into()), "window {id}");
    }
    // The truthiness FrameXML branches on.
    assert!(s
        .eval::<bool>(
            "for i = 3, 7 do local _,_,_,_,_,_,shown = GetChatWindowInfo(i) \
             if shown then return false end end return true"
        )
        .unwrap());
}

#[test]
fn wrath_combat_log_docks_after_the_primary_window() {
    let s = UiScript::new().unwrap();
    assert_eq!(
        s.eval::<i64>("local _,_,_,_,_,_,_,_,d = GetChatWindowInfo(2) return d")
            .unwrap(),
        2
    );
}

/// The stock records dock window 1 at 1 and window 2 at 2 (`0x501f00`).
#[test]
fn docked_is_a_dock_position_not_a_flag() {
    let mut s = UiScript::new().unwrap();
    let docked = |s: &UiScript, id: i64| {
        s.eval::<i64>(&format!(
            "local _,_,_,_,_,_,_,_,d = GetChatWindowInfo({id}) return d"
        ))
        .unwrap()
    };
    assert_eq!((docked(&s, 1), docked(&s, 2)), (1, 2));
    s.run("SetChatWindowDocked(2, 3)").unwrap();
    assert_eq!(docked(&s, 2), 3);
    // A dword (`0x4fc7f8`) the getter reads unsigned; a non-number stores 0, undocked.
    s.run("SetChatWindowDocked(2, -1)").unwrap();
    assert_eq!(docked(&s, 2), 4_294_967_295);
    s.run("SetChatWindowDocked(2, 'x')").unwrap();
    assert!(s.chat_window_looks()[1].docked.is_none());
    let _ = s.take_chat_window_changes();
}

/// The debug-window walk of MikScrollingBattleText and EnhTooltip, whose `string.lower(name)`
/// needs a string.
#[test]
fn the_corpus_debug_window_walk_completes_and_finds_none() {
    let s = UiScript::new().unwrap();
    let found: i64 = s
        .eval(
            "local debugWin = 0\n\
             for i = 1, 7 do\n\
               local name, _, _, _, _, _, shown = GetChatWindowInfo(i)\n\
               if string.lower(name) == 'debug' then debugWin = i break end\n\
             end\n\
             return debugWin",
        )
        .unwrap();
    assert_eq!(found, 0);
}

/// The id is `fistp`-truncated and read as its low dword (`0x4fbdc7`-`0x4fbdeb`); one outside
/// 1..10 answers and stores nothing, which stock's temporary windows (11 up) rely on.
#[test]
fn a_window_outside_one_to_ten_answers_nothing_and_ids_truncate() {
    let mut s = UiScript::new().unwrap();
    s.run("SetChatWindowName(1, 'One') SetChatWindowName(2, 'Two')")
        .unwrap();
    for (id, name) in [
        ("1.9", "One"),
        ("2.9", "Two"),
        ("'2'", "Two"),
        ("2^32 + 1", "One"),
    ] {
        let got: String = s
            .eval(&format!("return (GetChatWindowInfo({id}))"))
            .unwrap();
        assert_eq!(got, name, "GetChatWindowInfo({id})");
    }
    let _ = s.take_chat_window_changes();
    for call in [
        "GetChatWindowInfo(11)",
        "GetChatWindowInfo(0)",
        "GetChatWindowMessages(-1)",
        "GetChatWindowChannels(11)",
        "SetChatWindowName(11, 'x')",
        "SetChatWindowLocked(11, 0)",
        "SetChatWindowDocked(11, 3)",
        "SetChatWindowShown(11)",
        "SetChatWindowSize(11, 14)",
        "SetChatWindowColor(0, 1, 1, 1)",
        "SetChatWindowAlpha(11, 1)",
        "SetChatWindowUninteractable(11, 1)",
        "AddChatWindowMessages(11, 'SAY')",
        "RemoveChatWindowMessages(11, 'SAY')",
        "AddChatWindowChannel(11, 'MyChan')",
        "RemoveChatWindowChannel(11, 'MyChan')",
    ] {
        assert_eq!(s.arity(call).unwrap(), 0, "{call}");
    }
    assert!(s.take_chat_window_changes().is_empty());
}

/// Each verb raises its own usage line (`0x9fba04`..`0x9fbc44`, `0x9fc178`, `0x9fc19c`). Size,
/// colour, alpha and the message and channel verbs test every argument before the id; name,
/// locked, docked and shown test the id alone.
#[test]
fn each_window_verb_raises_its_own_usage_on_a_wrong_type() {
    let s = UiScript::new().unwrap();
    for (call, usage) in [
        ("GetChatWindowInfo()", "GetChatWindowInfo(index)"),
        ("GetChatWindowMessages('x')", "GetChatWindowMessages(index)"),
        ("GetChatWindowChannels({})", "GetChatWindowChannels(index)"),
        (
            "SetChatWindowName(nil, 'x')",
            "SetChatWindowName(index, \"name\")",
        ),
        ("SetChatWindowSize(11)", "SetChatWindowSize(index, size)"),
        (
            "SetChatWindowColor(1, 1, 1)",
            "SetChatWindowColor(index, r, g, b)",
        ),
        (
            "SetChatWindowAlpha(11, 'x')",
            "SetChatWindowAlpha(index, alpha)",
        ),
        (
            "SetChatWindowLocked(true)",
            "SetChatWindowLocked(index, locked)",
        ),
        (
            "SetChatWindowDocked()",
            "SetChatWindowDocked(index, docked)",
        ),
        (
            "SetChatWindowShown('one')",
            "SetChatWindowShown(index, shown)",
        ),
        (
            "AddChatWindowMessages(11)",
            "AddChatWindowMessages(index, \"messageGroup\")",
        ),
        (
            "RemoveChatWindowMessages(1, {})",
            "RemoveChatWindowMessages(index, \"messageGroup\")",
        ),
        (
            "AddChatWindowChannel(11, nil)",
            "AddChatWindowChannel(index, \"channel\")",
        ),
        (
            "RemoveChatWindowChannel(1)",
            "RemoveChatWindowChannel(index, \"channel\")",
        ),
    ] {
        let err = s.run(call).unwrap_err().to_string();
        assert!(err.contains(&format!("Usage: {usage}")), "{call}: {err}");
    }
    assert!(s
        .run("SetChatWindowName(3, {}) SetChatWindowLocked(3, {}) SetChatWindowDocked(3, 'x')")
        .is_ok());
}

/// The reference's `jle` at `0x4fc439`, after `__ftol2_sse`: one past `i32` is `i32::MIN`.
#[test]
fn a_non_positive_font_size_is_dropped_not_stored() {
    let s = UiScript::new().unwrap();
    s.run("SetChatWindowSize(1, 16)").unwrap();
    for bad in ["0", "-1", "0.5", "2^31"] {
        s.run(&format!("SetChatWindowSize(1, {bad})")).unwrap();
        assert_eq!(
            s.eval::<i64>("local _, size = GetChatWindowInfo(1) return size")
                .unwrap(),
            16,
            "SetChatWindowSize(1, {bad}) must change nothing"
        );
    }
}

/// `fistp(x · 255.0)` truncates: 0.4 stores 102 and 0.5 stores 127.
#[test]
fn the_setters_round_trip_through_the_engine_byte() {
    let s = UiScript::new().unwrap();
    s.run("SetChatWindowAlpha(1, 0.4)").unwrap();
    let a: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    // The stored byte times 1/255 in f32 (`[0xa45564]`), as build 12340 pushes it.
    assert_eq!(
        a,
        f64::from(102f32 * (1.0f32 / 255.0f32)),
        "alpha quantises to the stored byte"
    );

    s.run("SetChatWindowColor(1, 1, 0.5, 0)").unwrap();
    let (r, g, b): (f64, f64, f64) = s
        .eval("local _,_,r,g,b = GetChatWindowInfo(1) return r, g, b")
        .unwrap();
    let byte = |b: u8| f64::from(f32::from(b) * (1.0f32 / 255.0f32));
    assert_eq!((r, g, b), (1.0, byte(127), 0.0));

    s.run("SetChatWindowSize(1, 16)").unwrap();
    let size: i64 = s
        .eval("local _, size = GetChatWindowInfo(1) return size")
        .unwrap();
    assert_eq!(size, 16);

    s.run("SetChatWindowAlpha(1, 0.5)").unwrap();
    let a: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    assert_eq!(a, byte(127), "fistp truncates; 128/255 would be a round");
}

#[test]
fn a_setter_moves_only_the_window_it_names() {
    let s = UiScript::new().unwrap();
    s.run("SetChatWindowAlpha(2, 1)").unwrap();
    let one: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    let two: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(2) return a")
        .unwrap();
    // Window 1 keeps the default record's alpha byte, `0x28` (`0x501813`).
    assert_eq!(one, f64::from(40f32 * (1.0f32 / 255.0f32)));
    assert_eq!(two, 1.0);
}

/// The deviation: the reference stores the low byte of `__ftol(2.0 · 255) = 510`, answering
/// 254/255.
#[test]
fn an_out_of_domain_alpha_clamps_rather_than_wrapping() {
    let s = UiScript::new().unwrap();
    s.run("SetChatWindowAlpha(1, 2.0)").unwrap();
    let a: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    assert_eq!(a, 1.0);
    s.run("SetChatWindowAlpha(1, -1)").unwrap();
    let a: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    assert_eq!(a, 0.0);
}

#[test]
fn the_host_seam_dedupes_writes_and_stays_quiet_on_load() {
    let mut s = UiScript::new().unwrap();
    for step in 0..40 {
        s.run(&format!(
            "SetChatWindowAlpha(1, {})",
            f64::from(step) / 40.0
        ))
        .unwrap();
    }
    s.run("SetChatWindowColor(2, 0.2, 0.2, 0.2)").unwrap();
    assert_eq!(s.take_chat_window_changes(), vec![0, 1]);
    assert!(
        s.take_chat_window_changes().is_empty(),
        "the drain is a take"
    );

    s.set_chat_window_looks([(
        0,
        crate::script::ChatWindowLook {
            r: 1,
            g: 2,
            b: 3,
            a: 4,
            font_size: 14,
            locked: true,
            docked: Some(1),
            ..Default::default()
        },
    )]);
    assert!(
        s.take_chat_window_changes().is_empty(),
        "the load path never echoes"
    );
    assert_eq!(s.chat_window_looks()[0].font_size, 14);
}

#[test]
fn a_write_that_moves_nothing_queues_nothing() {
    let mut s = UiScript::new().unwrap();
    s.run("SetChatWindowAlpha(1, 0.16)").unwrap();
    s.run("SetChatWindowColor(1, 0, 0, 0)").unwrap();
    s.run("SetChatWindowSize(1, 0)").unwrap();
    assert!(s.take_chat_window_changes().is_empty());
}
