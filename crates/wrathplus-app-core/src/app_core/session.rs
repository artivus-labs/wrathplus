//! The net session's markers and edges that lower layers name: our own guid and entity, the body
//! we are attached to, the descriptor store every streamed object carries, the offline flag, the
//! world-enter and disconnect messages and the server's system lines. Moved here for the crate
//! split (`https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN`, step 0 residue); `crate::net` re-exports each from its
//! old path and keeps the systems that write them.

use bevy::prelude::*;
use wrathplus_protocol::ObjectFields;

/// This process has no IO thread (`net::NetPlugin::connect` false); `crate::preflight` announces
/// it.
/// Not `net::NetStatus`'s `connected`, which a dropped connection also clears.
#[derive(Resource)]
pub struct NetOffline;

/// A streamed object's descriptor fields, seeded by the create block and merged with each
/// `Values` delta. Speeds and pose are movement-block data, not here.
#[derive(Component, Clone, Default)]
pub struct ObjectStore(pub ObjectFields);

impl ObjectStore {
    /// The object manager's unit lookup (`0x468460`, `ecx = 8`): a unit or a player, which share
    /// the unit block; a game object, corpse or item is not one.
    pub fn is_unit(&self) -> bool {
        use wrathplus_protocol::types::ObjectType;
        matches!(
            self.0.object_type(),
            Some(ObjectType::Unit | ObjectType::Player)
        )
    }
}

/// Our own character's entity (guid == `net::SelfGuid`): identity, not what we steer
/// ([`Embodied`]).
#[derive(Component)]
pub struct SelfPlayer;

/// The body this client is attached to: the reference's camera anchor (`camera+0x88`), written
/// only by the constructor and `SetTarget 0x50d0f0`, never by a control update. Normally our own
/// body; under possession, the possessed unit.
///
/// At most one entity carries it, and a claimed mover not yet streamed is none: outbound
/// `MSG_MOVE_*` carry no guid, so a fallback to our body would write our pose onto the creature.
/// Whether it may move is `net::ActiveMover`; `player` owns the placement.
#[derive(Component)]
pub struct Embodied;

/// We entered the world.
#[derive(Message)]
pub struct EnteredWorldMessage {
    /// Rested billing minutes from `SMSG_AUTH_RESPONSE`, the only time they arrive.
    pub billing_time_rested: u32,
    /// The tutorial flags, if `SMSG_TUTORIAL_FLAGS` came during the login handshake.
    pub tutorial_flags: Option<Vec<u8>>,
}

/// The session ended; the IO thread is back at its pre-logon park.
#[derive(Message, Clone)]
pub struct DisconnectedMessage {
    pub reason: String,
    pub end: wrathplus_protocol::SessionEnd,
    /// The reference's `DISCONNECTED_FROM_SERVER`: back at the account screen, nothing retried.
    /// Decided once in [`Self::new`], so its four readers (teardown, screen, cover, credentials)
    /// cannot disagree.
    pub session_over: bool,
}

impl DisconnectedMessage {
    /// `session_over` is false after a logout (the relist is the select the player asked for) and
    /// in a run declared unattended ([`crate::run_mode::unattended`]), which reconnects. Not the
    /// login env vars: a person's launch line sets them too, and a reconnect would take the
    /// account back from whoever displaced us.
    pub fn new(reason: String, end: wrathplus_protocol::SessionEnd) -> Self {
        let session_over =
            end == wrathplus_protocol::SessionEnd::Lost && !crate::run_mode::unattended();
        Self {
            reason,
            end,
            session_over,
        }
    }
}

/// Our own player's guid, once in the world.
#[derive(Resource, Default)]
pub struct SelfGuid(pub Option<u64>);

/// One `CHAT_MSG_SYSTEM` line, such as a GM command's answer: the only evidence of server state
/// with no field, like vmangos god mode (`Unit::m_invincibilityHpThreshold`).
#[derive(Message, Clone)]
pub struct ServerSaidMessage {
    pub text: String,
}
