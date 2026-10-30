use super::*;
use crate::app_core::movement::TransportRider;
use crate::net::GuidIndex;
use bevy::ecs::system::RunSystemOnce;

fn tick(world: &mut World) {
    world
        .run_system_once(
            |time: Res<Time>,
             mut commands: Commands,
             mut player: ResMut<Player>,
             mut cam: ResMut<TestCamera>,
             net: Res<NetCommands>,
             mut teleports: MessageReader<TeleportMessage>,
             mut worldports: MessageReader<WorldportMessage>,
             mut speeds: MessageReader<SpeedChangeMessage>,
             mut modes: MessageReader<MoveModeMessage>,
             mut knocks: MessageReader<KnockBackMessage>,
             mut moves: MessageReader<SelfMoveMessage>,
             mut controls: MessageReader<ClientControlMessage>,
             transports: crate::player::TransportQuery,
             body: Query<(&Transform, Option<&TransportRider>), With<Embodied>>,
             index: Res<GuidIndex>| {
                let pose = body.single().ok();
                apply_server_moves(
                    &time,
                    &mut commands,
                    &mut player,
                    &mut cam.0,
                    &net,
                    &mut teleports,
                    &mut worldports,
                    &mut speeds,
                    &mut modes,
                    &mut knocks,
                    &mut moves,
                    &mut controls,
                    Some(7),
                    &transports,
                    pose.map(|(t, _)| (t.translation, 0.0)),
                    pose.and_then(|(_, r)| r),
                    &index,
                );
            },
        )
        .unwrap();
    world.resource_mut::<Messages<TeleportMessage>>().clear();
}

#[derive(Resource)]
struct TestCamera(FlyCam);

// packed mover 7, counter 8, Wrath MovementInfo, optional transport tail.
fn packet(transport: Option<u64>) -> Vec<u8> {
    let mut body = vec![1, 7];
    body.extend(8u32.to_le_bytes());
    body.extend(if transport.is_some() { 0x200u32 } else { 0 }.to_le_bytes());
    body.extend(0u16.to_le_bytes());
    body.extend(123u32.to_le_bytes());
    for v in [500.0f32, 600.0, 700.0, 2.0] {
        body.extend(v.to_le_bytes());
    }
    if let Some(guid) = transport {
        body.extend([1, guid as u8]);
        for v in [1.0f32, 2.0, 3.0, 0.4] {
            body.extend(v.to_le_bytes());
        }
        body.extend(456u32.to_le_bytes());
        body.push(255); // seat -1
    }
    body.extend(0u32.to_le_bytes()); // fall time
    body
}

#[test]
fn raw_near_teleport_reparents_by_packet_then_controller_carries_and_acks_once() {
    let mut app = App::new();
    app.add_plugins(crate::net::NetPlugin { connect: false });
    app.init_resource::<crate::app_core::cooldowns::Cooldowns>();
    app.world_mut().resource_mut::<crate::net::SelfGuid>().0 = Some(7);
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Player {
        active: true,
        ..Default::default()
    });
    app.insert_resource(TestCamera(FlyCam {
        yaw: 0.0,
        pitch: -0.45,
        speed: 1.0,
    }));
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx));
    let pose = Transform::from_translation(Vec3::new(100.0, 200.0, 300.0))
        .with_rotation(Quat::from_rotation_y(0.7));
    let deck = app
        .world_mut()
        .spawn((Transport::test_deck(), Guid(42), pose))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(42, deck);
    // Start attached to a DIFFERENT transport. This must not be blindly retained.
    app.world_mut().resource_mut::<Player>().ride = Some(super::super::state::PlayerRide {
        entity: Entity::PLACEHOLDER,
        guid: 99,
        local_pos: Vec3::splat(99.0),
        boat_yaw: -1.0,
    });
    crate::net::test_support::dispatch_mover_order(&mut app, 0xc7, &packet(Some(42)));
    let msg = app
        .world()
        .resource::<Messages<TeleportMessage>>()
        .iter_current_update_messages()
        .next()
        .unwrap();
    assert_eq!(msg.transport.unwrap().guid, 42);
    tick(app.world_mut());
    let p = app.world().resource::<Player>();
    assert_eq!(p.ride.as_ref().unwrap().entity, deck);
    let expected = pose.translation + pose.rotation * wow_to_bevy([1.0, 2.0, 3.0]);
    assert!(p.pos.distance(expected) < 0.001);
    assert!((p.face_yaw - 1.1).abs() < 0.001);
    assert!((p.model_yaw - 1.1).abs() < 0.001);
    assert!(!p.settling);
    let tail = super::super::movement_net::wire_transport(p).unwrap();
    assert_eq!(tail.guid, 42);
    assert_eq!([tail.pos.x, tail.pos.y, tail.pos.z], [1.0, 2.0, 3.0]);
    assert!((tail.orientation - 0.4).abs() < 0.001);
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientCommand::TeleportAck {
            guid: 7,
            counter: 8
        }
    ));
    assert!(rx.try_recv().is_err());
    app.world_mut()
        .get_mut::<Transform>(deck)
        .unwrap()
        .translation
        .x += 5.0;
    app.world_mut()
        .run_system_once(
            |mut p: ResMut<Player>,
             mut cam: ResMut<TestCamera>,
             decks: crate::player::TransportQuery| {
                super::super::ride::carry(&mut p, &mut cam.0, &decks);
            },
        )
        .unwrap();
    assert!(
        app.world()
            .resource::<Player>()
            .pos
            .distance(expected + Vec3::X * 5.0)
            < 0.001
    );
    // An absent packet tail authorizes deboarding even though the previous ride is still valid.
    crate::net::test_support::dispatch_mover_order(&mut app, 0xc7, &packet(None));
    tick(app.world_mut());
    let p = app.world().resource::<Player>();
    assert!(p.ride.is_none() && p.settling && p.world_stale);
    assert_eq!(p.pos, wow_to_bevy([500.0, 600.0, 700.0]));
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientCommand::TeleportAck { .. }
    ));
    // Missing deck: never treat a local offset as world, never keep the old deck.
    crate::net::test_support::dispatch_mover_order(&mut app, 0xc7, &packet(Some(88)));
    tick(app.world_mut());
    let p = app.world().resource::<Player>();
    assert!(p.ride.is_none() && p.settling);
    assert_eq!(p.pos, wow_to_bevy([500.0, 600.0, 700.0]));
    assert!(matches!(
        rx.try_recv().unwrap(),
        ClientCommand::TeleportAck { .. }
    ));
    tick(app.world_mut());
    assert!(rx.try_recv().is_err(), "no duplicate controller ACK");
    // Self filter belongs to the bridge, not to the controller.
    app.world_mut().resource_mut::<crate::net::SelfGuid>().0 = Some(9);
    crate::net::test_support::dispatch_mover_order(&mut app, 0xc7, &packet(Some(42)));
    tick(app.world_mut());
    assert!(rx.try_recv().is_err());
    assert!(app.world().resource::<Player>().ride.is_none());
}
