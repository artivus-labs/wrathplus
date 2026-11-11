//! Strict projection of Wrath build-12340 movement relays into shared session events.
//!
//! AzerothCore's `WorldSession::WriteMovementInfo` writes a packed mover GUID followed by Wrath
//! `MovementInfo`. Admitted: on-foot, swimming (with its pitch), flight (pitch kept; Wrath FLYING is
//! the shared ON_TRANSPORT bit, so it becomes the shared word's own FLYING `0x0800_0000`, and
//! CAN_FLY, ASCENDING and DESCENDING keep their bits) and transports (Wrath
//! `0x200` becomes the shared `ON_TRANSPORT`, the tail the shared `TransportPose`). The mode,
//! knockback and collision-height echoes relay their `MovementInfo`; their extra words are framed
//! and dropped. Spline elevation and the pending-* bits stay fenced.

use crate::events::SessionEvent;
use crate::types::{JumpInfo, RelayVerb};
use crate::wrath_gameplay::{CodecError, OutboundMovement};

const MSG_MOVE_START_FORWARD: u16 = 0x00b5;
const MSG_MOVE_STOP_TURN: u16 = 0x00be;
const MSG_MOVE_SET_RUN_MODE: u16 = 0x00c2;
const MSG_MOVE_SET_WALK_MODE: u16 = 0x00c3;
/// An observed near-teleport (AzerothCore `Unit::SendTeleportPacket`, sent to the set without us).
const MSG_MOVE_TELEPORT: u16 = 0x00c5;
/// Pitch keys (start up, start down, stop), the swim edges and a pitch set: swimming relays.
const MSG_MOVE_START_PITCH_UP: u16 = 0x00bf;
const MSG_MOVE_STOP_PITCH: u16 = 0x00c1;
const MSG_MOVE_START_SWIM: u16 = 0x00ca;
const MSG_MOVE_STOP_SWIM: u16 = 0x00cb;
const MSG_MOVE_SET_PITCH: u16 = 0x00db;
const MSG_MOVE_FALL_LAND: u16 = 0x00c9;
const MSG_MOVE_SET_FACING: u16 = 0x00da;
const MSG_MOVE_ROOT: u16 = 0x00ec;
const MSG_MOVE_UNROOT: u16 = 0x00ed;
const MSG_MOVE_HEARTBEAT: u16 = 0x00ee;
/// Echoes AzerothCore sends to observers once a mover ACKs (MovementHandler.cpp, MiscHandler.cpp).
const MSG_MOVE_KNOCK_BACK: u16 = 0x00f1;
const MSG_MOVE_HOVER: u16 = 0x00f7;
const MSG_MOVE_FEATHER_FALL: u16 = 0x02b0;
const MSG_MOVE_WATER_WALK: u16 = 0x02b1;
const MSG_MOVE_UPDATE_CAN_FLY: u16 = 0x03ad;
const MSG_MOVE_GRAVITY_CHNG: u16 = 0x04d2;
use crate::wrath_authority::MSG_MOVE_SET_COLLISION_HGT;
/// Flight's vertical keys.
const MSG_MOVE_START_ASCEND: u16 = 0x0359;
const MSG_MOVE_STOP_ASCEND: u16 = 0x035a;
const MSG_MOVE_START_DESCEND: u16 = 0x03a7;
/// `packed guid, u32 lag`: not a pose (`HandleMoveTimeSkippedOpcode`).
const MSG_MOVE_TIME_SKIPPED: u16 = 0x0319;

const WRATH_ROOT: u32 = 0x0000_0800;
const WRATH_FALLING: u32 = 0x0000_1000;
const WRATH_FALLING_FAR: u32 = 0x0000_2000;
const SHARED_ROOT: u32 = 0x0000_1000;
const SHARED_FALLING: u32 = 0x0000_2000;
const SHARED_FALLING_FAR: u32 = 0x0000_4000;
const WRATH_ON_TRANSPORT: u32 = 0x0000_0200;
const SHARED_ON_TRANSPORT: u32 = 0x0200_0000;
/// Wrath `DISABLE_GRAVITY` is the shared `LEVITATING` (`0x400`) and `HOVER` (`0x4000_0000`) is the same bit.
const WRATH_SHARED_MODES: u32 = 0x0000_0400 | 0x4000_0000;
/// CAN_FLY, FLYING, ASCENDING, DESCENDING. Wrath FLYING moves to [`SHARED_FLYING`]; the others keep
/// their bits, which the shared word holds for build 12340 (`wrathplus-app` `move_flags`).
const WRATH_FLIGHT_ONLY: u32 = 0x0100_0000 | 0x0200_0000 | 0x0040_0000 | 0x0080_0000;
const WRATH_FLYING: u32 = 0x0200_0000;
const SHARED_FLYING: u32 = 0x0800_0000;
/// Water walking and slow fall (`0x1000_0000`, `0x2000_0000`), swimming (`0x0020_0000`, which carries
/// the pitch) and the pitch keys (`0x40`, `0x80`) have the same bits in the shared word.
const WRATH_ON_FOOT_FLAGS: u32 = 0x3020_01ff | WRATH_ROOT | WRATH_FALLING | WRATH_FALLING_FAR;
const WRATH_RELAY_FLAGS: u32 =
    WRATH_ON_FOOT_FLAGS | WRATH_ON_TRANSPORT | WRATH_SHARED_MODES | WRATH_FLIGHT_ONLY;

/// Whether `opcode` has a source-verified AzerothCore on-foot relay layout.
///
/// This is intentionally a whitelist rather than a movement-family range: pitch, swimming,
/// flight, transport changes, knockback, and later expansion movement opcodes need their own
/// semantics before they may reach the shared movement consumer.
pub const fn handles(opcode: u16) -> bool {
    matches!(
        opcode,
        MSG_MOVE_START_FORWARD
            ..=MSG_MOVE_STOP_TURN
                | MSG_MOVE_SET_RUN_MODE
                | MSG_MOVE_SET_WALK_MODE
                | MSG_MOVE_TELEPORT
                | MSG_MOVE_START_PITCH_UP..=MSG_MOVE_STOP_PITCH
                | MSG_MOVE_START_SWIM
                | MSG_MOVE_STOP_SWIM
                | MSG_MOVE_SET_PITCH
                | MSG_MOVE_FALL_LAND
                | MSG_MOVE_SET_FACING
                | MSG_MOVE_ROOT
                | MSG_MOVE_UNROOT
                | MSG_MOVE_HEARTBEAT
                | MSG_MOVE_KNOCK_BACK
                | MSG_MOVE_HOVER
                | MSG_MOVE_FEATHER_FALL
                | MSG_MOVE_WATER_WALK
                | MSG_MOVE_UPDATE_CAN_FLY
                | MSG_MOVE_GRAVITY_CHNG
                | MSG_MOVE_SET_COLLISION_HGT
                | MSG_MOVE_START_ASCEND
                | MSG_MOVE_STOP_ASCEND
                | MSG_MOVE_START_DESCEND
                | MSG_MOVE_TIME_SKIPPED
    )
}

/// Decode one already-framed Wrath movement relay.
///
/// AzerothCore writes the fall tail as positive-up `zspeed, sin, cos, xyspeed`; the shared
/// Vanilla event uses down-positive z-speed and orders its named fields as cos then sin.
pub fn decode(opcode: u16, body: &[u8]) -> Result<SessionEvent, CodecError> {
    if !handles(opcode) {
        return Err(CodecError::InvalidFlags(
            "unsupported Wrath movement relay opcode",
        ));
    }

    if opcode == MSG_MOVE_TIME_SKIPPED {
        let mut input = body;
        let guid = crate::wire::read_packed_guid(&mut input)
            .map_err(|_| CodecError::InvalidFlags("short time-skipped relay"))?;
        let [a, b, c, d] = <[u8; 4]>::try_from(input)
            .map_err(|_| CodecError::InvalidFlags("time-skipped relay is not guid + u32"))?;
        return Ok(SessionEvent::MoveTimeSkipped {
            guid,
            lag_ms: u32::from_le_bytes([a, b, c, d]),
        });
    }
    // The knockback echo appends `sin, cos, xy, z` (already in the fall tail) and the collision
    // echo the new height: framed and dropped.
    let extra = match opcode {
        MSG_MOVE_KNOCK_BACK => 16,
        MSG_MOVE_SET_COLLISION_HGT => 4,
        _ => 0,
    };
    let Some(split) = body.len().checked_sub(extra) else {
        return Err(CodecError::InvalidFlags("relay shorter than its echo tail"));
    };
    if body[split..]
        .chunks(4)
        .any(|w| !f32::from_le_bytes(w.try_into().expect("whole words")).is_finite())
    {
        return Err(CodecError::InvalidFlags("non-finite relay echo tail"));
    }
    let outbound = OutboundMovement::from_bytes(&body[..split])?;
    let movement = outbound.movement;
    // `flags2` (pitching permissions, interpolation) has no shared meaning; the codec already
    // checked its pairing with the transport `time2`.
    if movement.spline_elevation.is_some() {
        return Err(CodecError::InvalidFlags(
            "Wrath on-foot relay has spline elevation",
        ));
    }
    if movement.flags & !WRATH_RELAY_FLAGS != 0 {
        return Err(CodecError::InvalidFlags(
            "unsupported Wrath relay movement flag",
        ));
    }
    if movement.flags & WRATH_FALLING_FAR != 0 && movement.flags & WRATH_FALLING == 0 {
        return Err(CodecError::InvalidFlags(
            "Wrath FALLING_FAR without FALLING",
        ));
    }
    if ![
        movement.position.x,
        movement.position.y,
        movement.position.z,
        movement.position.orientation,
    ]
    .into_iter()
    .all(f32::is_finite)
    {
        return Err(CodecError::InvalidFlags(
            "non-finite Wrath on-foot movement pose",
        ));
    }
    if movement.jump.is_some_and(|jump| {
        ![
            jump.vertical_speed,
            jump.sin_angle,
            jump.cos_angle,
            jump.horizontal_speed,
        ]
        .into_iter()
        .all(f32::is_finite)
    }) {
        return Err(CodecError::InvalidFlags(
            "non-finite Wrath on-foot fall tail",
        ));
    }

    let mut flags = movement.flags
        & !(WRATH_ROOT | WRATH_FALLING | WRATH_FALLING_FAR | WRATH_ON_TRANSPORT | WRATH_FLYING);
    if movement.flags & WRATH_ON_TRANSPORT != 0 {
        flags |= SHARED_ON_TRANSPORT;
    }
    if movement.flags & WRATH_FLYING != 0 {
        flags |= SHARED_FLYING;
    }
    if movement.flags & WRATH_ROOT != 0 {
        flags |= SHARED_ROOT;
    }
    if movement.flags & WRATH_FALLING != 0 {
        flags |= SHARED_FALLING;
    }
    if movement.flags & WRATH_FALLING_FAR != 0 {
        flags |= SHARED_FALLING_FAR;
    }
    let jump = movement.jump.map(|jump| JumpInfo {
        zspeed: -jump.vertical_speed,
        cos_angle: jump.cos_angle,
        sin_angle: jump.sin_angle,
        xy_speed: jump.horizontal_speed,
    });

    Ok(SessionEvent::UnitMove {
        guid: outbound.mover_guid,
        position: [
            movement.position.x,
            movement.position.y,
            movement.position.z,
        ],
        orientation: movement.position.orientation,
        flags,
        // Present exactly while swimming or flying (the codec checks the flag/data pairing).
        pitch: movement.pitch.unwrap_or(0.0),
        time: movement.timestamp,
        verb: RelayVerb::of(opcode),
        fall_time: movement.fall_time,
        jump,
        transport: movement.transport.map(|t| crate::types::TransportPose {
            guid: t.guid,
            pos: crate::wire::Vector3d {
                x: t.position.x,
                y: t.position.y,
                z: t.position.z,
            },
            orientation: t.position.orientation,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrath_gameplay::{
        JumpInfo as WrathJumpInfo, MovementInfo, OutboundMovement, Position, TransportInfo,
        MOVEMENT_FLAG2_ALWAYS_ALLOW_PITCHING, MOVEMENT_FLAG_FLYING, MOVEMENT_FLAG_ON_TRANSPORT,
        MOVEMENT_FLAG_SPLINE_ELEVATION, MOVEMENT_FLAG_SWIMMING,
    };
    use crate::wrath_ground::{self, FallTail};

    const GUID: u64 = 0x0000_0000_0500_0001;

    fn event(opcode: u16, body: &[u8]) -> SessionEvent {
        decode(opcode, body).expect("relay decodes")
    }

    fn movement(flags: u32) -> OutboundMovement {
        OutboundMovement {
            mover_guid: GUID,
            movement: MovementInfo {
                flags,
                flags2: 0,
                timestamp: 77,
                position: Position {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    orientation: 0.5,
                },
                transport: None,
                pitch: None,
                fall_time: 0,
                jump: (flags & WRATH_FALLING != 0).then_some(WrathJumpInfo {
                    vertical_speed: 7.0,
                    sin_angle: 0.6,
                    cos_angle: 0.8,
                    horizontal_speed: 7.0,
                }),
                spline_elevation: None,
            },
        }
    }

    #[test]
    fn whitelist_is_exact_and_admits_swimming_but_not_flight_families() {
        for opcode in 0x00b5..=0x00be {
            assert!(handles(opcode), "{opcode:#05x}");
        }
        for opcode in [0x00bf, 0x00c0, 0x00c1, 0x00ca, 0x00cb, 0x00db] {
            assert!(handles(opcode), "{opcode:#05x}");
        }
        for opcode in [
            0x0359, 0x035a, 0x03a7, 0x00f1, 0x00f7, 0x02b0, 0x02b1, 0x03ad, 0x04d2, 0x0518, 0x0319,
        ] {
            assert!(handles(opcode), "{opcode:#05x}");
        }
        for opcode in [0x00b4, 0x00c4, 0x00ef, 0x038d] {
            assert!(!handles(opcode), "{opcode:#05x}");
        }
        for opcode in [0x00c2, 0x00c3, 0x00c9, 0x00da, 0x00ec, 0x00ed, 0x00ee] {
            assert!(handles(opcode), "{opcode:#05x}");
        }
    }

    #[test]
    fn standing_and_running_jumps_project_exact_shared_flags_and_signs() {
        for (wrath_flags, shared_flags, xy_speed) in [
            (WRATH_FALLING, SHARED_FALLING, 0.0),
            (
                wrath_ground::MOVE_FORWARD | WRATH_FALLING | WRATH_FALLING_FAR,
                wrath_ground::MOVE_FORWARD | SHARED_FALLING | SHARED_FALLING_FAR,
                7.0,
            ),
        ] {
            let body = wrath_ground::encode_on_foot(
                crate::wrath_ground::MSG_MOVE_JUMP,
                GUID,
                wrath_flags,
                0x1122_3344,
                [1.0, 2.0, 3.0],
                0.5,
                0,
                Some(FallTail {
                    vertical_speed: 7.0,
                    sin_angle: 0.6,
                    cos_angle: 0.8,
                    horizontal_speed: xy_speed,
                }),
            )
            .unwrap();
            let SessionEvent::UnitMove {
                guid,
                position,
                orientation,
                flags,
                time,
                fall_time,
                jump,
                ..
            } = event(crate::wrath_ground::MSG_MOVE_JUMP as u16, &body)
            else {
                unreachable!()
            };
            assert_eq!(guid, GUID);
            assert_eq!(position, [1.0, 2.0, 3.0]);
            assert_eq!(orientation, 0.5);
            assert_eq!(flags, shared_flags);
            assert_eq!(time, 0x1122_3344);
            assert_eq!(fall_time, 0);
            assert_eq!(
                jump,
                Some(JumpInfo {
                    zspeed: -7.0,
                    cos_angle: 0.8,
                    sin_angle: 0.6,
                    xy_speed,
                })
            );
        }
    }

    #[test]
    fn landing_and_heartbeat_round_trip_without_touching_vanilla_bits() {
        let landing = wrath_ground::encode_on_foot(
            u32::from(MSG_MOVE_FALL_LAND),
            GUID,
            wrath_ground::MOVE_FORWARD,
            10,
            [4.0, 5.0, 6.0],
            0.25,
            1_234,
            None,
        )
        .unwrap();
        let SessionEvent::UnitMove {
            flags,
            fall_time,
            jump,
            verb,
            ..
        } = event(MSG_MOVE_FALL_LAND, &landing)
        else {
            unreachable!()
        };
        assert_eq!(flags, wrath_ground::MOVE_FORWARD);
        assert_eq!(fall_time, 1_234);
        assert_eq!(jump, None);
        assert_eq!(verb, RelayVerb::Pose);

        let heartbeat = wrath_ground::encode(
            u32::from(MSG_MOVE_HEARTBEAT),
            GUID,
            wrath_ground::MOVE_FORWARD | wrath_ground::MOVE_WALKING,
            11,
            [7.0, 8.0, 9.0],
            1.0,
        )
        .unwrap();
        let SessionEvent::UnitMove { flags, verb, .. } = event(MSG_MOVE_HEARTBEAT, &heartbeat)
        else {
            unreachable!()
        };
        assert_eq!(
            flags,
            wrath_ground::MOVE_FORWARD | wrath_ground::MOVE_WALKING
        );
        assert_eq!(verb, RelayVerb::Heartbeat);
    }

    #[test]
    fn root_relays_preserve_the_wire_root_bit_instead_of_inventing_ack_state() {
        for (opcode, raw_flags, shared_flags, verb) in [
            (
                MSG_MOVE_ROOT,
                WRATH_ROOT,
                SHARED_ROOT,
                RelayVerb::Root(true),
            ),
            (MSG_MOVE_UNROOT, 0, 0, RelayVerb::Root(false)),
        ] {
            let body = movement(raw_flags).to_bytes().unwrap();
            let SessionEvent::UnitMove {
                flags, verb: got, ..
            } = event(opcode, &body)
            else {
                unreachable!()
            };
            assert_eq!(flags, shared_flags);
            assert_eq!(got, verb);
        }
    }

    #[test]
    fn transport_flags2_pitch_spline_and_other_modes_fail_closed() {
        let mut transport = movement(MOVEMENT_FLAG_ON_TRANSPORT);
        transport.movement.transport = Some(TransportInfo {
            guid: 2,
            position: Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                orientation: 0.0,
            },
            time: 0,
            seat: 0,
            time2: None,
        });
        let mut flags2 = movement(0);
        flags2.movement.flags2 = MOVEMENT_FLAG2_ALWAYS_ALLOW_PITCHING;
        flags2.movement.pitch = Some(0.25);
        let mut flying = movement(MOVEMENT_FLAG_FLYING);
        flying.movement.pitch = Some(0.25);
        let mut spline = movement(MOVEMENT_FLAG_SPLINE_ELEVATION);
        spline.movement.spline_elevation = Some(1.0);
        let falling_far_alone = movement(WRATH_FALLING_FAR);

        // Transports, flags2 and flight relay now (see the translation test); these stay fenced.
        let _ = (transport, flags2, flying);
        for packet in [spline, falling_far_alone] {
            let body = packet.to_bytes().unwrap();
            assert!(matches!(
                decode(MSG_MOVE_HEARTBEAT, &body),
                Err(CodecError::InvalidFlags(_))
            ));
        }
    }

    #[test]
    fn flight_and_transport_relays_translate_into_the_shared_word() {
        // A flyer: Wrath FLYING (the shared ON_TRANSPORT bit) becomes the shared FLYING
        // `0x0800_0000`; CAN_FLY and ASCENDING keep their bits; its pitch rides.
        let mut flying = movement(0x0100_0000 | MOVEMENT_FLAG_FLYING | 0x0040_0000);
        flying.movement.pitch = Some(0.4);
        let SessionEvent::UnitMove {
            flags,
            pitch,
            transport,
            ..
        } = event(MSG_MOVE_START_ASCEND, &flying.to_bytes().unwrap())
        else {
            panic!()
        };
        assert_eq!(
            (flags, pitch, transport),
            (0x0100_0000 | 0x0800_0000 | 0x0040_0000, 0.4, None)
        );
        // A passenger: Wrath ON_TRANSPORT (0x200) becomes the shared 0x0200_0000 with its deck pose.
        let mut riding = movement(MOVEMENT_FLAG_ON_TRANSPORT | wrath_ground::MOVE_FORWARD);
        riding.movement.transport = Some(TransportInfo {
            guid: 0x1FC0_0000_0000_0042,
            position: Position {
                x: 1.5,
                y: -2.0,
                z: 3.0,
                orientation: 0.25,
            },
            time: 9,
            seat: -1,
            time2: None,
        });
        let SessionEvent::UnitMove {
            flags, transport, ..
        } = event(MSG_MOVE_HEARTBEAT, &riding.to_bytes().unwrap())
        else {
            panic!()
        };
        assert_eq!(flags, 0x0200_0000 | wrath_ground::MOVE_FORWARD);
        let t = transport.unwrap();
        assert_eq!(
            (t.guid, t.pos.x, t.pos.y, t.pos.z, t.orientation),
            (0x1FC0_0000_0000_0042, 1.5, -2.0, 3.0, 0.25)
        );
        // Gravity off is the shared LEVITATING bit, hover the same bit.
        let SessionEvent::UnitMove { flags, .. } = event(
            MSG_MOVE_GRAVITY_CHNG,
            &movement(0x0400 | 0x4000_0000).to_bytes().unwrap(),
        ) else {
            panic!()
        };
        assert_eq!(flags, 0x0400 | 0x4000_0000);
    }

    #[test]
    fn echo_tails_and_time_skipped_frame_strictly() {
        let base = movement(0).to_bytes().unwrap();
        let knock = [base.as_slice(), &[0u8; 16]].concat();
        assert!(matches!(
            event(MSG_MOVE_KNOCK_BACK, &knock),
            SessionEvent::UnitMove { .. }
        ));
        assert!(decode(MSG_MOVE_KNOCK_BACK, &base).is_err());
        let height = [base.as_slice(), &2.0f32.to_le_bytes()].concat();
        assert!(matches!(
            event(MSG_MOVE_SET_COLLISION_HGT, &height),
            SessionEvent::UnitMove { .. }
        ));
        assert!(decode(
            MSG_MOVE_SET_COLLISION_HGT,
            &[height.as_slice(), &[0]].concat()
        )
        .is_err());
        let mut skipped = vec![0x01, 0x05];
        skipped.extend(250u32.to_le_bytes());
        assert!(matches!(
            event(MSG_MOVE_TIME_SKIPPED, &skipped),
            SessionEvent::MoveTimeSkipped {
                guid: 5,
                lag_ms: 250
            }
        ));
        assert!(decode(MSG_MOVE_TIME_SKIPPED, &skipped[..5]).is_err());
        assert!(decode(MSG_MOVE_TIME_SKIPPED, &[skipped.as_slice(), &[0]].concat()).is_err());
    }

    #[test]
    fn a_swimmer_relays_its_pitch_and_the_swim_and_pitch_opcodes_decode() {
        let mut swim = movement(MOVEMENT_FLAG_SWIMMING | wrath_ground::MOVE_FORWARD);
        swim.movement.pitch = Some(-0.25);
        let body = swim.to_bytes().unwrap();
        for opcode in [
            MSG_MOVE_HEARTBEAT,
            MSG_MOVE_START_SWIM,
            MSG_MOVE_SET_PITCH,
            0x00c0,
        ] {
            let SessionEvent::UnitMove { flags, pitch, .. } = event(opcode, &body) else {
                panic!()
            };
            assert_eq!(
                (flags, pitch),
                (MOVEMENT_FLAG_SWIMMING | wrath_ground::MOVE_FORWARD, -0.25),
                "{opcode:#05x}"
            );
        }
        // SWIMMING without its pitch is malformed.
        let bare = movement(MOVEMENT_FLAG_SWIMMING).to_bytes();
        assert!(bare.is_err() || decode(MSG_MOVE_HEARTBEAT, &bare.unwrap()).is_err());
    }

    #[test]
    fn trailing_and_every_truncated_prefix_fail_closed() {
        let body = movement(WRATH_FALLING).to_bytes().unwrap();
        for len in 0..body.len() {
            assert!(matches!(
                decode(MSG_MOVE_HEARTBEAT, &body[..len]),
                Err(CodecError::Truncated)
            ));
        }
        assert!(decode(MSG_MOVE_HEARTBEAT, &body).is_ok());

        let mut trailing = body;
        trailing.push(0);
        assert!(matches!(
            decode(MSG_MOVE_HEARTBEAT, &trailing),
            Err(CodecError::TrailingBytes(1))
        ));
    }
}
