//! Production attack seam and retry regressions.
use super::integration_tests::*;
use super::*;

#[test]
fn repeat_press_while_moving_registers_controller_and_repress_stops_waiting() {
    let mut f = Fixture::new(20.0);
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.world.resource_mut::<Player>().move_flags = 1;
    let press = |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
        ladder.send_spell(AUTO_SHOT, &targeting.context(), false);
    };
    f.world.run_system_once(press).unwrap();
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    f.tick();
    assert_eq!(
        f.world.resource::<AutoRepeatActive>().1.queued,
        Some(AUTO_SHOT)
    );
    assert!(f.rx.is_empty());
    assert!(f
        .world
        .resource::<crate::app_core::errors::CastErrors>()
        .0
        .is_empty());
    f.world.run_system_once(press).unwrap();
    f.assert_stopped();
    assert!(f.rx.is_empty());
}
use crate::app_core::player::Player;
use crate::app_core::state::Spells;
use crate::creature_anim::{AutoRepeatArmed, Engaged, SheathRequest};
use crate::net::ClientCommand;
use bevy::ecs::system::RunSystemOnce;
use std::collections::HashMap;
use wrathplus_formats::{SpellCatalog, SpellDisplay};
#[test]
fn issue95_attack_seam_stop_tears_down_queued_and_running_controller_without_swing() {
    for running in [false, true] {
        let mut f = Fixture::new(20.0);
        f.world.init_resource::<crate::app_core::loot::LootState>();
        f.world.init_resource::<crate::app_core::state::LootLatch>();
        if running {
            f.tick();
            assert!(matches!(
                f.rx.try_recv(),
                Ok(ClientCommand::CastSpell {
                    spell_id: AUTO_SHOT,
                    ..
                })
            ));
        } else {
            f.world.resource_mut::<Player>().move_flags = 1;
            f.tick();
            f.assert_queued_without_start();
        }
        f.world
            .run_system_once(|mut seam: crate::creature_anim::AttackSeam| {
                seam.stop_controller(false);
            })
            .expect("production AttackSeam runs");
        f.assert_stopped();
        assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
        assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
        if running {
            assert!(matches!(
                f.rx.try_recv(),
                Ok(ClientCommand::CancelAutoRepeat)
            ));
        }
        assert!(f.rx.is_empty(), "stop must never emit AttackSwing");
        f.world.resource_mut::<Player>().move_flags = 0;
        f.tick();
        assert!(f.rx.is_empty(), "stopped controller must not restart");
    }
}

#[test]
fn issue95_attack_seam_stop_preserves_unrelated_running_repeat() {
    let mut f = Fixture::new(20.0);
    f.world.init_resource::<crate::app_core::loot::LootState>();
    f.world.init_resource::<crate::app_core::state::LootLatch>();
    {
        let mut active = f.world.resource_mut::<AutoRepeatActive>();
        active.1.queued = Some(AUTO_SHOT);
        active.0 = Some(5019);
    }
    f.world
        .entity_mut(f.me)
        .insert((AutoRepeatArmed, Engaged(MOB)));
    f.world
        .run_system_once(|mut seam: crate::creature_anim::AttackSeam| {
            seam.stop_controller(true);
        })
        .expect("production AttackSeam runs");
    f.assert_stopped();
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(5019));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    assert!(matches!(f.rx.try_recv(), Ok(ClientCommand::AttackStop)));
    assert!(
        f.rx.is_empty(),
        "unrelated repeat must not be cancelled or swung"
    );
    f.tick();
    assert!(f.rx.is_empty());
}

#[test]
fn issue95_attack_seam_forced_target_switch_preserves_learned_controller() {
    let mut f = Fixture::new(20.0);
    f.world.init_resource::<crate::app_core::loot::LootState>();
    f.world.init_resource::<crate::app_core::state::LootLatch>();
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            ..
        })
    ));
    f.world.entity_mut(f.me).insert(Engaged(MOB));
    // The scan transaction's forced StartAttack still admits the ranged controller,
    // rather than forcing a melee swing through the server engagement echo.
    f.world
        .run_system_once(|mut seam: crate::creature_anim::AttackSeam| {
            assert!(seam.stop(true));
            assert!(crate::target::send_selection(&seam.net, MOB));
            assert!(seam.start(MOB, true, true));
        })
        .expect("production AttackSeam runs");
    assert!(matches!(f.rx.try_recv(), Ok(ClientCommand::AttackStop)));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::SetSelection { guid: MOB })
    ));
    assert!(
        f.rx.is_empty(),
        "controller admission must not swing or cancel"
    );
    let active = f.world.resource::<AutoRepeatActive>();
    assert!(active.1.attacking);
    assert_eq!(active.1.queued, Some(AUTO_SHOT));
    assert_eq!(active.0, Some(AUTO_SHOT));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    f.tick();
    assert!(f.rx.is_empty(), "the running repeat must not be resent");
}

#[test]
fn issue95_attack_seam_forced_reswing_falls_back_to_melee_without_learned_spells() {
    let mut f = Fixture::new(20.0);
    f.world.init_resource::<crate::app_core::loot::LootState>();
    f.world.init_resource::<crate::app_core::state::LootLatch>();
    f.world
        .resource_mut::<crate::app_core::state::PlayerActions>()
        .spells
        .clear();
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.world.entity_mut(f.me).insert(Engaged(MOB));
    f.world
        .run_system_once(|mut seam: crate::creature_anim::AttackSeam| {
            assert!(seam.stop(true));
            assert!(crate::target::send_selection(&seam.net, MOB));
            assert!(seam.start(MOB, true, true));
        })
        .expect("production AttackSeam runs");
    assert!(matches!(f.rx.try_recv(), Ok(ClientCommand::AttackStop)));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::SetSelection { guid: MOB })
    ));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::AttackSwing { guid: MOB })
    ));
    assert!(f.rx.is_empty());
    f.assert_stopped();
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    f.tick();
    assert!(f.rx.is_empty());
}

#[test]
fn issue95_controller_deduplicates_repeated_local_equipment_failure() {
    let mut f = Fixture::new(20.0);
    f.world.resource_mut::<Spells>().catalog = SpellCatalog::from_displays(HashMap::from([(
        AUTO_SHOT,
        SpellDisplay {
            attributes: 0x2,
            attributes_ex2: 0x20,
            attributes_ex4: 0x0100_0000,
            implicit_target_a1: 6,
            range_index: 114,
            equipped_item_class: 2, // Weapon required; fixture has no equipped items.
            equipped_item_subclass_mask: u32::MAX,
            ..Default::default()
        },
    )]));
    for _ in 0..3 {
        f.tick();
        let errors = &f.world.resource::<crate::app_core::errors::CastErrors>().0;
        assert_eq!(errors.len(), 1, "identical retries yield one local error");
        assert_eq!(
            errors[0].reason,
            crate::spell::cast_fail_keys::reason::EQUIPPED_ITEM_CLASS
        );
        let active = f.world.resource::<AutoRepeatActive>();
        assert_eq!(active.0, None);
        assert_eq!(active.1.queued, Some(AUTO_SHOT));
        assert!(active.1.attacking);
        assert!(!active.1.attempting);
        assert_eq!(active.1.last_failure, Some(errors[0].reason));
        assert!(f.rx.is_empty());
        assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
        assert!(f.world.resource::<Messages<SheathRequest>>().is_empty());
    }
}

#[test]
fn issue95_melee_initiator_send_tail_preserves_registered_controller() {
    const STRIKE: u32 = 90001;
    let mut f = Fixture::new(3.0);
    f.world.resource_mut::<Spells>().catalog = SpellCatalog::from_displays(HashMap::from([
        (
            AUTO_SHOT,
            SpellDisplay {
                attributes_ex4: 0x0100_0000,
                ..Default::default()
            },
        ),
        (
            STRIKE,
            SpellDisplay {
                attributes_ex: 0x200, // INITIATES_COMBAT: runs the real post-send melee tail.
                implicit_target_a1: 6,
                ..Default::default()
            },
        ),
    ]));
    f.world.resource_mut::<AutoRepeatActive>().1.queued = Some(AUTO_SHOT);
    f.world.resource_mut::<AutoRepeatActive>().0 = Some(AUTO_SHOT);
    f.world.entity_mut(f.me).insert(AutoRepeatArmed);
    f.world
        .run_system_once(
            |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
                ladder.send(
                    STRIKE,
                    &targeting.context(),
                    crate::spell::CastCommit::Spell,
                );
            },
        )
        .expect("production CastLadder and targeting params run together");
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: STRIKE,
            target: Some(MOB)
        })
    ));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::AttackSwing { guid: MOB })
    ));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CancelAutoRepeat)
    ));
    assert!(f.rx.is_empty());
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
    assert!(f
        .world
        .resource::<crate::app_core::errors::CastErrors>()
        .0
        .is_empty());
}

#[test]
fn issue95_queued_continuation_does_not_toggle_off_waiting_controller() {
    let mut f = Fixture::new(20.0);
    f.world.resource_mut::<Player>().move_flags = 1;
    f.tick();
    f.assert_queued_without_start();
    f.world
        .run_system_once(
            |targeting: cast_target::CastTargeting, mut ladder: CastLadder| {
                ladder.send_continuation(AUTO_SHOT, &targeting.context());
            },
        )
        .expect("production continuation and targeting params run together");
    f.assert_queued_without_start();
    f.world.resource_mut::<Player>().move_flags = 0;
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            target: Some(MOB),
        })
    ));
    assert!(f.rx.is_empty());
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(AUTO_SHOT));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
    f.tick();
    assert!(f.rx.is_empty(), "continuation commits the repeat only once");
}
