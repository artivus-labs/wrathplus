//! Native fallback, held press and controller regressions.
use super::*;

#[test]
fn explicit_hostile_helpful_cast_obeys_native_autoself_branch() {
    for enabled in [false, true] {
        let mut f = frame(true);
        with_factions(&mut f, FOE, &[(Field::ObjectType.index(), 9)]);
        with_fortitude(&mut f);
        let world = f.app.world_mut();
        let events = {
            let mut c = world.resource_mut::<crate::cvars::Cvars>();
            c.set("autoSelfCast", if enabled { "1" } else { "0" });
            c.take_events()
        };
        for event in events {
            world.trigger(event);
        }
        run(&mut f, "CastSpellByName('Fortitude', 'target')");
        assert_eq!(
            casts(&f),
            if enabled {
                vec![(FORTITUDE, Some(ME))]
            } else {
                vec![]
            }
        );
        assert_eq!(selected(&f), Some(OLD));
    }
}

#[test]
fn explicit_sentinels_distinguish_nobody_from_none_and_targetless() {
    for (implicit, token, cursor, sent) in [
        (21, "party4", false, false),
        (21, "none", true, false),
        (6, "none", true, false),
        (0, "party4", false, true),
    ] {
        let mut f = frame(true);
        f.app
            .world_mut()
            .insert_resource(crate::app_core::state::Spells {
                catalog: wrathplus_formats::SpellCatalog::from_displays(
                    [(
                        HEAL,
                        wrathplus_formats::SpellDisplay {
                            implicit_target_a1: implicit,
                            ..Default::default()
                        },
                    )]
                    .into(),
                ),
                ..crate::app_core::state::Spells::empty_for_tests()
            });
        run(&mut f, &format!("CastSpellByName('Heal', '{token}')"));
        assert_eq!(!casts(&f).is_empty(), sent);
        assert_eq!(
            f.app
                .world()
                .resource::<crate::spell::SpellTargeting>()
                .active(),
            cursor
        );
        assert_eq!(selected(&f), Some(OLD));
    }
}

#[test]
fn explicit_combat_press_holds_resumes_and_registers_at_its_candidate() {
    let mut f = frame(true);
    with_factions(&mut f, FRIEND, &[(FLAGS, 8)]);
    let world = f.app.world_mut();
    let new = world.resource::<GuidIndex>().0[&NEW];
    world
        .entity_mut(new)
        .insert(ObjectStore(wrathplus_protocol::ObjectFields::from_fields(
            &[
                (Field::ObjectType, 9),
                (Field::UnitHealth, 100),
                (Field::UnitFactionTemplate, FOE),
            ],
        )));
    world.insert_resource(crate::app_core::state::Spells {
        catalog: wrathplus_formats::SpellCatalog::from_displays(
            [(
                HEAL,
                wrathplus_formats::SpellDisplay {
                    attributes: 0x400,
                    implicit_target_a1: 6,
                    ..Default::default()
                },
            )]
            .into(),
        ),
        ..crate::app_core::state::Spells::empty_for_tests()
    });
    run(&mut f, "CastSpellByName('Heal', 'focus')");
    let commands: Vec<_> = f.rx.try_iter().collect();
    assert!(
        commands.iter().any(|c| matches!(
            c,
            ClientCommand::CastSpell {
                target: Some(NEW),
                ..
            }
        )),
        "{commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, ClientCommand::AttackSwing { guid: NEW })),
        "{commands:?}"
    );
    assert_eq!(selected(&f), Some(OLD));
}

#[test]
fn held_explicit_candidate_survives_selection_change_before_retry() {
    let mut f = frame(true);
    f.app
        .world_mut()
        .insert_resource(crate::app_core::state::Spells {
            catalog: wrathplus_formats::SpellCatalog::from_displays(
                [(
                    HEAL,
                    wrathplus_formats::SpellDisplay {
                        attributes: 0x400,
                        implicit_target_a1: 6,
                        ..Default::default()
                    },
                )]
                .into(),
            ),
            ..crate::app_core::state::Spells::empty_for_tests()
        });
    let held = f
        .app
        .world_mut()
        .run_system_once(
            |targeting: crate::spell::cast_target::CastTargeting,
             mut ladder: crate::spell::CastLadder| {
                ladder.send_spell_at(
                    HEAL,
                    &targeting.context_at(Some(Some(NEW))),
                    false,
                    Some(NEW),
                );
                ladder
                    .take_held()
                    .expect("combat press pauses at attack validation")
            },
        )
        .unwrap();
    assert!(f.rx.is_empty());
    run(&mut f, "TargetUnit('player')");
    f.app
        .world_mut()
        .run_system_once(
            move |targeting: crate::spell::cast_target::CastTargeting,
                  mut ladder: crate::spell::CastLadder| {
                ladder.resume_after_pick(held, &targeting.context_at(held.explicit.map(Some)));
            },
        )
        .unwrap();
    assert_eq!(casts(&f), vec![(HEAL, Some(NEW))]);
    assert_eq!(selected(&f), Some(ME));
}

#[test]
fn explicit_lua_learned_repeat_keeps_focus_across_wait_and_selection_change() {
    for token in ["focus", "party4"] {
        let mut f = frame(true);
        let world = f.app.world_mut();
        let new = world.resource::<GuidIndex>().0[&NEW];
        world.entity_mut(new).insert((
            Transform::from_xyz(0.0, 0.0, -22.0),
            ObjectStore(wrathplus_protocol::ObjectFields::from_fields(&[
                (Field::ObjectType, 9),
                (Field::UnitHealth, 100),
            ])),
        ));
        world.insert_resource(crate::app_core::player::Player::default());
        let mut spells = crate::app_core::state::Spells::empty_for_tests();
        spells.catalog = wrathplus_formats::SpellCatalog::from_displays(
            [(
                HEAL,
                wrathplus_formats::SpellDisplay {
                    attributes: 2,
                    attributes_ex2: 0x20,
                    attributes_ex4: 0x0100_0000,
                    implicit_target_a1: 6,
                    range_index: 114,
                    ..Default::default()
                },
            )]
            .into(),
        );
        spells.ranges = wrathplus_formats::SpellRangeCatalog::from_rows(
            [(
                114,
                wrathplus_formats::SpellRange {
                    min: 8.0,
                    max: 35.0,
                    flags: 0,
                },
            )]
            .into(),
        );
        world.insert_resource(spells);
        world
            .resource_mut::<crate::app_core::state::PlayerActions>()
            .spells
            .insert(HEAL);
        run(&mut f, &format!("CastSpellByName('Heal', '{token}')"));
        if token == "party4" {
            assert!(
                !f.app
                    .world()
                    .resource::<crate::spell::AutoRepeatActive>()
                    .1
                    .attacking
            );
            f.app
                .world_mut()
                .run_system_once(crate::spell::auto_ranged::tick)
                .unwrap();
            assert!(f.rx.is_empty());
        } else {
            assert!(
                f.app
                    .world()
                    .resource::<crate::spell::AutoRepeatActive>()
                    .1
                    .attacking
            );
            f.app
                .world_mut()
                .resource_mut::<crate::app_core::player::Player>()
                .move_flags = 1;
            f.app
                .world_mut()
                .run_system_once(crate::spell::auto_ranged::tick)
                .unwrap();
            assert!(f.rx.is_empty());
            run(&mut f, "TargetUnit('player')");
            f.app
                .world_mut()
                .resource_mut::<crate::app_core::player::Player>()
                .move_flags = 0;
            f.app
                .world_mut()
                .run_system_once(crate::spell::auto_ranged::tick)
                .unwrap();
            assert_eq!(casts(&f), vec![(HEAL, Some(NEW))]);
            run(&mut f, &format!("CastSpellByName('Heal', '{token}')"));
            assert!(
                !f.app
                    .world()
                    .resource::<crate::spell::AutoRepeatActive>()
                    .1
                    .attacking
            );
        }
    }
}
