use super::*;
use crate::creature_anim::{AutoRepeatArmed, CastHold, NockLatch, NockedAmmo, RangedHold};

/// Escape's final ClearTarget tail and empty-world clicks must cancel even without melee.
#[test]
fn repeat_cancel_script_clear_and_empty_click_share_actual_teardown() {
    for script_clear in [false, true] {
        for engaged in [false, true] {
            let (mut world, target) = right_click_world();
            let (tx, rx) = crossbeam_channel::unbounded();
            world.insert_resource(NetCommands(tx));
            world.init_resource::<Messages<crate::creature_anim::spell_visual::SpellKitFx>>();
            world.init_resource::<Messages<crate::creature_anim::spell_visual::SpellKitSound>>();
            world.init_resource::<InspectMode>();
            world.init_resource::<crate::spell::SpellTargeting>();
            world.init_resource::<ClickConfig>();
            world.init_resource::<Messages<WorldClick>>();
            world.init_resource::<Messages<crate::sound::NpcGreetingRequest>>();
            let me = world
                .query_filtered::<Entity, With<SelfPlayer>>()
                .single(&world)
                .unwrap();
            world.entity_mut(me).insert((
                AutoRepeatArmed,
                RangedHold,
                NockedAmmo { display_id: 1 },
                NockLatch,
                CastHold {
                    anim_id: 106,
                    spell_id: 75,
                    ranged: true,
                },
            ));
            if engaged {
                world.entity_mut(me).insert(Engaged(BOAR));
            }
            world.resource_mut::<crate::spell::AutoRepeatActive>().0 = Some(75);
            *world.resource_mut::<Selection>() = Selection {
                target: Some(target),
                guid: Some(BOAR),
                last: None,
            };
            if script_clear {
                // Exercise ScriptSelect::clear_target, which Escape's ClearTarget() dispatch calls.
                world.init_resource::<crate::app_core::group::GroupState>();
                world.init_resource::<super::super::super::AssistAttack>();
                world.init_resource::<super::super::super::scan::LastEnemy>();
                world.init_resource::<super::super::super::scan::LastFriend>();
                world.init_resource::<super::super::super::scan::TabHistory>();
                world.init_resource::<crate::names::NameCache>();
                world.init_resource::<Time>();
                world
                    .run_system_once(|mut select: crate::target::script::ScriptSelect| {
                        select.clear_target();
                        select.clear_target(); // no selection remains: no second send
                    })
                    .unwrap();
            } else {
                world
                    .resource_mut::<Messages<WorldClick>>()
                    .write(WorldClick);
                world.run_system_once(select_on_click).unwrap();
                world
                    .resource_mut::<Messages<WorldClick>>()
                    .write(WorldClick);
                world.run_system_once(select_on_click).unwrap();
            }
            let selection = world.resource::<Selection>();
            assert_eq!(
                (selection.target, selection.guid, selection.last),
                (None, None, Some(BOAR))
            );
            assert_eq!(world.resource::<crate::spell::AutoRepeatActive>().0, None);
            assert!(world.get::<CastHold>(me).is_none());
            assert!(world.get::<AutoRepeatArmed>(me).is_none());
            assert!(world.get::<RangedHold>(me).is_none());
            assert!(world.get::<NockedAmmo>(me).is_none());
            assert!(world.get::<NockLatch>(me).is_none());
            let sent: Vec<_> = rx
                .try_iter()
                .map(|c| match c {
                    ClientCommand::SetSelection { guid: 0 } => "clear",
                    ClientCommand::AttackStop => "stop",
                    ClientCommand::CancelAutoRepeat => "cancel",
                    _ => "unexpected",
                })
                .collect();
            assert_eq!(
                sent,
                if engaged {
                    vec!["clear", "stop", "cancel"]
                } else {
                    vec!["clear", "cancel"]
                }
            );
        }
    }
}
