//! The ten stock 3.3.5 micro buttons, loaded behind `MainMenuBarArtFrame`, which they anchor to.
//! Below its level a button greys rather than hides, so the row never closes up.

use wrathplus_ui::script::{QuadContent, ScriptValue, TexCoords, UiScript, UnitState};

const ROW: [&str; 10] = [
    "CharacterMicroButton",
    "SpellbookMicroButton",
    "TalentMicroButton",
    "AchievementMicroButton",
    "QuestLogMicroButton",
    "SocialsMicroButton",
    "PVPMicroButton",
    "LFDMicroButton",
    "MainMenuMicroButton",
    "HelpMicroButton",
];

/// At 1024 wide the 1024-wide bar sits at x 0, so every stock offset is a screen coordinate.
fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    for file in [
        // The labels each button's OnLoad reads, through `TEXT()`.
        "Interface\\FrameXML\\GlobalStrings.lua",
        "Interface\\FrameXML\\Fonts.xml",
        "Interface\\FrameXML\\BasicControls.xml",
        r"Interface\FrameXML\UIParent.xml",
        r"Interface\FrameXML\MoneyFrame.lua",
        r"Interface\FrameXML\MoneyFrame.xml",
        "Interface\\FrameXML\\GameTooltip.xml",
        "Interface\\FrameXML\\TextStatusBar.lua",
        "Interface\\FrameXML\\TextStatusBar.xml",
        "Interface\\FrameXML\\MainMenuBar.xml",
        r"Interface\FrameXML\MainMenuBarMicroButtons.xml",
        "Interface\\FrameXML\\Cooldown.xml",
        "Interface\\FrameXML\\ActionButtonTemplate.xml",
        "Interface\\FrameXML\\ActionBarFrame.xml",
        "Interface\\FrameXML\\BonusActionBarFrame.xml",
    ] {
        super::test_ui::load_ui(&mut s, file);
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    s
}

fn set_player_level(s: &mut UiScript, level: u32) {
    s.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            level,
            // 3.3.5's PVP window builds a faction texture path on world entry.
            faction_group: Some("Alliance".into()),
            ..Default::default()
        }),
    );
    s.fire_event("UNIT_LEVEL", vec![ScriptValue::Str("player".into())]);
}

/// `CharacterMicroButton` sits at the art frame's `BOTTOMLEFT` +(552, 2), each button 28x58
/// (MainMenuBarMicroButtons.xml:4-7, :33-38), chained at -3 for a 25 px stride, at any level.
#[test]
fn the_micro_row_sits_where_the_reference_puts_it() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    for level in [9, 80] {
        set_player_level(&mut s, level);
        s.resolve();
        for (i, name) in ROW.iter().enumerate() {
            let (left, bottom, w, h) = s
                .eval::<(f64, f64, f64, f64)>(&format!(
                    "return {name}:GetLeft(), {name}:GetBottom(), {name}:GetWidth(), {name}:GetHeight()"
                ))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((w, h), (28.0, 58.0), "{name} size");
            assert_eq!(
                left,
                552.0 + 25.0 * i as f64,
                "{name} left edge at level {level}"
            );
            assert_eq!(bottom, 2.0, "{name} sits 2 above the bar's bottom");
        }
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `HitRectInsets top="18"` (`MainMenuBarMicroButtons.xml:8`): the art fills the lower 40 of 58.
#[test]
fn the_transparent_top_of_a_micro_button_does_not_capture_the_mouse() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    s.resolve();

    // Mid-button (552 + 14); the art band is y 2..42, the inset header y 42..60.
    assert_eq!(
        s.hit_test_name(566.0, 20.0).as_deref(),
        Some("CharacterMicroButton"),
        "the art band takes the mouse"
    );
    assert_ne!(
        s.hit_test_name(566.0, 50.0).as_deref(),
        Some("CharacterMicroButton"),
        "the inset header must be transparent to the mouse"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `MicroButtonPortrait` samples the `"player"` portrait through a crop window, so the 18x25
/// region shows a face; pushing swaps the window and dims it (`MainMenuBarMicroButtons.lua:109`).
#[test]
fn the_character_button_carries_the_player_portrait_through_the_reference_crop() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    // 3.3.5 seats the portrait on world entry (MainMenuBarMicroButtons.lua:145-146).
    set_player_level(&mut s, 80);
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    s.resolve();

    // Only the micro button's own portrait: the seated session's unit frames draw portraits too.
    let window = |s: &mut UiScript| {
        s.extract()
            .into_iter()
            .filter(|q| {
                s.target_owner_name(q.target)
                    .is_some_and(|n| n.contains("MicroButton"))
            })
            .find_map(|q| match q.content {
                QuadContent::Texture {
                    portrait_unit: Some(unit),
                    tex_coords,
                    circular,
                    ..
                } => Some((unit, tex_coords, circular)),
                _ => None,
            })
            .expect("the micro button's portrait quad")
    };

    let (unit, coords, circular) = window(&mut s);
    assert_eq!(unit, "player");
    assert!(
        circular,
        "the reference's round stencil lives in the bake's own UV space, so the crop below yields \
         a rectangular slice OF a masked face — not an ellipse fitted to this 18x25 region"
    );
    let round4 = |c: TexCoords| match c {
        TexCoords::Rect(e) => e.map(|v| (v * 10_000.0).round() / 10_000.0),
        TexCoords::Corners(_) => panic!("the 4-edge form"),
    };
    assert_eq!(
        round4(coords.expect("a crop window")),
        [0.2, 0.8, 0.0666, 0.9],
        "the normal window (CharacterMicroButton_SetNormal)"
    );

    s.run("CharacterMicroButton_SetPushed()").unwrap();
    s.resolve();
    let (_, coords, _) = window(&mut s);
    assert_eq!(
        round4(coords.expect("a crop window")),
        [0.2666, 0.8666, 0.0, 0.8333],
        "the held-down window (CharacterMicroButton_SetPushed)"
    );
    assert_eq!(
        s.eval::<f64>("return MicroButtonPortrait:GetAlpha()")
            .unwrap(),
        0.5,
        "…and the face dims while the button is down"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Below `SHOW_TALENT_LEVEL` (10) `UpdateMicroButtons` disables the talent button, which dims to 0.5
/// and keeps its seat; its hover adds the red "available at" line
/// (MainMenuBarMicroButtons.lua:36-46, .xml:12-20).
#[test]
fn the_talent_button_greys_below_level_ten_and_says_when_it_comes() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    s.run("SHOW_NEWBIE_TIPS = \"1\"").unwrap();

    set_player_level(&mut s, 9);
    s.resolve();
    assert_eq!(
        s.eval::<(i64, f64)>("return TalentMicroButton:IsEnabled(), TalentMicroButton:GetAlpha()")
            .unwrap(),
        (0, wrathplus_ui::widget::alpha_unit(127)),
        "disabled and dimmed to 0.5, the alpha byte 127, before 10"
    );
    assert!(s
        .eval::<bool>("return TalentMicroButton:IsVisible()")
        .unwrap());
    super::test_ui::hover(&mut s, "TalentMicroButton");
    let last = s
        .eval::<String>("return _G['GameTooltipTextLeft' .. GameTooltip:NumLines()]:GetText()")
        .unwrap();
    assert_eq!(
        last,
        s.eval::<String>("return format(FEATURE_BECOMES_AVAILABLE_AT_LEVEL, 10)")
            .unwrap()
    );
    super::test_ui::unhover(&mut s);

    set_player_level(&mut s, 10);
    s.resolve();
    assert_eq!(
        s.eval::<(i64, f64)>("return TalentMicroButton:IsEnabled(), TalentMicroButton:GetAlpha()")
            .unwrap(),
        (1, 1.0),
        "the button comes up at 10"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The template's `OnEnter` (MainMenuBarMicroButtons.xml:12-20) writes the label, the newbie line
/// and a blank line; the spellbook's own `OnEnter` (:104-107) omits the blank, and the main menu
/// button's (:424-427) only arms its latency poll, whose plate `perf_bar_tests` covers. A bound
/// label gains its key on `UPDATE_BINDINGS`.
#[test]
fn every_micro_button_hovers_with_its_reference_explanation() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    set_player_level(&mut s, 80);
    s.run("SHOW_NEWBIE_TIPS = \"1\"").unwrap();
    crate::ui_script::load_stock_bindings(&mut s);
    assert_eq!(
        s.eval::<Option<i64>>("return SetBinding(\"C\", \"TOGGLECHARACTER0\")")
            .unwrap(),
        Some(1)
    );
    s.fire_event("UPDATE_BINDINGS", vec![]);
    s.resolve();

    for (button, label, newbie, lines) in [
        (
            "CharacterMicroButton",
            "CHARACTER_BUTTON",
            "NEWBIE_TOOLTIP_CHARACTER",
            3,
        ),
        (
            "SpellbookMicroButton",
            "SPELLBOOK_ABILITIES_BUTTON",
            "NEWBIE_TOOLTIP_SPELLBOOK",
            2,
        ),
        (
            "TalentMicroButton",
            "TALENTS_BUTTON",
            "NEWBIE_TOOLTIP_TALENTS",
            3,
        ),
        (
            "AchievementMicroButton",
            "ACHIEVEMENT_BUTTON",
            "NEWBIE_TOOLTIP_ACHIEVEMENT",
            4, // no achievement yet: disabled, plus the red "available at" line (.lua:103-108)
        ),
        (
            "QuestLogMicroButton",
            "QUESTLOG_BUTTON",
            "NEWBIE_TOOLTIP_QUESTLOG",
            3,
        ),
        (
            "SocialsMicroButton",
            "SOCIAL_BUTTON",
            "NEWBIE_TOOLTIP_SOCIAL",
            3,
        ),
        ("PVPMicroButton", "PLAYER_V_PLAYER", "NEWBIE_TOOLTIP_PVP", 3),
        (
            "LFDMicroButton",
            "DUNGEONS_BUTTON",
            "NEWBIE_TOOLTIP_LFGPARENT",
            3,
        ),
        ("HelpMicroButton", "HELP_BUTTON", "NEWBIE_TOOLTIP_HELP", 3),
    ] {
        super::test_ui::hover(&mut s, button);
        assert_eq!(
            s.eval::<i64>("return GameTooltip:NumLines()").unwrap(),
            lines,
            "{button}: the label, the explanation{}",
            if lines >= 3 { ", the blank line" } else { "" }
        );
        let label = s.eval::<String>(&format!("return {label}")).unwrap();
        let line1 = s
            .eval::<String>("return GameTooltipTextLeft1:GetText()")
            .unwrap();
        assert!(
            line1.starts_with(&label),
            "{button}: line 1 is {line1:?}, expected it to open with {label:?}"
        );
        assert_eq!(
            s.eval::<String>("return GameTooltipTextLeft2:GetText()")
                .unwrap(),
            s.eval::<String>(&format!("return {newbie}")).unwrap(),
            "{button}: line 2 is the reference's {newbie}, verbatim"
        );
        super::test_ui::unhover(&mut s);
    }

    super::test_ui::hover(&mut s, "CharacterMicroButton");
    assert_eq!(
        s.eval::<String>("return GameTooltipTextLeft1:GetText()")
            .unwrap(),
        s.eval::<String>(
            "return CHARACTER_BUTTON .. \" \" .. NORMAL_FONT_COLOR_CODE .. \"(C)\" .. FONT_COLOR_CODE_CLOSE"
        )
        .unwrap(),
        "the key read on UPDATE_BINDINGS"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `UpdateMicroButtons` holds a button down while its panel is open, and a ding pulses the talent
/// button for 60 s unless the character sheet is open (MainMenuBarMicroButtons.lua:173-179,
/// `SetButtonPulse`, UIParent.lua:2624-2631).
#[test]
fn a_micro_button_pushes_while_its_panel_is_open_and_the_talent_button_pulses_on_a_ding() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    set_player_level(&mut s, 60);
    s.resolve();
    let state = |s: &UiScript, button: &str| {
        s.eval::<String>(&format!("return {button}:GetButtonState()"))
            .unwrap()
    };

    assert_eq!(state(&s, "SpellbookMicroButton"), "NORMAL");
    s.run("ToggleSpellBook(BOOKTYPE_SPELL)").unwrap();
    assert_eq!(
        state(&s, "SpellbookMicroButton"),
        "PUSHED",
        "open book ⇒ held down"
    );
    s.run("ToggleSpellBook(BOOKTYPE_SPELL)").unwrap();
    assert_eq!(state(&s, "SpellbookMicroButton"), "NORMAL");

    s.fire_event("PLAYER_LEVEL_UP", vec![ScriptValue::Int(61)]);
    assert_eq!(
        s.eval::<f64>("return TalentMicroButton.pulseTimeLeft")
            .unwrap(),
        60.0,
        "the sixty-second pulse"
    );
    assert!(
        s.eval::<bool>(
            "for _, b in ipairs(PULSEBUTTONS) do if b == TalentMicroButton then return true end end \
             return false"
        )
        .unwrap(),
        "on the pulse list"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
