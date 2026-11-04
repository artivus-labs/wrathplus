//! The macro wiring: persistence under `wrathplus-config/macros/`, the runner's route through the
//! stock chat frame, and the seed and dirty contract the plugin's systems rely on.

use wrathplus_ui::script::{MacroState, MacroView, UiScript};

use crate::local_state::test_env::{EnvGuard, ENV_LOCK};

/// An account macro under id 1.
fn macro_view(name: &str, body: &str) -> MacroView {
    MacroView {
        id: 1,
        name: name.into(),
        texture: Some("Interface\\Icons\\Ability_Ambush".into()),
        body: body.into(),
    }
}

/// A character macro under the first character id.
fn character_view(name: &str, body: &str) -> MacroView {
    MacroView {
        id: wrathplus_ui::script::CHARACTER_MACRO_BASE + 1,
        ..macro_view(name, body)
    }
}

/// A save writes the reference's format under `wrathplus-config/macros/`, and a load reads it back.
#[test]
fn a_saved_macro_table_round_trips_through_wrathplus_macros() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-macros-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());

    let account = crate::local_state::macros_account_path().unwrap();
    let character = crate::local_state::macros_character_path("Test Realm", "Probeone").unwrap();

    let state = MacroState {
        account: vec![macro_view("Ambush", "/cast Ambush\n/say pew")],
        character: vec![character_view("Charge", "/cast Charge")],
    };
    crate::local_state::write_atomic(&account, &super::store::write(&state.account)).unwrap();
    crate::local_state::write_atomic(&character, &super::store::write(&state.character)).unwrap();

    // The reference's format, with `\n` line ends.
    assert_eq!(
        std::fs::read_to_string(&account).unwrap(),
        "MACRO 1 \"Ambush\" Ability_Ambush\n/cast Ambush\n/say pew\nEND\n"
    );

    let back = MacroState {
        account: super::store::parse(&std::fs::read_to_string(&account).unwrap(), 0),
        character: super::store::parse(
            &std::fs::read_to_string(&character).unwrap(),
            wrathplus_ui::script::CHARACTER_MACRO_BASE,
        ),
    };
    assert_eq!(back, state);
    std::fs::remove_dir_all(&tmp).ok();
}

/// A capture resolves both paths to `None`, so its macros are session-only.
#[test]
fn a_capture_run_persists_nothing() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _h = EnvGuard::set("WRATHPLUS_HOME", "/tmp/wrathplus-should-not-exist");
    let _c = EnvGuard::set("WOW_CAPTURE", "ui-macro");
    assert_eq!(crate::local_state::macros_account_path(), None);
    assert_eq!(crate::local_state::macros_character_path("R", "C"), None);
}

/// The real UI, so the whole route is under test: `run_macro` fires `EXECUTE_CHAT_LINE`, and the
/// stock `ChatFrame1` sends the line through its edit box.
fn ui() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    // The in-game UI loads on world entry, so a player exists: the stock macro window's OnLoad
    // formats `UnitName("player")` into its character tab.
    s.set_unit(
        "player",
        Some(wrathplus_ui::script::UnitState {
            exists: true,
            name: Some("Probefour".into()),
            level: 60,
            ..Default::default()
        }),
    );
    let failures = crate::ui_script::load_default_ui(&mut s);
    assert!(failures.is_empty(), "manifest load errors: {failures:#?}");
    s
}

/// One `EXECUTE_CHAT_LINE` per non-empty line, in order (`0x4f14e0`), which `ChatFrame_OnEvent`
/// sends through the edit box (`ChatFrame.lua:1343-1347`): a macro line lands where a typed one
/// does.
#[test]
fn running_a_macro_runs_its_lines_through_the_references_edit_box() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = ui();
    s.set_macros(MacroState {
        account: vec![macro_view("Greet", "/wave\n\n  /say pew  \n/roll 1 100")],
        character: Vec::new(),
    });

    assert!(super::run_macro(&mut s, 1, None));
    let sends = s.take_chat_sends();
    assert_eq!(
        sends
            .iter()
            .map(|c| (c.text.as_str(), c.chat_type.as_str()))
            .collect::<Vec<_>>(),
        vec![("pew", "SAY")],
        "the line was trimmed before the parse, and sent as the type its slash named"
    );
    assert_eq!(
        s.take_emote_requests()
            .iter()
            .map(|e| e.token.as_str())
            .collect::<Vec<_>>(),
        vec!["WAVE"],
        "the emote line reached DoEmote through the reference's emote table"
    );
    assert_eq!(s.take_roll_requests(), vec![(1, 100)]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());

    // An empty macro and an empty slot both run nothing and queue nothing.
    s.set_macros(MacroState {
        account: vec![macro_view("Blank", "   \n\n")],
        character: Vec::new(),
    });
    assert!(!super::run_macro(&mut s, 1, None));
    assert!(!super::run_macro(&mut s, 7, None));
    assert!(s.take_chat_sends().is_empty());
}

/// Index 37 is the first per-character macro (`MAX_ACCOUNT_MACROS` + 1).
#[test]
fn a_character_macro_runs_by_its_own_index() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = ui();
    s.set_macros(MacroState {
        account: Vec::new(),
        character: vec![character_view("Charge", "/say Charge")],
    });
    assert!(super::run_macro(&mut s, 37, None));
    assert_eq!(
        s.take_chat_sends()
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>(),
        vec!["Charge".to_string()]
    );
}

/// The app's own load is not an edit, or every login would rewrite the file; a script mutation is.
#[test]
fn the_dirty_edge_distinguishes_a_load_from_an_edit() {
    let mut s = UiScript::new().unwrap();
    s.set_macros(MacroState {
        account: vec![macro_view("Ambush", "/cast Ambush")],
        character: Vec::new(),
    });
    assert!(
        !s.take_macros_dirty(),
        "loading from disk is not an edit — a save here would be a write-back loop"
    );

    s.run(r#"EditMacro(1, nil, nil, "/cast Backstab")"#)
        .unwrap();
    assert!(s.take_macros_dirty());
    assert_eq!(s.macros().account[0].body, "/cast Backstab");
}

#[test]
fn the_generation_moves_on_every_write_and_is_not_drained() {
    let mut s = UiScript::new().unwrap();
    let at_start = s.macros_generation();
    assert_eq!(s.macros_generation(), at_start, "reading never drains it");

    s.set_macros(MacroState {
        account: vec![macro_view("Ambush", "/cast Ambush")],
        character: Vec::new(),
    });
    let after_seed = s.macros_generation();
    assert_ne!(after_seed, at_start, "a seed changes the bar's icons too");

    s.run(r#"EditMacro(1, "Renamed", 1)"#).unwrap();
    assert_ne!(s.macros_generation(), after_seed);
}

/// In 1.12 the event is the whole mechanism; `ChatFrame1`'s registration is only the default UI's
/// use of it.
#[test]
fn a_registered_frame_sees_every_macro_line_as_an_event() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = ui();
    s.run(
        r#"MacroSpy = CreateFrame("Frame")
           MacroSpy.seen = {}
           MacroSpy:RegisterEvent("EXECUTE_CHAT_LINE")
           MacroSpy:SetScript("OnEvent", function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9)
               table.insert(MacroSpy.seen, arg1)
           end)"#,
    )
    .unwrap();
    s.set_macros(MacroState {
        account: vec![macro_view("Pull", "/wave\n/say Incoming!")],
        character: Vec::new(),
    });

    assert!(super::run_macro(&mut s, 1, None));
    assert_eq!(
        s.eval::<(String, String, i64)>(
            "return MacroSpy.seen[1], MacroSpy.seen[2], table.getn(MacroSpy.seen)"
        )
        .unwrap(),
        ("/wave".into(), "/say Incoming!".into(), 2),
        "each line arrives as arg1 of its own event, in body order"
    );
    // The same lines still reach the chat frame.
    assert_eq!(
        s.take_chat_sends()
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>(),
        vec!["Incoming!".to_string()]
    );
    assert_eq!(s.take_emote_requests().len(), 1);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The reference's tokenizer (`0x64ae50`) splits on either `\r` or `\n`.
#[test]
fn either_line_ending_splits_a_body() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = ui();
    s.set_macros(MacroState {
        account: vec![macro_view("Mixed", "/say Charge\r/say a\r\n/say b")],
        character: Vec::new(),
    });
    assert!(super::run_macro(&mut s, 1, None));
    assert_eq!(
        s.take_chat_sends()
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>(),
        vec!["Charge".to_string(), "a".to_string(), "b".to_string()],
        "\r alone splits; \r\n does not leave an empty token"
    );
}

#[test]
fn macro_icon_catalog_failure_is_cached_only_for_its_source() {
    let source = std::sync::Arc::new(());
    let replacement = std::sync::Arc::new(());
    let mut catalog: super::IconCatalogCache<()> = super::IconCatalogCache::default();
    assert!(catalog
        .get_or_load(&source, || Err("missing listfile".into()))
        .0
        .is_err());
    let (result, hit) = catalog.get_or_load(&source, || {
        panic!("failed immutable source should not be retried per VM")
    });
    assert!(hit);
    assert_eq!(result.as_ref().unwrap_err(), "missing listfile");
    assert!(catalog.get_or_load(&replacement, || Ok(vec![])).0.is_ok());
}

#[test]
fn dead_failed_catalog_source_is_not_reused() {
    let mut catalog: super::IconCatalogCache<()> = super::IconCatalogCache::default();
    {
        let source = std::sync::Arc::new(());
        let (result, hit) = catalog.get_or_load(&source, || Err("old failure".into()));
        assert!(!hit && result.is_err());
    }
    let replacement = std::sync::Arc::new(());
    let (result, hit) = catalog.get_or_load(&replacement, || Ok(vec![]));
    assert!(!hit && result.is_ok());
    let (result, hit) = catalog.get_or_load(&replacement, || panic!("replacement loaded once"));
    assert!(hit && result.is_ok());
}

#[test]
fn a_wrath_vm_gets_both_build_12340_lists_once_per_source() {
    let source = std::sync::Arc::new(());
    let mut cache = super::IconCatalogCache::default();
    let mut seeded = crate::app_core::vm_memo::VmMemo::default();
    let mut wrath = UiScript::new().unwrap();
    let icons = wrathplus_formats::WrathMacroIcons {
        spell: vec!["INV_Misc_QuestionMark".into(), "Spell_Holy_Heal".into()],
        item: vec!["INV_Misc_QuestionMark".into(), "INV_Axe_01".into()],
    };
    super::seed_wrath_icon_catalog(&mut wrath, &source, &mut seeded, &mut cache, || {
        Ok(icons.clone())
    });
    super::seed_wrath_icon_catalog(&mut wrath, &source, &mut seeded, &mut cache, || {
        panic!("same VM/source must not reload")
    });
    assert!(wrath
        .eval::<bool>(
            "return GetNumMacroIcons() == 2 and GetMacroIconInfo(2) == 'Interface\\\\Icons\\\\Spell_Holy_Heal' \
             and GetNumMacroItemIcons() == 2 and GetMacroItemIconInfo(2) == 'Interface\\\\Icons\\\\INV_Axe_01'"
        )
        .unwrap());
    let mut second = UiScript::new().unwrap();
    super::seed_wrath_icon_catalog(&mut second, &source, &mut seeded, &mut cache, || {
        panic!("a new VM on the same source uses the cache")
    });
    assert_eq!(second.eval::<i64>("return GetNumMacroIcons()").unwrap(), 2);
}

/// `GetRunningMacro`/`GetRunningMacroButton` (`0x564c10`/`0x564c70`) answer the run while its
/// lines fire and the caller's after: nil outside, the index and button inside a `RunMacro`, nil
/// and the button inside a `RunMacroText` (its scratch record has no id).
#[test]
fn the_running_macro_and_its_button_are_answered_only_while_its_lines_run() {
    let mut s = UiScript::new().unwrap();
    s.set_macros(MacroState {
        account: vec![macro_view("Probe", "/a\n/b")],
        character: Vec::new(),
    });
    s.run(
        "SEEN = {} local f = CreateFrame('Frame') f:RegisterEvent('EXECUTE_CHAT_LINE') \
         f:SetScript('OnEvent', function(_, _, line) \
           SEEN[#SEEN + 1] = line .. '=' .. tostring(GetRunningMacro()) .. ',' \
             .. tostring(GetRunningMacroButton()) end)",
    )
    .unwrap();
    assert!(super::run_macro(&mut s, 1, Some("RightButton".into())));
    super::run_macro_text(&mut s, "/c", None);
    let seen = s.eval::<String>("return table.concat(SEEN, ' ')").unwrap();
    assert_eq!(seen, "/a=1,RightButton /b=1,RightButton /c=nil,nil");
    assert!(s
        .eval::<bool>("return GetRunningMacro() == nil and GetRunningMacroButton() == nil")
        .unwrap());
}

/// A `SetMacroItem`/`SetMacroSpell` pair replaces the body's bound spell in the bar's map and in
/// the engine's copy until the body is edited (`0x565ad0`'s unlink, `0x5aa920`'s refresh).
#[test]
fn an_explicit_pair_rebinds_the_bar_until_the_body_changes() {
    use super::{BoundSpell, MacroBoundSpells};
    use bevy::prelude::*;
    let mut s = UiScript::new().unwrap();
    s.set_macros(MacroState {
        account: vec![macro_view("Bolt", "/cast Fireball")],
        character: Vec::new(),
    });
    s.set_item_template(
        6948,
        wrathplus_ui::script::ItemTemplateView {
            name: "Hearthstone".into(),
            ..Default::default()
        },
    );
    let table = crate::ui_chat::commands::SlashCommands::build(
        |name| (name == "SLASH_CAST1").then(|| "/cast".to_string()),
        |_| None,
    );
    let mut app = App::new();
    app.init_resource::<crate::app_core::state::PlayerActions>()
        .init_resource::<MacroBoundSpells>()
        .insert_resource(table)
        .add_systems(Update, super::rebind_macro_spells);
    app.insert_non_send_resource(s);
    let bound = |app: &App| {
        app.world()
            .resource::<MacroBoundSpells>()
            .0
            .get(&1)
            .copied()
    };
    app.update();
    assert_eq!(
        bound(&app),
        Some(BoundSpell::Unresolved),
        "no book: the /cast misses"
    );

    let lua = |app: &mut App, chunk: &str| {
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .run(chunk)
            .unwrap()
    };
    lua(&mut app, "SetMacroItem(1, 'Hearthstone')");
    app.update();
    assert_eq!(bound(&app), Some(BoundSpell::Item(6948)));
    lua(&mut app, "SetMacroItem(1, 'Nothing')");
    app.update();
    assert_eq!(
        bound(&app),
        Some(BoundSpell::Unresolved),
        "a missed item greys the slot"
    );
    lua(&mut app, "EditMacro(1, nil, nil, '/say hi')");
    app.update();
    assert_eq!(
        bound(&app),
        Some(BoundSpell::None),
        "the body's derivation again"
    );
}
