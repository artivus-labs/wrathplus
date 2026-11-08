use super::integration_tests::Fixture;
use super::{cast_target, AutoRepeatActive, CastLadder};
use crate::app_core::state::Spells;
use bevy::ecs::system::RunSystemOnce;
use std::collections::HashMap;
use wrathplus_formats::{SpellCatalog, SpellDisplay};
const AUTO_SHOT: u32 = 75;

#[test]
fn issue128_press_does_not_scale_reach_with_op5() {
    let mut f = Fixture::new(45.3);
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    let mut spells = f.world.resource_mut::<Spells>();
    spells.catalog = SpellCatalog::from_displays(HashMap::from([(
        AUTO_SHOT,
        SpellDisplay {
            range_index: 114,
            targets: 0x80,
            spell_family: 3,
            spell_family_flags: 1 << 5,
            ..Default::default()
        },
    )]));
    let mut mods = f.world.resource_mut::<crate::spell::SpellModifiers>();
    mods.set_class_family(3);
    mods.set(false, 5, crate::spell::OP_RANGE, 20);
    f.world
        .run_system_once(
            |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
                ladder.send_spell(AUTO_SHOT, &targeting.context(), false);
            },
        )
        .unwrap();
    assert!(
        f.rx.is_empty(),
        "35 * 1.2 + 3 = 45, not (35 + 3) * 1.2 = 45.6"
    );
    assert!(!f
        .world
        .resource::<crate::app_core::errors::CastErrors>()
        .0
        .is_empty());
}

fn press(f: &mut Fixture, attributes: u32) {
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.world.resource_mut::<Spells>().catalog = SpellCatalog::from_displays(HashMap::from([(
        AUTO_SHOT,
        SpellDisplay {
            range_index: 114,
            targets: 2,
            attributes,
            ..Default::default()
        },
    )]));
    f.world
        .run_system_once(
            move |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
                let ctx = targeting.context();
                ladder.send_spell(AUTO_SHOT, &ctx, false);
            },
        )
        .unwrap();
}

#[test]
fn issue128_press_uses_friendly_columns_not_spell_polarity() {
    for friendly in [false, true] {
        let mut f = Fixture::new(40.0);
        let ranges = std::mem::take(&mut f.world.resource_mut::<Spells>().ranges);
        f.world.resource_mut::<Spells>().ranges =
            ranges.with_friendly(HashMap::from([(114, (0.0, 45.0))]));
        if friendly {
            use crate::app_core::session::ObjectStore;
            use wrathplus_protocol::{types::Field, ObjectFields};
            f.world
                .insert_resource(crate::spell::targeting::corpse_fixture::factions());
            let target = f
                .world
                .resource::<crate::app_core::state::Selection>()
                .target
                .unwrap();
            for entity in [f.me, target] {
                f.world
                    .entity_mut(entity)
                    .insert(ObjectStore(ObjectFields::from_fields(&[
                        (Field::ObjectType, 0x19),
                        (Field::UnitHealth, 100),
                        (Field::UnitFactionTemplate, 1),
                        (Field::UnitCombatreach, 1.5f32.to_bits()),
                    ])));
            }
        }
        press(&mut f, 0);
        assert_eq!(
            !f.rx.is_empty(),
            friendly,
            "hostile max 38, friendly max 48"
        );
    }
}

#[test]
fn issue128_equipped_ranged_weapon_scales_authored_max_before_pad() {
    use crate::app_core::session::ObjectStore;
    use crate::net::GuidIndex;
    use wrathplus_protocol::{types::Field, ObjectFields};
    for attributes in [0, 2] {
        let mut f = Fixture::new(45.0);
        f.world
            .entity_mut(f.me)
            .insert(ObjectStore(ObjectFields::from_field_slots(&[
                (Field::ObjectType, 0, 0x19),
                (Field::UnitHealth, 0, 100),
                (Field::UnitCombatreach, 0, 1.5f32.to_bits()),
                (Field::PlayerInvSlotHead, 34, 123),
                (Field::PlayerInvSlotHead, 35, 0),
            ])));
        let weapon = f
            .world
            .spawn(ObjectStore(ObjectFields::from_fields(&[(
                Field::ObjectEntry,
                55,
            )])))
            .id();
        f.world.resource_mut::<GuidIndex>().0.insert(123, weapon);
        let mut template = crate::items::test_template("range fixture");
        template.ranged_mod_range = 150.0;
        f.world
            .resource_mut::<crate::items::Items>()
            .insert_template(55, Some(template));
        press(&mut f, attributes);
        assert_eq!(!f.rx.is_empty(), attributes == 2, "35 * 1.5 + 3 = 55.5");
    }
}

#[test]
fn issue128_self_binding_uses_caster_pose_unit_and_friendly_relation() {
    let mut f = Fixture::new(500.0);
    f.world
        .run_system_once(|targeting: cast_target::CastTargeting| {
            let selected = targeting.context();
            assert!(!selected.range.friendly);
            let own = selected.self_bound(true);
            assert_eq!(own.selection_guid, own.self_guid);
            assert_eq!(own.range.target_pos, own.range.self_pos);
            assert_eq!(
                own.range.target,
                crate::spell::cast_range::RangeTarget::Unit(own.range.caster)
            );
            assert!(own.range.friendly);
        })
        .unwrap();
}

#[test]
fn issue128_auto_self_fallback_does_not_use_hostile_selection_range() {
    let mut f = Fixture::new(500.0);
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.world.resource_mut::<Spells>().catalog = SpellCatalog::from_displays(HashMap::from([(
        AUTO_SHOT,
        SpellDisplay {
            range_index: 114,
            targets: 0x100,
            ..Default::default()
        },
    )]));
    f.world
        .run_system_once(
            |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
                ladder.send_spell(AUTO_SHOT, &targeting.context(), false);
            },
        )
        .unwrap();
    assert!(
        f.rx.try_iter().any(|cmd| matches!(
            cmd,
            crate::net::ClientCommand::CastSpell {
                target: Some(7),
                ..
            }
        )),
        "the self fallback sends despite a hostile selection 500 yards away"
    );
}
