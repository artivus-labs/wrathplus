use super::test_support::*;
use super::*;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::net::{ClientCommand, Guid};
use wrathplus_protocol::types::Field;

/// `0x6e6d90` past the binding's checks: a token naming no unit ends the cast with "Out of
/// range."; a unit goes to `BindTarget`, which waits on a relation failure, raises "Out of
/// range." for a unit out of range with the cursor still up, and commits otherwise. The
/// selection never moves.
#[test]
fn spell_target_unit_ends_waits_or_binds_as_the_reference() {
    let fail = |reason| vec![crate::app_core::errors::CastFail::local(HEAL, reason)];

    // The VM queued a token, but the host is no longer targeting: nothing.
    let (mut world, rx, _) = unit_world(10.0);
    spell_target_unit(&mut world, "target");
    assert!(rx.try_recv().is_err());
    assert!(errors(&mut world).is_empty());

    // A known token naming no unit: "Out of range." and targeting ends.
    let (mut world, rx, _) = unit_world(10.0);
    arm(&mut world, HEAL, 0x0002);
    spell_target_unit(&mut world, "party1");
    assert!(rx.try_recv().is_err(), "no send");
    assert_eq!(
        errors(&mut world),
        fail(crate::spell::cast_fail_keys::reason::OUT_OF_RANGE)
    );
    assert!(
        !world.resource::<SpellTargeting>().active(),
        "the cast ends"
    );

    // A unit the relation refuses (assist, neutral with no catalog): silent, still armed.
    let (mut world, rx, _) = unit_world(10.0);
    arm(&mut world, HEAL, 0x0100);
    spell_target_unit(&mut world, "target");
    assert!(rx.try_recv().is_err());
    assert!(errors(&mut world).is_empty(), "no error");
    assert!(world.resource::<SpellTargeting>().active(), "still armed");

    // Ourselves under a spell that excludes its caster: silent, still armed.
    let (mut world, rx, _) = unit_world(10.0);
    arm(&mut world, NOT_SELF, 0x0002);
    spell_target_unit(&mut world, "player");
    assert!(
        rx.try_recv().is_err(),
        "never ourselves under AttributesEx 0x80000"
    );
    assert!(errors(&mut world).is_empty(), "no error");
    assert!(world.resource::<SpellTargeting>().active(), "still armed");

    // In relation but out of range: "Out of range." and still armed.
    let (mut world, rx, _) = unit_world(40.0);
    arm(&mut world, HEAL, 0x0002);
    spell_target_unit(&mut world, "target");
    assert!(rx.try_recv().is_err());
    assert_eq!(
        errors(&mut world),
        fail(crate::spell::cast_fail_keys::reason::OUT_OF_RANGE)
    );
    assert!(world.resource::<SpellTargeting>().active(), "still armed");

    // In range: the cast goes to the unit and the cursor comes down.
    let (mut world, rx, ally) = unit_world(10.0);
    arm(&mut world, HEAL, 0x0002);
    spell_target_unit(&mut world, "target");
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: HEAL,
            target: Some(ALLY),
        })
    ));
    assert!(errors(&mut world).is_empty());
    assert!(!world.resource::<SpellTargeting>().active());
    let selection = world.resource::<crate::app_core::state::Selection>();
    assert_eq!((selection.target, selection.guid), (Some(ally), Some(ALLY)));
}

/// `SpellTargetUnit` takes a `target` chain as the resolver does (1.12 `0x6e6de1` into `0x515970`;
/// 12340 not located): `"pettarget"` and `"playertarget"` bind the unit our pet and we target,
/// where a chain that names no unit still ends the cast with "Out of range.".
#[test]
fn spell_target_unit_follows_a_target_chain() {
    const PET: u64 = 0xF140_0000_0000_0077;
    // `OBJECT_FIELD_TYPE` and `UNIT_FIELD_TARGET`, the latter a 2-field guid.
    let unit = |kind: u32, target: u64| {
        ObjectStore(wrathplus_protocol::ObjectFields::from_field_slots(&[
            (Field::ObjectType, 0, kind),
            (Field::UnitTarget, 0, target as u32),
            (Field::UnitTarget, 1, (target >> 32) as u32),
        ]))
    };
    let seat = |world: &mut World, ally: Entity| {
        // We and our pet target the ally; the ally targets nobody.
        let me = world
            .query_filtered::<Entity, With<SelfPlayer>>()
            .single(world)
            .unwrap();
        world.entity_mut(me).insert(unit(0x19, ALLY));
        let pet = world.spawn((Guid(PET), unit(0x09, ALLY))).id();
        let mut bar = crate::app_core::state::PetBar::default();
        bar.spells.pet_guid = PET;
        world.insert_resource(bar);
        let mut index = world.resource_mut::<crate::net::GuidIndex>();
        index.0.insert(ME, me);
        index.0.insert(ALLY, ally);
        index.0.insert(PET, pet);
    };
    let fail = vec![crate::app_core::errors::CastFail::local(
        HEAL,
        crate::spell::cast_fail_keys::reason::OUT_OF_RANGE,
    )];
    for token in ["pettarget", "playertarget", "PETTARGET"] {
        let (mut world, rx, ally) = unit_world(10.0);
        seat(&mut world, ally);
        arm(&mut world, HEAL, 0x0002);
        spell_target_unit(&mut world, token);
        assert!(
            matches!(
                rx.try_recv(),
                Ok(ClientCommand::CastSpell {
                    spell_id: HEAL,
                    target: Some(ALLY),
                })
            ),
            "{token} binds the ally"
        );
        assert!(errors(&mut world).is_empty(), "{token}");
        assert!(!world.resource::<SpellTargeting>().active(), "{token}");
    }

    // The ally targets nobody, so a hop off it names no unit: the cast ends.
    for token in ["pettargettarget", "party1target", "pettargetfoo"] {
        let (mut world, rx, ally) = unit_world(10.0);
        seat(&mut world, ally);
        arm(&mut world, HEAL, 0x0002);
        spell_target_unit(&mut world, token);
        assert!(rx.try_recv().is_err(), "{token}: no send");
        assert_eq!(errors(&mut world), fail, "{token}");
        assert!(!world.resource::<SpellTargeting>().active(), "{token}");
    }
}

/// `0x6e61a0`: the pick takes us only while targeting, for a word in `0x850e` whose spell
/// lacks `AttributesEx & 0x80000`; [`PicksSelf`] carries it to the picker.
#[test]
fn the_pick_takes_us_only_for_a_friendly_word_that_admits_the_caster() {
    let (mut world, _rx, _) = unit_world(10.0);
    let picks = |world: &mut World| {
        world
            .run_system_cached(publish_picks_self)
            .expect("the flag publishes");
        world.resource::<PicksSelf>().0
    };
    world.init_resource::<PicksSelf>();
    assert!(!picks(&mut world), "never outside targeting");
    for word in [0x0002, 0x0004, 0x0008, 0x0100, 0x0400, 0x8000, 0x0102] {
        arm(&mut world, HEAL, word);
        assert!(picks(&mut world), "{word:#06x} takes us");
    }
    for word in [0x0080, 0x0200, 0x0040, 0x0010, 0x4800] {
        arm(&mut world, HEAL, word);
        assert!(!picks(&mut world), "{word:#06x} never takes us");
    }
    arm(&mut world, NOT_SELF, 0x0100);
    assert!(!picks(&mut world), "a spell that excludes its caster");
    world.resource_mut::<SpellTargeting>().clear();
    assert!(!picks(&mut world), "and off again when the cursor ends");
}

/// A terrain click while a poison is armed must not ship a DEST block for an item spell.
#[test]
fn each_click_seam_only_sees_a_word_it_can_bind() {
    let commit = super::super::cast_send::CastCommit::Spell;
    let mut t = SpellTargeting::default();
    assert_eq!(
        t.pending_for(TargetingWants::Location),
        None,
        "idle binds nothing"
    );
    assert!(!t.wants(TargetingWants::Item), "idle wants nothing");

    // Blizzard, the bare DEST word.
    t.enter(2120, commit, 0x0040);
    assert!(t.active());
    assert_eq!(
        t.pending_for(TargetingWants::Location),
        Some((2120, commit))
    );
    for seam in [TargetingWants::Item, TargetingWants::GameObject] {
        assert_eq!(
            t.pending_for(seam),
            None,
            "{seam:?} cannot commit a Blizzard"
        );
    }

    // Instant Poison, the bare ITEM word.
    t.enter(8679, commit, 0x0010);
    assert_eq!(t.pending_for(TargetingWants::Item), Some((8679, commit)));
    for seam in [TargetingWants::Location, TargetingWants::GameObject] {
        assert_eq!(t.pending_for(seam), None, "{seam:?} cannot commit a poison");
    }
    // The action bar's re-press toggle reads the spell whatever the seam.
    assert_eq!(t.spell(), Some(8679));

    t.clear();
    assert!(!t.active());
}

/// A lock word (`LOCKED` plus arm 23's `GAMEOBJECT`) is in both `0x4010` and `0x4800`, so
/// whichever click lands first binds.
#[test]
fn a_lock_word_answers_both_the_bag_and_the_world_seam() {
    let commit = super::super::cast_send::CastCommit::Spell;
    let mut t = SpellTargeting::default();
    // Opening (3365): `Targets 0x4000` + implicit arm 23 ⇒ `0x4800`.
    t.enter(3365, commit, 0x4800);
    assert_eq!(t.pending_for(TargetingWants::Item), Some((3365, commit)));
    assert_eq!(
        t.pending_for(TargetingWants::GameObject),
        Some((3365, commit))
    );
    assert!(t.wants(TargetingWants::Item));
    assert!(t.wants(TargetingWants::GameObject));
    assert_eq!(
        t.pending_for(TargetingWants::Location),
        None,
        "a lock word still has no terrain leg"
    );

    // A bare GAMEOBJECT word (arm 23 over a `Targets`-less row) is the world seam's alone.
    t.enter(3365, commit, 0x0800);
    assert!(t.wants(TargetingWants::GameObject));
    assert!(!t.wants(TargetingWants::Item));
}
