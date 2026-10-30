use super::*;

#[test]
fn order_and_ack_match_azerothcore_layout_goldens() {
    let order = decode_unset_can_fly(&[0x09, 0x01, 0x05, 0x44, 0x33, 0x22, 0x11]).unwrap();
    assert_eq!(
        order,
        UnsetCanFlyOrder {
            mover_guid: 0x0000_0000_0500_0001,
            counter: 0x1122_3344,
        }
    );

    let body = encode_unset_can_fly_ack(
        order.mover_guid,
        order.counter,
        GroundPose::new(0, [1.0, 2.0, 3.0], 0.5).unwrap(),
        0xaabb_ccdd,
    )
    .unwrap();
    assert_eq!(
        body,
        vec![
            0x09, 0x01, 0x05, // packed guid
            0x44, 0x33, 0x22, 0x11, // exact order counter
            0, 0, 0, 0, // flags
            0, 0, // flags2
            0xdd, 0xcc, 0xbb, 0xaa, // timestamp
            0, 0, 0x80, 0x3f, // x
            0, 0, 0, 0x40, // y
            0, 0, 0x40, 0x40, // z
            0, 0, 0, 0x3f, // orientation
            0, 0, 0, 0, // fall time
            0, 0, 0, 0, // isApplied = false
        ]
    );
}

#[test]
fn malformed_orders_and_unsupported_or_nonfinite_poses_fail_closed() {
    assert!(decode_unset_can_fly(&[1, 7, 1, 2, 3]).is_err());
    assert!(decode_unset_can_fly(&[1, 7, 1, 2, 3, 4, 5]).is_err());
    assert!(matches!(
        GroundPose::new(0x0200_0000, [0.0; 3], 0.0),
        Err(AuthorityCodecError::UnsupportedGroundState)
    ));
    assert!(matches!(
        GroundPose::new(0, [f32::NAN, 0.0, 0.0], 0.0),
        Err(AuthorityCodecError::NonFinitePose)
    ));
}

#[test]
fn root_and_unroot_goldens_have_exact_counter_flags_and_no_trailer() {
    let order = decode_force_root(&[0x09, 0x01, 0x05, 0x44, 0x33, 0x22, 0x11]).unwrap();
    let pose = GroundPose::new(0, [1.0, 2.0, 3.0], 0.5).unwrap();
    let root =
        encode_force_root_ack(order.mover_guid, order.counter, pose, 0xaabb_ccdd, true).unwrap();
    let unroot =
        encode_force_root_ack(order.mover_guid, order.counter, pose, 0xaabb_ccdd, false).unwrap();
    assert_eq!(&root[..7], &[0x09, 0x01, 0x05, 0x44, 0x33, 0x22, 0x11]);
    assert_eq!(&root[7..11], &MOVEMENT_FLAG_ROOT.to_le_bytes());
    assert_eq!(&unroot[7..11], &[0, 0, 0, 0]);
    assert_eq!(root.len(), 37, "root ACK has no trailing applied word");
    assert_eq!(unroot.len(), 37, "unroot ACK has no trailing applied word");
    assert_eq!(&root[11..], &unroot[11..]);
    assert!(decode_force_root(&[0x09, 0x01, 0x05, 1, 0, 0, 0, 9]).is_err());
}

fn airborne_pose() -> OnFootPose {
    OnFootPose {
        flags: wrath_ground::MOVE_FORWARD | wrath_ground::MOVE_FALLING,
        position: [4.0, 5.0, 8.0],
        orientation: 0.5,
        fall_time: 325,
        fall: Some(FallTail {
            vertical_speed: 7.955547,
            sin_angle: 0.25,
            cos_angle: 0.75,
            horizontal_speed: 7.0,
        }),
        pitch: None,
        transport: None,
    }
}

#[test]
fn on_foot_authority_acks_preserve_airborne_xyzo_time_and_sin_cos_order() {
    let pose = airborne_pose();
    let expected = wrath_ground::encode_on_foot(
        wrath_ground::MSG_MOVE_HEARTBEAT,
        0x0500_0001,
        pose.flags,
        900,
        pose.position,
        pose.orientation,
        pose.fall_time,
        pose.fall,
    )
    .unwrap();
    let mut movement = expected.as_slice();
    let _ = read_packed_guid(&mut movement).unwrap();
    let ack = encode_unset_can_fly_on_foot_ack(0x0500_0001, 77, pose, 900).unwrap();
    assert_eq!(&ack[..7], &[9, 1, 5, 77, 0, 0, 0]);
    assert_eq!(&ack[7..ack.len() - 4], movement);
    assert_eq!(&ack[ack.len() - 4..], &[0; 4]);
    let unroot = encode_force_root_on_foot_ack(0x0500_0001, 77, pose, 900, false).unwrap();
    assert_eq!(&unroot[7..], movement);
    assert!(encode_force_root_on_foot_ack(0x0500_0001, 77, pose, 900, true).is_err());
    let root = encode_force_root_on_foot_ack(0x0500_0001, 77, pose.halted(), 900, true).unwrap();
    assert_eq!(&root[7..11], &MOVEMENT_FLAG_ROOT.to_le_bytes());
    assert_eq!(&root[17..29], &expected[13..25]);
    assert_eq!(&root[33..37], &325u32.to_le_bytes());
    assert_eq!(root.len(), 37);
}

#[test]
fn on_foot_ack_ground_encoding_is_backward_compatible_and_unsupported_modes_fail_closed() {
    let ground = GroundPose::new(0, [1.0, 2.0, 3.0], 0.5).unwrap();
    assert_eq!(
        encode_unset_can_fly_ack(0x0500_0001, 1, ground, 99).unwrap(),
        encode_unset_can_fly_on_foot_ack(0x0500_0001, 1, ground.into(), 99).unwrap()
    );
    for flags in [0x0200_0000, 0x0020_0000, 0x0000_0200, 0x0800_0000] {
        let pose = OnFootPose {
            flags,
            ..airborne_pose()
        };
        assert!(encode_unset_can_fly_on_foot_ack(1, 1, pose, 99).is_err());
    }
    let pose = OnFootPose {
        position: [f32::NAN, 0.0, 0.0],
        ..airborne_pose()
    };
    assert!(encode_unset_can_fly_on_foot_ack(1, 1, pose, 99).is_err());
}

// ── Mover orders ──

fn order_body(opcode: u16, tail: &[u8]) -> Vec<u8> {
    let _ = opcode;
    let mut b = vec![0x09, 0x01, 0x05];
    b.extend(0x1122_3344u32.to_le_bytes());
    b.extend(tail);
    b
}

fn pose() -> OnFootPose {
    OnFootPose {
        flags: 0,
        position: [1.0, 2.0, 3.0],
        orientation: 0.5,
        fall_time: 0,
        fall: None,
        pitch: None,
        transport: None,
    }
}

const GUID: u64 = 0x0000_0000_0500_0001;

#[test]
fn speed_orders_read_the_run_byte_and_ack_one_opcode_up_with_the_exact_speed() {
    for (opcode, kind) in [
        (SMSG_FORCE_WALK_SPEED_CHANGE, SpeedOrderKind::Walk),
        (SMSG_FORCE_RUN_BACK_SPEED_CHANGE, SpeedOrderKind::RunBack),
        (SMSG_FORCE_SWIM_SPEED_CHANGE, SpeedOrderKind::Swim),
        (SMSG_FORCE_SWIM_BACK_SPEED_CHANGE, SpeedOrderKind::SwimBack),
        (SMSG_FORCE_TURN_RATE_CHANGE, SpeedOrderKind::TurnRate),
        (SMSG_FORCE_FLIGHT_SPEED_CHANGE, SpeedOrderKind::Flight),
        (
            SMSG_FORCE_FLIGHT_BACK_SPEED_CHANGE,
            SpeedOrderKind::FlightBack,
        ),
        (SMSG_FORCE_PITCH_RATE_CHANGE, SpeedOrderKind::PitchRate),
    ] {
        let body = order_body(opcode, &7.5f32.to_le_bytes());
        assert_eq!(
            decode_mover_order(opcode, &body).unwrap(),
            MoverOrder::Speed {
                kind,
                mover_guid: GUID,
                counter: 0x1122_3344,
                speed: 7.5
            }
        );
        assert_eq!(kind.ack_opcode(), u32::from(opcode) + 1);
        assert!(decode_mover_order(opcode, &body[..body.len() - 1]).is_err());
        assert!(decode_mover_order(opcode, &[body.as_slice(), &[0]].concat()).is_err());
    }
    // Run alone carries AC's u8 0 before the speed.
    let body = order_body(SMSG_FORCE_RUN_SPEED_CHANGE, &[0, 0, 0, 0xe0, 0x40]);
    assert!(matches!(
        decode_mover_order(SMSG_FORCE_RUN_SPEED_CHANGE, &body).unwrap(),
        MoverOrder::Speed { kind: SpeedOrderKind::Run, speed, .. } if speed == 7.0
    ));
    assert!(decode_mover_order(
        SMSG_FORCE_RUN_SPEED_CHANGE,
        &order_body(0, &7.0f32.to_le_bytes())
    )
    .is_err());
    let nan = order_body(SMSG_FORCE_WALK_SPEED_CHANGE, &f32::NAN.to_le_bytes());
    assert!(decode_mover_order(SMSG_FORCE_WALK_SPEED_CHANGE, &nan).is_err());

    let ack = encode_value_ack(GUID, 0x1122_3344, pose(), 0xaabb_ccdd, 7.0).unwrap();
    assert_eq!(
        ack,
        vec![
            0x09, 0x01, 0x05, // packed guid
            0x44, 0x33, 0x22, 0x11, // counter
            0, 0, 0, 0, // flags
            0, 0, // flags2
            0xdd, 0xcc, 0xbb, 0xaa, // timestamp
            0, 0, 0x80, 0x3f, 0, 0, 0, 0x40, 0, 0, 0x40, 0x40, 0, 0, 0, 0x3f, // xyzo
            0, 0, 0, 0, // fall time
            0, 0, 0xe0, 0x40, // the ordered speed
        ]
    );
}

#[test]
fn mode_orders_set_or_clear_their_flag_and_gravity_has_no_applied_word() {
    for (opcode, kind, apply) in [
        (SMSG_MOVE_WATER_WALK, ModeOrderKind::WaterWalk, true),
        (SMSG_MOVE_LAND_WALK, ModeOrderKind::WaterWalk, false),
        (SMSG_MOVE_FEATHER_FALL, ModeOrderKind::FeatherFall, true),
        (SMSG_MOVE_NORMAL_FALL, ModeOrderKind::FeatherFall, false),
        (SMSG_MOVE_SET_HOVER, ModeOrderKind::Hover, true),
        (SMSG_MOVE_UNSET_HOVER, ModeOrderKind::Hover, false),
        (SMSG_MOVE_SET_CAN_FLY, ModeOrderKind::CanFly, true),
        (
            SMSG_MOVE_GRAVITY_DISABLE,
            ModeOrderKind::DisableGravity,
            true,
        ),
        (
            SMSG_MOVE_GRAVITY_ENABLE,
            ModeOrderKind::DisableGravity,
            false,
        ),
    ] {
        let body = order_body(opcode, &[]);
        assert_eq!(
            decode_mover_order(opcode, &body).unwrap(),
            MoverOrder::Mode {
                kind,
                apply,
                mover_guid: GUID,
                counter: 0x1122_3344
            }
        );
        assert!(decode_mover_order(opcode, &[body.as_slice(), &[0]].concat()).is_err());
        let ack = encode_mode_ack(GUID, 1, pose(), 9, kind, apply).unwrap();
        let flags = u32::from_le_bytes(ack[7..11].try_into().unwrap());
        assert_eq!(flags & kind.flag() != 0, apply, "{kind:?}");
        let applied_word = kind != ModeOrderKind::DisableGravity;
        assert_eq!(
            ack.len(),
            3 + 4 + 30 + if applied_word { 4 } else { 0 },
            "{kind:?}"
        );
    }
    // Unset-can-fly keeps its own codec.
    assert!(!is_mover_order(SMSG_MOVE_UNSET_CAN_FLY));
    assert!(decode_mover_order(SMSG_MOVE_UNSET_CAN_FLY, &order_body(0, &[])).is_err());
    // A granted water walk stays on the pose; the revoke clears it.
    let walking = OnFootPose {
        flags: MOVEMENT_FLAG_WATER_WALKING,
        ..pose()
    };
    let land = encode_mode_ack(GUID, 1, walking, 9, ModeOrderKind::WaterWalk, false).unwrap();
    assert_eq!(u32::from_le_bytes(land[7..11].try_into().unwrap()), 0);
}

#[test]
fn knockback_acks_the_launch_frame_with_the_received_tail() {
    let mut tail = Vec::new();
    for f in [0.6f32, 0.8, 12.0, -9.0] {
        tail.extend(f.to_le_bytes());
    }
    let order = decode_mover_order(SMSG_MOVE_KNOCK_BACK, &order_body(0, &tail)).unwrap();
    assert_eq!(
        order,
        MoverOrder::KnockBack {
            mover_guid: GUID,
            counter: 0x1122_3344,
            cos_angle: 0.6,
            sin_angle: 0.8,
            horizontal_speed: 12.0,
            vertical_speed: -9.0,
        }
    );
    let ack = encode_knock_back_ack(GUID, 0x1122_3344, pose(), 5, &order).unwrap();
    let flags = u32::from_le_bytes(ack[7..11].try_into().unwrap());
    assert_eq!(flags, MOVEMENT_FLAG_FALLING);
    // fall time 0, then z-speed, sin, cos, horizontal speed as received.
    let tail_at = 3 + 4 + 4 + 2 + 4 + 16;
    assert_eq!(&ack[tail_at..tail_at + 4], &0u32.to_le_bytes());
    let floats: Vec<f32> = ack[tail_at + 4..]
        .chunks(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    // The order's down-positive -9 rises at 9 in the positive-up Wrath tail.
    assert_eq!(floats, [9.0, 0.8, 0.6, 12.0]);
}

#[test]
fn collision_height_and_near_teleport() {
    let order = decode_mover_order(
        SMSG_MOVE_SET_COLLISION_HGT,
        &order_body(0, &2.5f32.to_le_bytes()),
    )
    .unwrap();
    assert!(matches!(order, MoverOrder::CollisionHeight { height, .. } if height == 2.5));

    let movement = OutboundMovement {
        mover_guid: GUID,
        movement: MovementInfo {
            flags: 0,
            flags2: 0,
            timestamp: 1,
            position: Position {
                x: 10.0,
                y: 20.0,
                z: 30.0,
                orientation: 1.5,
            },
            transport: None,
            pitch: None,
            fall_time: 0,
            jump: None,
            spline_elevation: None,
        },
    }
    .to_bytes()
    .unwrap();
    let mut body = vec![0x09, 0x01, 0x05];
    body.extend(7u32.to_le_bytes());
    body.extend(&movement[3..]);
    assert_eq!(
        decode_mover_order(MSG_MOVE_TELEPORT_ACK, &body).unwrap(),
        MoverOrder::TeleportNear {
            mover_guid: GUID,
            counter: 7,
            position: [10.0, 20.0, 30.0],
            orientation: 1.5,
            transport: None,
        }
    );
    assert!(decode_mover_order(MSG_MOVE_TELEPORT_ACK, &[body.as_slice(), &[0]].concat()).is_err());
    assert_eq!(
        encode_teleport_ack(GUID, 7, 0xaabb_ccdd),
        vec![0x09, 0x01, 0x05, 7, 0, 0, 0, 0xdd, 0xcc, 0xbb, 0xaa]
    );
}
#[test]
fn near_teleport_decodes_authoritative_transport_and_rejects_bad_tail() {
    let mut body = vec![1, 7];
    body.extend(8u32.to_le_bytes());
    body.extend(0x200u32.to_le_bytes());
    body.extend(0u16.to_le_bytes());
    body.extend(123u32.to_le_bytes());
    for v in [500.0f32, 600.0, 700.0, 2.0] {
        body.extend(v.to_le_bytes());
    }
    body.extend([1, 42]);
    let local_at = body.len();
    for v in [1.0f32, 2.0, 3.0, 0.4] {
        body.extend(v.to_le_bytes());
    }
    body.extend(456u32.to_le_bytes());
    body.push(255);
    body.extend(0u32.to_le_bytes());
    assert_eq!(
        decode_mover_order(MSG_MOVE_TELEPORT_ACK, &body).unwrap(),
        MoverOrder::TeleportNear {
            mover_guid: 7,
            counter: 8,
            position: [500.0, 600.0, 700.0],
            orientation: 2.0,
            transport: Some(wrath_ground::TransportTail {
                guid: 42,
                position: [1.0, 2.0, 3.0],
                orientation: 0.4,
                time: 456
            }),
        }
    );
    assert!(decode_mover_order(MSG_MOVE_TELEPORT_ACK, &body[..body.len() - 1]).is_err());
    assert!(decode_mover_order(MSG_MOVE_TELEPORT_ACK, &[body.as_slice(), &[0]].concat()).is_err());
    body[local_at..local_at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(decode_mover_order(MSG_MOVE_TELEPORT_ACK, &body).is_err());
}

#[test]
fn a_flier_keeps_its_medium_under_root_and_lands_when_the_grant_goes() {
    use crate::wrath_ground::{MOVE_ASCENDING, MOVE_CAN_FLY, MOVE_FLYING, MOVE_FORWARD};
    let flier = OnFootPose {
        flags: MOVE_CAN_FLY | MOVE_FLYING | MOVE_ASCENDING | MOVE_FORWARD,
        position: [1.0, 2.0, 30.0],
        orientation: 0.5,
        fall_time: 0,
        fall: None,
        pitch: Some(0.4),
        transport: None,
    };
    let movement = |body: &[u8]| {
        // `packed guid, u32 counter, MovementInfo`: re-frame the info as an `OutboundMovement`.
        let mut rest = body;
        let _ = read_packed_guid(&mut rest).unwrap();
        let rest = &rest[4..];
        let mut framed = Vec::new();
        write_packed_guid(1, &mut framed).unwrap();
        framed.extend(rest);
        framed
    };
    // A rooted flier hangs in the air: FLYING, CAN_FLY and the pitch survive, the keys go.
    let halted = flier.halted();
    assert_eq!(halted.flags, MOVE_CAN_FLY | MOVE_FLYING);
    assert_eq!(halted.pitch, Some(0.4));
    let ack = encode_force_root_on_foot_ack(0x0500_0001, 3, halted, 9, true).unwrap();
    let info = OutboundMovement::from_bytes(&movement(&ack))
        .unwrap()
        .movement;
    assert_eq!(info.flags, MOVEMENT_FLAG_ROOT | MOVE_CAN_FLY | MOVE_FLYING);
    assert_eq!(info.pitch, Some(0.4));
    assert!(encode_force_root_on_foot_ack(0x0500_0001, 3, flier, 9, true).is_err());

    // Revoking the grant lands first (`0x6ef860` event 0x30), on either path.
    let revoke = encode_mode_ack(0x0500_0001, 4, flier, 9, ModeOrderKind::CanFly, false).unwrap();
    let info = OutboundMovement::from_bytes(&movement(&revoke[..revoke.len() - 4]))
        .unwrap()
        .movement;
    assert_eq!(info.flags, MOVE_FORWARD);
    assert_eq!(info.pitch, None);
    assert_eq!(revoke[revoke.len() - 4..], APPLIED_FALSE.to_le_bytes());
    let unset = encode_unset_can_fly_on_foot_ack(0x0500_0001, 4, flier, 9).unwrap();
    assert_eq!(unset, revoke);
    // The grant sets the bit on a walker.
    let walker = OnFootPose {
        flags: MOVE_FORWARD,
        pitch: None,
        ..flier
    };
    let grant = encode_mode_ack(0x0500_0001, 5, walker, 9, ModeOrderKind::CanFly, true).unwrap();
    let info = OutboundMovement::from_bytes(&movement(&grant[..grant.len() - 4]))
        .unwrap()
        .movement;
    assert_eq!(info.flags, MOVE_FORWARD | MOVE_CAN_FLY);

    // A knockback launches a flier from the landed pose.
    let knock = MoverOrder::KnockBack {
        mover_guid: 0x0500_0001,
        counter: 6,
        cos_angle: 1.0,
        sin_angle: 0.0,
        horizontal_speed: 5.0,
        vertical_speed: -7.0,
    };
    let body = encode_knock_back_ack(0x0500_0001, 6, flier, 9, &knock).unwrap();
    let info = OutboundMovement::from_bytes(&movement(&body))
        .unwrap()
        .movement;
    assert_eq!(
        info.flags,
        MOVE_CAN_FLY | MOVE_FORWARD | crate::wrath_ground::MOVE_FALLING
    );
    assert_eq!(info.pitch, None);
}
