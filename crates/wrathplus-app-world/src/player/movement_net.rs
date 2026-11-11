//! Our own movement onto the wire: a `MSG_MOVE_*` per flag transition, the jump and the landing,
//! a heartbeat while moving and a `SET_FACING` each frame the facing changes. The wire must mirror
//! the avatar's actual motion: vmangos relays each packet verbatim and observers extrapolate from
//! its flags, so a flag left set is a phantom walk on every other screen.

use crate::app_core::movement::move_flags;
use crate::net::{ClientCommand, CommandSender, MoveKind};
use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_protocol::{JumpInfo, TransportPose};

use super::Player;

/// Physical movement outside the Wrath writer's deliberately bounded on-foot subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WrathGroundLimit {
    /// GM levitation and hover (Wrath DISABLE_GRAVITY / HOVER), which the writer does not report.
    Flight,
}

impl WrathGroundLimit {
    pub(super) const fn bit(self) -> u8 {
        match self {
            Self::Flight => 1 << 2,
        }
    }

    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::Flight => "levitation and hover movement",
        }
    }
}

/// The unsupported mode already present before input is integrated. Swimming and CAN_FLY flight are
/// supported: the Wrath writer reports SWIMMING or FLYING with its pitch
/// (`wrath_ground::encode_movement`).
pub(super) fn wrath_ground_limit(
    player: &Player,
    _liquid_surface: Option<f32>,
) -> Option<WrathGroundLimit> {
    if player.modes.levitating || player.modes.hover {
        return Some(WrathGroundLimit::Flight);
    }
    None
}

/// The `MSG_MOVE_HEARTBEAT` period (s) while moving: the reference arms its send deadline
/// `[mgr+0x130]` to now + 500 ms (`0x615b80`).
const HEARTBEAT_INTERVAL: f32 = 0.5;
/// The flag bits we send, each with its tail where it has one (`FALLING` the jump quad, `SWIMMING`
/// the pitch, `ON_TRANSPORT` the deck pose). `FALLING_FAR` changes no opcode; it rides the arc's
/// packets as the reference's live flags do.
const OUTBOUND_FLAG_MASK: u32 = move_flags::FORWARD
    | move_flags::BACKWARD
    | move_flags::STRAFE_LEFT
    | move_flags::STRAFE_RIGHT
    | move_flags::TURN_LEFT
    | move_flags::TURN_RIGHT
    | move_flags::WALK_MODE
    | move_flags::FALLING
    | move_flags::FALLING_FAR
    | move_flags::SWIMMING
    // The granted modes echo back: the reference builds its packets from the flags word the
    // server's merges land in. vmangos stores what we report, so a dropped `LEVITATING` returns
    // cleared on the next server-authored move and ends GM flight.
    | move_flags::LEVITATING
    // vmangos re-adds `ROOT` to a rooted mover's report (`MovementHandler.cpp:1064-1065`) and kicks
    // a root-apply ack without it (`MovementHandler.cpp:715-722`).
    | move_flags::ROOT
    | move_flags::WATER_WALKING
    | move_flags::SAFE_FALL
    | move_flags::HOVER
    | move_flags::ON_TRANSPORT
    // Build 12340 flight: the grant rides every packet like the other modes; FLYING carries the
    // pitch, and the vertical keys their edges.
    | move_flags::CAN_FLY
    | move_flags::FLYING
    | move_flags::ASCENDING
    | move_flags::DESCENDING
    | move_flags::PITCH_UP
    | move_flags::PITCH_DOWN;

/// The bits under which the position changes every frame and the move stream carries it; the
/// position reconcile runs only with none set. Turning is absent: a turn in place moves nothing.
const IN_MOTION: u32 = move_flags::ANY_MOVE
    | move_flags::FALLING
    | move_flags::FALLING_FAR
    | move_flags::SWIMMING
    | move_flags::FLYING
    | move_flags::ON_TRANSPORT;

/// This frame's airborne edges, as the send rules read them.
pub(super) struct ArcEdges {
    /// A take-off launched this frame.
    pub(super) jumped: bool,
    /// The take-off came from the server, a knockback or the jump a hover grant owes
    /// ([`super::Player::hover_launch`]), so it sends no `MSG_MOVE_JUMP`: that packet's one
    /// emission site is the move-command drain's jump arm (`0x615ed1`); the hover handler
    /// `0x61a620` sends nothing and the knockback arm sends its ack (`0xf0`).
    pub(super) wire_launch: bool,
    /// The standstill air nudge fired ([`super::mover::step`]): the one mid-air press that moves
    /// us, so the one that breaks the airborne silence.
    pub(super) air_nudged: bool,
    /// The arc ended this frame: `MSG_MOVE_FALL_LAND`.
    pub(super) landed: bool,
    /// Ms since take-off, the caller's snapshot: the landing frame has already cleared
    /// `airborne_since`, and vmangos deals fall damage only from a landing `fallTime` of 1229 ms
    /// (`Player.cpp:20954`).
    pub(super) fall_time: u32,
}

/// Streams this frame's movement as the reference does: a `MSG_MOVE_*` per axis transition (its
/// broadcaster `0x61a820` picks the opcode from the flag delta, `0x619f00`), a `SET_FACING` each
/// frame the facing changes off the turn axis, and a heartbeat every 500 ms while moving. Airborne,
/// the forward, back and strafe transitions go silent and their live bits ride the packets that do
/// go out. Sends are fire-and-forget; updates `player`'s last-sent flags, facing and heartbeat.
///
/// `CMSG_MOVE_TIME_SKIPPED` is the controller's, for a frame longer than the mover's 250 ms step
/// (`mover::capped_step`).
#[allow(clippy::too_many_arguments)]
pub(super) fn stream_self_movement(
    sender: &CommandSender,
    player: &mut Player,
    move_flags_now: u32,
    swim_pitch: f32,
    arc: ArcEdges,
    now: f32,
    speed_acks: &[crate::net::SpeedChangeMessage],
    knock_ack: Option<super::state::PendingKnockback>,
    transport: Option<TransportPose>,
) {
    let ArcEdges {
        jumped,
        wire_launch,
        air_nudged,
        landed,
        fall_time,
    } = arc;
    let wow_pos = bevy_to_wow(player.pos);
    // `face_yaw` is unbounded; the reference sends [0, 2π), and vmangos drops a packet with
    // `|o| > 4π` unrelayed (`VerifyMovementInfo`, `GridDefines.h:203`), a stop included.
    let facing = player.face_yaw.rem_euclid(std::f32::consts::TAU);
    let wire_flags = move_flags_now & OUTBOUND_FLAG_MASK;
    // The jump tail, on every airborne packet: the take-off vertical speed and the horizontal
    // velocity frozen at take-off, in world XY.
    let wire_jump = (wire_flags & move_flags::FALLING != 0).then(|| {
        let v = bevy_to_wow(player.horiz_vel); // WoW velocity [vx, vy, 0] (the transform is linear)
        let xy = v[0].hypot(v[1]);
        let (cos_angle, sin_angle) = if xy > 1.0e-4 {
            (v[0] / xy, v[1] / xy)
        } else {
            (facing.cos(), facing.sin()) // a standing jump has no direction: use the facing
        };
        JumpInfo {
            // Down-positive on the wire: the 1.12 client sends -7.955547 (`0xc0fe93d8`) rising.
            zspeed: -player.jump_zspeed,
            cos_angle,
            sin_angle,
            xy_speed: xy,
        }
    });
    // A forced speed change is acked by the writer itself (`net::io::wrath`'s `send_mover_ack`),
    // from the reported pose; the frame still counts as a position report, as it always has.
    if !speed_acks.is_empty() {
        player.last_pos = wow_pos;
    }

    // The knockback is acked by the writer itself, as a speed change is; the take-off frame still
    // counts as a position report.
    if knock_ack.is_some() {
        player.last_pos = wow_pos;
    }
    let prev = player.move_flags;
    let added = wire_flags & !prev;
    let removed = prev & !wire_flags;
    let mut sent = false;
    macro_rules! send_move {
        ($kind:expr) => {{
            if *crate::net::CAST_TRACE {
                bevy::log::info!(
                    "cast-trace: SEND move {:?} flags={:#x} pos=[{:.3},{:.3},{:.3}] o={:.3}",
                    $kind,
                    wire_flags,
                    wow_pos[0],
                    wow_pos[1],
                    wow_pos[2],
                    facing
                );
            }
            super::move_trace::sent($kind, wire_flags, facing, wow_pos);
            let _ = sender.send(ClientCommand::Move {
                kind: $kind,
                flags: to_wire_flags(wire_flags),
                pos: wow_pos,
                orientation: facing,
                // Written only while SWIMMING is set.
                pitch: swim_pitch,
                fall_time,
                jump: to_wire_jump(wire_jump),
                // Written only with ON_TRANSPORT: the rider's deck-local pose.
                transport,
            });
            sent = true;
            // vmangos relocates the mover to every one of these (`HandleMoverRelocation`).
            player.last_pos = wow_pos;
        }};
    }
    const FB: u32 = move_flags::FORWARD | move_flags::BACKWARD;
    const STRAFE: u32 = move_flags::STRAFE_LEFT | move_flags::STRAFE_RIGHT;
    const TURN: u32 = move_flags::TURN_LEFT | move_flags::TURN_RIGHT;
    // Airborne, the forward, back and strafe transitions are silent: while FALLING the reference's
    // `StartMove` (`0x7c6ae0`) defers a press into an inert latch (`0x20000`/`0x40000`), so the
    // broadcaster `0x61a820` sees no flag delta. It defers nothing with nothing moving, so the
    // standstill air nudge does send, carrying the re-seeded jump tail.
    let falling = wire_flags & move_flags::FALLING != 0;
    // The arc is JUMP at a keyboard take-off and FALL_LAND at the end, with no trailing Stop, as a
    // mid-air release only updated the flags. A jumpless fall opens with nothing: the broadcaster
    // sends only on a locomotion-nibble change (`0x61a99d` test al,0xf), and vmangos would read
    // the JUMPING bit as movement and stand a seated player up (`Unit.cpp:10398-10399`). A
    // server-driven take-off sends no JUMP: vmangos flags a JUMP faster than run speed as a cheat,
    // knockbacks included (`MovementAnticheat.cpp:650`).
    if jumped && !wire_launch {
        send_move!(MoveKind::Jump);
    } else if landed {
        send_move!(MoveKind::FallLand);
    }
    // `CMSG_MOVE_SET_FLY` (0x346) goes out the frame FLYING flips, taking off or landing (`0x6ef860`
    // events 0x2d/0x2e), ahead of the vertical edges the drain sends after it.
    if (added | removed) & move_flags::FLYING != 0 {
        send_move!(MoveKind::SetFly);
    }
    // The vertical keys while flying (events 6/7/8): `START_ASCEND` (0x359), `START_DESCEND` (0x3a7),
    // and `STOP_ASCEND` (0x35a) for either released while still flying. Landing clears them with
    // the SET_FLY above.
    const VERTICAL: u32 = move_flags::ASCENDING | move_flags::DESCENDING;
    if added & move_flags::ASCENDING != 0 {
        send_move!(MoveKind::StartAscend);
    } else if added & move_flags::DESCENDING != 0 {
        send_move!(MoveKind::StartDescend);
    } else if removed & VERTICAL != 0
        && wire_flags & VERTICAL == 0
        && wire_flags & (move_flags::FLYING | move_flags::SWIMMING) != 0
    {
        send_move!(MoveKind::StopAscend);
    }
    // The pitch keys while swimming or flying (events 0xe/0xf/0x10): `START_PITCH_UP` (0xbf),
    // `START_PITCH_DOWN` (0xc0), and `STOP_PITCH` (0xc1) for either released in the medium. Leaving
    // the medium clears them with its own edge.
    const PITCH_KEYS: u32 = move_flags::PITCH_UP | move_flags::PITCH_DOWN;
    if added & move_flags::PITCH_UP != 0 {
        send_move!(MoveKind::StartPitchUp);
    } else if added & move_flags::PITCH_DOWN != 0 {
        send_move!(MoveKind::StartPitchDown);
    } else if removed & PITCH_KEYS != 0
        && wire_flags & PITCH_KEYS == 0
        && wire_flags & (move_flags::SWIMMING | move_flags::FLYING) != 0
    {
        send_move!(MoveKind::StopPitch);
    }
    // `START_SWIM` (0xca) and `STOP_SWIM` (0xcb) go out the frame `SWIMMING` flips, enqueued by
    // the swim decision `0x730d10`; airborne and swimming never overlap, so the arc cannot race.
    if added & move_flags::SWIMMING != 0 {
        send_move!(MoveKind::StartSwim);
    } else if removed & move_flags::SWIMMING != 0 {
        send_move!(MoveKind::StopSwim);
    }
    // `SET_WALK_MODE` (0xc3) and `SET_RUN_MODE` (0xc2) go out the frame the bit flips, airborne or
    // standing still: the reference's `ToggleRun` enqueues its own event, since the broadcaster
    // gates on the locomotion nibble (`0x61a99d` test al,0xf) and would drop it.
    if added & move_flags::WALK_MODE != 0 {
        send_move!(MoveKind::SetWalkMode);
    } else if removed & move_flags::WALK_MODE != 0 {
        send_move!(MoveKind::SetRunMode);
    }
    // The stop arms stay silent while falling even on the nudge frame: a mid-air release is always
    // deferred, and this keeps one on the other axis from slipping out with the nudge.
    if !falling || air_nudged {
        if added & move_flags::FORWARD != 0 {
            send_move!(MoveKind::StartForward);
        } else if added & move_flags::BACKWARD != 0 {
            send_move!(MoveKind::StartBackward);
        } else if !falling && removed & FB != 0 && wire_flags & FB == 0 {
            send_move!(MoveKind::Stop);
        }
        if added & move_flags::STRAFE_LEFT != 0 {
            send_move!(MoveKind::StartStrafeLeft);
        } else if added & move_flags::STRAFE_RIGHT != 0 {
            send_move!(MoveKind::StartStrafeRight);
        } else if !falling && removed & STRAFE != 0 && wire_flags & STRAFE == 0 {
            send_move!(MoveKind::StopStrafe);
        }
    }
    // The turn axis sends airborne too: turning works mid-air.
    if added & move_flags::TURN_LEFT != 0 {
        send_move!(MoveKind::StartTurnLeft);
    } else if added & move_flags::TURN_RIGHT != 0 {
        send_move!(MoveKind::StartTurnRight);
    } else if removed & TURN != 0 && wire_flags & TURN == 0 {
        send_move!(MoveKind::StopTurn);
    }
    // One `SET_FACING` per frame the facing changed, moving or not, with no rate limit: the
    // reference drains each facing set into a send (`0x615fc6` push `0xda`). None while a turn bit
    // is set, which already rotates the mover for observers, and not gated on `sent`: the facing
    // report and the broadcaster are independent emitters.
    if wire_flags & TURN == 0 && facing != player.last_facing {
        send_move!(MoveKind::SetFacing);
    }
    // Boarding or leaving a deck has no opcode of its own, so a heartbeat carries the flip that
    // frame if nothing else went out.
    if !sent && (added | removed) & move_flags::ON_TRANSPORT != 0 {
        send_move!(MoveKind::Heartbeat);
    }
    // The heartbeat runs while any sent flag is set; the reference's (`0x616620`) runs only while
    // `flags & 0x200f`, the directions and FALLING (`0x616725`). Every send re-arms its deadline
    // (`0x600a30` → `0x615b80`), hence the stamp on any send below.
    // A standing CAN_FLY grant is no motion: a flying mount on the ground sends nothing extra.
    if !sent
        && wire_flags & !move_flags::CAN_FLY != 0
        && now - player.last_heartbeat >= HEARTBEAT_INTERVAL
    {
        send_move!(MoveKind::Heartbeat);
    }
    // At rest, a position change goes out the frame it happens: the resolver settles a resting
    // body by fractions of a millimetre after the packet that reported it, and vmangos compares
    // positions exactly (`Player.cpp:6092`), so a stale copy arrives with the next packet as
    // movement and interrupts the cast in flight (`Unit.cpp:10387`).
    if !sent && wire_flags & IN_MOTION == 0 && wow_pos != player.last_pos {
        send_move!(MoveKind::Heartbeat);
    }
    if sent {
        player.last_heartbeat = now;
    }
    // The facing compare is against the previous frame's facing, sent or not, so a keyboard turn
    // leaves no catch-up `SET_FACING` behind.
    player.last_facing = facing;
    player.move_flags = wire_flags;
}

fn to_wire_flags(flags: u32) -> u32 {
    // The shared controller keeps Vanilla's internal arc bits. Wrath 12340 moved FALLING and
    // FALLING_FAR down one bit; convert only at the command boundary.
    let mut wrath = flags
        & !(move_flags::FALLING
            | move_flags::FALLING_FAR
            | move_flags::ON_TRANSPORT
            | move_flags::FLYING
            | move_flags::ROOT);
    if flags & move_flags::FALLING != 0 {
        wrath |= wrathplus_protocol::wrath_ground::MOVE_FALLING;
    }
    if flags & move_flags::FALLING_FAR != 0 {
        wrath |= wrathplus_protocol::wrath_ground::MOVE_FALLING_FAR;
    }
    // The shared ON_TRANSPORT (`0x0200_0000`) is Wrath FLYING; Wrath ON_TRANSPORT is `0x200`.
    if flags & move_flags::ON_TRANSPORT != 0 {
        wrath |= wrathplus_protocol::wrath_ground::MOVE_ON_TRANSPORT;
    }
    // The shared ROOT (`0x1000`) is Wrath FALLING; Wrath ROOT is `0x800` (the writer sets it from the
    // root orders in any case).
    if flags & move_flags::ROOT != 0 {
        wrath |= wrathplus_protocol::wrath_ground::MOVE_ROOT;
    }
    // ...and the controller's own FLYING bit is Wrath FLYING. CAN_FLY, ASCENDING and DESCENDING
    // are already the Wrath bits.
    if flags & move_flags::FLYING != 0 {
        wrath |= wrathplus_protocol::wrath_ground::MOVE_FLYING;
    }
    wrath
}

/// Translate a Wrath on-foot echo back into the shared controller's established internal bits.
#[cfg(test)]
pub(super) fn from_wire_flags(flags: u32) -> u32 {
    let falling = flags & wrathplus_protocol::wrath_ground::MOVE_FALLING != 0;
    let falling_far = flags & wrathplus_protocol::wrath_ground::MOVE_FALLING_FAR != 0;
    let on_transport = flags & wrathplus_protocol::wrath_ground::MOVE_ON_TRANSPORT != 0;
    let mut shared = flags
        & !(wrathplus_protocol::wrath_ground::MOVE_FALLING
            | wrathplus_protocol::wrath_ground::MOVE_FALLING_FAR
            | wrathplus_protocol::wrath_ground::MOVE_ON_TRANSPORT);
    if on_transport {
        shared |= move_flags::ON_TRANSPORT;
    }
    if falling {
        shared |= move_flags::FALLING;
    }
    if falling_far {
        shared |= move_flags::FALLING_FAR;
    }
    shared
}

fn to_wire_jump(jump: Option<JumpInfo>) -> Option<JumpInfo> {
    // Build 12340 relays `zspeed` positive-up; the controller builds it down-positive, so the
    // command boundary reverses it.
    jump.map(|mut jump| {
        jump.zspeed = -jump.zspeed;
        jump
    })
}

/// Sends one flag-less `MSG_MOVE_STOP` when the controller stops driving the avatar (free-fly),
/// as nothing else would clear the flags observers extrapolate from; a no-op once stopped.
pub(super) fn park_mover(sender: &CommandSender, player: &mut Player) {
    if player.move_flags == 0 {
        return;
    }
    let facing = player.face_yaw.rem_euclid(std::f32::consts::TAU);
    let pos = bevy_to_wow(player.pos);
    player.last_pos = pos; // the park is a position report too
    super::move_trace::sent(MoveKind::Stop, 0, facing, pos);
    let _ = sender.send(ClientCommand::Move {
        kind: MoveKind::Stop,
        flags: 0,
        pos,
        orientation: facing,
        pitch: 0.0, // flags cleared → not swimming → no pitch tail written
        fall_time: 0,
        jump: None,
        transport: None, // flags cleared → no transport tail written
    });
    player.move_flags = 0;
    player.last_facing = facing;
}

/// Park only when the pose is ground movement: a flag-less ground STOP is never fabricated for
/// another mode.
pub(super) fn park_mover_on_ground(sender: &CommandSender, player: &mut Player) {
    if wrath_ground_limit(player, player.liquid_surface).is_none() {
        park_mover(sender, player);
    }
}

/// The rider's deck-local pose for the `ON_TRANSPORT` tail, or `None` off a deck. `bevy_to_wow` is
/// a pure rotation, so it converts the local offset too; the local facing is `face_yaw - boat_yaw`.
pub(super) fn wire_transport(player: &Player) -> Option<TransportPose> {
    player.ride.as_ref().map(|r| {
        let local = bevy_to_wow(r.local_pos);
        TransportPose {
            guid: r.guid,
            pos: wrathplus_protocol::wire::Vector3d {
                x: local[0],
                y: local[1],
                z: local[2],
            },
            orientation: (player.face_yaw - r.boat_yaw).rem_euclid(std::f32::consts::TAU),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::CommandSender;
    use std::f32::consts::TAU;

    #[test]
    fn wrath_abbey_dry_indoor_room_does_not_trip_the_swimming_gate() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::prelude::*;
        use wrathplus_assets::coords::wow_to_bevy;
        use wrathplus_world::liquid::LiquidClaim;
        use wrathplus_world::wmo_portal::{PlayerWmoRoom, WmoPortalInstance, WmoRoom};
        use wrathplus_world::world_point::{init_world_point_resources, Subject, WorldPoint};

        const ANCHOR: [f32; 3] = [-8910.12, -144.18, 81.97];
        let mut world = World::new();
        init_world_point_resources(&mut world);
        // NSABBEY groups 3/5: resolved Wrath liquidID 0, no MLIQ. Exercise the production
        // decoder and asset query projection from installed root flags 5 and raw zero/no-MLIQ.
        let chunk = |tag: &[u8; 4], payload: &[u8]| {
            let mut bytes = tag.to_vec();
            bytes.extend((payload.len() as u32).to_le_bytes());
            bytes.extend(payload);
            bytes
        };
        let mut mohd = vec![0; 64];
        mohd[60..64].copy_from_slice(&5u32.to_le_bytes());
        let root = wrathplus_formats::parse_wmo_root(&chunk(b"DHOM", &mohd)).unwrap();
        let native =
            wrathplus_formats::wmo_group_wrath_liquid(&chunk(b"PGOM", &[0; 68]), &root).unwrap();
        let state = wrathplus_assets::WmoLiquidState { native, kind: None };
        let placement = world
            .spawn(WmoPortalInstance {
                handle: Handle::default(),
                world_from_local: bevy::math::Affine3A::IDENTITY,
                name_set: 0,
                visible: vec![true; 14],
                interior_fog: vec![false; 14],
                liquid_visited: vec![false; 14],
                flooded: vec![state.flood_kind(); 14],
            })
            .id();
        for group in [3, 5] {
            let room = WmoRoom {
                instance: placement,
                group,
            };
            world.insert_resource(PlayerWmoRoom(Some(room)));
            world
                .run_system_once(move |point: WorldPoint| {
                    assert_eq!(
                        point.claim(Subject::Player),
                        LiquidClaim::Inside {
                            room,
                            flooded: None
                        }
                    );
                    let player = Player {
                        pos: wow_to_bevy(ANCHOR),
                        ..Default::default()
                    };
                    let surface = super::super::swim::surface_over_feet(&point, player.pos);
                    assert_eq!(surface, None, "Abbey's dry room has no swim surface");
                    assert_eq!(
                        wrath_ground_limit(&player, surface),
                        None,
                        "a dry indoor Abbey anchor must allow Wrath ground movement"
                    );
                })
                .unwrap();
        }
    }

    #[test]
    fn wrath_gate_refuses_unsupported_modes_without_changing_vanilla() {
        let player = Player::default();

        // Deep water and swimming are reported natively now, not held.
        let surface =
            player.pos.y + super::super::swim_enter_depth(player.collision_height.0) + 0.1;
        assert_eq!(wrath_ground_limit(&player, Some(surface)), None);
        let swimmer = Player {
            swimming: true,
            ..Player::default()
        };
        assert_eq!(wrath_ground_limit(&swimmer, Some(surface)), None);
        // A passenger on a deck is reported natively too (Wrath ON_TRANSPORT plus the tail).
        let rider = Player {
            move_flags: move_flags::ON_TRANSPORT,
            ..Player::default()
        };
        assert_eq!(wrath_ground_limit(&rider, None), None);
        assert_eq!(
            to_wire_flags(move_flags::ON_TRANSPORT | move_flags::FORWARD),
            wrathplus_protocol::wrath_ground::MOVE_ON_TRANSPORT | move_flags::FORWARD
        );
        assert_eq!(
            from_wire_flags(wrathplus_protocol::wrath_ground::MOVE_ON_TRANSPORT),
            move_flags::ON_TRANSPORT
        );
        let levitating = {
            let mut p = Player::default();
            p.modes.levitating = true;
            p
        };
        assert_eq!(
            wrath_ground_limit(&levitating, None),
            Some(WrathGroundLimit::Flight)
        );
    }

    #[test]
    fn wrath_converts_only_its_airborne_flags_and_vertical_sign() {
        let flags = move_flags::FORWARD | move_flags::FALLING | move_flags::FALLING_FAR;
        assert_eq!(
            to_wire_flags(flags),
            move_flags::FORWARD
                | wrathplus_protocol::wrath_ground::MOVE_FALLING
                | wrathplus_protocol::wrath_ground::MOVE_FALLING_FAR
        );
        let wrath = to_wire_flags(flags);
        assert_eq!(
            from_wire_flags(wrath),
            flags,
            "a self echo restores the controller's internal arc bits"
        );
        let jump = JumpInfo {
            zspeed: -7.955_547,
            cos_angle: 0.75,
            sin_angle: 0.5,
            xy_speed: 7.0,
        };
        assert_eq!(to_wire_jump(Some(jump)).unwrap().zspeed, 7.955_547);
    }

    #[test]
    fn wrath_jump_and_landing_reuse_the_ordinary_command_path() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            jump_zspeed: 7.955_547,
            horiz_vel: bevy::prelude::Vec3::new(0.0, 0.0, -7.0),
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: true,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            1.0,
            &[],
            None,
            None,
        );
        let ClientCommand::Move {
            kind,
            flags,
            fall_time,
            jump,
            ..
        } = rx.try_recv().expect("Wrath jump command")
        else {
            panic!("expected Move");
        };
        assert_eq!(kind, MoveKind::Jump);
        assert_eq!(
            kind.opcode(),
            wrathplus_protocol::wrath_ground::MSG_MOVE_JUMP as u16
        );
        assert_eq!(
            flags,
            move_flags::FORWARD | wrathplus_protocol::wrath_ground::MOVE_FALLING
        );
        assert_eq!(fall_time, 0);
        let jump = jump.expect("FALLING carries the Wrath tail");
        assert_eq!(jump.zspeed, 7.955_547);
        assert_eq!(jump.xy_speed, 7.0);

        while rx.try_recv().is_ok() {}
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: true,
                fall_time: 1_234,
            },
            2.0,
            &[],
            None,
            None,
        );
        let ClientCommand::Move {
            kind,
            flags,
            fall_time,
            jump,
            ..
        } = rx.try_recv().expect("Wrath landing command")
        else {
            panic!("expected Move");
        };
        assert_eq!(kind, MoveKind::FallLand);
        assert_eq!(
            kind.opcode(),
            wrathplus_protocol::wrath_ground::MSG_MOVE_FALL_LAND as u16
        );
        assert_eq!(flags, move_flags::FORWARD);
        assert_eq!(fall_time, 1_234);
        assert_eq!(jump, None);
    }

    #[test]
    fn wrath_flight_sends_set_fly_and_the_vertical_edges_as_valid_packets() {
        use wrathplus_protocol::wrath_ground as g;
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::CAN_FLY | move_flags::FALLING,
            ..Default::default()
        };
        let frame = |player: &mut Player, flags: u32, now: f32| {
            stream_self_movement(
                &tx,
                player,
                flags,
                0.25,
                ArcEdges {
                    jumped: false,
                    wire_launch: false,
                    air_nudged: false,
                    landed: false,
                    fall_time: 0,
                },
                now,
                &[],
                None,
                None,
            );
            rx.try_iter()
                .map(|command| {
                    let ClientCommand::Move {
                        kind, flags, pitch, ..
                    } = command
                    else {
                        panic!("expected Move, got {command:?}");
                    };
                    // Every packet passes the Wrath writer's validation.
                    let pitched = flags & (g::MOVE_SWIMMING | g::MOVE_FLYING) != 0;
                    g::encode_movement(
                        u32::from(kind.opcode()),
                        1,
                        flags,
                        0,
                        [0.0; 3],
                        0.0,
                        pitched.then_some(pitch),
                        0,
                        None,
                    )
                    .unwrap_or_else(|e| panic!("{kind:?} {flags:#x}: {e}"));
                    (kind, flags)
                })
                .collect::<Vec<_>>()
        };
        let flying = move_flags::CAN_FLY | move_flags::FLYING;
        // Take-off with the ascend key held: SET_FLY, then START_ASCEND.
        let sent = frame(&mut player, flying | move_flags::ASCENDING, 1.0);
        assert_eq!(
            sent,
            vec![
                (
                    MoveKind::SetFly,
                    g::MOVE_CAN_FLY | g::MOVE_FLYING | g::MOVE_ASCENDING
                ),
                (
                    MoveKind::StartAscend,
                    g::MOVE_CAN_FLY | g::MOVE_FLYING | g::MOVE_ASCENDING
                ),
            ]
        );
        assert_eq!(MoveKind::SetFly.opcode(), 0x0346);
        assert_eq!(MoveKind::StartAscend.opcode(), 0x0359);
        // Released: STOP_ASCEND; then descend.
        let sent = frame(&mut player, flying, 1.1);
        assert_eq!(
            sent,
            vec![(MoveKind::StopAscend, g::MOVE_CAN_FLY | g::MOVE_FLYING)]
        );
        let sent = frame(&mut player, flying | move_flags::DESCENDING, 1.2);
        assert_eq!(sent[0].0, MoveKind::StartDescend);
        assert_eq!(MoveKind::StartDescend.opcode(), 0x03a7);
        // Landing with the key still held: one SET_FLY, FLYING and the key cleared, no STOP_ASCEND.
        let sent = frame(&mut player, move_flags::CAN_FLY, 1.3);
        assert_eq!(sent, vec![(MoveKind::SetFly, g::MOVE_CAN_FLY)]);
    }

    #[test]
    fn the_walk_toggle_sends_its_own_opcode_with_no_movement_at_all() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player::default();
        let arc = || ArcEdges {
            jumped: false,
            wire_launch: false,
            air_nudged: false,
            landed: false,
            fall_time: 0,
        };
        // Entering walk: MSG_MOVE_SET_WALK_MODE (0xc3).
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::WALK_MODE,
            0.0,
            arc(),
            0.0,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, flags, .. } =
            rx.try_recv().expect("a standing walk toggle still sends")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::SetWalkMode);
        assert_eq!(kind.opcode(), 0x00C3);
        assert_eq!(flags & move_flags::WALK_MODE, move_flags::WALK_MODE);

        // Leaving it: MSG_MOVE_SET_RUN_MODE (0xc2).
        stream_self_movement(&tx, &mut player, 0, 0.0, arc(), 0.0, &[], None, None);
        let ClientCommand::Move { kind, flags, .. } =
            rx.try_recv().expect("the run half sends too")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::SetRunMode);
        assert_eq!(kind.opcode(), 0x00C2);
        assert_eq!(flags & move_flags::WALK_MODE, 0);

        stream_self_movement(
            &tx,
            &mut player,
            move_flags::WALK_MODE,
            0.0,
            arc(),
            0.0,
            &[],
            None,
            None,
        );
        while let Ok(ClientCommand::Move { kind, .. }) = rx.try_recv() {
            assert_eq!(kind, MoveKind::SetWalkMode, "the re-entry edge only");
        }
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::WALK_MODE,
            0.1,
            arc(),
            0.1,
            &[],
            None,
            None,
        );
        assert!(
            rx.try_recv().is_err(),
            "a walk already announced is silent until it flips back"
        );
    }

    #[test]
    fn a_walk_toggle_taken_mid_air_is_not_swallowed_by_the_airborne_silence() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD | move_flags::FALLING,
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::FALLING | move_flags::WALK_MODE,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 400,
            },
            0.0,
            &[],
            None,
            None,
        );
        let kinds: Vec<_> = rx
            .try_iter()
            .map(|c| match c {
                ClientCommand::Move { kind, .. } => kind,
                _ => panic!("expected Move commands"),
            })
            .collect();
        assert!(
            kinds.contains(&MoveKind::SetWalkMode),
            "the walk edge rides out mid-arc: {kinds:?}"
        );
    }

    #[test]
    fn wire_orientation_is_normalized_into_0_2pi() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            face_yaw: 100.0, // ~15.9 turns, far past vmangos's 4π bound
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.0,
            &[],
            None,
            None,
        );

        let ClientCommand::Move { orientation, .. } = rx
            .try_recv()
            .expect("a StartForward is sent on first FORWARD")
        else {
            panic!("expected a Move command");
        };
        assert!(
            (0.0..TAU).contains(&orientation),
            "orientation must be normalized into [0, 2π), got {orientation}"
        );
        assert!(
            (orientation - 100.0_f32.rem_euclid(TAU)).abs() < 1e-4,
            "the wrap preserves the angle (100 mod 2π): got {orientation}"
        );
    }

    #[test]
    fn fall_land_reports_the_accumulated_fall_time() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player::default(); // airborne_since already cleared, as on a landing frame
        stream_self_movement(
            &tx,
            &mut player,
            0,
            // grounded: no FALLING on the land packet itself
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: true,
                fall_time: 1700,
            },
            // 1.7 s of fall, past vmangos's 1229 ms fall-damage gate
            0.0,
            &[],
            None,
            None,
        );

        let ClientCommand::Move {
            kind, fall_time, ..
        } = rx.try_recv().expect("a FALL_LAND is sent on landing")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::FallLand);
        assert_eq!(
            fall_time, 1700,
            "the FALL_LAND carries the accumulated fall time, not a cleared clock"
        );
    }

    #[test]
    fn airborne_direction_release_is_silent_and_the_landing_sends_only_fall_land() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD | move_flags::FALLING, // last sent: the JUMP's flags
            ..Default::default()
        };
        // Mid-air, W released.
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 300,
            },
            0.2,
            &[],
            None,
            None,
        );
        assert!(rx.try_recv().is_err(), "a mid-air release sends nothing");
        assert_eq!(
            player.move_flags,
            move_flags::FALLING,
            "the flag state still updated silently"
        );
        // The landing frame, no keys held.
        stream_self_movement(
            &tx,
            &mut player,
            0,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: true,
                fall_time: 800,
            },
            0.8,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, flags, .. } = rx.try_recv().expect("the landing packet")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::FallLand);
        assert_eq!(flags, 0, "the FALL_LAND carries the live (released) flags");
        assert!(
            rx.try_recv().is_err(),
            "no trailing Stop after the FALL_LAND"
        );
    }

    #[test]
    fn a_jumpless_fall_opens_with_nothing_and_heartbeats_on_the_deadline() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        // Grounded last frame, airborne now with no jump: a walk-off.
        let mut player = Player::default();
        let step_off = |now: f32, player: &mut Player| {
            stream_self_movement(
                &tx,
                player,
                move_flags::FALLING,
                0.0,
                ArcEdges {
                    jumped: false,
                    wire_launch: false,
                    air_nudged: false,
                    landed: false,
                    fall_time: (now * 1000.0) as u32,
                },
                now,
                &[],
                None,
                None,
            );
        };

        step_off(0.1, &mut player);
        assert!(
            rx.try_recv().is_err(),
            "the arc's first frame puts NOTHING on the wire — a packet here un-seats the player"
        );
        assert_eq!(
            player.move_flags,
            move_flags::FALLING,
            "the flag state still updated silently, to ride the next packet that does go out"
        );

        step_off(0.4, &mut player);
        assert!(rx.try_recv().is_err(), "inside the deadline, still silent");

        step_off(0.7, &mut player);
        let ClientCommand::Move { kind, flags, .. } = rx
            .try_recv()
            .expect("past the deadline the fall heartbeats")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::Heartbeat);
        assert_ne!(
            flags & wrathplus_protocol::wrath_ground::MOVE_FALLING,
            0,
            "and it carries the arc, so observers learn of a long fall in progress"
        );
    }

    #[test]
    fn the_standstill_air_nudge_is_the_one_press_that_breaks_the_airborne_silence() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FALLING, // the standing jump: airborne, no direction
            // Post-nudge: the mover re-seeded it this frame (Bevy −Z = WoW +X, so |xy| = 2.5).
            horiz_vel: bevy::prelude::Vec3::new(0.0, 0.0, -2.5),
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: true,
                landed: false,
                fall_time: 300,
            },
            0.3,
            &[],
            None,
            None,
        );
        let ClientCommand::Move {
            kind, flags, jump, ..
        } = rx.try_recv().expect("the nudge broadcasts its transition")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::StartForward);
        assert_ne!(
            flags & wrathplus_protocol::wrath_ground::MOVE_FALLING,
            0,
            "it rides the arc"
        );
        let tail = jump.expect("an airborne packet carries the ballistic tail");
        assert!(
            (tail.xy_speed - 2.5).abs() < 1.0e-3,
            "the tail carries the RE-SEEDED horizontal — the whole point: an observer whose \
             arc says xy_speed = 0 learns the mover started moving, got {}",
            tail.xy_speed
        );
        assert!(rx.try_recv().is_err(), "one packet, not a burst");

        // A press mid-arc with momentum is deferred, so silent.
        let mut moving = Player {
            move_flags: move_flags::FORWARD | move_flags::FALLING,
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut moving,
            move_flags::FORWARD | move_flags::STRAFE_LEFT | move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 300,
            },
            0.3,
            &[],
            None,
            None,
        );
        assert!(
            rx.try_recv().is_err(),
            "a deferred mid-air press sends nothing"
        );
    }

    #[test]
    fn airborne_turn_transitions_and_facing_still_stream() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD | move_flags::FALLING,
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::TURN_LEFT | move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 200,
            },
            0.2,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, flags, .. } =
            rx.try_recv().expect("a mid-air turn broadcasts")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::StartTurnLeft);
        assert_ne!(
            flags & wrathplus_protocol::wrath_ground::MOVE_FALLING,
            0,
            "the packet rides the arc"
        );
        assert!(rx.try_recv().is_err(), "the turn axis carries the facing");
        // A mouse turn as the turn key releases, past the heartbeat deadline: the heartbeat is
        // `!sent`-gated, so only the StopTurn and the SET_FACING go out.
        player.face_yaw = 1.0;
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 900,
            },
            1.5,
            &[],
            None,
            None,
        );
        let kinds: Vec<_> = rx
            .try_iter()
            .map(|c| match c {
                ClientCommand::Move { kind, .. } => kind,
                _ => panic!("expected Move commands"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![MoveKind::StopTurn, MoveKind::SetFacing],
            "the turn closes, then the facing reports — the frame already sent, so no heartbeat"
        );
    }

    #[test]
    fn facing_streams_while_running() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD, // already running: no transition this frame
            ..Default::default()
        };
        player.face_yaw = 0.7;
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.05,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, flags, .. } = rx
            .try_recv()
            .expect("a mouse-turn while running reports its facing")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::SetFacing);
        assert_eq!(
            flags,
            move_flags::FORWARD,
            "the facing report carries the live direction flags"
        );
        assert!(
            rx.try_recv().is_err(),
            "exactly one packet per changed frame"
        );
        // The same facing, inside the heartbeat deadline.
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.06,
            &[],
            None,
            None,
        );
        assert!(rx.try_recv().is_err(), "an unchanged facing is silent");
    }

    #[test]
    fn the_turn_axis_carries_its_own_facing() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD | move_flags::TURN_RIGHT,
            ..Default::default()
        };
        for (i, yaw) in [0.3_f32, 0.6, 0.9].into_iter().enumerate() {
            player.face_yaw = yaw;
            stream_self_movement(
                &tx,
                &mut player,
                move_flags::FORWARD | move_flags::TURN_RIGHT,
                0.0,
                ArcEdges {
                    jumped: false,
                    wire_launch: false,
                    air_nudged: false,
                    landed: false,
                    fall_time: 0,
                },
                0.05 * (i as f32 + 1.0),
                &[],
                None,
                None,
            );
            assert!(
                rx.try_recv().is_err(),
                "the turn axis is silent (frame {i})"
            );
        }
        // Release the turn key, the facing unchanged since the last frame.
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.2,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, .. } = rx.try_recv().expect("the STOP_TURN") else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::StopTurn);
        assert!(
            rx.try_recv().is_err(),
            "no catch-up SET_FACING after a suppressed turn"
        );
    }

    #[test]
    fn a_transition_does_not_swallow_the_facing_report() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD,
            ..Default::default()
        };
        player.face_yaw = 1.2;
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FORWARD | move_flags::STRAFE_RIGHT,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.05,
            &[],
            None,
            None,
        );
        let kinds: Vec<_> = rx
            .try_iter()
            .map(|c| match c {
                ClientCommand::Move { kind, .. } => kind,
                _ => panic!("expected Move commands"),
            })
            .collect();
        assert_eq!(kinds, vec![MoveKind::StartStrafeRight, MoveKind::SetFacing]);
    }

    /// One idle frame at `pos`.
    fn idle_frame(tx: &CommandSender, player: &mut Player, pos: bevy::prelude::Vec3) {
        player.pos = pos;
        stream_self_movement(
            tx,
            player,
            player.move_flags,
            0.0,
            ArcEdges {
                jumped: false,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.0,
            &[],
            None,
            None,
        );
    }

    #[test]
    fn a_resting_body_that_drifts_reports_it_once() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player::default();
        idle_frame(&tx, &mut player, bevy::prelude::Vec3::new(1.0, 2.0, 3.0));
        let ClientCommand::Move {
            kind, flags, pos, ..
        } = rx.try_recv().expect("the drift is reported")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::Heartbeat);
        assert_eq!(flags, 0, "at rest — the reconcile invents no motion");
        assert_eq!(pos, bevy_to_wow(bevy::prelude::Vec3::new(1.0, 2.0, 3.0)));
        idle_frame(&tx, &mut player, bevy::prelude::Vec3::new(1.0, 2.0, 3.0));
        assert!(
            rx.try_recv().is_err(),
            "an unchanged resting position is silent — one packet per settle, not per frame"
        );
        // One ULP still counts: vmangos's compare has no epsilon.
        idle_frame(
            &tx,
            &mut player,
            bevy::prelude::Vec3::new(1.0, f32::from_bits(2.0f32.to_bits() - 1), 3.0),
        );
        assert!(
            rx.try_recv().is_ok(),
            "a one-ULP settle is exactly what the server's exact compare would read as movement"
        );
    }

    #[test]
    fn the_reconcile_never_fires_while_the_body_is_in_motion() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        for flags in [move_flags::FORWARD, move_flags::FALLING] {
            let mut player = Player {
                move_flags: flags, // already streaming this state: no transition this frame
                ..Default::default()
            };
            idle_frame(&tx, &mut player, bevy::prelude::Vec3::new(9.0, 9.0, 9.0));
            assert!(
                rx.try_recv().is_err(),
                "moving ({flags:#x}): the position rides the movement stream, not a reconcile"
            );
        }
    }

    #[test]
    fn park_mover_flushes_a_stop_and_clears_stale_flags() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            move_flags: move_flags::FORWARD,
            face_yaw: -3.0, // negative: must still go out normalized
            ..Default::default()
        };
        park_mover(&tx, &mut player);

        let ClientCommand::Move {
            flags, orientation, ..
        } = rx
            .try_recv()
            .expect("a Stop is flushed when flags were stale")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(flags, 0, "the parked Stop clears the move-flags");
        assert!(
            (0.0..TAU).contains(&orientation),
            "the parked facing is normalized into [0, 2π), got {orientation}"
        );
        assert_eq!(player.move_flags, 0, "bookkeeping is zeroed after parking");
    }

    #[test]
    fn a_knockback_sends_no_jump() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        // Mid-launch, the arc seeded from the quad below: Bevy −Z is WoW +X (north), so a 25 yd/s
        // push north is `horiz_vel.z = −25`, and `jump_zspeed` is up-positive.
        let launch = wrathplus_protocol::JumpInfo {
            zspeed: -12.0,
            cos_angle: 1.0,
            sin_angle: 0.0,
            xy_speed: 25.0,
        };
        let mut player = Player {
            horiz_vel: bevy::prelude::Vec3::new(0.0, 0.0, -25.0),
            jump_zspeed: 12.0,
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: true, // the arc opened; only `wire_launch` tells a knockback apart
                wire_launch: true,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.0,
            &[],
            Some(super::super::state::PendingKnockback {
                guid: 0x1234,
                counter: 7,
                launch,
            }),
            None,
        );

        // The writer acks the knockback itself; the movement stream only must not jump.
        for extra in rx.try_iter() {
            if let ClientCommand::Move { kind, .. } = extra {
                assert_ne!(
                    kind,
                    MoveKind::Jump,
                    "a knockback must not put MSG_MOVE_JUMP on the wire — instant \
                     CHEAT_TYPE_OVERSPEED_JUMP, and the reference's jump arm is a different drain arm"
                );
            }
        }
    }

    #[test]
    fn an_ordinary_jump_still_sends_its_jump() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player {
            jump_zspeed: 7.955547,
            ..Default::default()
        };
        stream_self_movement(
            &tx,
            &mut player,
            move_flags::FALLING,
            0.0,
            ArcEdges {
                jumped: true,
                wire_launch: false,
                air_nudged: false,
                landed: false,
                fall_time: 0,
            },
            0.0,
            &[],
            None,
            None,
        );
        let ClientCommand::Move { kind, jump, .. } =
            rx.try_recv().expect("a jump take-off is announced")
        else {
            panic!("expected a Move command");
        };
        assert_eq!(kind, MoveKind::Jump);
        let tail = jump.expect("the JUMP carries the ballistic tail");
        assert!(
            (tail.zspeed - 7.955547).abs() < 1.0e-4,
            "the build-12340 wire zspeed is up-positive: a rising jump reports positive, got {}",
            tail.zspeed
        );
    }

    #[test]
    fn park_mover_is_a_noop_once_already_stopped() {
        let (raw_tx, rx) = crossbeam_channel::unbounded();
        let tx = CommandSender::Raw(raw_tx);
        let mut player = Player::default(); // move_flags == 0
        park_mover(&tx, &mut player);
        assert!(
            rx.try_recv().is_err(),
            "no Stop is sent when we were already reported stopped"
        );
    }
}
