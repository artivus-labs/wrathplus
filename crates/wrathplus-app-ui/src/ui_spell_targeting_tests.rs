//! [`super::feed_targeting_to_vm`]'s VM tests, on the cast ladder harness `spell::targeting` shares.

use super::*;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::net::{ClientCommand, Guid};
use crate::spell::targeting::test_support::*;
use std::collections::HashMap;
use wrathplus_protocol::types::Field;

/// `SpellCanTargetUnit` resolves a chain as `SpellTargetUnit` does and answers the verdict of the
/// unit it ends on (1.12 `0x6e6d3a` into `0x515970`, then `0x6e6460`; 12340 not located): the ally
/// we and our pet target, in range of the spell or not, and nobody past a unit with no target.
#[test]
fn spell_can_target_unit_follows_a_target_chain() {
    const PET: u64 = 0xF140_0000_0000_0077;
    // `OBJECT_FIELD_TYPE`, `UNIT_FIELD_HEALTH` and `UNIT_FIELD_TARGET`, the last a 2-field guid.
    let unit = |kind: u32, target: u64| {
        ObjectStore(wrathplus_protocol::ObjectFields::from_field_slots(&[
            (Field::ObjectType, 0, kind),
            (Field::UnitHealth, 0, 100),
            (Field::UnitTarget, 0, target as u32),
            (Field::UnitTarget, 1, (target >> 32) as u32),
        ]))
    };
    let can = |distance: f32, token: &str| {
        let (mut world, _rx, ally) = unit_world(distance);
        // We and our pet target the ally; the ally targets nobody.
        let me = world
            .query_filtered::<Entity, With<SelfPlayer>>()
            .single(&world)
            .unwrap();
        world.entity_mut(me).insert(unit(0x19, ALLY));
        world.entity_mut(ally).insert(unit(0x09, 0));
        let pet = world
            .spawn((Guid(PET), GlobalTransform::default(), unit(0x09, ALLY)))
            .id();
        let mut bar = crate::app_core::state::PetBar::default();
        bar.spells.pet_guid = PET;
        world.insert_resource(bar);
        let mut index = world.resource_mut::<crate::net::GuidIndex>();
        index.0.insert(PET, pet);
        arm(&mut world, HEAL, 0x0002);
        // The index of us and the ally, then the aura feed's inputs with the chain's units:
        // each held unit and the target it names.
        publish_guids(&mut world);
        world.non_send_resource_mut::<UiScript>().set_unit_guids(
            &wrathplus_ui::script::UnitGuids {
                player: ME,
                pet: PET,
                target: ALLY,
                held: HashMap::from([(ME, ALLY), (PET, ALLY), (ALLY, 0)]),
                ..Default::default()
            },
        );
        world
            .run_system_cached(feed_targeting_to_vm)
            .expect("the feed runs");
        world
            .non_send_resource::<UiScript>()
            .eval::<bool>(&format!("return SpellCanTargetUnit({token:?}) == 1"))
            .expect("a boolean")
    };
    for token in ["pettarget", "playertarget", "PETTARGET", "target"] {
        assert!(can(10.0, token), "{token}: the ally in range");
        assert!(!can(40.0, token), "{token}: the ally out of range");
    }
    for token in ["pettargettarget", "playertargettarget", "party1target"] {
        assert!(!can(10.0, token), "{token}: past the ally, nobody");
    }
}

/// The press (`resolve_cast_target`), the click and `SpellTargetUnit` ([`bind_target_unit`]) and
/// the hover verdict (`SpellCanTargetUnit`) all ask [`crate::spell::cast_target::unit_binds`],
/// so one table of units decides all three: `BindTarget`'s gates refuse the same units at
/// each, and the control row beside each refusal binds at each.
#[test]
fn the_press_the_click_and_the_hover_bind_the_same_units() {
    use crate::names::{CreatureRecord, NameCache};
    use crate::spell::cast_target::{
        cast_target_mask, resolve_cast_target, CastCandidates, CastWireTarget, TargetRelations,
    };
    use wrathplus_formats::SpellDisplay;
    use wrathplus_protocol::ObjectFields;

    const SPELL: u32 = 3000;
    /// Absolute descriptor indices: `OBJECT_FIELD_ENTRY`, `UNIT_FIELD_HEALTH`,
    /// `UNIT_FIELD_FLAGS` and `UNIT_DYNAMIC_FLAGS`.
    const ENTRY: u16 = Field::ObjectEntry.index();
    const HEALTH: u16 = Field::UnitHealth.index();
    const FLAGS: u16 = Field::UnitFlags.index();
    const DYNAMIC_FLAGS: u16 = Field::UnitDynamicFlags.index();
    const HUMANOID: u32 = 69;
    const BEAST: u32 = 70;

    let make_names = || {
        let mut names = NameCache::default();
        for (entry, creature_type) in [(HUMANOID, 7), (BEAST, 1)] {
            names.insert_creature(
                entry,
                Some(CreatureRecord {
                    name: String::new(),
                    subname: None,
                    creature_type,
                    pet_family: 0,
                    rank: 0,
                    type_flags: 0,
                    civilian: false,
                    racial_leader: false,
                    display_id: 0,
                    modifiers: None,
                }),
            );
        }
        names
    };
    fn plain() -> SpellDisplay {
        SpellDisplay {
            targets: 0x2,
            ..Default::default()
        }
    }
    fn hibernate() -> SpellDisplay {
        SpellDisplay {
            target_creature_type: 0x3,
            ..plain()
        }
    }
    fn excluding() -> SpellDisplay {
        SpellDisplay {
            attributes_ex: 0x0008_0000,
            ..plain()
        }
    }
    fn skinning() -> SpellDisplay {
        SpellDisplay {
            targets: 0x402,
            ..Default::default()
        }
    }
    fn raise() -> SpellDisplay {
        SpellDisplay {
            attributes_ex2: 1,
            ..plain()
        }
    }
    // (label, spell, the caster is the candidate, the candidate's fields, binds)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, fn() -> SpellDisplay, bool, Vec<(u16, u32)>, bool)> = vec![
        ("a live unit", plain, false, vec![(HEALTH, 100)], true),
        ("a dead unit", plain, false, vec![(HEALTH, 0)], false),
        ("no health streamed", plain, false, vec![], false),
        (
            "a dead unit, AttributesEx2 & 1",
            raise,
            false,
            vec![(HEALTH, 0)],
            true,
        ),
        (
            "a dead unit, the dynamic dead flag",
            plain,
            false,
            vec![(HEALTH, 0), (DYNAMIC_FLAGS, 0x20)],
            true,
        ),
        (
            "UNIT_FIELD_FLAGS & 0x10000",
            plain,
            false,
            vec![(HEALTH, 100), (FLAGS, 0x1_0000)],
            false,
        ),
        (
            "a Humanoid under Hibernate's mask",
            hibernate,
            false,
            vec![(HEALTH, 100), (ENTRY, HUMANOID)],
            false,
        ),
        (
            "a Beast under Hibernate's mask",
            hibernate,
            false,
            vec![(HEALTH, 100), (ENTRY, BEAST)],
            true,
        ),
        (
            "the caster, AttributesEx & 0x80000",
            excluding,
            true,
            vec![(HEALTH, 100)],
            false,
        ),
        (
            "another unit, AttributesEx & 0x80000",
            excluding,
            false,
            vec![(HEALTH, 100)],
            true,
        ),
        (
            "a living unit under the 0x400 word",
            skinning,
            false,
            vec![(HEALTH, 100)],
            false,
        ),
        (
            "a dead unit under the 0x400 word",
            skinning,
            false,
            vec![(HEALTH, 0)],
            true,
        ),
    ];

    for (label, make_def, on_self, fields, binds) in cases {
        let (mut world, rx, ally) = unit_world(10.0);
        let me = world
            .query_filtered::<Entity, With<SelfPlayer>>()
            .single(&world)
            .expect("the player");
        let (entity, guid, token) = if on_self {
            (me, ME, "player")
        } else {
            (ally, ALLY, "target")
        };
        let store = ObjectStore(ObjectFields::from_pairs(&fields));
        world.entity_mut(entity).insert(store.clone());
        let word = cast_target_mask(&make_def());
        let mut spells = world.resource_mut::<crate::app_core::state::Spells>();
        spells.catalog = wrathplus_formats::SpellCatalog::from_displays(HashMap::from([(
            SPELL,
            SpellDisplay {
                range_index: 5,
                ..make_def()
            },
        )]));
        world.insert_resource(make_names());
        arm(&mut world, SPELL, word);

        // The hover verdict.
        publish_guids(&mut world);
        world
            .run_system_cached(feed_targeting_to_vm)
            .expect("the feed runs");
        let hover = world
            .non_send_resource::<UiScript>()
            .eval::<bool>(&format!("return SpellCanTargetUnit({token:?}) == 1"))
            .expect("a boolean");
        assert_eq!(hover, binds, "{label}: the hover verdict");

        // The click, through `SpellTargetUnit`.
        spell_target_unit(&mut world, token);
        let clicked = matches!(
            rx.try_recv(),
            Ok(ClientCommand::CastSpell {
                spell_id: SPELL,
                target: Some(sent),
            }) if sent == guid
        );
        assert_eq!(clicked, binds, "{label}: the cursor's click");
        assert_eq!(
            world.resource::<SpellTargeting>().active(),
            !binds,
            "{label}: a bind ends the cursor, a refusal leaves it up"
        );

        // The press, from the same fields.
        let (def, names) = (make_def(), make_names());
        let pressed = resolve_cast_target(
            Some(&def),
            &CastCandidates {
                selection: Some(guid),
                caster: Some(ME),
                main_hand_item: None,
            },
            false,
            &TargetRelations {
                target_store: Some(&store),
                target_owner_store: None,
                relations: crate::target::RelationContext::none(),
                self_store: Some(&store),
                factions: None,
                reputations: &crate::net::Reputations(Vec::new()),
                types: crate::creature_type::CreatureTypeSources {
                    names: Some(&names),
                    forms: None,
                },
                group: Default::default(),
            },
        );
        assert_eq!(
            pressed == CastWireTarget::Unit(guid),
            binds,
            "{label}: the press ({pressed:?})"
        );
    }
}

/// `SpellCanTargetUnit` asks `0x6e6460`'s unit leg per token: nil out of range, inside the
/// minimum included, and nil for ourselves under a spell that excludes its caster.
#[test]
fn spell_can_target_unit_is_nil_out_of_range() {
    let can = |distance: f32, spell: u32, token: &str| {
        let (mut world, _rx, _) = unit_world(distance);
        arm(&mut world, spell, 0x0002);
        publish_guids(&mut world);
        world
            .run_system_cached(feed_targeting_to_vm)
            .expect("the feed runs");
        world
            .non_send_resource::<UiScript>()
            .eval::<bool>(&format!("return SpellCanTargetUnit({token:?}) == 1"))
            .expect("a boolean")
    };
    assert!(can(10.0, HEAL, "target"), "in range");
    assert!(!can(40.0, HEAL, "target"), "out of range answers nil");
    assert!(can(40.0, HEAL, "player"), "ourselves at distance 0");
    assert!(
        !can(10.0, NOT_SELF, "player"),
        "never ourselves under AttributesEx 0x80000"
    );
    assert!(can(10.0, NOT_SELF, "target"));
    assert!(can(20.0, BANDED, "target"), "inside the band");
    assert!(
        !can(5.0, BANDED, "target"),
        "inside the minimum answers nil"
    );
}
