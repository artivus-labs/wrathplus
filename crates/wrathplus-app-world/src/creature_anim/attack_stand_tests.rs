use super::*;
use bevy::ecs::system::RunSystemOnce;

fn start_once(engaged: bool, stop_in_flight: bool, keep_receiver: bool) -> (bool, App) {
    let mut app = App::new();
    app.add_message::<SheathRequest>()
        .add_message::<crate::player::StandStateRequest>();
    let (tx, rx) = crossbeam_channel::unbounded();
    if !keep_receiver {
        drop(rx);
    }
    app.insert_resource(crate::net::NetCommands(tx));
    app.insert_resource(crate::spell::AutoRepeatActive(Some(75), Default::default()));
    let me = app.world_mut().spawn_empty().id();
    let sent = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  mut auto_repeat: ResMut<crate::spell::AutoRepeatActive>,
                  mut sheath: MessageWriter<SheathRequest>,
                  net: Res<crate::net::NetCommands>| {
                start_attack_local(
                    me,
                    0xdead_beef,
                    engaged,
                    stop_in_flight,
                    &mut auto_repeat,
                    &mut sheath,
                    &mut commands,
                    &net,
                )
            },
        )
        .expect("start helper runs");
    (sent, app)
}

/// StartAttack stands a seated player (`0x5ecc9b`) even on the already-engaged skip, which
/// jumps past the stand to `0x5ecd78` and sends no `CMSG_ATTACKSWING`.
#[test]
fn starting_an_attack_stands_you_even_when_the_swing_is_skipped() {
    let (sent, mut app) = start_once(true, false, true);
    assert!(!sent, "the already-engaged tail did not enqueue a swing");

    let stands: Vec<u8> = app
        .world_mut()
        .resource_mut::<bevy::ecs::message::Messages<crate::player::StandStateRequest>>()
        .drain()
        .map(|r| r.state)
        .collect();
    assert_eq!(
        stands,
        vec![0],
        "StartAttack asks for stand state 0 exactly once, even on the leg that sends no swing"
    );
}

#[test]
fn wrath_start_admits_native_melee_and_cancels_auto_repeat() {
    let (sent, app) = start_once(false, false, true);
    assert!(sent);
    // `CMSG_CANCEL_AUTO_REPEAT_SPELL` has a native route, so the start arm cancels as 1.12 did.
    assert_eq!(
        app.world().resource::<crate::spell::AutoRepeatActive>().0,
        None
    );
    assert_eq!(app.world().resource::<Messages<SheathRequest>>().len(), 1);
    assert_eq!(
        app.world()
            .resource::<Messages<crate::player::StandStateRequest>>()
            .len(),
        1
    );
}

#[test]
fn wrath_closed_command_endpoint_publishes_no_attack_setters() {
    let (sent, app) = start_once(false, false, false);
    assert!(!sent);
    assert_eq!(
        app.world().resource::<crate::spell::AutoRepeatActive>().0,
        Some(75)
    );
    assert!(app.world().resource::<Messages<SheathRequest>>().is_empty());
    assert!(app
        .world()
        .resource::<Messages<crate::player::StandStateRequest>>()
        .is_empty());
}
