//! The stance bar feed's unit tests: the `isActive` fork, the icon election and the aura edge.

use super::*;
use crate::ui_action::toggle::AFLAG_POSITIVE;
use wrathplus_formats::SpellDisplay;

/// One of our player's packet auras.
fn aura(spell_id: u32, flags: u8) -> StampedAura {
    StampedAura {
        aura: wrathplus_protocol::wrath_auras::NativeAura {
            slot: 0,
            spell_id,
            flags,
            level: 80,
            stacks: 1,
            caster_guid: None,
            max_duration_ms: None,
            remaining_ms: None,
        },
        received_at: 0.0,
    }
}

/// Devotion Aura as the shipped `Spell.dbc` has it: `AttributesEx2 0x10`, no `MOD_SHAPESHIFT`
/// effect, `ActiveIconID` 122, `StanceBarOrder` 0.
fn devotion_aura() -> SpellDisplay {
    SpellDisplay {
        attributes_ex2: ATTR_EX2_STANCE_BAR_FORCE,
        shapeshift_form: None,
        active_icon_id: 122,
        stance_bar_order: 0,
        ..Default::default()
    }
}

/// Both arms reach the block that elects `ActiveIconID` (`0x4b45c0`); the zero fallback
/// (`4b4763 je`) is reachable only on the form arm.
#[test]
fn the_active_icon_swap_reaches_both_arms() {
    let shield = Some("Interface\\Icons\\Spell_Holy_DevotionAura".to_string());
    let swirl = Some("Interface\\Icons\\Spell_Nature_WispSplode".to_string());

    // Cat Form, the form arm.
    let cat = SpellDisplay {
        shapeshift_form: Some(1),
        active_icon_id: 122,
        active_icon: swirl.clone(),
        icon: shield.clone(),
        ..Default::default()
    };
    assert_eq!(form_texture(&cat, true), swirl);
    assert_eq!(form_texture(&cat, false), shield);

    // A paladin aura, the force-admit arm, elects the same column (`4b4754`).
    let devotion = SpellDisplay {
        active_icon: swirl.clone(),
        icon: shield.clone(),
        ..devotion_aura()
    };
    assert_eq!(
        form_texture(&devotion, true),
        swirl,
        "the aura-scan hit falls through into the block that elects ActiveIconID"
    );
    assert_eq!(form_texture(&devotion, false), shield);

    // `4b475c` sets `isActive` before `4b4763` tests the column: a warrior stance is active
    // yet keeps `SpellIconID`.
    let stance = SpellDisplay {
        shapeshift_form: Some(17),
        active_icon_id: 0,
        active_icon: None,
        icon: shield.clone(),
        ..Default::default()
    };
    assert_eq!(
        form_texture(&stance, true),
        shield,
        "active and unswapped are decided by different bytes and do not imply each other"
    );
}

/// A form spell reads the form byte, never its aura; this pins the fork, not the data.
#[test]
fn a_shapeshift_form_still_latches_on_the_form_byte() {
    let battle_stance = SpellDisplay {
        shapeshift_form: Some(17),
        active_icon_id: 0,
        ..Default::default()
    };
    assert!(form_active(2457, &battle_stance, 17, &[]));
    assert!(!form_active(2457, &battle_stance, 1, &[]));
    assert!(
        !form_active(2457, &battle_stance, 0, &[aura(2457, AFLAG_POSITIVE)]),
        "out of the form: a live aura is not what the MOD_SHAPESHIFT arm reads"
    );
}

/// `0x802cb0`'s aura loop: a force-admitted spell is active while its positive aura is up.
#[test]
fn a_paladin_aura_lights_on_its_positive_packet_aura() {
    let devotion = devotion_aura();
    assert!(!form_active(465, &devotion, 0, &[]));
    assert!(form_active(465, &devotion, 0, &[aura(465, AFLAG_POSITIVE)]));
    assert!(
        !form_active(465, &devotion, 0, &[aura(465, 0x80)]),
        "the loop wants AFLAG_POSITIVE"
    );
    assert!(!form_active(
        465,
        &devotion,
        0,
        &[aura(7294, AFLAG_POSITIVE)]
    ));
}

/// The aura sites (`0x72213f`, `0x727830`) fire `UPDATE_SHAPESHIFT_FORM` on a stance aura's
/// apply and remove even when the bar's views did not move.
#[test]
fn a_stance_aura_coming_or_going_fires_the_form_edge() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "LOG = {} local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_SHAPESHIFT_FORM') \
         f:SetScript('OnEvent', function(_, e) table.insert(LOG, e) end)",
    )
    .unwrap();
    let fired = |s: &UiScript| s.eval::<i64>("local n = #LOG LOG = {} return n").unwrap();
    let view = ShapeshiftFormView {
        spell_id: 465,
        ..Default::default()
    };
    let mut memory = StanceMemory::default();
    let push = |s: &mut UiScript, m: &mut StanceMemory, auras: Vec<u32>| {
        push_stance(s, m, vec![view.clone()], Some(auras))
    };
    assert_eq!(push(&mut s, &mut memory, vec![]), FormsEdge::List);
    assert_eq!(fired(&s), 0);
    assert_eq!(push(&mut s, &mut memory, vec![465]), FormsEdge::Form);
    assert_eq!(fired(&s), 1, "the apply site");
    assert_eq!(push(&mut s, &mut memory, vec![465]), FormsEdge::Unchanged);
    assert_eq!(fired(&s), 0);
    assert_eq!(push(&mut s, &mut memory, vec![]), FormsEdge::Form);
    assert_eq!(fired(&s), 1, "the remove site");
}

/// A predicted form cancel (`0x721cf0`): the form byte reads 0, so the lit form goes dark and
/// `GetShapeshiftForm` answers 0; its start and its clear fire `UPDATE_SHAPESHIFT_FORM` once each,
/// whether or not the bar's own diff already did.
#[test]
fn a_predicted_form_cancel_reads_no_form_and_fires_once() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "LOG = {} local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_SHAPESHIFT_FORM') \
         f:SetScript('OnEvent', function(_, e) table.insert(LOG, e) end)",
    )
    .unwrap();
    let fired = |s: &UiScript| s.eval::<i64>("local n = #LOG LOG = {} return n").unwrap();
    let bear = SpellDisplay {
        shapeshift_form: Some(5),
        ..Default::default()
    };
    let view = |form_byte: u8| ShapeshiftFormView {
        spell_id: 5487,
        active: form_active(5487, &bear, form_byte, &[]),
        castable: true,
        ..Default::default()
    };
    let mut memory = StanceMemory::default();
    // In bear form.
    s.set_shapeshift_form_byte(5);
    push_stance(&mut s, &mut memory, vec![view(5)], None);
    fire_owed_form_event(&mut s, &memory, false);
    fired(&s);
    assert_eq!(s.eval::<i64>("return GetShapeshiftForm()").unwrap(), 1);

    // Predicted: the feed sees form 0; the bar's diff fires, the owed event does not repeat it.
    s.set_shapeshift_form_byte(0);
    assert_eq!(
        push_stance(&mut s, &mut memory, vec![view(0)], None),
        FormsEdge::Form
    );
    fire_owed_form_event(&mut s, &memory, true);
    assert_eq!(fired(&s), 1);
    assert_eq!(s.eval::<i64>("return GetShapeshiftForm()").unwrap(), 0);
    assert!(
        s.eval::<bool>("return select(3, GetShapeshiftFormInfo(1)) == nil")
            .unwrap(),
        "no MOD_SHAPESHIFT slot is active meanwhile"
    );

    // The server's form byte 0 clears it: the bar is unmoved, so the owed event fires alone.
    assert_eq!(
        push_stance(&mut s, &mut memory, vec![view(0)], None),
        FormsEdge::Unchanged
    );
    fire_owed_form_event(&mut s, &memory, true);
    assert_eq!(fired(&s), 1);
    fire_owed_form_event(&mut s, &memory, false);
    assert_eq!(fired(&s), 0);
}

/// Ghost Wolf has no stance bar (`AttributesEx2 & 0x2`): the owed event is the only one.
#[test]
fn a_predicted_cancel_with_no_bar_still_fires() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "N = 0 local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_SHAPESHIFT_FORM') \
         f:SetScript('OnEvent', function() N = N + 1 end)",
    )
    .unwrap();
    let mut memory = StanceMemory::default();
    push_stance(&mut s, &mut memory, vec![], None);
    fire_owed_form_event(&mut s, &memory, true);
    assert_eq!(s.eval::<i64>("return N").unwrap(), 1);
}

/// Exercise the catalog-to-VM seam used by `feed_shapeshift_bar`, not a flags setter fixture.
#[test]
fn current_form_feed_reads_catalog_flags_and_clears_a_missing_row() {
    let mut script = UiScript::new().unwrap();
    script.set_unit(
        "player",
        Some(wrathplus_ui::script::UnitState {
            exists: true,
            damage_bonus_stat: Some(0),
            ..Default::default()
        }),
    );
    let mut forms = std::collections::HashMap::from([
        (
            1,
            wrathplus_formats::ShapeshiftForm {
                flags: 0,
                ..Default::default()
            },
        ),
        (
            5,
            wrathplus_formats::ShapeshiftForm {
                flags: 0x20,
                ..Default::default()
            },
        ),
    ]);
    for (form, expected) in [(1, 0), (5, 40), (31, 0), (5, 40), (0, 0)] {
        push_current_form(&mut script, form, &forms);
        assert_eq!(
            script
                .eval::<i32>("return GetAttackPowerForStat(2, 40)")
                .unwrap(),
            expected
        );
    }
    forms.get_mut(&5).unwrap().flags = 0;
    push_current_form(&mut script, 5, &forms);
    assert_eq!(
        script
            .eval::<i32>("return GetAttackPowerForStat(2, 40)")
            .unwrap(),
        0
    );
}
