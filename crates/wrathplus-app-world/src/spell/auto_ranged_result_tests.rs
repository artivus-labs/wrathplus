//! Cast-result continuations must preserve an Auto Shot already queued by the controller.
//! Drive the real packet handler and controller, not a direct continuation/seam call.
use super::integration_tests::{Fixture, AUTO_SHOT, MOB};
use super::*;

use bevy::ecs::system::RunSystemOnce;
use std::collections::HashMap;
use std::time::Instant;
use wrathplus_formats::{SpellCatalog, SpellDisplay};
use wrathplus_protocol::SessionEvent;

use crate::app_core::anim::PlaySeq;
use crate::app_core::player::Player;
use crate::app_core::state::{PlayerActions, Spells};
use crate::creature_anim::{AutoRepeatArmed, CastEvent, SheathRequest};
use crate::net::ClientCommand;
use crate::spell::{cast_fail_keys::reason, PendingCast};

const STING: u32 = 1978;

fn queued_auto_shot_survives_result(success: bool) {
    let mut f = Fixture::new(20.0);
    // Keep the fixture's production Auto Shot row and add a hunter shot whose reply
    // chains it. Bit 24 and the learned book entry make 75 the controller's choice.
    f.world.resource_mut::<Spells>().catalog = SpellCatalog::from_displays(HashMap::from([
        (
            AUTO_SHOT,
            SpellDisplay {
                attributes: 0x2,
                attributes_ex2: 0x20,
                attributes_ex4: 0x0100_0000,
                implicit_target_a1: 6,
                range_index: 114,
                ..Default::default()
            },
        ),
        (
            STING,
            SpellDisplay {
                modal_next_spell: AUTO_SHOT,
                ..Default::default()
            },
        ),
    ]));
    f.world
        .resource_mut::<PlayerActions>()
        .spells
        .insert(AUTO_SHOT);
    // Reply requires these in addition to the fixture's CastLadder resources.
    f.world.init_resource::<Messages<CastEvent>>();
    f.world.init_resource::<PlaySeq>();

    f.world.resource_mut::<Player>().move_flags = 1;
    f.tick();
    f.assert_queued_without_start();
    // A ranged commit does not guard the next cast, but must still match the
    // packet handler's committed-id check before reading modal_next_spell.
    f.world
        .resource_mut::<PendingCast>()
        .arm(STING, Instant::now(), false);
    f.world
        .run_system_once_with(
            super::super::net::on_cast_result,
            SessionEvent::CastResult {
                spell_id: STING,
                success,
                reason: if success { None } else { Some(reason::MOVING) },
                arg: None,
            },
        )
        .expect("production CastResult handler and its SystemParams run");

    // A continuation is not a second press: it must not toggle off the queued
    // intent or send a shot while moving. Failure's UI error is independent.
    let active = f.world.resource::<AutoRepeatActive>();
    assert_eq!(active.1.queued, Some(AUTO_SHOT));
    assert!(active.1.attacking);
    assert!(!active.1.attempting);
    assert_eq!(active.0, None);
    assert!(f.rx.is_empty(), "the result must not send while moving");
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    assert!(f.world.resource::<Messages<SheathRequest>>().is_empty());
    assert_eq!(
        f.world.resource::<PendingCast>().committed(Instant::now()),
        None,
        "the reply must resolve the original committed slot"
    );

    f.tick();
    assert_eq!(
        f.world.resource::<AutoRepeatActive>().1.queued,
        Some(AUTO_SHOT)
    );
    assert!(f.rx.is_empty(), "a moving tick must keep waiting");

    f.world.resource_mut::<Player>().move_flags = 0;
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            target: Some(MOB),
        })
    ));
    assert!(f.rx.is_empty(), "stationary retry sends exactly one shot");
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(AUTO_SHOT));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
    f.tick();
    assert!(f.rx.is_empty(), "the running repeat must not be resent");
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
}

#[test]
fn successful_cast_result_preserves_queued_auto_shot_until_stationary() {
    queued_auto_shot_survives_result(true);
}

#[test]
fn failed_cast_result_preserves_queued_auto_shot_until_stationary() {
    queued_auto_shot_survives_result(false);
}
