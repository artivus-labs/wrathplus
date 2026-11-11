//! The movement model the wire and the world both read: the movement-flag word, the capsule and
//! step geometry, the swim predicates, landings and server-pushed kits, and the transport pose
//! components. Moved here for the crate split (`https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN`, step 0 item 3);
//! the owning modules re-export each item from its old path, and their systems stay with them.

use std::time::Instant;

use avian3d::prelude::Collider;
use bevy::prelude::*;
use wrathplus_assets::coords::{wow_rotation_to_bevy, wow_to_bevy};

/// The client's `MOVEMENTFLAGS` bits (cached at `unit+0x9e8`), tested as the binary tests them.
pub mod move_flags {
    pub const FORWARD: u32 = 0x1;
    pub const BACKWARD: u32 = 0x2;
    pub const STRAFE_LEFT: u32 = 0x4;
    pub const STRAFE_RIGHT: u32 = 0x8;
    /// The keyboard turn keys (with `TURN_RIGHT`), which rotate the facing (`0x7c4f30`).
    pub const TURN_LEFT: u32 = 0x10;
    pub const TURN_RIGHT: u32 = 0x20;
    /// The pitch keys while swimming or flying (the same bits in 1.12 and build 12340, whose
    /// `0x989220` sets one and clears the other).
    pub const PITCH_UP: u32 = 0x40;
    pub const PITCH_DOWN: u32 = 0x80;
    /// Walk mode: it scales only the rate numerator, never the id choice.
    pub const WALK_MODE: u32 = 0x100;
    /// `MOVEFLAG_LEVITATING`: the swim decision `0x6030c0` bails on it first (`0x6030d2`), so
    /// liquid can neither latch nor clear [`SWIMMING`]; GM flight sets both (`Player.cpp:4595`).
    pub const LEVITATING: u32 = 0x400;
    /// JUMPING/FALLING: set for the whole airborne arc.
    pub const FALLING: u32 = 0x2000;
    /// `MOVEFLAG_FALLINGFAR` (`0x633220`/`0x633240`): a jump latches it 1/9 yd below its launch
    /// height, a step-off fall after 500 ms airborne; only StopFalling (`0x7c6290`) clears it.
    pub const FALLING_FAR: u32 = 0x4000;
    /// `MOVEFLAG_ROOT`: vmangos kicks a root-apply ack without it unless the ack still falls
    /// (`MovementHandler.cpp:715-722`); a moving bit beside it freezes the 1.12 client.
    pub const ROOT: u32 = 0x1000;
    pub const SWIMMING: u32 = 0x20_0000;
    /// `MOVEFLAG_ONTRANSPORT` (`MovementInfo.h:56`, not TBC's 0x200): every packet carrying it
    /// also carries the transport-local pose.
    pub const ON_TRANSPORT: u32 = 0x0200_0000;
    /// `MOVEFLAG_WATERWALKING`: liquid is walkable ground; read by the liquid-mask selector
    /// `0x6315f0` (`0x63160d`, only when not swimming) and the opcode 0x22 apply `0x61a430`.
    pub const WATER_WALKING: u32 = 0x1000_0000;
    /// `MOVEFLAG_SAFE_FALL`, feather fall: its one effect is the terminal fall speed (`0x7c5d23`),
    /// 7.0 yd/s (`[0x87d898]`) instead of 60.148 (`[0x87d894]`).
    pub const SAFE_FALL: u32 = 0x2000_0000;
    /// `MOVEFLAG_HOVER`: the body rests 1.0 yd above the ground (Levitate): the walk resolver
    /// `0x6367b0` adds the `[0x7ff9d8]` offset and the step-down reach widens by it (`0x633e35`).
    pub const HOVER: u32 = 0x4000_0000;
    /// Build 12340's CAN_FLY (the same bit on the Wrath wire); 1.12 never sets it.
    pub const CAN_FLY: u32 = 0x0100_0000;
    /// Build 12340's ASCENDING and DESCENDING (the same bits on the Wrath wire), set by the jump and
    /// descend keys while swimming or flying (`0x9898e0`).
    pub const ASCENDING: u32 = 0x0040_0000;
    pub const DESCENDING: u32 = 0x0080_0000;
    /// Build 12340's FLYING, held in a bit of its own: the Wrath wire's `0x0200_0000` is the shared
    /// word's [`ON_TRANSPORT`], so `player::movement_net` and the Wrath relay map between them at
    /// the boundary. Inside [`SERVER_AUTHORED`]: a relayed flier's word carries it.
    pub const FLYING: u32 = 0x0800_0000;

    /// The bits a server-authored move packet owns, the reference's merge mask (`0x618c30` @
    /// `0x618deb`), with build 12340's [`FLYING`] and [`ASCENDING`] (a relayed flier's state). [`ON_TRANSPORT`] is outside it: a server pose relocates a rider but never
    /// boards or deboards them.
    pub const SERVER_AUTHORED: u32 = 0x75a0_7dff | FLYING | ASCENDING;

    /// Merge a server-authored packet's flags under [`SERVER_AUTHORED`] (`0x618de7`), for our mover
    /// and every watched one: all thirty relay opcodes take this mask (`0x618f20`).
    pub const fn merge_server_authored(local: u32, wire: u32) -> u32 {
        (local & !SERVER_AUTHORED) | (wire & SERVER_AUTHORED)
    }

    /// Any horizontal direction bit: the client's `[9e8] & 0xf` gate.
    pub const ANY_MOVE: u32 = FORWARD | BACKWARD | STRAFE_LEFT | STRAFE_RIGHT;

    /// The bits that get a mover integrated at all, the client's `0x20ff` (`0x616e20`, `0x6166f5`):
    /// without one a mover keeps its last packet's pose, and a flag-less unit is not even in the
    /// mover list (`0x618940`). Swim and the mode bits are not among them.
    pub const INTEGRATED: u32 = 0xff | FALLING;

    /// The committed-lower-body test routing a one-shot to the masked overlay (`0x5fe6dc`); the
    /// client's separate mouse-turn test (`d58 & 0x1800`) reads a field WrathPlus does not model.
    pub const ROUTE_COMMITTED_MOVE: u32 =
        FORWARD | BACKWARD | STRAFE_LEFT | STRAFE_RIGHT | TURN_LEFT | TURN_RIGHT | SWIMMING;

    /// The stationary-cast pin's moving test (`0x5fde80`): no turn bits, so a caster turning in
    /// place keeps the pin, where [`ROUTE_COMMITTED_MOVE`]'s turn bits would flap it.
    pub const CAST_PIN_MOVE: u32 = ANY_MOVE | SWIMMING;
}

/// The swim body pitch for every mover with a reported pitch (`0x60a110` to `0x710620`): the
/// reference's `Rz(facing)·Ry(2π − pitch)` is `Ry(yaw)·Rx(pitch)` here, nose up positive, and
/// only SWIMMING with forward or back pitches (`CMovement+0x40 & 3` at `0x60857d`).
pub fn swim_body_rotation(yaw: f32, flags: u32, pitch: f32) -> Quat {
    if flags & move_flags::SWIMMING != 0
        && flags & (move_flags::FORWARD | move_flags::BACKWARD) != 0
    {
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)
    } else {
        Quat::from_rotation_y(yaw)
    }
}

/// Wrap an angle to `(−π, π]`, the shortest arc.
pub fn wrap_pi(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    PI - (PI - angle).rem_euclid(TAU)
}

/// The hard-landing floor, `[0x80c414]` = 13.0 yd of fall height (`0x602d00`); below it only the
/// land sound plays. Below `[0x80c418]` = 70.0 yd a dead or ghost mover lands soft (`0x605f30`),
/// which is not built. The server damages only from 14.57 yd (vmangos `Player.cpp:20968`), so a
/// shorter hard landing grunts and puffs with no damage packet.
pub const HARD_LANDING_DESCENT: f32 = 13.0;

/// A mover's landing, reported on every landing; consumers apply [`HARD_LANDING_DESCENT`].
#[derive(Message, Clone, Copy)]
pub struct HardLanding {
    pub entity: Entity,
    /// Fall height in yd, launch height minus landing height. The reference's `0x7c60c0` measures
    /// from the apex, which a jump off a ledge puts up to about 1.6 yd above the launch.
    pub descent: f32,
}

/// A server-pushed kit (`SMSG_PLAY_SPELL_VISUAL`), played at stage 0 with spell id 0: its effects
/// end only on their own (`0x6e98d0` → `0x60edf0`).
#[derive(Message, Clone, Copy)]
pub struct KitPush {
    pub entity: Entity,
    pub kit_id: u32,
    /// [`crate::creature_anim::PlaySeq`] stamp at the wire drain, in packet order.
    pub seq: u64,
}

/// Capsule radius (yd), the `CMovement` ctor's placeholder 1/3. The reference overwrites it per
/// model from `CreatureModelData` (`0x6e9570`: human male 0.30555, female 0.20835); not built here.
pub const CAPSULE_RADIUS: f32 = 1.0 / 3.0;

/// Capsule total height (yd), one for every body: the swept shape and the head and feet offsets.
/// Numerically [`DEFAULT_COLLISION_HEIGHT`], but a unit's depth lines use its per-model
/// [`crate::entities::CollisionHeight`] instead.
pub const CAPSULE_HEIGHT: f32 = 2.027_777_7;

/// The `CMovement` ctor's collision height (`0x6ebd75` in `0x6ebd30`), before `0x6e9570` sets the
/// model's: the depth lines' height for a creature without the model-collision flag or model row.
pub const DEFAULT_COLLISION_HEIGHT: f32 = 2.027_777_7;

/// Terminal fall speed under feather fall (yd/s), the whole of Slow Fall: `0x7c5d20` and
/// `0x7c5f50` pick `[0x87d898]` over `[0x87d894]` while `MOVEFLAG_SAFE_FALL` is set (vmangos's
/// `terminalSavefallVelocity`).
pub const FEATHER_TERMINAL_VELOCITY: f32 = 7.0;

/// Hover's clearance (yd), the whole of Hover: the walk resolver probes `[0x7ff9d8]` further down
/// (`0x636dd2`) and writes the body's z a yard clear of the floor (`0x636e81`-`0x636ea9`).
pub const HOVER_HEIGHT: f32 = 1.0;

/// The step-up rise ceiling (yd), the reference's `H`: `0x617430` returns `CMovement+0xb8`, the
/// ratio `max(SCALE_X / CreatureModelScale, 1)`, which is 1.0 for the local player.
pub const STEP_UP_HEIGHT: f32 = 1.0;

/// The rise ceiling of a body the reference does not treat as player-controlled, `[0x801628]` on
/// `0x5fa550`'s false leg of `0x617430`: a creature, a pet, or a charmed, possessed, feared,
/// confused or rooted player.
pub const CREATURE_STEP_UP_HEIGHT: f32 = 2.0;

/// The player body's collision capsule, swept by avian's `MoveAndSlide`: origin at its centre, the
/// player's `pos` at its feet. Every player body sweeps it, remote ones too, as the reference runs
/// every mover through one controller (`0x616620`); the reference sizes each from its model.
#[derive(Resource)]
pub struct PlayerCapsule(pub Collider);

/// The share of the unit's collision height the water must cover to swim: 12340's swim decision
/// `0x730d10` compares the depth with `[+0x854] × 0.75` (`0x9e9ee4`), exit band 1/36 (`0xa3f854`).
const SWIM_DEPTH_FRAC: f32 = 0.75;

/// The `UNIT_FIELD_FLAGS` bits that let a unit swim, 12340's permit `0x4f53d0`, which `0x730d10`
/// reads on both legs: `PLAYER_CONTROLLED` (0x8, on every player), `RENAME` (0x10, a renamed pet),
/// `PET_IN_COMBAT` (0x800) and `SWIMMING` (0x8000, the creature's `CAN_SWIM`), any one of them,
/// unless [`CANNOT_SWIM`] is set. Without a permit a unit cannot enter and is stopped at any depth.
/// Labelled: `0x730d10` also requires a unit vfunc (`+0x40`) to answer 0, unread. Every body a
/// player can be [`super::session::Embodied`] in has 0x8, so only the creature marker
/// (`motion`'s `mark_swimming_creatures`) needs this.
pub const SWIM_UNIT_FLAGS: u32 = 0x8 | 0x10 | 0x800 | 0x8000;

/// `UNIT_FLAG_CANNOT_SWIM` (AzerothCore `UnitDefines.h`): `0x4f53d0` refuses before any permit bit.
pub const CANNOT_SWIM: u32 = 0x4000;

/// Whether a unit may swim at all ([`SWIM_UNIT_FLAGS`]): false blocks entry and forces an exit.
pub fn may_swim(unit_flags: u32) -> bool {
    unit_flags & CANNOT_SWIM == 0 && unit_flags & SWIM_UNIT_FLAGS != 0
}

/// The depth over the feet (yd) past which a unit swims: 0.75 × its own collision height
/// ([`crate::entities::CollisionHeight`]), about 1.52 yd for a human male and 0.91 for a gnome
/// female. Also the wade ceiling: wading is being in liquid without swimming.
pub fn swim_enter_depth(h: f32) -> f32 {
    SWIM_DEPTH_FRAC * h
}

/// The create-block anchor: the server's path-progress ms and the local instant it arrived,
/// inserted on every create that carries `UPDATE_FLAG_TRANSPORT`.
#[derive(Component)]
pub struct TransportAnchor {
    pub progress_ms: u32,
    pub at: Instant,
}

/// A pending type-11 lift, captured off the create block: its keyframe path is keyed by the
/// template entry, which `OBJECT_FIELD_ENTRY` carries, so no template query is needed.
#[derive(Component)]
pub struct ElevatorSeed {
    /// `gameobject_template` entry, `TransportAnimation.dbc`'s `TransportID` key.
    pub entry: u32,
    /// Spawn position (raw WoW coords), from the movement block: a transport create carries no
    /// `GAMEOBJECT_POS_*`.
    pub base_pos: [f32; 3],
    /// Spawn facing (WoW radians), the car's constant heading.
    pub yaw: f32,
    /// `GAMEOBJECT_PARENTROTATION`, the path quaternion, NOT the movement block's model
    /// rotation. `None` falls back to pure yaw for legacy seeds without descriptor data.
    pub quat: Option<[f32; 4]>,
}

/// An observed mover's pose in a transport's frame, the `MOVEFLAG_ONTRANSPORT` tail of its
/// `LIVING` create block or `MSG_MOVE_*` relay. `transport`'s `compose_riders` carries it
/// every frame; the local pose changes only per packet, with no extrapolation in the deck frame.
#[derive(Component)]
pub struct TransportRider {
    pub transport_guid: u64,
    /// Position in the transport's frame, raw WoW coords.
    pub local_pos: [f32; 3],
    /// Facing in the transport's frame (WoW radians); world facing is `local + transport yaw`.
    pub local_orientation: f32,
}

/// The rate (/s) the strafing yaw eases at: the reference closes a quarter of the gap per frame
/// (`0x607ed0`, `[0x8029b0]`), 17.3/s at 60 fps. Deviation: a time-based rate, because the
/// reference's per-frame step changes with the frame rate.
const STRAFE_BLEND_RATE: f32 = 17.26;

/// Ease the rendered yaw toward `aim + offset` through the aim-relative offset, which swings round
/// the front where the absolute shortest arc ties at the ±90° strafe flip.
pub fn ease_strafe_yaw(current_yaw: f32, aim: f32, offset: f32, dt: f32) -> f32 {
    let cur = wrap_pi(current_yaw - aim);
    let eased = cur + (offset - cur) * (1.0 - (-STRAFE_BLEND_RATE * dt).exp());
    wrap_pi(aim + eased)
}

/// The strafe body heading's offset from the aim (`0x607ed0`): ±π/2, ±π/4 with forward or back,
/// left positive, mirrored while backpedaling. Both strafe bits give 0, as they cancel to no
/// lateral motion, where the client's fold keys on the left bit alone: an unobservable edge.
pub fn strafe_body_offset(flags: u32) -> f32 {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    let left = flags & move_flags::STRAFE_LEFT != 0;
    let right = flags & move_flags::STRAFE_RIGHT != 0;
    if left == right {
        return 0.0;
    }
    let diagonal = flags & (move_flags::FORWARD | move_flags::BACKWARD) != 0;
    let magnitude = if diagonal { FRAC_PI_4 } else { FRAC_PI_2 };
    let back = flags & move_flags::BACKWARD != 0;
    if left != back {
        magnitude
    } else {
        -magnitude
    }
}

/// Gravity (yd/s²), the reference's and vmangos's `Movement::gravity`; also avian's `Gravity`, and
/// [`crate::net`] replays relayed jump arcs under it so an observer's arc matches the mover's.
pub const GRAVITY: f32 = 19.291_105;

/// Terminal fall speed (yd/s), `[0x87d894]` and vmangos's `terminalVelocity`; [`crate::net`]'s arc
/// replay caps at it too.
pub const TERMINAL_VELOCITY: f32 = 60.148_003;

/// The maps a transport's cycle lies on, the cross-map worldport's spare predicate: the wire
/// layer's despawn sweep keeps a transport whose path touches the destination. Inserted beside
/// `crate::transport::Transport` when it arms, so the sweep need not read the drive. A lift
/// carries no timetable and answers no: the server re-sends the destination's transports on
/// arrival (`SendInitTransports`), so a lift despawns and streams back like any object.
#[derive(Component, Clone)]
pub struct TransportMaps(pub Option<std::sync::Arc<wrathplus_formats::TransportTimetable>>);

impl TransportMaps {
    /// Whether any leg of the cycle lies on `map_id`.
    pub fn touches_map(&self, map_id: u32) -> bool {
        self.0.as_ref().is_some_and(|t| t.touches_map(map_id))
    }
}

/// The resets a Wrath `Connected` runs synchronously, before the new command endpoint is
/// published (the wire layer's `activate_command_endpoint`), so the session-end reader, which runs
/// later in `Input`, never gets a frame in which it can stream the old session's state to the new
/// socket. Each layer installs the reset of the state it owns; the wire layer owns none of it.
#[derive(Resource, Default)]
pub struct ConnectResets(pub Vec<fn(&mut World)>);

/// The Bevy rotation of a wire yaw (radians about WoW +Z): a Bevy +Y yaw with no 180° offset,
/// unlike doodad and WMO placement.
pub fn wire_yaw(orientation: f32) -> Quat {
    Quat::from_rotation_y(orientation)
}

/// The Bevy rotation of a GameObject placement: its `GAMEOBJECT_ROTATION` quaternion, which the
/// reference's render matrix and collision use (`0x5f7910`); `GAMEOBJECT_FACING` (`0x5f9fb0`) is
/// never consulted for placement.
///
/// vmangos fills `rotation2/3` from the facing when the spawn row leaves them zero
/// (`GameObject::UpdateRotationFields`), so a yaw-only row still sends `rot_z(o)`, and a row with
/// a tilt may send a non-unit quat, which `wow_rotation_to_bevy` normalizes. An absent or all-zero
/// quat (a create block folds absent fields to zero) falls back to the facing, since it would
/// normalize to `NaN`.
pub fn gameobject_rotation(quat: Option<[f32; 4]>, orientation: f32) -> Quat {
    match quat {
        Some(q) if q.iter().map(|c| c * c).sum::<f32>() > 0.5 => wow_rotation_to_bevy(q),
        _ => wire_yaw(orientation),
    }
}

/// A spawn [`Transform`] from a raw-WoW position and a resolved rotation ([`wire_yaw`] or
/// [`gameobject_rotation`]).
pub fn pose_transform(position: [f32; 3], rotation: Quat) -> Transform {
    Transform {
        translation: wow_to_bevy(position),
        rotation,
        ..default()
    }
}

/// Writes a raw-WoW pose onto an existing entity, keeping the renderer's baked scale; an entity
/// not yet query-visible on its spawn frame is skipped, as its spawn pose already placed it.
/// Whether it was written: `crate::motion`'s `write_pose` then drops the display smoother.
pub fn set_pose(
    transforms: &mut Query<&mut Transform>,
    e: Entity,
    position: [f32; 3],
    rotation: Quat,
) -> bool {
    let Ok(mut t) = transforms.get_mut(e) else {
        return false;
    };
    t.translation = wow_to_bevy(position);
    t.rotation = rotation;
    true
}
