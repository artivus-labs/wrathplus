//! Wrath build-12340 server movement-authority order codecs.
//!
//! AzerothCore sends authority orders as packed mover GUID plus order counter. Root ACKs carry that
//! prefix and a complete `MovementInfo`; unlike can-fly ACKs they have no trailing applied word.

use std::fmt;

use crate::wire::{read_packed_guid, write_packed_guid};
use crate::wrath_gameplay::{MovementInfo, OutboundMovement, Position};
use crate::wrath_ground::GROUND_FLAG_MASK;
use crate::wrath_ground::{self, FallTail};

pub const SMSG_MOVE_UNSET_CAN_FLY: u16 = 0x0344;
pub const CMSG_MOVE_SET_CAN_FLY_ACK: u32 = 0x0345;
pub const SMSG_FORCE_MOVE_ROOT: u16 = 0x00e8;
pub const CMSG_FORCE_MOVE_ROOT_ACK: u32 = 0x00e9;
pub const SMSG_FORCE_MOVE_UNROOT: u16 = 0x00ea;
pub const CMSG_FORCE_MOVE_UNROOT_ACK: u32 = 0x00eb;
pub const MOVEMENT_FLAG_ROOT: u32 = 0x0000_0800;
pub const APPLIED_FALSE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsetCanFlyOrder {
    pub mover_guid: u64,
    pub counter: u32,
}

pub type ForceRootOrder = UnsetCanFlyOrder;

/// What survives a root: the root itself, the medium a mover is in (with its tail) and its granted
/// modes. The turn keys go too: the halted pose is the parting STOP's.
const HALT_KEEPS: u32 = wrath_ground::MOVE_ROOT
    | wrath_ground::MOVE_SWIMMING
    | wrath_ground::MOVE_FLYING
    | wrath_ground::MOVE_CAN_FLY
    | wrath_ground::MOVE_ON_TRANSPORT
    | wrath_ground::MOVE_WATER_WALKING
    | wrath_ground::MOVE_FALLING_SLOW;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundPose {
    pub flags: u32,
    pub position: [f32; 3],
    pub orientation: f32,
}

/// The last successfully reported on-foot pose, including airborne tails. Authority ACKs must
/// not rewind an airborne mover to a cached takeoff/ground pose (AzerothCore processes ACK XYZO).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnFootPose {
    pub flags: u32,
    pub position: [f32; 3],
    pub orientation: f32,
    pub fall_time: u32,
    pub fall: Option<FallTail>,
    /// The pitch, present exactly while `MOVE_SWIMMING` or `MOVE_FLYING` is set.
    pub pitch: Option<f32>,
    /// The deck-local pose, present exactly while `MOVE_ON_TRANSPORT` is set.
    pub transport: Option<wrath_ground::TransportTail>,
}

impl From<GroundPose> for OnFootPose {
    fn from(pose: GroundPose) -> Self {
        Self {
            flags: pose.flags,
            position: pose.position,
            orientation: pose.orientation,
            fall_time: 0,
            fall: None,
            pitch: None,
            transport: None,
        }
    }
}

impl OnFootPose {
    pub fn from_movement(movement: &MovementInfo) -> Result<Self, AuthorityCodecError> {
        if movement.flags2 != 0
            || movement
                .transport
                .as_ref()
                .is_some_and(|t| t.time2.is_some())
            || movement.spline_elevation.is_some()
        {
            return Err(AuthorityCodecError::UnsupportedGroundState);
        }
        let pose = Self {
            flags: movement.flags,
            position: [
                movement.position.x,
                movement.position.y,
                movement.position.z,
            ],
            orientation: movement.position.orientation,
            fall_time: movement.fall_time,
            fall: movement.jump.map(|jump| FallTail {
                vertical_speed: jump.vertical_speed,
                sin_angle: jump.sin_angle,
                cos_angle: jump.cos_angle,
                horizontal_speed: jump.horizontal_speed,
            }),
            pitch: movement.pitch,
            transport: movement
                .transport
                .as_ref()
                .map(|t| wrath_ground::TransportTail {
                    guid: t.guid,
                    position: [t.position.x, t.position.y, t.position.z],
                    orientation: t.position.orientation,
                    time: t.time,
                }),
        };
        pose.validate()?;
        Ok(pose)
    }

    fn validate(self) -> Result<(), AuthorityCodecError> {
        self.encode(wrath_ground::MSG_MOVE_HEARTBEAT, 1, 0)
            .map(|_| ())
            .map_err(|_| AuthorityCodecError::UnsupportedGroundState)
    }

    /// A server root halts movement at the latest reported XYZ, not at an invented landing.
    /// ROOT cannot coexist with FALLING/FALLING_FAR or the ascend/descend keys in AzerothCore's
    /// moving-mask validation. A rooted swimmer stays in the water, a rooted flier in the air and a
    /// rooted passenger on its deck: SWIMMING, FLYING and ON_TRANSPORT survive with their tails, as
    /// do the granted modes (CAN_FLY, water walking, slow fall).
    pub fn halted(self) -> Self {
        Self {
            flags: self.flags & HALT_KEEPS,
            fall: None,
            ..self
        }
    }

    /// `Wow.exe` `0x98c240` (stop flying): FLYING, the ascend/descend keys and the pitch go; the
    /// mover keeps its position and falls from there on its next frame.
    pub fn landed(self) -> Self {
        if self.flags & wrath_ground::MOVE_FLYING == 0 {
            return self;
        }
        Self {
            flags: self.flags
                & !(wrath_ground::MOVE_FLYING
                    | wrath_ground::MOVE_ASCENDING
                    | wrath_ground::MOVE_DESCENDING),
            pitch: None,
            ..self
        }
    }

    /// This pose as `opcode`'s `packed guid + MovementInfo`.
    pub fn encode(
        self,
        opcode: u32,
        mover_guid: u64,
        timestamp: u32,
    ) -> Result<Vec<u8>, wrath_ground::GroundError> {
        wrath_ground::encode_full(
            opcode,
            mover_guid,
            self.flags,
            timestamp,
            self.position,
            self.orientation,
            self.transport,
            self.pitch,
            self.fall_time,
            self.fall,
        )
    }
}

fn encode_on_foot_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    rooted: Option<bool>,
) -> Result<Vec<u8>, AuthorityCodecError> {
    pose.validate()?;
    let mut encoded = pose
        .encode(wrath_ground::MSG_MOVE_HEARTBEAT, mover_guid, timestamp)
        .map_err(|_| AuthorityCodecError::UnsupportedGroundState)?;
    let mut prefix = encoded.as_slice();
    let _ = read_packed_guid(&mut prefix)?;
    let prefix_len = encoded.len() - prefix.len();
    if let Some(rooted) = rooted {
        // The caller supplies the explicitly halted pose for ROOT. Never discard an airborne
        // tail implicitly while encoding an ACK that only changes a movement-authority bit.
        if rooted {
            // A rooted swimmer, flier or passenger keeps SWIMMING / FLYING / ON_TRANSPORT (and their
            // tails) and its granted modes; nothing else may ride with ROOT.
            if pose.flags & !HALT_KEEPS != 0 || pose.fall.is_some() {
                return Err(AuthorityCodecError::UnsupportedGroundState);
            }
            let flags = MOVEMENT_FLAG_ROOT | pose.flags;
            encoded[prefix_len..prefix_len + 4].copy_from_slice(&flags.to_le_bytes());
        } else {
            // The unroot ACK reports the mover free.
            let flags = pose.flags & !MOVEMENT_FLAG_ROOT;
            encoded[prefix_len..prefix_len + 4].copy_from_slice(&flags.to_le_bytes());
        }
    }
    let mut body = Vec::new();
    write_packed_guid(mover_guid, &mut body)?;
    body.extend(counter.to_le_bytes());
    body.extend(&encoded[prefix_len..]);
    if rooted.is_none() {
        body.extend(APPLIED_FALSE.to_le_bytes());
    }
    Ok(body)
}

/// The unset-can-fly ACK lands a flier and clears CAN_FLY (`0x6ef860` event 0x30).
pub fn encode_unset_can_fly_on_foot_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let pose = pose.landed();
    let pose = OnFootPose {
        flags: pose.flags & !wrath_ground::MOVE_CAN_FLY,
        ..pose
    };
    encode_on_foot_ack(mover_guid, counter, pose, timestamp, None)
}

pub fn encode_force_root_on_foot_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    rooted: bool,
) -> Result<Vec<u8>, AuthorityCodecError> {
    encode_on_foot_ack(mover_guid, counter, pose, timestamp, Some(rooted))
}

#[derive(Debug)]
pub enum AuthorityCodecError {
    Io(std::io::Error),
    WrongLength(usize),
    UnsupportedGroundState,
    NonFinitePose,
    Movement(crate::wrath_gameplay::CodecError),
}

impl fmt::Display for AuthorityCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Wrath authority order: {error}"),
            Self::WrongLength(length) => {
                write!(f, "Wrath authority order has {length} trailing byte(s)")
            }
            Self::UnsupportedGroundState => {
                write!(f, "Wrath authority ACK requires an ordinary ground state")
            }
            Self::NonFinitePose => write!(f, "Wrath authority ACK pose is not finite"),
            Self::Movement(error) => write!(f, "Wrath authority movement codec: {error}"),
        }
    }
}

impl std::error::Error for AuthorityCodecError {}

impl From<std::io::Error> for AuthorityCodecError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<crate::wrath_gameplay::CodecError> for AuthorityCodecError {
    fn from(value: crate::wrath_gameplay::CodecError) -> Self {
        Self::Movement(value)
    }
}

fn decode_order(body: &[u8]) -> Result<UnsetCanFlyOrder, AuthorityCodecError> {
    let mut input = body;
    let mover_guid = read_packed_guid(&mut input)?;
    if input.len() < 4 {
        return Err(AuthorityCodecError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "missing order counter",
        )));
    }
    let counter = u32::from_le_bytes(input[..4].try_into().expect("four-byte slice"));
    input = &input[4..];
    if !input.is_empty() {
        return Err(AuthorityCodecError::WrongLength(input.len()));
    }
    Ok(UnsetCanFlyOrder {
        mover_guid,
        counter,
    })
}

pub fn decode_unset_can_fly(body: &[u8]) -> Result<UnsetCanFlyOrder, AuthorityCodecError> {
    decode_order(body)
}

pub fn decode_force_root(body: &[u8]) -> Result<ForceRootOrder, AuthorityCodecError> {
    decode_order(body)
}

impl GroundPose {
    pub fn from_movement(movement: &MovementInfo) -> Result<Self, AuthorityCodecError> {
        if movement.flags & !GROUND_FLAG_MASK != 0
            || movement.flags2 != 0
            || movement.transport.is_some()
            || movement.pitch.is_some()
            || movement.fall_time != 0
            || movement.jump.is_some()
            || movement.spline_elevation.is_some()
        {
            return Err(AuthorityCodecError::UnsupportedGroundState);
        }
        Self::new(
            movement.flags,
            [
                movement.position.x,
                movement.position.y,
                movement.position.z,
            ],
            movement.position.orientation,
        )
    }

    pub fn stationary(position: &Position) -> Result<Self, AuthorityCodecError> {
        Self::new(
            0,
            [position.x, position.y, position.z],
            position.orientation,
        )
    }

    pub fn new(
        flags: u32,
        position: [f32; 3],
        orientation: f32,
    ) -> Result<Self, AuthorityCodecError> {
        if flags & !GROUND_FLAG_MASK != 0 {
            return Err(AuthorityCodecError::UnsupportedGroundState);
        }
        if !position
            .into_iter()
            .chain([orientation])
            .all(f32::is_finite)
        {
            return Err(AuthorityCodecError::NonFinitePose);
        }
        Ok(Self {
            flags,
            position,
            orientation,
        })
    }
}

pub fn encode_unset_can_fly_ack(
    mover_guid: u64,
    counter: u32,
    pose: GroundPose,
    timestamp: u32,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let mut body = Vec::new();
    write_packed_guid(mover_guid, &mut body)?;
    body.extend(counter.to_le_bytes());
    let movement = OutboundMovement {
        mover_guid,
        movement: MovementInfo {
            flags: pose.flags,
            flags2: 0,
            timestamp,
            position: Position {
                x: pose.position[0],
                y: pose.position[1],
                z: pose.position[2],
                orientation: pose.orientation,
            },
            transport: None,
            pitch: None,
            fall_time: 0,
            jump: None,
            spline_elevation: None,
        },
    }
    .to_bytes()?;
    // The ACK already wrote its mover GUID before the counter; MovementInfo follows without the
    // OutboundMovement wrapper's second packed GUID.
    let mut movement_input = movement.as_slice();
    let _ = read_packed_guid(&mut movement_input)?;
    body.extend(movement_input);
    body.extend(APPLIED_FALSE.to_le_bytes());
    Ok(body)
}

/// Encode AzerothCore's `CMSG_FORCE_MOVE_{UN,}ROOT_ACK` body. This intentionally bypasses the
/// ordinary ground codec's ROOT rejection only for the server-directed root acknowledgement.
pub fn encode_force_root_ack(
    mover_guid: u64,
    counter: u32,
    pose: GroundPose,
    timestamp: u32,
    rooted: bool,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let mut body = Vec::new();
    write_packed_guid(mover_guid, &mut body)?;
    body.extend(counter.to_le_bytes());
    let movement = OutboundMovement {
        mover_guid,
        movement: MovementInfo {
            flags: if rooted { MOVEMENT_FLAG_ROOT } else { 0 },
            flags2: 0,
            timestamp,
            position: Position {
                x: pose.position[0],
                y: pose.position[1],
                z: pose.position[2],
                orientation: pose.orientation,
            },
            transport: None,
            pitch: None,
            fall_time: 0,
            jump: None,
            spline_elevation: None,
        },
    }
    .to_bytes()?;
    let mut movement_input = movement.as_slice();
    let _ = read_packed_guid(&mut movement_input)?;
    body.extend(movement_input);
    Ok(body)
}

// ── The remaining self orders: speeds, modes, knockback, collision height, near teleport ──
//
// AzerothCore `Unit::SendSpeedToController`, `SetWaterWalking`/`SetFeatherFall`/`SetHover`/
// `SetCanFly`/`SetDisableGravity`, `KnockbackFrom`, `SetCollisionHeight` (Unit.cpp) and
// `Player::SendTeleportAckPacket` (Player.cpp) all draw from the session's one order counter.
// The ACKs are read by `HandleForceSpeedChangeAck`, `HandleMoveFlagChangeOpcode` (MiscHandler.cpp),
// `HandleMoveKnockBackAck` and `HandleMoveTeleportAck` (MovementHandler.cpp).

pub const MSG_MOVE_TELEPORT_ACK: u16 = 0x00c7;
pub const SMSG_MOVE_WATER_WALK: u16 = 0x00de;
pub const SMSG_MOVE_LAND_WALK: u16 = 0x00df;
pub const SMSG_FORCE_RUN_SPEED_CHANGE: u16 = 0x00e2;
pub const SMSG_FORCE_RUN_BACK_SPEED_CHANGE: u16 = 0x00e4;
pub const SMSG_FORCE_SWIM_SPEED_CHANGE: u16 = 0x00e6;
pub const SMSG_MOVE_KNOCK_BACK: u16 = 0x00ef;
pub const CMSG_MOVE_KNOCK_BACK_ACK: u32 = 0x00f0;
pub const SMSG_MOVE_FEATHER_FALL: u16 = 0x00f2;
pub const SMSG_MOVE_NORMAL_FALL: u16 = 0x00f3;
pub const SMSG_MOVE_SET_HOVER: u16 = 0x00f4;
pub const SMSG_MOVE_UNSET_HOVER: u16 = 0x00f5;
pub const CMSG_MOVE_HOVER_ACK: u32 = 0x00f6;
pub const CMSG_MOVE_FEATHER_FALL_ACK: u32 = 0x02cf;
pub const CMSG_MOVE_WATER_WALK_ACK: u32 = 0x02d0;
pub const SMSG_FORCE_WALK_SPEED_CHANGE: u16 = 0x02da;
pub const SMSG_FORCE_SWIM_BACK_SPEED_CHANGE: u16 = 0x02dc;
pub const SMSG_FORCE_TURN_RATE_CHANGE: u16 = 0x02de;
pub const SMSG_MOVE_SET_CAN_FLY: u16 = 0x0343;
pub const SMSG_FORCE_FLIGHT_SPEED_CHANGE: u16 = 0x0381;
pub const SMSG_FORCE_FLIGHT_BACK_SPEED_CHANGE: u16 = 0x0383;
pub const SMSG_FORCE_PITCH_RATE_CHANGE: u16 = 0x045c;
pub const SMSG_MOVE_GRAVITY_DISABLE: u16 = 0x04ce;
pub const CMSG_MOVE_GRAVITY_DISABLE_ACK: u32 = 0x04cf;
pub const SMSG_MOVE_GRAVITY_ENABLE: u16 = 0x04d0;
pub const CMSG_MOVE_GRAVITY_ENABLE_ACK: u32 = 0x04d1;
pub const SMSG_MOVE_SET_COLLISION_HGT: u16 = 0x0516;
pub const CMSG_MOVE_SET_COLLISION_HGT_ACK: u32 = 0x0517;

pub const MOVEMENT_FLAG_DISABLE_GRAVITY: u32 = 0x0000_0400;
pub const MOVEMENT_FLAG_FALLING: u32 = wrath_ground::MOVE_FALLING;
pub const MOVEMENT_FLAG_CAN_FLY: u32 = 0x0100_0000;
pub const MOVEMENT_FLAG_WATER_WALKING: u32 = wrath_ground::MOVE_WATER_WALKING;
pub const MOVEMENT_FLAG_FALLING_SLOW: u32 = wrath_ground::MOVE_FALLING_SLOW;
pub const MOVEMENT_FLAG_HOVER: u32 = 0x4000_0000;
pub const APPLIED_TRUE: u32 = 1;

/// AzerothCore's `UnitMoveType`, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedOrderKind {
    Walk,
    Run,
    RunBack,
    Swim,
    SwimBack,
    TurnRate,
    Flight,
    FlightBack,
    PitchRate,
}

impl SpeedOrderKind {
    pub fn of(opcode: u16) -> Option<Self> {
        Some(match opcode {
            SMSG_FORCE_WALK_SPEED_CHANGE => Self::Walk,
            SMSG_FORCE_RUN_SPEED_CHANGE => Self::Run,
            SMSG_FORCE_RUN_BACK_SPEED_CHANGE => Self::RunBack,
            SMSG_FORCE_SWIM_SPEED_CHANGE => Self::Swim,
            SMSG_FORCE_SWIM_BACK_SPEED_CHANGE => Self::SwimBack,
            SMSG_FORCE_TURN_RATE_CHANGE => Self::TurnRate,
            SMSG_FORCE_FLIGHT_SPEED_CHANGE => Self::Flight,
            SMSG_FORCE_FLIGHT_BACK_SPEED_CHANGE => Self::FlightBack,
            SMSG_FORCE_PITCH_RATE_CHANGE => Self::PitchRate,
            _ => return None,
        })
    }

    /// The `CMSG_FORCE_*_CHANGE_ACK`, one above its order in every case.
    pub fn ack_opcode(self) -> u32 {
        u32::from(self.order_opcode()) + 1
    }

    pub fn order_opcode(self) -> u16 {
        match self {
            Self::Walk => SMSG_FORCE_WALK_SPEED_CHANGE,
            Self::Run => SMSG_FORCE_RUN_SPEED_CHANGE,
            Self::RunBack => SMSG_FORCE_RUN_BACK_SPEED_CHANGE,
            Self::Swim => SMSG_FORCE_SWIM_SPEED_CHANGE,
            Self::SwimBack => SMSG_FORCE_SWIM_BACK_SPEED_CHANGE,
            Self::TurnRate => SMSG_FORCE_TURN_RATE_CHANGE,
            Self::Flight => SMSG_FORCE_FLIGHT_SPEED_CHANGE,
            Self::FlightBack => SMSG_FORCE_FLIGHT_BACK_SPEED_CHANGE,
            Self::PitchRate => SMSG_FORCE_PITCH_RATE_CHANGE,
        }
    }
}

/// A server-granted movement mode and the flag its ACK reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeOrderKind {
    WaterWalk,
    FeatherFall,
    Hover,
    CanFly,
    /// Gravity disabled while applied.
    DisableGravity,
}

impl ModeOrderKind {
    /// The mode and whether the order applies it.
    pub fn of(opcode: u16) -> Option<(Self, bool)> {
        Some(match opcode {
            SMSG_MOVE_WATER_WALK => (Self::WaterWalk, true),
            SMSG_MOVE_LAND_WALK => (Self::WaterWalk, false),
            SMSG_MOVE_FEATHER_FALL => (Self::FeatherFall, true),
            SMSG_MOVE_NORMAL_FALL => (Self::FeatherFall, false),
            SMSG_MOVE_SET_HOVER => (Self::Hover, true),
            SMSG_MOVE_UNSET_HOVER => (Self::Hover, false),
            SMSG_MOVE_SET_CAN_FLY => (Self::CanFly, true),
            SMSG_MOVE_UNSET_CAN_FLY => (Self::CanFly, false),
            SMSG_MOVE_GRAVITY_DISABLE => (Self::DisableGravity, true),
            SMSG_MOVE_GRAVITY_ENABLE => (Self::DisableGravity, false),
            _ => return None,
        })
    }

    pub fn flag(self) -> u32 {
        match self {
            Self::WaterWalk => MOVEMENT_FLAG_WATER_WALKING,
            Self::FeatherFall => MOVEMENT_FLAG_FALLING_SLOW,
            Self::Hover => MOVEMENT_FLAG_HOVER,
            Self::CanFly => MOVEMENT_FLAG_CAN_FLY,
            Self::DisableGravity => MOVEMENT_FLAG_DISABLE_GRAVITY,
        }
    }

    pub fn ack_opcode(self, apply: bool) -> u32 {
        match self {
            Self::WaterWalk => CMSG_MOVE_WATER_WALK_ACK,
            Self::FeatherFall => CMSG_MOVE_FEATHER_FALL_ACK,
            Self::Hover => CMSG_MOVE_HOVER_ACK,
            Self::CanFly => CMSG_MOVE_SET_CAN_FLY_ACK,
            Self::DisableGravity if apply => CMSG_MOVE_GRAVITY_DISABLE_ACK,
            Self::DisableGravity => CMSG_MOVE_GRAVITY_ENABLE_ACK,
        }
    }
}

/// A self order beyond root and unset-can-fly. Every variant names the mover and the counter its
/// ACK must echo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoverOrder {
    /// `packed guid, u32 counter`, Run's `u8` (AC writes 0), `f32 speed`.
    Speed {
        kind: SpeedOrderKind,
        mover_guid: u64,
        counter: u32,
        speed: f32,
    },
    /// `packed guid, u32 counter`.
    Mode {
        kind: ModeOrderKind,
        apply: bool,
        mover_guid: u64,
        counter: u32,
    },
    /// `packed guid, u32 counter, f32 cos, f32 sin, f32 horizontal speed, f32 vertical speed`
    /// (AC writes `-speedZ`, so the field is down-positive like the 1.12 jump tail).
    KnockBack {
        mover_guid: u64,
        counter: u32,
        cos_angle: f32,
        sin_angle: f32,
        horizontal_speed: f32,
        vertical_speed: f32,
    },
    /// `packed guid, u32 counter, f32 height`.
    CollisionHeight {
        mover_guid: u64,
        counter: u32,
        height: f32,
    },
    /// `packed guid, u32 counter`, then the mover's `MovementInfo` at its destination.
    TeleportNear {
        mover_guid: u64,
        counter: u32,
        position: [f32; 3],
        orientation: f32,
        transport: Option<wrath_ground::TransportTail>,
    },
}

impl MoverOrder {
    pub fn mover_guid(&self) -> u64 {
        match *self {
            Self::Speed { mover_guid, .. }
            | Self::Mode { mover_guid, .. }
            | Self::KnockBack { mover_guid, .. }
            | Self::CollisionHeight { mover_guid, .. }
            | Self::TeleportNear { mover_guid, .. } => mover_guid,
        }
    }

    pub fn counter(&self) -> u32 {
        match *self {
            Self::Speed { counter, .. }
            | Self::Mode { counter, .. }
            | Self::KnockBack { counter, .. }
            | Self::CollisionHeight { counter, .. }
            | Self::TeleportNear { counter, .. } => counter,
        }
    }
}

/// Whether `opcode` is a [`MoverOrder`]. `SMSG_MOVE_UNSET_CAN_FLY` keeps its own codec above.
pub fn is_mover_order(opcode: u16) -> bool {
    opcode == MSG_MOVE_TELEPORT_ACK
        || opcode == SMSG_MOVE_KNOCK_BACK
        || opcode == SMSG_MOVE_SET_COLLISION_HGT
        || SpeedOrderKind::of(opcode).is_some()
        || (opcode != SMSG_MOVE_UNSET_CAN_FLY && ModeOrderKind::of(opcode).is_some())
}

fn take<const N: usize>(input: &mut &[u8]) -> Result<[u8; N], AuthorityCodecError> {
    if input.len() < N {
        return Err(AuthorityCodecError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "short Wrath movement order",
        )));
    }
    let (head, rest) = input.split_at(N);
    *input = rest;
    Ok(head.try_into().expect("N-byte slice"))
}

fn take_f32(input: &mut &[u8]) -> Result<f32, AuthorityCodecError> {
    let value = f32::from_le_bytes(take(input)?);
    if !value.is_finite() {
        return Err(AuthorityCodecError::NonFinitePose);
    }
    Ok(value)
}

/// Decode one [`is_mover_order`] opcode; a short, non-finite or over-long body fails.
pub fn decode_mover_order(opcode: u16, body: &[u8]) -> Result<MoverOrder, AuthorityCodecError> {
    let mut input = body;
    if opcode == MSG_MOVE_TELEPORT_ACK {
        let mover_guid = read_packed_guid(&mut input)?;
        let counter = u32::from_le_bytes(take(&mut input)?);
        // `OutboundMovement` is `packed guid + MovementInfo`; the counter sits between them here.
        let mut framed = Vec::with_capacity(9 + input.len());
        write_packed_guid(mover_guid, &mut framed)?;
        framed.extend(input);
        let movement = OutboundMovement::from_bytes(&framed)?.movement;
        let position = [
            movement.position.x,
            movement.position.y,
            movement.position.z,
        ];
        if !position
            .into_iter()
            .chain([movement.position.orientation])
            .all(f32::is_finite)
        {
            return Err(AuthorityCodecError::NonFinitePose);
        }
        if let Some(t) = &movement.transport {
            if t.guid == 0
                || ![
                    t.position.x,
                    t.position.y,
                    t.position.z,
                    t.position.orientation,
                ]
                .into_iter()
                .all(f32::is_finite)
            {
                return Err(AuthorityCodecError::NonFinitePose);
            }
        }
        return Ok(MoverOrder::TeleportNear {
            mover_guid,
            counter,
            position,
            orientation: movement.position.orientation,
            transport: movement.transport.map(|t| wrath_ground::TransportTail {
                guid: t.guid,
                position: [t.position.x, t.position.y, t.position.z],
                orientation: t.position.orientation,
                time: t.time,
            }),
        });
    }
    let mover_guid = read_packed_guid(&mut input)?;
    let counter = u32::from_le_bytes(take(&mut input)?);
    let order = if let Some(kind) = SpeedOrderKind::of(opcode) {
        if kind == SpeedOrderKind::Run {
            let _unknown = take::<1>(&mut input)?;
        }
        MoverOrder::Speed {
            kind,
            mover_guid,
            counter,
            speed: take_f32(&mut input)?,
        }
    } else if opcode == SMSG_MOVE_KNOCK_BACK {
        MoverOrder::KnockBack {
            mover_guid,
            counter,
            cos_angle: take_f32(&mut input)?,
            sin_angle: take_f32(&mut input)?,
            horizontal_speed: take_f32(&mut input)?,
            vertical_speed: take_f32(&mut input)?,
        }
    } else if opcode == SMSG_MOVE_SET_COLLISION_HGT {
        MoverOrder::CollisionHeight {
            mover_guid,
            counter,
            height: take_f32(&mut input)?,
        }
    } else if let Some((kind, apply)) =
        ModeOrderKind::of(opcode).filter(|_| opcode != SMSG_MOVE_UNSET_CAN_FLY)
    {
        MoverOrder::Mode {
            kind,
            apply,
            mover_guid,
            counter,
        }
    } else {
        return Err(AuthorityCodecError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a Wrath mover order",
        )));
    };
    if !input.is_empty() {
        return Err(AuthorityCodecError::WrongLength(input.len()));
    }
    Ok(order)
}

/// `packed guid, u32 counter` and the pose's `MovementInfo` with `extra` flags ORed in after the
/// on-foot validation (mode bits the ordinary encoder does not admit).
fn encode_mover_ack_prefix(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    extra: u32,
) -> Result<Vec<u8>, AuthorityCodecError> {
    pose.validate()?;
    let mut encoded = pose
        .encode(wrath_ground::MSG_MOVE_HEARTBEAT, mover_guid, timestamp)
        .map_err(|_| AuthorityCodecError::UnsupportedGroundState)?;
    let mut prefix = encoded.as_slice();
    let _ = read_packed_guid(&mut prefix)?;
    let prefix_len = encoded.len() - prefix.len();
    let flags = pose.flags | extra;
    encoded[prefix_len..prefix_len + 4].copy_from_slice(&flags.to_le_bytes());
    let mut body = Vec::new();
    write_packed_guid(mover_guid, &mut body)?;
    body.extend(counter.to_le_bytes());
    body.extend(&encoded[prefix_len..]);
    Ok(body)
}

/// `CMSG_FORCE_*_CHANGE_ACK` and `CMSG_MOVE_SET_COLLISION_HGT_ACK`: the pose, then the ordered
/// value verbatim (AzerothCore resets or kicks a mismatched speed).
pub fn encode_value_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    value: f32,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let mut body = encode_mover_ack_prefix(mover_guid, counter, pose, timestamp, 0)?;
    body.extend(value.to_le_bytes());
    Ok(body)
}

/// A mode ACK: the pose with the mode's flag set when applied and cleared when revoked, then
/// `u32 applied`, which the gravity ACKs do not carry.
pub fn encode_mode_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    kind: ModeOrderKind,
    apply: bool,
) -> Result<Vec<u8>, AuthorityCodecError> {
    // Revoking CAN_FLY lands a flier first (`0x6ef860` event 0x30: `0x98c240`, then the bit).
    let pose = if kind == ModeOrderKind::CanFly && !apply {
        pose.landed()
    } else {
        pose
    };
    let pose = OnFootPose {
        flags: pose.flags
            & !(kind.flag() & (wrath_ground::GROUND_FLAG_MASK | wrath_ground::MOVE_CAN_FLY)),
        ..pose
    };
    let extra = if apply { kind.flag() } else { 0 };
    let mut body = encode_mover_ack_prefix(mover_guid, counter, pose, timestamp, extra)?;
    if kind != ModeOrderKind::DisableGravity {
        body.extend(if apply { APPLIED_TRUE } else { APPLIED_FALSE }.to_le_bytes());
    }
    Ok(body)
}

/// `CMSG_MOVE_KNOCK_BACK_ACK`: the launch frame, the mover at its pose with `FALLING`, a zero fall
/// time and the order's tail. The order's vertical speed is down-positive (AzerothCore writes
/// `-speedZ`); the Wrath jump tail is positive-up (the controller negates at the command boundary,
/// `player::movement_net::jump_for_dialect`), so the ACK negates it.
pub fn encode_knock_back_ack(
    mover_guid: u64,
    counter: u32,
    pose: OnFootPose,
    timestamp: u32,
    order: &MoverOrder,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let MoverOrder::KnockBack {
        cos_angle,
        sin_angle,
        horizontal_speed,
        vertical_speed,
        ..
    } = *order
    else {
        return Err(AuthorityCodecError::UnsupportedGroundState);
    };
    let pose = pose.landed();
    let launch = OnFootPose {
        flags: (pose.flags
            & !(wrath_ground::MOVE_FALLING_FAR
                | wrath_ground::MOVE_SWIMMING
                | wrath_ground::MOVE_ROOT
                | wrath_ground::MOVE_PITCH_UP
                | wrath_ground::MOVE_PITCH_DOWN))
            | wrath_ground::MOVE_FALLING,
        pitch: None,
        fall_time: 0,
        fall: Some(FallTail {
            vertical_speed: -vertical_speed,
            sin_angle,
            cos_angle,
            horizontal_speed,
        }),
        ..pose
    };
    encode_mover_ack_prefix(mover_guid, counter, launch, timestamp, 0)
}

// ── Control hand-over and the controlled vehicle (`Wow.exe` `0x729010`, `0x6ef860` events
// 0x34/0x37; AzerothCore `HandleMoveNotActiveMover`, `HandleDismissControlledVehicle`,
// `HandleChangeSeatsOnControlledVehicle`) ──

pub const CMSG_MOVE_NOT_ACTIVE_MOVER: u32 = 0x02d1;
pub const CMSG_DISMISS_CONTROLLED_VEHICLE: u32 = 0x046d;
pub const CMSG_CHANGE_SEATS_ON_CONTROLLED_VEHICLE: u32 = 0x049b;

/// A request about the vehicle we drive, sent with its halted pose (`0x6ef860` events 0x34/0x37
/// halt the mover through `0x6e9980` first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VehicleControl {
    /// `CMSG_DISMISS_CONTROLLED_VEHICLE`: leave the vehicle we drive.
    Dismiss,
    /// `CMSG_CHANGE_SEATS_ON_CONTROLLED_VEHICLE`: `accessory` 0 with `seat`'s sign for the previous
    /// or next seat of this vehicle, else a seat on that accessory.
    ChangeSeats { accessory: u64, seat: i8 },
}

impl VehicleControl {
    pub fn opcode(self) -> u32 {
        match self {
            Self::Dismiss => CMSG_DISMISS_CONTROLLED_VEHICLE,
            Self::ChangeSeats { .. } => CMSG_CHANGE_SEATS_ON_CONTROLLED_VEHICLE,
        }
    }

    /// The body for the vehicle `vehicle_guid` at `pose`.
    pub fn encode(
        self,
        vehicle_guid: u64,
        pose: OnFootPose,
        timestamp: u32,
    ) -> Result<Vec<u8>, AuthorityCodecError> {
        match self {
            Self::Dismiss => encode_mover_pose(vehicle_guid, pose, timestamp),
            Self::ChangeSeats { accessory, seat } => {
                encode_change_seats(vehicle_guid, pose, timestamp, accessory, seat)
            }
        }
    }
}

/// `packed guid + MovementInfo`: the parting pose of the mover we stop driving
/// (`CMSG_MOVE_NOT_ACTIVE_MOVER`), or the driven vehicle's pose as we dismiss it
/// (`CMSG_DISMISS_CONTROLLED_VEHICLE`).
pub fn encode_mover_pose(
    mover_guid: u64,
    pose: OnFootPose,
    timestamp: u32,
) -> Result<Vec<u8>, AuthorityCodecError> {
    pose.encode(wrath_ground::MSG_MOVE_HEARTBEAT, mover_guid, timestamp)
        .map_err(|_| AuthorityCodecError::UnsupportedGroundState)
}

/// `CMSG_CHANGE_SEATS_ON_CONTROLLED_VEHICLE`: the vehicle's `packed guid + MovementInfo`, then the
/// target `packed guid` (zero for a seat on this vehicle, else the accessory) and `i8 seat`
/// (with a zero target AzerothCore moves to the previous or next seat by its sign).
pub fn encode_change_seats(
    vehicle_guid: u64,
    pose: OnFootPose,
    timestamp: u32,
    accessory: u64,
    seat: i8,
) -> Result<Vec<u8>, AuthorityCodecError> {
    let mut body = encode_mover_pose(vehicle_guid, pose, timestamp)?;
    write_packed_guid(accessory, &mut body)?;
    body.push(seat as u8);
    Ok(body)
}

/// `MSG_MOVE_TELEPORT_ACK` from us: `packed guid, u32 counter, u32 time`.
pub fn encode_teleport_ack(mover_guid: u64, counter: u32, timestamp: u32) -> Vec<u8> {
    let mut body = Vec::new();
    write_packed_guid(mover_guid, &mut body).expect("Vec write");
    body.extend(counter.to_le_bytes());
    body.extend(timestamp.to_le_bytes());
    body
}

#[cfg(test)]
mod tests;
