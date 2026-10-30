use super::*;

fn spline(guid: u64, spline_id: u32) -> SessionEvent {
    SessionEvent::MonsterMove {
        guid,
        spline_id,
        transport: None,
        start: [0.0; 3],
        path: vec![[1.0; 3]],
        facing: wrathplus_protocol::types::MonsterMoveFacing::None,
        stop: false,
        duration_ms: 1000,
        flying: true,
        interpolation: wrathplus_protocol::events::SplineInterpolation::CatmullRom,
        cyclic: false,
        effect: None,
        run_mode: None,
    }
}

fn ack(authority: &MovementAuthorityHandle, id: u32) -> usize {
    let mut sent = 0;
    authority
        .send_taxi_done(0, [1.0; 3], 0.0, id, 55, |op, _| {
            assert_eq!(op, 0x2c9);
            sent += 1;
            Ok(())
        })
        .unwrap();
    sent
}

fn ready(shared: &Arc<MovementAuthority>, epoch: u64) -> MovementAuthorityHandle {
    let authority = shared.install(epoch, 7);
    authority
        .observe_self_create(
            7,
            &MovementPosition::Stationary(wrathplus_protocol::wrath_gameplay::Position {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                orientation: 0.5,
            }),
        )
        .unwrap();
    authority.observe_control(7, false).unwrap();
    authority
}

#[test]
fn observed_own_taxi_spline_acks_once_without_control_grant() {
    let shared = Arc::new(MovementAuthority::default());
    let authority = ready(&shared, 1);
    authority.observe_taxi_spline(&spline(8, 77));
    assert_eq!(ack(&authority, 77), 0);
    authority.observe_taxi_spline(&spline(7, 77));
    assert_eq!(ack(&authority, 78), 0);
    assert!(authority.with_movement_send(|| ()).is_none());
    assert_eq!(ack(&authority, 77), 1);
    assert_eq!(ack(&authority, 77), 0);
    authority.observe_taxi_spline(&spline(7, 77));
    assert_eq!(ack(&authority, 77), 0);
    assert!(authority.with_movement_send(|| ()).is_none());
}

#[test]
fn stale_epoch_and_retirement_refuse_pending_taxi_ack() {
    let shared = Arc::new(MovementAuthority::default());
    let old = ready(&shared, 1);
    old.observe_taxi_spline(&spline(7, 77));
    let current = ready(&shared, 2);
    current.observe_taxi_spline(&spline(7, 88));
    assert_eq!(ack(&old, 77), 0);
    old.revoke();
    assert_eq!(ack(&current, 88), 1);
    current.observe_taxi_spline(&spline(7, 90));
    current.send_logout(|| Ok(())).unwrap();
    assert_eq!(ack(&current, 90), 1);
    // Revoke a still-outstanding spline, not merely an already-consumed ACK.
    current.observe_taxi_spline(&spline(7, 92));
    current.revoke();
    assert_eq!(ack(&current, 92), 0);
    assert_eq!(ack(&current, 90), 0);

    let next = ready(&shared, 3);
    next.observe_taxi_spline(&spline(7, 91));
    // A server near teleport (`MSG_MOVE_TELEPORT_ACK` order) ends the flight path.
    next.raise_mover_barrier(
        wrathplus_protocol::wrath_authority::MoverOrder::TeleportNear {
            mover_guid: 7,
            counter: 1,
            position: [5.0; 3],
            orientation: 0.0,
            transport: None,
        },
    )
    .unwrap();
    assert_eq!(ack(&next, 91), 0);
}

#[test]
fn non_taxi_splines_and_non_rest_ack_do_not_authorize_sends() {
    let shared = Arc::new(MovementAuthority::default());
    let authority = ready(&shared, 1);
    for kind in 0..4 {
        let mut event = spline(7, 77);
        if let SessionEvent::MonsterMove {
            flying,
            stop,
            cyclic,
            transport,
            ..
        } = &mut event
        {
            match kind {
                0 => *flying = false,
                1 => *stop = true,
                2 => *cyclic = true,
                _ => *transport = Some(9),
            }
        }
        authority.observe_taxi_spline(&event);
        assert_eq!(ack(&authority, 77), 0);
    }
    authority.observe_taxi_spline(&spline(7, 78));
    authority
        .send_taxi_done(1, [1.0; 3], 0.0, 78, 55, |_, _| panic!("non-rest ACK sent"))
        .unwrap();
    assert_eq!(ack(&authority, 78), 1);
    assert_eq!(
        route(&ClientCommand::MoveSplineDone {
            flags: 0,
            pos: [1.0; 3],
            orientation: 0.0,
            spline_id: 78
        }),
        WrathRoute::TaxiSplineDone
    );
}
