//! Production action-feed range regressions for build 12340's `0x7ff480`.
use super::*;
use crate::app_core::state::Factions;
use wrathplus_formats::{SpellCatalog, SpellDisplay, SpellRange, SpellRangeCatalog};
use wrathplus_protocol::{
    types::{ActionButton, Field},
    ObjectFields,
};

fn verdict(flags: u32, friendly: bool, talent: bool, distance: f32) -> i64 {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    let mut actions = PlayerActions::default();
    actions.buttons.insert(
        0,
        ActionButton {
            slot: 0,
            action: 133,
            kind: ACTION_KIND_SPELL,
        },
    );
    let mut spells = Spells::empty_for_tests();
    spells.catalog = SpellCatalog::from_displays(
        [(
            133,
            SpellDisplay {
                range_index: 4,
                spell_family: 3,
                spell_family_flags: 1 << 5,
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
    );
    spells.ranges = SpellRangeCatalog::from_rows(
        [(
            4,
            SpellRange {
                min: 0.0,
                max: 30.0,
                flags,
            },
        )]
        .into_iter()
        .collect(),
    )
    .with_friendly([(4, (8.0, 40.0))].into_iter().collect());
    let mut mods = crate::spell::SpellModifiers::default();
    mods.set_class_family(3);
    if talent {
        mods.set(false, 5, crate::spell::OP_RANGE, 20);
    }
    app.insert_resource(actions)
        .insert_resource(spells)
        .insert_resource(mods)
        .init_resource::<crate::ui_macro::MacroBoundSpells>()
        .init_resource::<Cooldowns>()
        .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
        .init_resource::<crate::app_core::ui_flags::UiClock>()
        .init_resource::<AutoRepeatActive>()
        .init_resource::<crate::spell::PendingCast>()
        .init_resource::<crate::spell::QueuedMeleeSpell>()
        .init_resource::<crate::spell::ActiveChannel>()
        .init_resource::<crate::spell::SpellTargeting>()
        .init_resource::<crate::spell::HeldForPick>()
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::net::Reputations>()
        .init_resource::<Items>()
        .insert_resource(NetCommands(tx));
    // Equal faction templates are friendly; different hostile masks exercise the enemy column.
    let faction = |group_mask, enemy_group_mask| wrathplus_formats::FactionTemplate {
        group_mask,
        friend_group_mask: group_mask,
        enemy_group_mask,
        ..Default::default()
    };
    app.insert_resource(Factions::from_catalog(
        wrathplus_formats::FactionCatalog::from_rows(
            [(1, faction(2, 4)), (2, faction(4, 2))]
                .into_iter()
                .collect(),
            Default::default(),
        ),
    ));
    let store = |id| {
        ObjectStore(ObjectFields::from_fields(&[
            (Field::UnitHealth, 100),
            (Field::UnitFactionTemplate, id),
        ]))
    };
    app.world_mut()
        .spawn((SelfPlayer, Transform::default(), store(1)));
    let target = app
        .world_mut()
        .spawn((
            Transform::from_xyz(distance, 0.0, 0.0),
            store(if friendly { 1 } else { 2 }),
        ))
        .id();
    app.world_mut()
        .resource_mut::<crate::net::GuidIndex>()
        .0
        .insert(7, target);
    app.insert_resource(Selection {
        target: Some(target),
        guid: Some(7),
        last: None,
    });
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.add_systems(Update, feed_action_state);
    app.update();
    let script = app.world().non_send_resource::<UiScript>();
    assert!(script
        .eval::<bool>("return ActionHasRange(1) == 1")
        .unwrap());
    script
        .eval::<i64>("return IsActionInRange(1) or -1")
        .unwrap()
}

#[test]
fn action_range_modifier_precedes_the_reach_pad() {
    assert_eq!(verdict(0, false, true, 38.5), 1, "30 * 1.2 + 3 = 39");
    assert_eq!(
        verdict(0, false, true, 39.5),
        0,
        "not (30 + 3) * 1.2 = 39.6"
    );
}

#[test]
fn action_range_uses_the_assistable_units_friendly_columns() {
    assert_eq!(verdict(0, true, false, 42.5), 1, "friendly max40 + reach3");
    assert_eq!(verdict(0, true, false, 43.5), 0);
    assert_eq!(verdict(0, true, false, 10.5), 0, "friendly min8 + reach3");
    assert_eq!(verdict(0, true, false, 11.5), 1);
    assert_eq!(verdict(0, false, false, 32.5), 1, "hostile max30 + reach3");
    assert_eq!(verdict(0, false, false, 33.5), 0);
}

#[test]
fn action_ranged_row_adds_the_melee_floor_to_its_minimum() {
    assert_eq!(verdict(2, false, false, 4.5), 0, "0 + melee floor5");
    assert_eq!(verdict(2, false, false, 5.5), 1);
    assert_eq!(
        verdict(2, true, false, 12.5),
        0,
        "friendly8 + floor5, not reach3"
    );
    assert_eq!(verdict(2, true, false, 13.5), 1);
}
