//! Issue 95: drive the production controller with both of its real SystemParams.
//! Included as a child of auto_ranged; no renderer, data archives or realm required.
use super::*;

use bevy::ecs::system::RunSystemOnce;
use crossbeam_channel::Receiver;
use std::collections::HashMap;
use wrathplus_formats::{SpellCatalog, SpellDisplay, SpellRange, SpellRangeCatalog};
use wrathplus_protocol::{types::Field, ObjectFields};

use crate::app_core::player::Player;
use crate::app_core::session::{Embodied, ObjectStore, SelfGuid, SelfPlayer};
use crate::app_core::state::{PlayerActions, Selection, Spells};
use crate::creature_anim::{AutoRepeatArmed, Engaged, SheathRequest};
use crate::net::{ClientCommand, Guid, GuidIndex, NetCommands, Reputations};

pub(super) const AUTO_SHOT: u32 = 75;
const ME: u64 = 7;
pub(super) const MOB: u64 = 0xf130_0000_0000_00f1;

pub(super) struct Fixture {
    pub(super) world: World,
    pub(super) rx: Receiver<ClientCommand>,
    pub(super) me: Entity,
    target: Entity,
}

impl Fixture {
    pub(super) fn new(distance: f32) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut world = World::new();
        world.insert_resource(NetCommands(tx));
        // CastLadder's resource set, intentionally initialized rather than mocking send().
        world.init_resource::<crate::items::Items>();
        world.init_resource::<GuidIndex>();
        world.init_resource::<crate::spell::PendingCast>();
        world.init_resource::<crate::spell::QueuedMeleeSpell>();
        world.init_resource::<crate::app_core::cooldowns::Cooldowns>();
        world.init_resource::<crate::spell::SpellModifiers>();
        world.init_resource::<crate::net::wrath_combo_points::NativeComboPoints>();
        world.init_resource::<crate::app_core::errors::CastErrors>();
        world.init_resource::<AutoRepeatActive>();
        world.init_resource::<crate::app_core::interaction::TradeSkillOpens>();
        world.init_resource::<crate::spell::SpellTargeting>();
        world.init_resource::<crate::spell::HeldForPick>();
        world.init_resource::<Messages<SheathRequest>>();
        world.init_resource::<Messages<crate::player::StandStateRequest>>();
        // cancel_auto_repeat_local queues visual teardown, even without a rendered model.
        world.init_resource::<Messages<crate::creature_anim::SpellKitSound>>();
        world.init_resource::<Messages<crate::creature_anim::SpellKitFx>>();
        // CastTargeting's required inputs, in addition to the shared read-only Spells/Player.
        world.insert_resource(SelfGuid(Some(ME)));
        world.init_resource::<cast_target::AutoSelfCast>();
        world.insert_resource(Reputations(Vec::new()));
        world.insert_resource(Player::default());

        let mut spells = Spells::empty_for_tests();
        spells.catalog = SpellCatalog::from_displays(HashMap::from([(
            AUTO_SHOT,
            SpellDisplay {
                attributes: 0x2, // ranged attack
                attributes_ex2: 0x20,
                attributes_ex4: 0x0100_0000, // learned controller spell + running auto-repeat
                implicit_target_a1: 6,       // enemy; exercise the real relation/binding leg
                range_index: 114,
                ..Default::default()
            },
        )]));
        spells.ranges = SpellRangeCatalog::from_rows(HashMap::from([(
            114,
            SpellRange {
                min: 8.0,
                max: 35.0,
                flags: 0,
            },
        )]));
        world.insert_resource(spells);
        let mut book = PlayerActions::default();
        book.spells.insert(AUTO_SHOT);
        world.insert_resource(book);

        // As in cast_target's resolution_wire_shapes fixture: player-controlled caster,
        // non-player target, both alive. Without a faction catalog the mixed CanAttack
        // arm sees neutral (< friendly), which is attackable and satisfies the enemy word.
        let me = world
            .spawn((
                SelfPlayer,
                Embodied,
                Guid(ME),
                Transform::default(),
                ObjectStore(ObjectFields::from_fields(&[
                    (Field::ObjectType, 0x19),
                    (Field::UnitHealth, 100),
                    (Field::UnitMaxHealth, 100),
                    (Field::UnitFlags, 0x8),
                    (Field::UnitCombatreach, 1.5f32.to_bits()),
                ])),
            ))
            .id();
        let target = world
            .spawn((
                Guid(MOB),
                Transform::from_xyz(0.0, 0.0, -distance),
                ObjectStore(ObjectFields::from_fields(&[
                    (Field::ObjectType, 0x9),
                    (Field::UnitHealth, 100),
                    (Field::UnitMaxHealth, 100),
                    (Field::UnitFactionTemplate, 0),
                    (Field::UnitCombatreach, 1.5f32.to_bits()),
                ])),
            ))
            .id();
        world
            .resource_mut::<GuidIndex>()
            .0
            .extend([(ME, me), (MOB, target)]);
        world.insert_resource(Selection {
            target: Some(target),
            guid: Some(MOB),
            ..Default::default()
        });
        {
            let mut active = world.resource_mut::<AutoRepeatActive>();
            assert!(request(&mut active, Some(AUTO_SHOT), false));
        }
        Self {
            world,
            rx,
            me,
            target,
        }
    }

    pub(super) fn tick(&mut self) {
        // Initialization itself is part of the regression: shared Spells/Player and the
        // nested targeting/range queries must not conflict with CastLadder's accesses.
        self.world
            .run_system_once(tick)
            .expect("production controller SystemParams are compatible");
    }

    fn distance(&mut self, distance: f32) {
        self.world
            .get_mut::<Transform>(self.target)
            .unwrap()
            .translation = Vec3::new(0.0, 0.0, -distance);
    }

    pub(super) fn assert_queued_without_start(&self) {
        let active = self.world.resource::<AutoRepeatActive>();
        assert_eq!(active.1.queued, Some(AUTO_SHOT));
        assert!(active.1.attacking);
        assert!(!active.1.attempting);
        assert_eq!(active.0, None);
        assert!(
            self.rx.is_empty(),
            "waiting must not send a cast or attack packet"
        );
        assert!(self.world.get::<AutoRepeatArmed>(self.me).is_none());
        assert!(self.world.resource::<Messages<SheathRequest>>().is_empty());
        assert!(self
            .world
            .resource::<crate::app_core::errors::CastErrors>()
            .0
            .is_empty());
    }

    pub(super) fn assert_stopped(&self) {
        let active = self.world.resource::<AutoRepeatActive>();
        assert_eq!(active.1.queued, None);
        assert!(!active.1.attacking);
        assert!(!active.1.close_only);
        assert!(!active.1.attempting);
    }
}

#[test]
fn issue95_waits_without_animation_or_packets_then_commits_once_when_stationary_and_facing() {
    let mut f = Fixture::new(20.0);
    f.world.resource_mut::<Player>().move_flags = 1;
    f.tick();
    f.assert_queued_without_start();
    // Still waiting on facing once movement ends; a queued intent is not a toggle-off.
    f.world.resource_mut::<Player>().move_flags = 0;
    f.world.resource_mut::<Player>().face_yaw = std::f32::consts::PI;
    f.tick();
    f.assert_queued_without_start();
    f.world.resource_mut::<Player>().face_yaw = 0.0;
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            target: Some(MOB),
        })
    ));
    assert!(f.rx.is_empty());
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(AUTO_SHOT));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
    assert!(!f.world.resource::<AutoRepeatActive>().1.attempting);
    f.tick();
    assert!(
        f.rx.is_empty(),
        "the running repeat is not recast on the next tick"
    );
    assert_eq!(f.world.resource::<Messages<SheathRequest>>().len(), 1);
}

#[test]
fn issue95_out_of_range_intent_waits_and_strict_max_does_not_commit() {
    let mut f = Fixture::new(60.0);
    f.tick();
    f.assert_queued_without_start();
    // Row 114's 35 yards plus both 1.5 reaches: native controller uses strict < max.
    f.distance(38.0);
    f.tick();
    f.assert_queued_without_start();
    f.distance(20.0);
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            target: Some(MOB),
        })
    ));
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(AUTO_SHOT));
}

#[test]
fn issue95_close_target_transitions_from_running_repeat_to_melee() {
    let mut f = Fixture::new(20.0);
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            ..
        })
    ));
    f.distance(3.0);
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::AttackSwing { guid: MOB })
    ));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CancelAutoRepeat)
    ));
    assert!(f.rx.is_empty());
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    assert!(
        f.world.resource::<AutoRepeatActive>().1.attacking,
        "melee helper clears intention but controller restores registration"
    );
    assert_eq!(f.world.resource::<AutoRepeatActive>().1.queued, None);
    // The server owns Engaged; simulate its echo before the next tick.
    f.world.entity_mut(f.me).insert(Engaged(MOB));
    f.tick();
    assert!(
        f.rx.is_empty(),
        "already-engaged melee does not resend AttackSwing"
    );
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    assert_eq!(f.world.resource::<AutoRepeatActive>().1.queued, None);
}

#[test]
fn issue95_unlearning_or_disabling_cvar_clears_waiting_intent() {
    let mut f = Fixture::new(60.0);
    f.tick();
    f.assert_queued_without_start();
    f.world.resource_mut::<PlayerActions>().spells.clear();
    f.tick();
    f.assert_stopped();
    assert!(f.rx.is_empty());

    let mut f = Fixture::new(60.0);
    f.tick();
    f.world.add_observer(on_cvar);
    f.world.trigger(crate::cvars::CvarChanged {
        name: "autoRangedCombat".into(),
        old: "1".into(),
        new: "0".into(),
    });
    f.world.flush();
    f.tick();
    f.assert_stopped();
    assert!(!f.world.resource::<AutoRepeatActive>().1.enabled);
    assert!(f.rx.is_empty());
}

#[test]
fn issue95_lost_selection_stops_the_controllers_running_repeat() {
    let mut f = Fixture::new(20.0);
    f.tick();
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CastSpell {
            spell_id: AUTO_SHOT,
            ..
        })
    ));
    *f.world.resource_mut::<Selection>() = Selection::default();
    f.tick();
    f.assert_stopped();
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::CancelAutoRepeat)
    ));
    f.tick();
    assert!(f.rx.is_empty(), "stopping is idempotent");
}

#[test]
fn issue95_explicit_intent_stop_and_reset_do_not_restart_the_controller() {
    let mut f = Fixture::new(60.0);
    f.tick();
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.distance(20.0);
    f.tick();
    f.assert_stopped();
    assert!(f.rx.is_empty());

    // A close-only attack request must not turn into a ranged attack at distance.
    assert!(request(
        &mut f.world.resource_mut::<AutoRepeatActive>(),
        Some(AUTO_SHOT),
        true
    ));
    f.tick();
    f.assert_stopped();
    assert!(f.rx.is_empty());

    {
        let mut active = f.world.resource_mut::<AutoRepeatActive>();
        active.0 = Some(AUTO_SHOT);
        active.1.queued = Some(AUTO_SHOT);
        active.1.attacking = true;
    }
    f.world.run_system_once(reset).expect("session reset runs");
    f.tick();
    f.assert_stopped();
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(
        f.rx.is_empty(),
        "reset clears local controller state without sending"
    );
}

#[test]
fn issue95_idle_combat_initiator_registers_controller_and_later_shoots_at_distance() {
    const STRIKE: u32 = 90001;
    let mut f = Fixture::new(3.0);
    f.world.resource_mut::<AutoRepeatActive>().clear_intent();
    f.assert_stopped();
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
            STRIKE,
            SpellDisplay {
                attributes_ex: 0x200, // INITIATES_COMBAT: the normal post-send melee tail.
                implicit_target_a1: 6,
                ..Default::default()
            },
        ),
    ]));
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
            target: Some(MOB),
        })
    ));
    assert!(matches!(
        f.rx.try_recv(),
        Ok(ClientCommand::AttackSwing { guid: MOB })
    ));
    assert!(f.rx.is_empty());
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    assert_eq!(f.world.resource::<AutoRepeatActive>().1.queued, None);
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_none());
    // Resolve the ordinary initiating cast before the controller can enter another cast.
    f.world
        .resource_mut::<crate::spell::PendingCast>()
        .clear_if(STRIKE);
    f.distance(20.0);
    f.world.resource_mut::<Player>().move_flags = 1;
    f.tick();
    assert_eq!(
        f.world.resource::<AutoRepeatActive>().1.queued,
        Some(AUTO_SHOT)
    );
    assert!(f.world.resource::<AutoRepeatActive>().1.attacking);
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, None);
    assert!(f.rx.is_empty());
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
    assert_eq!(f.world.resource::<AutoRepeatActive>().0, Some(AUTO_SHOT));
    assert!(f.world.get::<AutoRepeatArmed>(f.me).is_some());
    assert!(f
        .world
        .resource::<crate::app_core::errors::CastErrors>()
        .0
        .is_empty());
    f.tick();
    assert!(f.rx.is_empty(), "registered repeat is not resent");
}
