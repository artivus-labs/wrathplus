//! End-to-end tests of the chat windows and the input box on the stock `ChatFrame.xml` and
//! `FloatingChatFrame.xml`, driven as the app drives them.

use wrathplus_ui::script::{ExtractedQuad, QuadContent, UiScript};

use super::test_ui::load_ui as load_xml;

/// The chat stack, fonts first so `inherits="ChatFontNormal"` resolves.
fn chat_frame() -> UiScript {
    let mut s = UiScript::new().unwrap();
    load_xml(&mut s, "Interface\\FrameXML\\GlobalStrings.lua");
    load_xml(&mut s, "Interface\\FrameXML\\Fonts.xml");
    load_xml(&mut s, "Interface\\FrameXML\\BasicControls.xml");
    // UIParent's OnUpdate runs `FCF_OnUpdate`, the dock's driver, as in the reference.
    load_xml(&mut s, r"Interface\FrameXML\UIParent.xml");
    // `FCF_Tab_OnClick` uses the dropdown kit, whose backdrop reads GameTooltip.lua's colour.
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.lua");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\GameTooltip.xml");
    load_xml(&mut s, "Interface\\FrameXML\\UIMenu.xml"); // the kit the chat menus build from
    load_xml(&mut s, "Interface\\FrameXML\\UIDropDownMenu.xml");
    load_xml(&mut s, "Interface\\FrameXML\\UIPanelTemplates.lua");
    load_xml(&mut s, "Interface\\FrameXML\\UIPanelTemplates.xml");
    load_xml(&mut s, r"Interface\FrameXML\StaticPopup.xml");
    load_xml(&mut s, "Interface\\FrameXML\\TextStatusBar.lua");
    load_xml(&mut s, "Interface\\FrameXML\\TextStatusBar.xml");
    // `FCF_ValidateChatFramePosition` reads `MainMenuBar:GetHeight()`, so the bar loads first.
    load_xml(&mut s, "Interface\\FrameXML\\MainMenuBar.xml");
    load_xml(&mut s, "Interface\\FrameXML\\Cooldown.xml");
    load_xml(&mut s, "Interface\\FrameXML\\ActionButtonTemplate.xml");
    load_xml(&mut s, "Interface\\FrameXML\\ChatFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\FloatingChatFrame.xml");
    super::fire_chat_login(&mut s);
    s.set_screen_size(1600.0, 900.0);
    s.resolve();
    s
}

fn text_color(quads: &[ExtractedQuad], t: &str) -> Option<[f32; 4]> {
    quads.iter().find_map(|q| match &q.content {
        QuadContent::Text {
            text: Some(x),
            color: Some(c),
            ..
        } if x == t => Some(*c),
        _ => None,
    })
}

/// Past `CHAT_TAB_HIDE_DELAY` (1 s) and `CHAT_FRAME_FADE_OUT_TIME` (2 s), so a fade-out has landed
/// (FloatingChatFrame.lua:9-11, :1087-1092).
fn leave(s: &mut UiScript) {
    for _ in 0..200 {
        s.tick(0.016);
        s.resolve();
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn injected_lines_render_in_the_pinned_colors() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    // The app's color table as 0..1 floats: SAY FFFFFF, SYSTEM FFFF00, LOOT 00AA00.
    s.add_chat_message("ChatFrame1", "[Tri] says: hi", 1.0, 1.0, 1.0);
    s.add_chat_message("ChatFrame1", "You give 500 copper.", 1.0, 1.0, 0.0);
    s.add_chat_message(
        "ChatFrame1",
        "You receive loot: [Tough Jerky].",
        0.0,
        170.0 / 255.0,
        0.0,
    );
    s.resolve();
    let quads = s.extract();

    let say = text_color(&quads, "[Tri] says: hi").expect("say line rendered");
    assert!(
        close(say[0], 1.0) && close(say[1], 1.0) && close(say[2], 1.0),
        "say white: {say:?}"
    );
    assert!(close(say[3], 1.0), "a fresh line is fully opaque");

    let sys = text_color(&quads, "You give 500 copper.").expect("system line rendered");
    assert!(
        close(sys[0], 1.0) && close(sys[1], 1.0) && close(sys[2], 0.0),
        "system yellow: {sys:?}"
    );

    let loot = text_color(&quads, "You receive loot: [Tough Jerky].").expect("loot line rendered");
    assert!(
        close(loot[0], 0.0) && close(loot[1], 170.0 / 255.0) && close(loot[2], 0.0),
        "loot green: {loot:?}"
    );
}

#[test]
fn newest_line_sits_at_the_bottom() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.add_chat_message("ChatFrame1", "older", 1.0, 1.0, 1.0);
    s.add_chat_message("ChatFrame1", "newer", 1.0, 1.0, 1.0);
    s.resolve();
    let quads = s.extract();
    let y = |t: &str| {
        quads
            .iter()
            .find_map(|q| match &q.content {
                QuadContent::Text { text: Some(x), .. } if x == t => q.rect.map(|r| r.bottom),
                _ => None,
            })
            .unwrap()
    };
    // y-up: the newest line's band is lower than the older one's.
    assert!(y("newer") < y("older"), "newest renders at the bottom");
}

/// A scroll re-arms a fading line to full and holds it there until the view is back at the
/// bottom: every scroll entry reaches the re-arm at `0x788b80` or the relayout's `0x788af0`.
#[test]
fn wheel_scroll_re_arms_the_fade_then_freezes_it() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.run("ChatFrame1:SetTimeVisible(0); ChatFrame1:SetFadeDuration(4)")
        .unwrap();
    for t in ["L0", "L1", "L2"] {
        s.add_chat_message("ChatFrame1", t, 1.0, 1.0, 1.0);
    }
    s.resolve();
    s.tick(1.0);
    s.resolve();
    let a1 = text_color(&s.extract(), "L1").expect("L1 visible")[3];
    assert!(a1 < 1.0 && a1 > 0.0, "the line faded partway: {a1}");
    // The 1.12 chat frame takes no wheel (`enableMouse="false"`), so this is the buttons' verb.
    s.run("ChatFrame1:ScrollUp()").unwrap();
    s.resolve();
    let a2 = text_color(&s.extract(), "L1").expect("L1 still visible")[3];
    assert!(close(a2, 1.0), "the scroll brought the line back: {a2}");
    s.tick(2.0);
    s.resolve();
    let a3 = text_color(&s.extract(), "L1").expect("L1 still visible")[3];
    assert!(close(a3, 1.0), "frozen while scrolled up: {a3}");
    s.run("ChatFrame1:ScrollToBottom()").unwrap();
    s.tick(1.0);
    s.resolve();
    let a4 = text_color(&s.extract(), "L1").expect("L1 visible")[3];
    assert!(a4 < 1.0 && a4 > 0.0, "the fade resumed at the bottom: {a4}");
    assert!(close(a4, a1), "and from a full re-arm: {a4} vs {a1}");
}

#[test]
fn input_editbox_enter_drains_the_typed_line() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    assert!(s.focus_editbox("ChatFrame1EditBox"), "the edit box focuses");
    assert!(s.has_keyboard_focus(), "focus gates the world's keys");
    s.char_input("/yell hi");
    assert!(s.key_input("ENTER"), "the box consumes ENTER");
    let sends = s.take_chat_sends();
    assert_eq!(
        sends
            .iter()
            .map(|c| (c.text.as_str(), c.chat_type.as_str()))
            .collect::<Vec<_>>(),
        vec![("hi", "YELL")]
    );
    assert!(
        !s.has_keyboard_focus(),
        "submit closes the box (ChatEdit_OnEscapePressed: ClearFocus + Hide)"
    );
    assert!(s.take_chat_sends().is_empty(), "drained");
}

/// The edit box always carries a language: the first known one once the list changes
/// (`LanguageMenu_LoadLanguages`, ChatFrame.lua:4350-4366); the Language list's pick replaces it
/// (:4372) and `ChatEdit_SendText` passes it on every send (:3669-3686).
#[test]
fn the_edit_box_speaks_the_language_picked_in_the_chat_menu() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.set_language_table(vec![(2, "Darnassian".into()), (7, "Common".into())]);
    s.set_known_languages(vec!["Darnassian".into(), "Common".into()]);
    for line in ["/s before", "/s ishnu", "/y alah"] {
        if line == "/s ishnu" {
            s.run("ChatFrame1EditBox.language = GetLanguageByIndex(2)")
                .unwrap();
        }
        assert!(s.focus_editbox("ChatFrame1EditBox"));
        s.char_input(line);
        assert!(s.key_input("ENTER"));
    }
    let sends = s.take_chat_sends();
    assert_eq!(
        sends
            .iter()
            .map(|c| (c.text.as_str(), c.chat_type.as_str(), c.language))
            .collect::<Vec<_>>(),
        vec![
            ("before", "SAY", Some(2)),
            ("ishnu", "SAY", Some(7)),
            ("alah", "YELL", Some(7)),
        ]
    );
}

#[test]
fn input_escape_closes_without_submitting() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    assert!(s.focus_editbox("ChatFrame1EditBox"));
    s.char_input("hello");
    assert!(s.key_input("ESCAPE"), "the box consumes ESCAPE");
    assert!(s.take_chat_input().is_empty(), "escape submits nothing");
    assert!(!s.has_keyboard_focus(), "escape closes the box");
}

/// The stock chat box edits, and Up/Down walk its history and back to the draft.
#[test]
fn chat_box_arrows_edit_and_history_recalls() {
    wrathplus_formats::wow_data_or_skip!();
    use wrathplus_ui::script::{EditAction, EditUnit};
    let mut s = chat_frame();
    assert!(s.focus_editbox("ChatFrame1EditBox"));
    s.char_input("ab");
    // The stock template's `ignoreArrows="true"` is AltArrowKeyMode: the app hands a plain arrow
    // to the bindings (the character turns while you type), and the box still edits.
    assert!(
        s.editbox_alt_arrow_mode(),
        "the stock template's ignoreArrows landed as AltArrowKeyMode on the focused box"
    );
    s.editbox_action(EditAction::Delete {
        unit: EditUnit::Char,
        back: true,
    });
    assert_eq!(
        s.eval::<String>("return ChatFrame1EditBox:GetText()")
            .unwrap(),
        "a"
    );
    s.run("ChatFrame1EditBox:SetText('')").unwrap();
    s.char_input("/yell hi");
    assert!(s.key_input("ENTER"));
    let sends = s.take_chat_sends();
    assert_eq!(
        sends
            .iter()
            .map(|c| (c.text.as_str(), c.chat_type.as_str()))
            .collect::<Vec<_>>(),
        vec![("hi", "YELL")]
    );
    assert!(s.focus_editbox("ChatFrame1EditBox"));
    s.char_input("dra");
    s.editbox_action(EditAction::Move {
        unit: EditUnit::Row,
        back: true,
        extend: false,
    });
    // `ChatEdit_AddHistory` filed "/y hi" (`ChatFrame.lua:1916-1937`); the recall's `OnTextSet`
    // parse (l.2077-2079) turns it back into YELL with "hi".
    assert!(
        s.eval::<bool>(
            "return ChatFrame1EditBox:GetAttribute('chatType') == 'YELL' \
             and ChatFrame1EditBox:GetText() == 'hi'"
        )
        .unwrap(),
        "Up recalls the filed line, live-parsed: {:?} {:?}",
        s.eval::<Option<String>>("return ChatFrame1EditBox:GetAttribute('chatType')")
            .unwrap(),
        s.eval::<String>("return ChatFrame1EditBox:GetText()")
            .unwrap()
    );
    s.editbox_action(EditAction::Move {
        unit: EditUnit::Row,
        back: false,
        extend: false,
    });
    assert_eq!(
        s.eval::<String>("return ChatFrame1EditBox:GetText()")
            .unwrap(),
        "dra",
        "Down past the newest restores the in-progress draft"
    );
}

/// A drag released over the chat keeps carrying; the completed left click after it drops a spell,
/// never an item, which it would destroy. Deviation: the reference drops a spell on a left click
/// only over empty sky, which leaves no left-click way to drop one over ground.
#[test]
fn chat_click_dismisses_a_stuck_spell_but_not_an_item() {
    wrathplus_formats::wow_data_or_skip!();
    use wrathplus_ui::script::{
        ContainerSlot, ContainerState, SpellBookState, SpellSlotView, SpellTabView,
    };
    let mut s = super::spellbook_tests::spellbook_ui_with(
        1024.0,
        768.0,
        &[
            "Interface\\FrameXML\\UIMenu.xml", // the kit ChatMenu/EmoteMenu/VoiceMacroMenu build from
            "Interface\\FrameXML\\ChatFrame.xml",
            "Interface\\FrameXML\\FloatingChatFrame.xml",
        ],
    );
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    s.set_spellbook(SpellBookState {
        tabs: vec![SpellTabView {
            name: "Fire".into(),
            texture: Some("Interface\\Icons\\Spell_Fire_FlameBolt".into()),
            offset: 0,
            num_spells: 1,
        }],
        slots: vec![SpellSlotView {
            spell_id: 133,
            name: "Fireball".into(),
            rank: Some("Rank 1".into()),
            texture: Some("Interface\\Icons\\Spell_Fire_FlameBolt".into()),
            passive: false,
            current: false,
            cooldown: None,
            ..Default::default()
        }],
    });
    s.run("ToggleSpellBook(BOOKTYPE_SPELL)").unwrap();
    s.resolve();

    // Drag Fireball off its book button (press → past the 4px threshold → the payload is up).
    let (l, r, t, b) = (
        s.eval::<f32>("return SpellButton1:GetLeft()").unwrap(),
        s.eval::<f32>("return SpellButton1:GetRight()").unwrap(),
        s.eval::<f32>("return SpellButton1:GetTop()").unwrap(),
        s.eval::<f32>("return SpellButton1:GetBottom()").unwrap(),
    );
    let (x1, y1) = ((l + r) * 0.5, (t + b) * 0.5);
    s.mouse_button(x1, y1, "LeftButton", true);
    s.mouse_move(x1 + 20.0, y1);
    assert!(
        s.cursor_payload().is_some(),
        "OnDragStart picked the spell up"
    );

    // Release the drag over the chat body: OnClick never fires on a drag, so it keeps carrying.
    let (cx, cy) = (200.0, 150.0); // inside ChatFrame1 (BOTTOMLEFT 32,85 + 430×120)
    s.mouse_move(cx, cy);
    s.mouse_button(cx, cy, "LeftButton", false);
    assert!(
        s.cursor_payload().is_some(),
        "a drag release over the chat keeps carrying"
    );

    s.mouse_button(cx, cy, "LeftButton", true);
    s.mouse_button(cx, cy, "LeftButton", false);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert!(
        s.cursor_payload().is_none(),
        "a chat click dismisses a spell payload"
    );

    // An item survives the same click.
    let mut slots = std::collections::HashMap::new();
    slots.insert(
        1,
        ContainerSlot {
            bar_placeable: true,
            texture: Some("Interface\\Icons\\INV_Misc_Food_16".into()),
            count: 5,
            quality: Some(1),
            item_id: 117,
            ..Default::default()
        },
    );
    s.set_container(
        0,
        Some(ContainerState {
            name: Some("Backpack".into()),
            num_slots: 16,
            slots,
        }),
    );
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some(), "fixture: the item is held");
    s.mouse_button(cx, cy, "LeftButton", true);
    s.mouse_button(cx, cy, "LeftButton", false);
    assert!(
        s.cursor_item().is_some(),
        "a chat click never touches an item payload"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

// ── ChatTypeInfo: the addon-facing color table ────────────────────────────────────────────────

/// `ChatTypeInfo` and [`crate::ui_chat::default_color`] both carry build 12340's registry
/// (`0x9fa558`, `wrathplus_ui::script::chat_types`): every kind agrees to the byte, and `id` is its
/// 1-based slot, which `GetChatTypeIndex` answers (ChatFrame.lua:2269-2274).
#[test]
fn chat_type_info_matches_the_host_color_table() {
    wrathplus_formats::wow_data_or_skip!();
    use crate::app_core::chat::default_color;
    use crate::app_core::chat::ChatEventKind as K;

    /// Each modeled kind, its `ChatTypeInfo` key and its registry id.
    const PAIRS: &[(&str, K, i64)] = &[
        ("SAY", K::Say, 2),
        ("PARTY", K::Party, 3),
        ("RAID", K::Raid, 4),
        ("GUILD", K::Guild, 5),
        ("OFFICER", K::Officer, 6),
        ("YELL", K::Yell, 7),
        ("WHISPER", K::Whisper, 8),
        ("WHISPER_INFORM", K::WhisperInform, 10),
        ("EMOTE", K::Emote, 11),
        ("TEXT_EMOTE", K::TextEmote, 12),
        ("SYSTEM", K::System, 1),
        ("MONSTER_SAY", K::MonsterSay, 13),
        ("MONSTER_YELL", K::MonsterYell, 15),
        ("MONSTER_EMOTE", K::MonsterEmote, 17),
        ("MONSTER_WHISPER", K::MonsterWhisper, 16),
        ("CHANNEL", K::Channel, 18),
        ("CHANNEL_JOIN", K::ChannelJoin, 19),
        ("CHANNEL_LEAVE", K::ChannelLeave, 20),
        ("CHANNEL_NOTICE", K::ChannelNotice, 22),
        ("CHANNEL_NOTICE_USER", K::ChannelNoticeUser, 23),
        ("CHANNEL_LIST", K::ChannelList, 21),
        ("AFK", K::Afk, 24),
        ("DND", K::Dnd, 25),
        ("IGNORED", K::Ignored, 26),
        ("SKILL", K::Skill, 27),
        ("LOOT", K::Loot, 28),
        ("MONEY", K::Money, 29),
        ("COMBAT_XP_GAIN", K::CombatXpGain, 34),
        ("RAID_LEADER", K::RaidLeader, 40),
        ("RAID_WARNING", K::RaidWarning, 41),
        ("RAID_BOSS_EMOTE", K::RaidBossEmote, 42),
        ("BATTLEGROUND", K::Battleground, 45),
        ("BATTLEGROUND_LEADER", K::BattlegroundLeader, 46),
        ("BG_SYSTEM_NEUTRAL", K::BgSystemNeutral, 37),
        ("BG_SYSTEM_ALLIANCE", K::BgSystemAlliance, 38),
        ("BG_SYSTEM_HORDE", K::BgSystemHorde, 39),
    ];

    let s = chat_frame();
    for (name, kind, want_id) in PAIRS {
        let (r, g, b, id): (f64, f64, f64, i64) = s
            .eval(&format!(
                r#"local i = ChatTypeInfo["{name}"] return i.r, i.g, i.b, i.id"#
            ))
            .unwrap_or_else(|e| panic!(r#"ChatTypeInfo["{name}"]: {e}"#));
        let got = [
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (b * 255.0).round() as u8,
        ];
        assert_eq!(got, default_color(*kind), r#"ChatTypeInfo["{name}"] color"#);
        // Asserted exactly: four of these share FFDBB7, so only the id catches a transposition.
        assert_eq!(id, *want_id, r#"ChatTypeInfo["{name}"].id"#);
    }
}

#[test]
fn chat_type_info_has_the_references_shape() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();

    let count: i64 = s
        .eval("local n = 0 for _ in pairs(ChatTypeInfo) do n = n + 1 end return n")
        .unwrap();
    assert_eq!(count, 73, "stock 3.3.5 ChatFrame.lua declares 73 keys");

    let sticky: String = s
        .eval(
            "local t = {} for k, v in pairs(ChatTypeInfo) do if v.sticky == 1 then \
             table.insert(t, k) end end table.sort(t) return table.concat(t, \" \")",
        )
        .unwrap();
    assert_eq!(
        sticky,
        "BATTLEGROUND BN_CONVERSATION BN_WHISPER CHANNEL CHANNEL1 CHANNEL10 CHANNEL2 CHANNEL3 \
         CHANNEL4 CHANNEL5 CHANNEL6 CHANNEL7 CHANNEL8 CHANNEL9 GUILD OFFICER PARTY RAID SAY WHISPER"
    );

    // REPLY is FrameXML's own, absent from the engine's registry, so its id is 0; the
    // `UPDATE_CHAT_COLOR` arm copies WHISPER's FF80FF into it (ChatFrame.lua:2515-2530).
    let (id, r, g, b): (i64, f64, f64, f64) = s
        .eval(r#"local i = ChatTypeInfo["REPLY"] return i.id, i.r, i.g, i.b"#)
        .unwrap();
    assert_eq!(id, 0, "REPLY is not in the engine's registry");
    let got = [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ];
    assert_eq!(got, [255, 128, 255], "REPLY wears WHISPER's colour");

    // CHANNEL1..CHANNEL10 are the registry's boot extras, slots 63..72 after its 62 fixed rows.
    for n in 1..=10 {
        let (id, r, g, b): (i64, f64, f64, f64) = s
            .eval(&format!(
                r#"local i = ChatTypeInfo["CHANNEL{n}"] return i.id, i.r, i.g, i.b"#
            ))
            .unwrap();
        assert_eq!(id, 62 + n, "CHANNEL{n}.id");
        let rgb = [
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (b * 255.0).round() as u8,
        ];
        assert_eq!(rgb, [255, 192, 192], "CHANNEL{n} color");
    }
}

/// `ChatFrame_OnEvent` looks a line's type up as `ChatTypeInfo[strsub(event, 10)]`
/// (`ChatFrame.lua:1370`), so every `CHAT_MSG_*` we fire must name one of its keys.
#[test]
fn fired_event_names_are_all_chat_type_info_keys() {
    wrathplus_formats::wow_data_or_skip!();
    use crate::app_core::chat::ChatEventKind as K;
    use crate::ui_chat::event_name;

    let s = chat_frame();
    for &kind in K::ALL {
        let name = event_name(kind);
        let key = name
            .strip_prefix("CHAT_MSG_")
            .unwrap_or_else(|| panic!("{name}: every fired chat event is CHAT_MSG_-prefixed"));
        let present: bool = s
            .eval(&format!(r#"return ChatTypeInfo["{key}"] ~= nil"#))
            .unwrap();
        assert!(present, r#"{name} → ChatTypeInfo["{key}"] is missing"#);
    }
}

// ── The ten chat windows (NUM_CHAT_WINDOWS) ────────────────────────────────────────────────

/// `NUM_CHAT_WINDOWS` is 10 (`ChatFrame.lua:5`), and addons index every window unguarded.
#[test]
fn every_window_num_chat_windows_promises_is_a_real_frame() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();
    assert_eq!(s.eval::<i64>("return NUM_CHAT_WINDOWS").unwrap(), 10);
    for i in 1..=10 {
        let ok: bool = s
            .eval(&format!(
                "local f = getglobal('ChatFrame{i}') \
                 return f ~= nil and f.AddMessage ~= nil and f:GetID() == {i}"
            ))
            .unwrap();
        assert!(ok, "ChatFrame{i} is a real message frame carrying its id");
    }
}

/// The `_LazyPig` window walk, verbatim from `LazyPig.lua:1992`.
#[test]
fn the_lazypig_window_walk_survives_all_seven_indices() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();
    let visible: i64 = s
        .eval(
            "local n = 0\n\
             for i = 1, NUM_CHAT_WINDOWS do\n\
               local ChatFrame = getglobal('ChatFrame'..i)\n\
               if ChatFrame:IsVisible() then n = n + 1 end\n\
             end\n\
             return n",
        )
        .unwrap();
    assert_eq!(visible, 1, "only the selected dock window is visible");
}

/// ChatFrame3..7 ship hidden and undocked, the reference's default chat cache (`DOCKED 0`,
/// `SHOWN 0`); ChatFrame1 and 2 carry `isDocked`, which the stock dock code reads.
#[test]
fn the_undocked_windows_are_hidden_and_carry_no_is_docked() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();
    for i in 1..=2 {
        let docked: bool = s
            .eval(&format!("return ChatFrame{i}.isDocked ~= nil"))
            .unwrap();
        assert!(docked, "ChatFrame{i} is docked and says so");
    }
    for i in 2..=7 {
        let shown: bool = s.eval(&format!("return ChatFrame{i}:IsShown()")).unwrap();
        assert!(!shown, "ChatFrame{i} ships hidden");
    }
    for i in 3..=7 {
        let docked: bool = s
            .eval(&format!("return ChatFrame{i}.isDocked ~= nil"))
            .unwrap();
        assert!(!docked, "ChatFrame{i} carries no isDocked");
    }
    // `Outfitter.lua:3099` reads the tab of every visible or docked window unguarded.
    let ok: bool = s
        .eval(
            "for i = 1, NUM_CHAT_WINDOWS do\n\
               local f = getglobal('ChatFrame'..i)\n\
               if f and (f:IsVisible() or f.isDocked) then\n\
                 local tab = getglobal('ChatFrame'..i..'Tab')\n\
                 if not tab then return false end\n\
                 local _ = tab:GetText()\n\
               end\n\
             end\n\
             return true",
        )
        .unwrap();
    assert!(ok, "the Outfitter tab walk never touches a missing tab");
}

#[test]
fn get_chat_window_info_shown_matches_the_shipped_frames() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();
    for i in 1..=7 {
        let agrees: bool = s
            .eval(&format!(
                // `IsShown` answers 1 or nil, so both sides are compared as booleans.
                "local _, _, _, _, _, _, shown = GetChatWindowInfo({i})\n\
                 return (shown ~= nil) == (ChatFrame{i}:IsShown() ~= nil)"
            ))
            .unwrap();
        assert!(
            agrees,
            "window {i}: GetChatWindowInfo disagrees with the frame"
        );
    }
}

#[test]
fn a_line_added_to_chat_frame3_lands_in_chat_frame3_only() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.add_chat_message("ChatFrame1", "a real line", 1.0, 1.0, 1.0);
    s.run("ChatFrame3:AddMessage('Radar: debug', 1, 1, 0)")
        .unwrap();
    assert_eq!(
        s.eval::<i64>("return ChatFrame3:GetNumMessages()").unwrap(),
        1
    );
    assert_eq!(
        s.eval::<i64>("return ChatFrame1:GetNumMessages()").unwrap(),
        1
    );
    s.resolve();
    assert!(
        text_color(&s.extract(), "Radar: debug").is_none(),
        "a hidden window renders nothing"
    );
}

/// `FCF_SelectDockFrame` takes a frame; for an undocked one the reference assigns the selection and
/// `FCF_DockUpdate` hides every docked window, leaving the undocked one as it was.
#[test]
fn fcf_select_dock_frame_selects_by_frame_and_leaves_an_undocked_one_alone() {
    wrathplus_formats::wow_data_or_skip!();
    let s = chat_frame();
    s.run("FCF_SelectDockFrame(ChatFrame2)").unwrap();
    let (one, two): (bool, bool) = (
        s.eval("return ChatFrame1:IsShown()").unwrap(),
        s.eval("return ChatFrame2:IsShown()").unwrap(),
    );
    assert!(!one && two, "selecting the Combat Log swaps the dock");

    s.run("if not DEFAULT_CHAT_FRAME:IsVisible() then FCF_SelectDockFrame(DEFAULT_CHAT_FRAME) end")
        .unwrap();
    assert!(
        s.eval::<bool>("return ChatFrame1:IsShown()").unwrap(),
        "the corpus guard brings the default frame back"
    );

    let shown5_before: bool = s
        .eval("return ChatFrame5:IsShown() and true or false")
        .unwrap();
    s.run("FCF_SelectDockFrame(ChatFrame5)").unwrap();
    assert_eq!(
        s.eval::<String>("return SELECTED_DOCK_FRAME:GetName()")
            .unwrap(),
        "ChatFrame5"
    );
    assert!(
        !s.eval::<bool>("return ChatFrame1:IsShown()").unwrap()
            && !s.eval::<bool>("return ChatFrame2:IsShown()").unwrap(),
        "FCF_DockUpdate hides every docked window when the selection is not one of them"
    );
    assert_eq!(
        s.eval::<bool>("return ChatFrame5:IsShown() and true or false")
            .unwrap(),
        shown5_before,
        "an undocked window is not in the loop, so nothing shows or hides it"
    );
}

/// With the cursor away, `FCF_OnUpdate` writes no tab alpha: a sentinel alpha survives ten ticks.
#[test]
fn an_idle_dock_stops_rewriting_the_tab_alpha_every_frame() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.mouse_move(1500.0, 850.0); // far from the dock, so no hover
    for _ in 0..8 {
        s.tick(0.016); // let the dock settle
        s.resolve();
    }

    s.run("ChatFrame1Tab:SetAlpha(0.42)").unwrap();
    for _ in 0..10 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());

    let alpha: f64 = s.eval("return ChatFrame1Tab:GetAlpha()").unwrap();
    // 0.42 truncates to the alpha byte 107 (12340 `SetAlpha` `0x49f9e0`).
    assert_eq!(
        alpha,
        wrathplus_ui::widget::alpha_unit(107),
        "a settled dock must not rewrite its tab alpha — got {alpha}"
    );
}

/// A stationary hover past `CHAT_TAB_SHOW_DELAY` fades the selected tab from its resting 0.4 to
/// its mouse-over 1.0 (`FCF_FadeInChatFrame`, FloatingChatFrame.lua:1014-1015).
#[test]
fn hovering_the_dock_still_reveals_the_tabs() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.mouse_move(1500.0, 850.0);
    for _ in 0..4 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(
        close(s.eval::<f32>("return ChatFrame1Tab:GetAlpha()").unwrap(), 0.4),
        "the selected tab rests at CHAT_FRAME_TAB_SELECTED_NOMOUSE_ALPHA (FloatingChatFrame.lua:15)"
    );
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    s.mouse_move(x, y);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let alpha: f64 = s.eval("return ChatFrame1Tab:GetAlpha()").unwrap();
    assert!(
        s.eval::<bool>("return ChatFrame1Tab:IsVisible()").unwrap() && (alpha - 1.0).abs() < 1e-6,
        "a stationary hover past CHAT_TAB_SHOW_DELAY reveals the selected tab at full alpha — got {alpha}"
    );
}

/// At the bottom, `ChatFrame_OnUpdate` only hides a lit flash and returns
/// (`ChatFrame.lua:1508-1513`), so a sentinel `flashTimer` survives; scrolled up, it blinks.
#[test]
fn a_chat_view_at_the_bottom_stops_rewriting_the_flash() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    for t in ["L0", "L1", "L2"] {
        s.add_chat_message("ChatFrame1", t, 1.0, 1.0, 1.0);
    }
    for _ in 0..3 {
        s.tick(0.016);
        s.resolve();
    }
    s.run("ChatFrame1.flashTimer = 0.42").unwrap();
    for _ in 0..10 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let timer: f64 = s.eval("return ChatFrame1.flashTimer").unwrap();
    assert!(
        (timer - 0.42).abs() < 1e-6,
        "a settled view must not rewrite flashTimer every frame — got {timer}"
    );
    assert!(
        !s.eval::<bool>("return ChatFrame1ButtonFrameBottomButtonFlash:IsVisible()")
            .unwrap(),
        "the flash stays hidden at rest"
    );
    s.run("ChatFrame1:ScrollUp()").unwrap();
    s.tick(0.3); // 0.42 + 0.3 = 0.72 >= CHAT_BUTTON_FLASH_TIME -> toggle on
    s.resolve();
    assert!(
        s.eval::<bool>("return ChatFrame1ButtonFrameBottomButtonFlash:IsVisible()")
            .unwrap(),
        "scrolled up, the bottom-button blink lights"
    );
    s.run("ChatFrame1:ScrollToBottom()").unwrap();
    s.tick(0.016);
    s.resolve();
    assert!(
        !s.eval::<bool>("return ChatFrame1ButtonFrameBottomButtonFlash:IsVisible()")
            .unwrap(),
        "returning to the bottom hides a lit flash"
    );
    // The residual phase stands: the at-bottom arm never zeroes the timer.
    let timer: f64 = s.eval("return ChatFrame1.flashTimer").unwrap();
    assert!(
        (timer - 0.22).abs() < 1e-6,
        "the residual phase stands: {timer}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Selecting the Combat Log hides ChatFrame1, and a hidden frame gets no OnUpdate, but
/// `FCF_OnUpdate` runs from UIParent's (`UIParent.xml:19`), so the dock keeps its driver. Only the
/// clock is driven: calling `FCF_OnUpdate()` by hand would skip that visibility gate.
#[test]
fn selecting_the_combat_log_keeps_the_dock_driver_running() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    s.mouse_move(x, y);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(
        close(
            s.eval::<f32>("return ChatFrame1Tab:GetAlpha()").unwrap(),
            1.0
        ) && close(
            s.eval::<f32>("return ChatFrame2Tab:GetAlpha()").unwrap(),
            0.6
        ),
        "the hovered dock starts with General selected — this test must not pass vacuously"
    );
    s.run("FCF_SelectDockFrame(ChatFrame2)").unwrap();
    for _ in 0..10 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert!(
        s.eval::<bool>("return ChatFrame2:IsShown() and not ChatFrame1:IsShown()")
            .unwrap(),
        "the selection swapped the windows"
    );
    let (a1, a2): (f32, f32) = s
        .eval("return ChatFrame1Tab:GetAlpha(), ChatFrame2Tab:GetAlpha()")
        .unwrap();
    assert!(
        close(a1, 0.6) && close(a2, 1.0),
        "the selected tab is the one at full alpha — got General {a1}, Combat Log {a2}"
    );
}

#[test]
fn the_dock_still_fades_out_with_the_combat_log_selected() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    s.mouse_move(x, y);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    s.run("FCF_SelectDockFrame(ChatFrame2)").unwrap();
    for _ in 0..10 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(
        close(
            s.eval::<f32>("return ChatFrame2Tab:GetAlpha()").unwrap(),
            1.0
        ),
        "revealed before the cursor leaves"
    );
    s.mouse_move(1500.0, 850.0); // away from the dock
    leave(&mut s);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    // `CHAT_TAB_HIDE_DELAY` then the fade back to the selected tab's resting alpha.
    assert!(
        close(
            s.eval::<f32>("return ChatFrame2Tab:GetAlpha()").unwrap(),
            0.4
        ),
        "the dock fades back to rest on leave"
    );
}

#[test]
fn the_combat_log_window_runs_its_own_bottom_button_blink() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.run("FCF_SelectDockFrame(ChatFrame2)").unwrap();
    for i in 0..40 {
        s.add_chat_message("ChatFrame2", &format!("line {i}"), 1.0, 1.0, 1.0);
    }
    s.run("ChatFrame2:ScrollUp()").unwrap();
    assert!(
        !s.eval::<bool>("return ChatFrame2:AtBottom()").unwrap(),
        "scrolled off the bottom — this test must not pass vacuously"
    );

    // CHAT_BUTTON_FLASH_TIME is 0.5 s; 40 frames of 16 ms crosses it.
    for _ in 0..40 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert!(
        s.eval::<bool>("return ChatFrame2ButtonFrameBottomButtonFlash:IsVisible()")
            .unwrap(),
        "window 2's bottom-button flash blinks while it is scrolled up"
    );
}

#[test]
fn the_chat_menu_builds_its_rows_on_the_references_kit() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    for f in [
        "Interface\\FrameXML\\GlobalStrings.lua",
        "Interface\\FrameXML\\Fonts.xml",
        "Interface\\FrameXML\\BasicControls.xml",
        r"Interface\FrameXML\UIParent.xml",
        r"Interface\FrameXML\MoneyFrame.lua",
        r"Interface\FrameXML\MoneyFrame.xml",
        "Interface\\FrameXML\\GameTooltip.xml",
        r"Interface\FrameXML\UIMenu.xml",
        "Interface\\FrameXML\\UIDropDownMenu.xml",
        r"Interface\FrameXML\UIPanelTemplates.lua",
        r"Interface\FrameXML\UIPanelTemplates.xml",
        "Interface\\FrameXML\\StaticPopup.xml",
        "Interface\\FrameXML\\ChatFrame.xml",
        "Interface\\FrameXML\\FloatingChatFrame.xml",
    ] {
        load_xml(&mut s, f);
    }
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    let _ = s.errors();

    s.run("ChatMenu:Show()").unwrap();
    assert!(s.errors().is_empty(), "opening it raises: {:?}", s.errors());
    assert!(
        s.eval::<i64>("return ChatMenu.numButtons or 0").unwrap() >= 7,
        "the reference's `UIMenu_AddButton` counted the rows this window adds"
    );
    assert_eq!(
        s.eval::<String>("return ChatMenuButton1:GetText()")
            .unwrap(),
        "Say",
        // A row's label is the Button's own text (`button:SetText`), with no `…Button1Text`.
        "row 1 is the Say row, built by the chain's kit"
    );

    s.run("ChatMenuButton1:Click()").unwrap();
    assert!(
        s.errors().is_empty(),
        "a row click raises: {:?}",
        s.errors()
    );
    assert!(
        !s.eval::<bool>("return ChatMenu:IsShown() and true or false")
            .unwrap(),
        "the reference's row click hides the menu"
    );
    // `ChatMenu_Say` opens the box and sets its `chatType` attribute, typing no slash
    // (`ChatMenu_SetChatType`, ChatFrame.lua:4188-4196).
    assert!(
        s.eval::<bool>(
            "return ChatFrame1EditBox:IsVisible() == 1 \
             and ChatFrame1EditBox:GetAttribute('chatType') == 'SAY'"
        )
        .unwrap(),
        "and it opened the edit box as SAY"
    );
}

/// A glass window (chat cache `COLOR 0 0 0 0`) rests its plate at alpha 0, and `FCF_OnUpdate`
/// fades it to `DEFAULT_CHATFRAME_ALPHA` and back on every hover (FloatingChatFrame.lua:994-1032).
#[test]
fn a_glass_windows_plate_fades_in_on_every_hover() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_frame();
    s.mouse_move(1500.0, 850.0);
    for _ in 0..4 {
        s.tick(0.016);
        s.resolve();
    }
    let bg =
        |s: &mut UiScript| -> f64 { s.eval("return ChatFrame1Background:GetAlpha()").unwrap() };
    assert_eq!(bg(&mut s), 0.0, "a glass window rests at alpha 0");
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    for round in 1..=3 {
        s.mouse_move(x, y);
        for _ in 0..45 {
            s.tick(0.016);
            s.resolve();
        }
        assert!(s.errors().is_empty(), "{:?}", s.errors());
        let a = bg(&mut s);
        assert!(
            (a - 0.25).abs() < 1e-6,
            "hover {round}: the plate fades to DEFAULT_CHATFRAME_ALPHA — got {a} (oldAlpha={:?})",
            s.eval::<Option<f64>>("return ChatFrame1.oldAlpha").unwrap()
        );
        s.mouse_move(1500.0, 850.0);
        leave(&mut s);
        assert!(s.errors().is_empty(), "{:?}", s.errors());
        let a = bg(&mut s);
        assert!(
            a.abs() < 1e-6,
            "leave {round}: the plate fades back to its saved alpha — got {a}"
        );
    }
}

#[test]
fn a_glass_windows_plate_fades_in_on_every_hover_under_the_full_manifest() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1600.0, 900.0);
    let failures = super::load_default_ui(&mut s);
    assert!(failures.is_empty(), "{failures:?}");
    s.resolve();
    s.mouse_move(1500.0, 850.0);
    for _ in 0..4 {
        s.tick(0.016);
        s.resolve();
    }
    super::fire_chat_login(&mut s);
    s.resolve();
    let bg =
        |s: &mut UiScript| -> f64 { s.eval("return ChatFrame1Background:GetAlpha()").unwrap() };
    assert_eq!(bg(&mut s), 0.0, "a glass window rests at alpha 0");
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    for round in 1..=3 {
        s.mouse_move(x, y);
        for _ in 0..45 {
            s.tick(0.016);
            s.resolve();
        }
        assert!(s.errors().is_empty(), "{:?}", s.errors());
        // The plate as the renderer gets it: a black Background quad over ChatFrame1, faded.
        let plates: Vec<(Option<wrathplus_ui::layout::Rect>, f32, Option<[f32; 4]>)> = s
            .extract()
            .iter()
            .filter_map(|q| match &q.content {
                QuadContent::Texture {
                    path: Some(p),
                    color,
                    ..
                } if p.to_ascii_lowercase().contains("chatframebackground") => {
                    Some((q.rect, q.alpha, *color))
                }
                _ => None,
            })
            .collect();
        let (left, bottom): (f32, f32) = s
            .eval("return ChatFrame1:GetLeft(), ChatFrame1:GetBottom()")
            .unwrap();
        // The texture's own anchors: TOPLEFT (-2, 3) and BOTTOMLEFT (-2, -6) off the frame.
        let over_frame1 = plates.iter().find(|(r, _, _)| {
            r.is_some_and(|r| {
                (r.left - (left - 2.0)).abs() < 1.0 && (r.bottom - (bottom - 6.0)).abs() < 1.0
            })
        });
        assert!(
            over_frame1.is_some_and(|(_, a, c)| {
                (a - 0.25).abs() < 1e-3 && c.is_some_and(|c| c[0] == 0.0 && c[1] == 0.0 && c[2] == 0.0)
            }),
            "hover {round}: the extracted plate quad over ChatFrame1 ({left}, {bottom}) — {plates:?}"
        );
        let a = bg(&mut s);
        assert!(
            (a - 0.25).abs() < 1e-6,
            "hover {round}: the plate fades to DEFAULT_CHATFRAME_ALPHA — got {a} (oldAlpha={:?}, hover={:?}, hasBeenFaded={:?}, init={:?})",
            s.eval::<Option<f64>>("return ChatFrame1.oldAlpha").unwrap(),
            s.eval::<Option<f64>>("return ChatFrame1.hover").unwrap(),
            s.eval::<Option<f64>>("return ChatFrame1.hasBeenFaded").unwrap(),
            s.eval::<Option<f64>>("return ChatFrame1.init").unwrap(),
        );
        s.mouse_move(1500.0, 850.0);
        leave(&mut s);
        assert!(s.errors().is_empty(), "{:?}", s.errors());
        let a = bg(&mut s);
        assert!(
            a.abs() < 1e-6,
            "leave {round}: the plate fades back to its saved alpha — got {a}"
        );
    }
}

/// The stock `/afk` and `/dnd` call `SendChatMessage(msg, "AFK")` or `"DND"`
/// (`ChatFrame.lua:1005-1011`), and the token must resolve to `CMSG_MESSAGECHAT` type `0x14` or
/// `0x15`: the server toggles the flag. Either half alone passes with the seam between them broken.
#[test]
fn the_stock_afk_and_dnd_commands_reach_the_wire() {
    wrathplus_formats::wow_data_or_skip!();
    use crate::net::ChatKind;
    use crate::ui_chat::edit::SendType;

    let typed = |line: &str| -> (String, String) {
        let mut s = chat_frame();
        assert!(s.focus_editbox("ChatFrame1EditBox"));
        s.char_input(line);
        assert!(s.key_input("ENTER"), "the box consumes ENTER");
        let sends = s.take_chat_sends();
        assert_eq!(sends.len(), 1, "{line:?} produced one send");
        (sends[0].text.clone(), sends[0].chat_type.clone())
    };

    // Half one: the stock `SlashCmdList` body sends the token.
    let (text, token) = typed("/afk Away from Keyboard");
    assert_eq!(
        (text.as_str(), token.as_str()),
        ("Away from Keyboard", "AFK")
    );
    // Half two: the token resolves to a wire kind.
    assert_eq!(
        SendType::from_token(&token).map(SendType::wire),
        Some(ChatKind::Afk),
        "the token the stock file sends must resolve to a wire kind"
    );

    let (text, token) = typed("/dnd Do not Disturb");
    assert_eq!((text.as_str(), token.as_str()), ("Do not Disturb", "DND"));
    assert_eq!(
        SendType::from_token(&token).map(SendType::wire),
        Some(ChatKind::Dnd)
    );

    // A bare `/afk` sends an empty body, which vmangos reads as a toggle; a non-empty one sets
    // the flag and the auto-reply (`ChatHandler.cpp:611-630`).
    let (text, token) = typed("/afk");
    assert_eq!((text.as_str(), token.as_str()), ("", "AFK"));
    assert_eq!(
        SendType::from_token(&token).map(SendType::wire),
        Some(ChatKind::Afk),
        "the bare toggle is a send too, not a no-op"
    );

    // The control: a token with no send still answers None.
    assert!(SendType::from_token("NOT_A_CHAT_TYPE").is_none());
    assert!(
        SendType::from_token("TEXT_EMOTE").is_none(),
        "a real ChatTypeInfo key that is nonetheless not sendable stays None"
    );
}
