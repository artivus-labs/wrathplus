//! The selection calls' tests, split out of `script.rs`.

use super::*;
use wrathplus_protocol::types::Field;

/// `field`'s native descriptor index.
const fn at(field: Field) -> u16 {
    field.index()
}

/// A world holding what [`ScriptSelect`] reads, with a bare VM and no units.
fn script_world() -> World {
    use crate::net::NetCommands;
    use wrathplus_ui::script::UiScript;

    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut world = World::new();
    world.insert_resource(NetCommands(tx));
    world.init_resource::<crate::spell::QueuedMeleeSpell>();
    world.init_resource::<crate::spell::AutoRepeatActive>();
    world.init_resource::<crate::app_core::loot::LootState>();
    world.init_resource::<crate::app_core::state::LootLatch>();
    world.init_resource::<Messages<crate::creature_anim::SheathRequest>>();
    world.init_resource::<crate::app_core::group::GroupState>();
    world.init_resource::<crate::net::GuidIndex>();
    world.init_resource::<crate::net::Reputations>();
    world.init_resource::<super::super::AssistAttack>();
    world.init_resource::<super::super::Selection>();
    world.init_resource::<scan::LastEnemy>();
    world.init_resource::<scan::LastFriend>();
    world.init_resource::<crate::names::NameCache>();
    world.init_resource::<scan::TabHistory>();
    world.init_resource::<Time>();
    world.insert_non_send_resource(UiScript::new().expect("a bare VM"));
    world
}

/// The selection calls from Lua, through the binding bodies of ASSISTTARGET (`AssistUnit`) and
/// TARGETLASTHOSTILE (`TargetLastEnemy`): an empty basis, a garbage token and a stale memory
/// are each a no-op (`0x489a40`'s bare `ret`), never a deselect or a panic.
#[test]
fn the_selection_calls_run_assist_and_last_enemy_without_ever_deselecting() {
    use crate::app_core::session::ObjectStore;
    use crate::app_core::session::SelfPlayer;
    use crate::net::Guid;
    use bevy::ecs::system::RunSystemOnce;
    use wrathplus_ui::script::UiScript;

    const ME: u64 = 1;
    const BASIS: u64 = 0xB0A2;
    const VICTIM: u64 = 0xC0DE;
    const GONE: u64 = 0x6017;
    // `UNIT_FIELD_TARGET` is a 2-field guid at index 16; HEALTH/MAXHEALTH keep the units live.
    let store =
        |pairs: &[(u16, u32)]| ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(pairs));

    let mut world = script_world();

    world.spawn((
        SelfPlayer,
        Guid(ME),
        store(&[
            (at(Field::UnitHealth), 100),
            (at(Field::UnitMaxHealth), 100),
        ]),
    ));
    // The basis is pointing at the victim; the victim points at nobody.
    let basis = world
        .spawn((
            Guid(BASIS),
            store(&[
                (at(Field::UnitHealth), 100),
                (at(Field::UnitMaxHealth), 100),
                (at(Field::UnitTarget), VICTIM as u32),
            ]),
        ))
        .id();
    let victim = world
        .spawn((
            Guid(VICTIM),
            store(&[
                (at(Field::UnitHealth), 100),
                (at(Field::UnitMaxHealth), 100),
            ]),
        ))
        .id();
    let index = &mut world.resource_mut::<crate::net::GuidIndex>().0;
    index.insert(BASIS, basis);
    index.insert(VICTIM, victim);

    let run = |world: &mut World, lua: &str| {
        world
            .non_send_resource_mut::<UiScript>()
            .eval::<()>(lua)
            .expect("the binding runs");
        world
            .run_system_once(
                |mut script: NonSendMut<UiScript>, mut select: ScriptSelect| {
                    for request in script.take_selection_requests() {
                        select.select(request);
                    }
                },
            )
            .expect("the applier runs as a one-shot system");
        world.resource::<super::super::Selection>().guid
    };
    let set = |world: &mut World, target: Option<(Entity, u64)>| {
        let mut sel = world.resource_mut::<super::super::Selection>();
        sel.target = target.map(|(e, _)| e);
        sel.guid = target.map(|(_, g)| g);
    };

    // Assist the current target: one hop onto its target.
    set(&mut world, Some((basis, BASIS)));
    assert_eq!(run(&mut world, r#"AssistUnit("target")"#), Some(VICTIM));

    // The victim targets nobody: assisting it is a no-op, not a deselect.
    assert_eq!(run(&mut world, r#"AssistUnit("target")"#), Some(VICTIM));

    assert_eq!(run(&mut world, r#"AssistUnit("nosuchunit")"#), Some(VICTIM));
    assert_eq!(run(&mut world, r#"TargetUnit("nosuchunit")"#), Some(VICTIM));

    // TargetLastEnemy with nothing remembered: also a no-op.
    assert_eq!(run(&mut world, "TargetLastEnemy()"), Some(VICTIM));

    world.resource_mut::<scan::LastEnemy>().0 = Some(VICTIM);
    set(&mut world, None);
    assert_eq!(run(&mut world, "TargetLastEnemy()"), Some(VICTIM));

    // A stale memory (the unit despawned, so its guid is in no index) leaves the target alone.
    world.resource_mut::<scan::LastEnemy>().0 = Some(GONE);
    assert_eq!(run(&mut world, "TargetLastEnemy()"), Some(VICTIM));
    set(&mut world, None);
    assert_eq!(run(&mut world, "TargetLastEnemy()"), None);
}

/// `TargetUnit("partypet1")`, the TARGETPARTYPET1 binding and the pet half of
/// TARGETPARTYMEMBER1, commits the pet the member's descriptor names (`0x515970` to `0x4e81d0`);
/// a slot with no member, or one whose pet is not held, is a no-op, never a deselect.
#[test]
fn target_unit_selects_a_party_pet() {
    use crate::app_core::session::ObjectStore;
    use crate::app_core::session::SelfPlayer;
    use crate::net::Guid;
    use bevy::ecs::system::RunSystemOnce;
    use wrathplus_protocol::types::{member_status, GroupMemberEntry};
    use wrathplus_ui::script::UiScript;

    const ME: u64 = 1;
    const MEMBER: u64 = 0x1234;
    const PET: u64 = 0xF140_0000_0000_0077;
    // HEALTH/MAXHEALTH keep the units live; `UNIT_FIELD_SUMMON` is the 2-field guid at 8.
    let store =
        |pairs: &[(u16, u32)]| ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(pairs));

    let mut world = script_world();
    world.spawn((
        SelfPlayer,
        Guid(ME),
        store(&[
            (at(Field::UnitHealth), 100),
            (at(Field::UnitMaxHealth), 100),
        ]),
    ));
    let pet = world
        .spawn((
            Guid(PET),
            store(&[
                (at(Field::UnitHealth), 100),
                (at(Field::UnitMaxHealth), 100),
            ]),
        ))
        .id();
    let member = world
        .spawn((
            Guid(MEMBER),
            store(&[
                (22, 100),
                (28, 100),
                (8, PET as u32),
                (9, (PET >> 32) as u32),
            ]),
        ))
        .id();
    let index = &mut world.resource_mut::<crate::net::GuidIndex>().0;
    index.insert(PET, pet);
    index.insert(MEMBER, member);
    world
        .resource_mut::<crate::app_core::group::GroupState>()
        .apply_list(
            0,
            0,
            vec![GroupMemberEntry {
                name: "Brisca".into(),
                guid: MEMBER,
                status: member_status::ONLINE,
                flags: 0,
            }],
            ME,
            None,
            Some(ME),
        );

    let run = |world: &mut World, lua: &str| {
        world
            .non_send_resource_mut::<UiScript>()
            .eval::<()>(lua)
            .expect("the binding runs");
        world
            .run_system_once(
                |mut script: NonSendMut<UiScript>, mut select: ScriptSelect| {
                    for request in script.take_selection_requests() {
                        select.select(request);
                    }
                },
            )
            .expect("the applier runs as a one-shot system");
        world.resource::<super::super::Selection>().guid
    };

    assert_eq!(run(&mut world, r#"TargetUnit("party1")"#), Some(MEMBER));
    assert_eq!(run(&mut world, r#"TargetUnit("partypet1")"#), Some(PET));
    assert_eq!(run(&mut world, r#"TargetUnit("PartyPet1")"#), Some(PET));
    // No second member, and slot 0 and 5 are no slot: each leaves the selection alone.
    for token in ["partypet2", "partypet0", "partypet5"] {
        assert_eq!(
            run(&mut world, &format!(r#"TargetUnit("{token}")"#)),
            Some(PET),
            "{token}"
        );
    }
    // Out of view, the pet is named but not held: nothing to select, no deselect.
    world.resource_mut::<crate::net::GuidIndex>().0.remove(&PET);
    assert_eq!(run(&mut world, r#"TargetUnit("party1")"#), Some(MEMBER));
    assert_eq!(run(&mut world, r#"TargetUnit("partypet1")"#), Some(MEMBER));
}

/// `TargetUnit` and `AssistUnit` take the tokens the reference's resolver takes (`0x515970`,
/// from `0x489a08` and `0x489ba9`): a base then a hop off each held unit's target, so
/// `party1target` names the unit party1 targets and `AssistUnit("party1target")` its target's.
/// A chain that breaks, and text that is not a hop, is a no-op, never a deselect.
#[test]
fn target_and_assist_unit_follow_a_target_chain() {
    use crate::app_core::session::ObjectStore;
    use crate::app_core::session::SelfPlayer;
    use crate::net::Guid;
    use bevy::ecs::system::RunSystemOnce;
    use wrathplus_protocol::types::{member_status, GroupMemberEntry};
    use wrathplus_ui::script::UiScript;

    const ME: u64 = 1;
    const MEMBER: u64 = 0x1234;
    const PET: u64 = 0xF140_0000_0000_0077;
    const VICTIM: u64 = 0xC0DE;
    const THIRD: u64 = 0xF130_0000_0000_0003;
    // `OBJECT_FIELD_TYPE` 2 (a hop is off a unit, typemask 8), `UNIT_FIELD_SUMMON` 8 and
    // `UNIT_FIELD_TARGET` 16, the latter two each a 2-field guid.
    let unit = |kind: u32, summon: u64, target: u64| {
        ObjectStore(wrathplus_protocol::ObjectFields::from_field_slots(&[
            (wrathplus_protocol::types::Field::ObjectType, 0, kind),
            (
                wrathplus_protocol::types::Field::UnitSummon,
                0,
                summon as u32,
            ),
            (
                wrathplus_protocol::types::Field::UnitSummon,
                1,
                (summon >> 32) as u32,
            ),
            (
                wrathplus_protocol::types::Field::UnitTarget,
                0,
                target as u32,
            ),
            (
                wrathplus_protocol::types::Field::UnitTarget,
                1,
                (target >> 32) as u32,
            ),
        ]))
    };
    const PLAYER: u32 = 0x19;
    const CREATURE: u32 = 0x09;

    let mut world = script_world();
    // We target the victim, party1 targets the victim, the victim targets `THIRD`, our pet
    // targets `THIRD`; `THIRD` targets nobody.
    world.spawn((SelfPlayer, Guid(ME), unit(PLAYER, PET, VICTIM)));
    let member = world.spawn((Guid(MEMBER), unit(PLAYER, 0, VICTIM))).id();
    let victim = world.spawn((Guid(VICTIM), unit(CREATURE, 0, THIRD))).id();
    let third = world.spawn((Guid(THIRD), unit(CREATURE, 0, 0))).id();
    let pet = world.spawn((Guid(PET), unit(CREATURE, 0, THIRD))).id();
    let index = &mut world.resource_mut::<crate::net::GuidIndex>().0;
    index.insert(MEMBER, member);
    index.insert(VICTIM, victim);
    index.insert(THIRD, third);
    index.insert(PET, pet);
    world
        .resource_mut::<crate::app_core::group::GroupState>()
        .apply_list(
            0,
            0,
            vec![GroupMemberEntry {
                name: "Brisca".into(),
                guid: MEMBER,
                status: member_status::ONLINE,
                flags: 0,
            }],
            ME,
            None,
            Some(ME),
        );
    let mut bar = crate::app_core::state::PetBar::default();
    bar.spells.pet_guid = PET;
    world.insert_resource(bar);

    let run = |world: &mut World, lua: &str| {
        world
            .non_send_resource_mut::<UiScript>()
            .eval::<()>(lua)
            .expect("the binding runs");
        world
            .run_system_once(
                |mut script: NonSendMut<UiScript>, mut select: ScriptSelect| {
                    for request in script.take_selection_requests() {
                        select.select(request);
                    }
                },
            )
            .expect("the applier runs as a one-shot system");
        world.resource::<super::super::Selection>().guid
    };
    let reset = |world: &mut World| {
        let mut sel = world.resource_mut::<super::super::Selection>();
        sel.target = None;
        sel.guid = None;
    };

    assert_eq!(run(&mut world, r#"TargetUnit("party1")"#), Some(MEMBER));
    assert_eq!(
        run(&mut world, r#"TargetUnit("party1target")"#),
        Some(VICTIM)
    );
    assert_eq!(
        run(&mut world, r#"TargetUnit("party1targettarget")"#),
        Some(THIRD)
    );
    // `THIRD` targets nobody: the chain ends there, and the selection stays.
    for token in [
        "party1targettargettarget",
        "party1targetfoo",
        "party1foo",
        "party2target",
        "npctarget",
    ] {
        assert_eq!(
            run(&mut world, &format!(r#"TargetUnit("{token}")"#)),
            Some(THIRD),
            "{token}"
        );
    }

    reset(&mut world);
    assert_eq!(
        run(&mut world, r#"TargetUnit("PARTY1TARGET")"#),
        Some(VICTIM)
    );
    reset(&mut world);
    assert_eq!(
        run(&mut world, r#"TargetUnit("playertarget")"#),
        Some(VICTIM)
    );
    reset(&mut world);
    assert_eq!(run(&mut world, r#"TargetUnit("pettarget")"#), Some(THIRD));
    reset(&mut world);
    assert_eq!(run(&mut world, r#"TargetUnit("partypet1target")"#), None);

    // Assisting a unit selects its target; its target is a chain's end, so one hop further.
    reset(&mut world);
    assert_eq!(run(&mut world, r#"AssistUnit("party1")"#), Some(VICTIM));
    reset(&mut world);
    assert_eq!(
        run(&mut world, r#"AssistUnit("party1target")"#),
        Some(THIRD)
    );
    assert_eq!(
        run(&mut world, r#"AssistUnit("party1targettarget")"#),
        Some(THIRD),
        "THIRD targets nobody: silent, nothing deselected"
    );
}
