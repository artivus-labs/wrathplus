//! Animation markers and requests `net` writes from the wire: the ranged hold and nock latch, the
//! packet-order play stamp, the body twist, and the game-object custom animation and despawn
//! markers. `creature_anim` and `go_anim` keep their systems and re-export these from their old
//! paths.

use bevy::prelude::*;

/// The ranged weapon-visual hold, `[+0xd58] & 0x400`, set for any caster whose ranged visual
/// plays (`0x60d020`); cleared by a non-ranged visual (`0x6ec39e`), the local cancel, melee
/// attack-start or a spline move (`0x6020e8`), never by a sheath change or volley end. It gates
/// nothing here: the reference re-arms the Hold at each completion while it holds (`0x5fc3f0`,
/// via `0x7075af`), ours loops the Hold once armed. It is no entry into the drawn idle: read as
/// one, it leaves a shooter aiming after a single Serpent Sting.
#[derive(Component)]
pub struct RangedHold;

/// The nock latch `[+0xd58] & 0x4000`: set at `$BWP` (`0x624b70`, LoadBow ~0.6 s), cleared at
/// `$BWR` (`0x60016c`, AttackBow ~0.067 s) and on un-nock; the arrow model shows only while set.
#[derive(Component)]
pub struct NockLatch;

/// The reference's PlayAnimation order: handlers run in packet order and the later play wins
/// bone 0 (`0x5fe2f0`). WrathPlus's swing and cast pipelines are separate streams, so each
/// animation-bearing message carries a stamp (the wire in packet order, scene-time emitters when
/// they fire), and the driver resolves a same-frame collision by the highest.
#[derive(Resource, Default)]
pub struct PlaySeq(pub(crate) u64);

impl PlaySeq {
    pub fn advance(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
}

/// The twist state, on a unit's root when its model has either key bone (the client's
/// `[+0xd58] & 0x80`/`0x100`).
#[derive(Component)]
pub struct BodyTwist {
    /// `wrap(aim − rendered root yaw)` in radians; zero while the body faces its aim.
    pub yaw_gap: f32,
    pub spine: Option<Channel>,
    pub head: Option<Channel>,
}

impl BodyTwist {
    pub fn new(spine: Option<u16>, head: Option<u16>) -> Self {
        Self {
            yaw_gap: 0.0,
            spine: spine.map(Channel::new),
            head: head.map(Channel::new),
        }
    }
}

/// One twist channel's bone and bookkeeping.
pub struct Channel {
    pub bone: u16,
    /// The animated local rotation the twist last composed on.
    pub base: Quat,
    /// What we last wrote (`base * twist`). A bone still holding it was not re-keyed this frame, so
    /// `base` stays; composing onto it would accumulate the twist and spin the bone.
    pub last_out: Quat,
}

impl Channel {
    pub(crate) fn new(bone: u16) -> Self {
        Self {
            bone,
            base: Quat::IDENTITY,
            last_out: Quat::IDENTITY,
        }
    }
}

/// `SMSG_GAMEOBJECT_CUSTOM_ANIM`, bridged from the net layer; the fishing bobber's bite sends
/// `anim_id` 0 beside the forced state flip and the server's `SMSG_PLAY_OBJECT_SOUND`.
#[derive(Message, Clone, Copy)]
pub struct GoCustomAnim {
    pub go_guid: u64,
    pub anim_id: u32,
}

/// `SMSG_GAMEOBJECT_DESPAWN_ANIM` arrived: a component inserted at packet time, so it is on the
/// entity before the same tick's `SMSG_DESTROY_OBJECT` frees it. It asserts nothing alone:
/// `SendObjectDeSpawnAnim` also fires for totems, DynamicObjects and two boss scripts' living
/// objects, so it only arms a [`GoAnim`] and defers a destroy that arrives.
#[derive(Component)]
pub struct DespawnAnimAnnounced;

/// The reference's pending-destroy mark (`[GO+0xe4]` bit `0x10`), set by the destroy `0x464920` on
/// a pinned object. Arming a non-resting substate pins (`0x5f3b27 call 0x4683e0`, refcount
/// `[obj+0xe8]`), slot 14 unpins at the window end (`0x468410`), and the deferred destroy then runs
/// (`0x46844a call 0x464920`). WrathPlus pins only [`ANIM_DESPAWN`]; an object destroyed
/// mid-transition still goes at once (not built).
///
/// The reference keeps a pinned object addressable by guid while it plays; WrathPlus drops the guid
/// at destroy time, so a respawn reusing it cannot refresh a condemned object. Nothing observable
/// differs: the object still draws, and the server no longer answers for that guid.
#[derive(Component)]
pub struct PendingDestroy;
