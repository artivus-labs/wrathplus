//! The stock 3.3.5 buff bar (`BuffFrame.xml`, `BuffFrame.lua`) under a stubbed aura feed:
//! `UNIT_AURA` for `PlayerFrame.unit` runs `BuffFrame_Update`, which builds `BuffButtonN` and
//! `DebuffButtonN` on demand from `UnitAura` (`BuffFrame.lua:42-47,79-110,122-224`).

use wrathplus_ui::script::{AuraState, ScriptValue, UiScript, WeaponEnchant};

use super::test_ui::load_ui as load_xml;

fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    for f in [
        "Interface\\FrameXML\\GlobalStrings.lua",
        "Interface\\FrameXML\\Fonts.xml",
        r"Interface\FrameXML\BasicControls.xml",
        r"Interface\FrameXML\UIParent.xml",
        "Interface\\FrameXML\\GameTooltip.xml",
        "Interface\\FrameXML\\TextStatusBar.xml",
        "Interface\\FrameXML\\UnitFrame.xml",
        "Interface\\FrameXML\\PlayerFrame.xml",
        "Interface\\FrameXML\\BuffFrame.xml",
    ] {
        load_xml(&mut s, f);
    }
    // `"player"` resolves through the guid table, as the app seats it.
    s.set_unit_guids(&wrathplus_ui::script::UnitGuids {
        player: 1,
        ..Default::default()
    });
    // Timers on, then the CVar's applyFunc (`BuffFrame_UpdatePositions`, BuffFrame.lua:112-121).
    s.run(r#"SHOW_BUFF_DURATIONS = "1" BuffFrame_UpdatePositions()"#)
        .unwrap();
    s
}

/// An [`AuraState`]; `duration` 0 is permanent. The harness's `GetTime()` starts at 0.
fn aura(
    spell_id: u32,
    name: &str,
    helpful: bool,
    count: u8,
    debuff_type: Option<&str>,
    duration: f64,
) -> AuraState {
    AuraState {
        spell_id,
        name: Some(name.into()),
        icon: Some(format!("Interface\\Icons\\Spell_{spell_id}")),
        count,
        debuff_type: debuff_type.map(Into::into),
        duration,
        expiration_time: duration,
        helpful,
        cancelable: helpful,
        until_cancelled: duration == 0.0,
        channeled: false,
        ..Default::default()
    }
}

/// Push the list and fire `UNIT_AURA` for the player, as the app's feed does.
fn push(s: &mut UiScript, auras: Vec<AuraState>) {
    s.set_player_auras(auras);
    s.fire_event("UNIT_AURA", vec![ScriptValue::Str("player".into())]);
}

fn shown(s: &UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!("return {name} ~= nil and {name}:IsShown()"))
        .unwrap()
}

fn text(s: &UiScript, name: &str) -> String {
    s.eval::<String>(&format!("return tostring(({name}:GetText()) or \"\")"))
        .unwrap()
}

#[test]
fn buffs_and_debuffs_fill_their_own_rows_with_counts_and_the_dispel_tint() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(
        &mut s,
        vec![
            aura(1, "Fortitude", true, 0, None, 0.0),
            aura(2, "Rejuvenation", true, 3, None, 0.0),
            aura(3, "Polymorph", false, 0, Some("Magic"), 0.0),
            aura(4, "Rend", false, 0, None, 0.0),
        ],
    );
    assert!(shown(&s, "BuffButton1") && shown(&s, "BuffButton2"));
    assert!(
        !shown(&s, "BuffButton3"),
        "only two buffs, so no third button"
    );
    assert!(shown(&s, "DebuffButton1") && shown(&s, "DebuffButton2"));
    assert_eq!(
        s.eval::<String>("return BuffButton1Icon:GetTexture()")
            .unwrap(),
        "Interface\\Icons\\Spell_1"
    );
    assert!(
        !shown(&s, "BuffButton1Count"),
        "a single stack shows no count"
    );
    assert_eq!(text(&s, "BuffButton2Count"), "3");
    // `DebuffTypeColor` by dispel class, "none" for an untyped one (BuffFrame.lua:16-21,160-178).
    let tint = |n: u32| {
        s.eval::<(f64, f64, f64)>(&format!("return DebuffButton{n}Border:GetVertexColor()"))
            .unwrap()
    };
    let close = |(r, g, b): (f64, f64, f64), w: (f64, f64, f64)| {
        (r - w.0).abs() < 1e-3 && (g - w.1).abs() < 1e-3 && (b - w.2).abs() < 1e-3
    };
    assert!(close(tint(1), (0.2, 0.6, 1.0)), "Magic blue: {:?}", tint(1));
    assert!(close(tint(2), (0.8, 0.0, 0.0)), "none red: {:?}", tint(2));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// A timed aura arms `AuraButton_OnUpdate` and counts down through `SecondsToTimeAbbrev`; a
/// permanent one hides its duration (BuffFrame.lua:180-197,226-258).
#[test]
fn a_timed_aura_counts_down_and_a_permanent_one_shows_no_timer() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(
        &mut s,
        vec![
            aura(1, "Arcane Intellect", true, 0, None, 600.0),
            aura(2, "Trueshot Aura", true, 0, None, 0.0),
        ],
    );
    s.tick(0.1);
    assert!(shown(&s, "BuffButton1Duration"));
    assert_eq!(text(&s, "BuffButton1Duration"), "10 m");
    assert!(!shown(&s, "BuffButton2Duration"), "permanent: no timer");
    for _ in 0..3 {
        s.tick(200.0);
    }
    s.tick(0.1);
    assert!(
        text(&s, "BuffButton1Duration").ends_with(" s"),
        "seconds at the end"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Under `BUFF_WARNING_TIME` (31 s) a button takes the bar's flash alpha; above it, 1
/// (BuffFrame.lua:226-233,53-77).
#[test]
fn the_warning_flash_pulses_a_low_aura_but_leaves_a_long_one_solid() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(
        &mut s,
        vec![
            aura(1, "Short", true, 0, None, 20.0),
            aura(2, "Long", true, 0, None, 3600.0),
        ],
    );
    let mut lowest = 1.0f64;
    for _ in 0..20 {
        s.tick(0.1);
        lowest = lowest.min(s.eval::<f64>("return BuffButton1:GetAlpha()").unwrap());
        assert_eq!(s.eval::<f64>("return BuffButton2:GetAlpha()").unwrap(), 1.0);
    }
    assert!(lowest < 0.9, "the short one pulses: lowest {lowest}");
    assert!(lowest >= 0.3 - 1e-6, "never under BUFF_MIN_ALPHA");
}

/// A right click cancels the buff under the cursor: `CancelUnitBuff(self.unit, self:GetID(),
/// self.filter)` (BuffFrame.lua:264-270), queued by spell id.
#[test]
fn right_clicking_a_cancelable_buff_queues_its_spell_cancel() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(
        &mut s,
        vec![
            aura(10, "Fortitude", true, 0, None, 0.0),
            aura(11, "Mark of the Wild", true, 0, None, 0.0),
        ],
    );
    s.run("BuffButton2:Click(\"RightButton\")").unwrap();
    assert_eq!(s.take_cancel_aura_requests(), vec![11]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn an_emptied_bar_hides_every_button() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(
        &mut s,
        vec![
            aura(1, "A", true, 0, None, 60.0),
            aura(2, "B", false, 0, None, 0.0),
        ],
    );
    assert!(shown(&s, "BuffButton1") && shown(&s, "DebuffButton1"));
    push(&mut s, vec![]);
    assert!(!shown(&s, "BuffButton1") && !shown(&s, "DebuffButton1"));
    assert!(!shown(&s, "BuffButton1Duration"));
}

/// A refreshed aura re-reads its expiry on the next `UNIT_AURA` (BuffFrame.lua:186-192).
#[test]
fn a_refreshed_duration_reaches_the_bar() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(&mut s, vec![aura(1, "Renew", true, 0, None, 15.0)]);
    s.tick(10.0);
    s.tick(0.1);
    let secs = |s: &UiScript| -> i64 {
        let t = text(s, "BuffButton1Duration");
        t.trim_end_matches(" s")
            .parse()
            .unwrap_or_else(|_| panic!("{t:?}"))
    };
    let before = secs(&s);
    assert!((3..=5).contains(&before), "about 5 s left: {before}");
    let mut renewed = aura(1, "Renew", true, 0, None, 15.0);
    renewed.expiration_time = 10.1 + 15.0;
    push(&mut s, vec![renewed]);
    s.tick(0.1);
    assert!(secs(&s) >= 13, "renewed to about 15 s: {}", secs(&s));
}

/// `SHOW_BUFF_DURATIONS` "0" hides the timers and narrows the row spacing
/// (BuffFrame.lua:112-121,180-185).
#[test]
fn the_duration_switch_hides_the_timers() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(&mut s, vec![aura(1, "Renew", true, 0, None, 15.0)]);
    s.tick(0.1);
    assert!(shown(&s, "BuffButton1Duration"));
    s.run(r#"SHOW_BUFF_DURATIONS = "0" BuffFrame_UpdatePositions()"#)
        .unwrap();
    s.tick(0.1);
    assert!(!shown(&s, "BuffButton1Duration"));
    assert_eq!(s.eval::<i64>("return BUFF_ROW_SPACING").unwrap(), 5);
}

/// The weapon enchant row: an off-hand enchant takes slot 1 and the main hand the next, each
/// counting down from milliseconds, and the buff row starts left of the enchant frame
/// (BuffFrame.lua:381-461,309-313).
#[test]
fn the_temporary_enchant_row_shows_a_weapon_enchant_and_moves_the_buffs_aside() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    push(&mut s, vec![aura(1, "Fortitude", true, 0, None, 0.0)]);
    s.tick(0.1);
    assert!(!shown(&s, "TempEnchant1"));
    assert_eq!(
        s.eval::<String>("local _, rel = BuffButton1:GetPoint(1) return rel:GetName()")
            .unwrap(),
        "BuffFrame",
        "no enchants: the first buff is index 1, at BuffFrame (BuffFrame.lua:302-303)"
    );

    s.set_weapon_enchants(
        Some(WeaponEnchant {
            remaining_ms: Some(480_000),
            charges: 0,
        }),
        None,
    );
    s.tick(0.1);
    assert!(shown(&s, "TempEnchant1") && !shown(&s, "TempEnchant2"));
    assert_eq!(s.eval::<i64>("return TempEnchant1:GetID()").unwrap(), 16);
    assert_eq!(text(&s, "TempEnchant1Duration"), "8 m");
    assert_eq!(
        s.eval::<String>("local _, rel = BuffButton1:GetPoint(1) return rel:GetName()")
            .unwrap(),
        "TemporaryEnchantFrame",
        "the first buff hangs left of the enchants"
    );

    s.set_weapon_enchants(None, None);
    s.tick(0.1);
    assert!(!shown(&s, "TempEnchant1"));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
