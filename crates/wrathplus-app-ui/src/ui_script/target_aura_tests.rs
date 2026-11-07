//! The stock 3.3.5 target frame's aura rows (`TargetFrame_UpdateAuras`, `TargetFrame.lua:342`)
//! under a stubbed feed.
//! BuffFrame.xml loads first for the `DebuffTypeColor` and `RefreshBuffs` the frames use.

use wrathplus_ui::script::{AuraState, QuadContent, ScriptValue, UiScript, UnitGuids, UnitState};

use super::test_ui::load_ui as load_xml;

fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_xml(&mut s, "Interface\\FrameXML\\GlobalStrings.lua");
    load_xml(&mut s, "Interface\\FrameXML\\Fonts.xml");
    load_xml(&mut s, "Interface\\FrameXML\\BasicControls.xml");
    // UIParent.xml comes first: `UnitPopup.lua:47` reads its `ITEM_QUALITY_COLORS` at file scope.
    load_xml(&mut s, r"Interface\FrameXML\UIParent.xml");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.lua");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\GameTooltip.xml"); // TOOLTIP_DEFAULT_* for the dropdowns
    load_xml(&mut s, "Interface\\FrameXML\\UIDropDownMenu.xml"); // TargetFrameDropDown's template
    load_xml(&mut s, "Interface\\FrameXML\\TextStatusBar.lua");
    load_xml(&mut s, "Interface\\FrameXML\\TextStatusBar.xml");
    load_xml(&mut s, "Interface\\FrameXML\\MainMenuBar.xml");
    load_xml(&mut s, "Interface\\FrameXML\\Cooldown.xml");
    load_xml(&mut s, "Interface\\FrameXML\\ActionButtonTemplate.xml");
    load_xml(&mut s, "Interface\\FrameXML\\ActionBarFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\BuffFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\CombatFeedback.xml");
    load_xml(&mut s, "Interface\\FrameXML\\UnitPopup.xml");
    load_xml(&mut s, "Interface\\FrameXML\\UnitFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\PlayerFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\PartyFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\TargetFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\PetFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\BonusActionBarFrame.xml");

    // `TargetofTargetFrame` loads shown (`TargetFrame.xml:515`) and hides only after the rows
    // lay out (`TargetFrame.lua:66`), so a first target lays its rows out for the 5-wide wrap;
    // one no-target change leaves it in the state every later target sees.
    s.fire_event("PLAYER_TARGET_CHANGED", vec![]);
    s
}

/// Target a unit of the given reaction (2 = hostile, 5 = friendly) carrying `auras`.
fn target(s: &mut UiScript, reaction: u8, auras: Vec<AuraState>) {
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            name: Some("Subject".into()),
            health: 40,
            max_health: 40,
            level: 5,
            reaction,
            ..UnitState::default()
        }),
    );
    s.set_unit_guids(&UnitGuids {
        player: 1,
        target: TARGET,
        ..Default::default()
    });
    s.set_unit_auras(TARGET, Some(auras));
    s.fire_event("PLAYER_TARGET_CHANGED", vec![]);
}

/// The target's guid, which `"target"` resolves to.
const TARGET: u64 = 0xF130_0000_0000_0001;

fn debuff(spell_id: u32, name: &str, count: u8, debuff_type: Option<&str>) -> AuraState {
    AuraState {
        spell_id,
        name: Some(name.into()),
        icon: Some(format!("Interface\\Icons\\Spell_{spell_id}")),
        count,
        debuff_type: debuff_type.map(Into::into),
        // No duration: the cooldown sweep stays hidden.
        duration: 0.0,
        expiration_time: 0.0,
        helpful: false,
        cancelable: false,
        // Only the player's auras carry `untilCancelled`, read by `GetPlayerBuff` alone.
        until_cancelled: false,
        channeled: false,
        ..Default::default()
    }
}

fn buff(spell_id: u32, name: &str) -> AuraState {
    AuraState {
        spell_id,
        name: Some(name.into()),
        icon: Some(format!("Interface\\Icons\\Spell_{spell_id}")),
        count: 1,
        debuff_type: None,
        duration: 0.0,
        expiration_time: 0.0,
        helpful: true,
        cancelable: false,
        until_cancelled: false,
        channeled: false,
        ..Default::default()
    }
}

fn shown(s: &UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!("return {name}:IsVisible()"))
        .unwrap()
}

/// The first anchor of `name`: (point, relative frame's name, relativePoint, x, y).
fn anchor(s: &UiScript, name: &str) -> (String, String, String, f64, f64) {
    s.eval(&format!(
        r#"local p, rel, rp, x, y = {name}:GetPoint()
           return p, (rel and rel:GetName()) or "<screen>", rp, x, y"#
    ))
    .unwrap()
}

/// Whether the named button was ever created.
fn exists(s: &UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!("return _G[{name:?}] ~= nil"))
        .unwrap()
}

fn size(s: &UiScript, name: &str) -> (f64, f64) {
    s.eval(&format!("return {name}:GetWidth(), {name}:GetHeight()"))
        .unwrap()
}

#[test]
fn a_hostile_target_draws_debuffs_first_with_tint_and_count() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    target(
        &mut s,
        2,
        vec![
            debuff(589, "Shadow Word: Pain", 1, Some("Magic")),
            debuff(772, "Rend", 3, None),
            buff(1126, "Mark of the Wild"),
        ],
    );

    assert!(shown(&s, "TargetFrameDebuff1"), "first debuff shows");
    assert!(shown(&s, "TargetFrameDebuff2"), "second debuff shows");
    // 3.3.5 creates a button only for an aura that exists (`TargetFrame.lua:413-421`).
    assert!(
        !exists(&s, "TargetFrameDebuff3"),
        "no third debuff, no button"
    );
    assert!(shown(&s, "TargetFrameBuff1"), "the buff shows");
    assert!(!exists(&s, "TargetFrameBuff2"), "no second buff");

    // Hostile: debuffs open at BOTTOMLEFT (5, 32), buffs under the debuff container
    // (`TargetFrame_UpdateBuffAnchor`, `:535`).
    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameDebuff1");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrame", "BOTTOMLEFT", 5.0, 32.0),
        "hostile: debuffs first"
    );
    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameBuff1");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrameDebuffs", "BOTTOMLEFT", 0.0, -3.0),
        "hostile: buffs under the debuff row"
    );

    // Not cast by the player or its pet: SMALL_AURA_SIZE 17, borders 2 wider (`:13`, `:593`).
    assert_eq!(size(&s, "TargetFrameDebuff1"), (17.0, 17.0));
    assert_eq!(size(&s, "TargetFrameDebuff1Border"), (19.0, 19.0));
    assert_eq!(size(&s, "TargetFrameBuff1"), (17.0, 17.0));

    // Stack count shows only above 1.
    assert_eq!(
        s.eval::<String>(r#"return tostring((TargetFrameDebuff2Count:GetText()) or "")"#)
            .unwrap(),
        "3"
    );
    assert_eq!(
        s.eval::<String>(r#"return tostring((TargetFrameDebuff1Count:GetText()) or "")"#)
            .unwrap(),
        ""
    );

    // `DebuffTypeColor` Magic is 0.20, 0.60, 1.00 and "none" 0.80, 0, 0 (`BuffFrame.lua:10`).
    s.resolve();
    let tints: Vec<[f32; 4]> = s
        .extract()
        .into_iter()
        .filter_map(|q| match q.content {
            QuadContent::Texture {
                path: Some(p),
                color: Some(c),
                ..
            } if p.contains("UI-Debuff-Overlays") => Some(c),
            _ => None,
        })
        .collect();
    assert!(
        tints
            .iter()
            .any(|c| (c[0] - 0.20).abs() < 1e-3 && (c[1] - 0.60).abs() < 1e-3),
        "a Magic-tinted border drew, got {tints:?}"
    );
    assert!(
        tints
            .iter()
            .any(|c| (c[0] - 0.80).abs() < 1e-3 && c[1].abs() < 1e-3),
        "an untyped border wears the none-red, got {tints:?}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn a_friendly_target_puts_the_buff_row_first() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    target(
        &mut s,
        5,
        vec![
            buff(1126, "Mark of the Wild"),
            debuff(589, "Shadow Word: Pain", 1, Some("Magic")),
        ],
    );

    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameBuff1");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrame", "BOTTOMLEFT", 5.0, 32.0),
        "friendly: buffs first"
    );
    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameDebuff1");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrameBuffs", "BOTTOMLEFT", 0.0, -3.0),
        "friendly: debuffs under the buff container (`TargetFrame_UpdateDebuffAnchor`, `:563`)"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Six small auras fill the 122px row (6 × 17 + 5 × 4); the seventh opens a second row under
/// the first (`TargetFrame_UpdateAuraPositions`, `:489`).
#[test]
fn a_seventh_aura_opens_a_second_row_under_the_first() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    let debuffs: Vec<AuraState> = (0..7)
        .map(|i| debuff(1000 + i, &format!("D{i}"), 1, None))
        .collect();
    target(&mut s, 2, debuffs);

    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameDebuff6");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrameDebuff5", "TOPRIGHT", 4.0, 0.0),
        "the sixth still fits the row"
    );
    let (p, rel, rp, x, y) = anchor(&s, "TargetFrameDebuff7");
    assert_eq!(
        (p.as_str(), rel.as_str(), rp.as_str(), x, y),
        ("TOPLEFT", "TargetFrameDebuff1", "BOTTOMLEFT", 0.0, -3.0),
        "the seventh wraps"
    );
    assert_eq!(size(&s, "TargetFrameDebuff7"), (17.0, 17.0));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn clearing_the_list_or_the_target_hides_the_buttons() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    target(&mut s, 2, vec![debuff(589, "Pain", 1, Some("Magic"))]);
    assert!(shown(&s, "TargetFrameDebuff1"));

    s.set_unit_auras(TARGET, Some(vec![]));
    s.fire_event("UNIT_AURA", vec![ScriptValue::Str("target".into())]);
    assert!(
        !shown(&s, "TargetFrameDebuff1"),
        "an emptied list hides the button"
    );

    // Deselect: the frame and its buttons hide; the token clears without a UNIT_AURA.
    target(&mut s, 2, vec![debuff(589, "Pain", 1, Some("Magic"))]);
    assert!(shown(&s, "TargetFrameDebuff1"));
    s.set_unit("target", None);
    s.set_unit_auras(TARGET, None);
    s.fire_event("PLAYER_TARGET_CHANGED", vec![]);
    assert!(!shown(&s, "TargetFrameDebuff1"), "no target, no buttons");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
