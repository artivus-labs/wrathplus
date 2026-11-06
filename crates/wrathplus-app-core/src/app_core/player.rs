//! Our avatar's mover state, which the wire writes and the world drives; `player` keeps the
//! systems and re-exports the types from its old paths.

use bevy::prelude::*;
use wrathplus_protocol::{JumpInfo, MoveMode};

/// The mover modes the server grants, as typed state: the outbound flag word is rebuilt from these
/// every frame, and a mode missing from it is cleared by the server's next echo. Four arrive
/// through the acked [`wrathplus_protocol::MoveMode`] opcodes, `levitating` unacked in a
/// server-authored move; Levitate (spell 1706) grants three at once.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveModes {
    /// Rooted (`SMSG_FORCE_MOVE_ROOT`): translation and jumps die but turning stays live, since the
    /// input tick's allow-list (`0x615c71`, table `0x618054`) permits turn, pitch and facing. A
    /// stun, which takes the pivot too, is [`crate::player::UNIT_FLAG_STUNNED`].
    pub rooted: bool,
    /// Water walking (`SMSG_MOVE_WATER_WALK`): the liquid surface is ground. It does not stop the
    /// swim latch (`0x6030c0` tests only LEVITATING); swimming turns it off instead (`0x631617`).
    pub water_walking: bool,
    /// Feather fall (`SMSG_MOVE_FEATHER_FALL`): the terminal velocity drops to
    /// [`crate::player::FEATHER_TERMINAL_VELOCITY`], and nothing else changes.
    pub feather_fall: bool,
    /// Hover (`SMSG_MOVE_SET_HOVER`): ground contact sits [`crate::player::HOVER_HEIGHT`] up and the jump is
    /// refused (the first test in `CMovement::Jump` `0x7c6230`); the grant itself jumps the body
    /// ([`Player::hover_launch`]). With water walking it seems to leave a swimmer no way up, since
    /// swimming turns water walking off and hover refuses the breach: each gate is traced, the sum
    /// is not.
    pub hover: bool,
    /// `MOVEFLAG_LEVITATING` (GM `.cheat fly`), merged unacked from a server-authored move: the
    /// swim decision does not run while it is set (`0x6030d2`), so a server-set swim is flight.
    pub levitating: bool,
    /// Build 12340's CAN_FLY (`SMSG_MOVE_SET_CAN_FLY`): the jump key may start flight
    /// (`0x989890`); the bit rides every outbound packet while granted.
    pub can_fly: bool,
}

impl MoveModes {
    /// Grant or revoke one acked mode; `levitating` has no opcode and arrives by the wire merge.
    pub fn set(&mut self, mode: MoveMode, apply: bool) {
        match mode {
            MoveMode::Root => self.rooted = apply,
            MoveMode::WaterWalk => self.water_walking = apply,
            MoveMode::FeatherFall => self.feather_fall = apply,
            MoveMode::Hover => self.hover = apply,
            MoveMode::CanFly => self.can_fly = apply,
        }
    }

    /// The granted modes as `MOVEMENTFLAGS` bits, which every outbound packet must carry: the
    /// reference's wire word is its live state (`[cmov+0x40]`), and a mode we drop is cleared by
    /// the server's next echo.
    pub fn wire_flags(&self) -> u32 {
        use crate::app_core::movement::move_flags as f;
        let mut flags = 0;
        if self.rooted {
            flags |= f::ROOT;
        }
        if self.water_walking {
            flags |= f::WATER_WALKING;
        }
        if self.feather_fall {
            flags |= f::SAFE_FALL;
        }
        if self.hover {
            flags |= f::HOVER;
        }
        if self.levitating {
            flags |= f::LEVITATING;
        }
        if self.can_fly {
            flags |= f::CAN_FLY;
        }
        flags
    }

    /// Lift the granted modes out of a server-authored flag word; the reference's merge mask
    /// `0x75a0_7dff` holds all five. Deviation: root is not merged, because a bare pose unrooting
    /// us while the server still roots us streams motion into `CHEAT_TYPE_ROOT_MOVE`; vmangos keeps
    /// a rooted mover's root bit (`MovementHandler.cpp:1064`), so the reference's merge of it is a
    /// no-op anyway.
    pub fn merge_from_wire(&mut self, wire: u32) {
        use crate::app_core::movement::move_flags as f;
        self.water_walking = wire & f::WATER_WALKING != 0;
        self.feather_fall = wire & f::SAFE_FALL != 0;
        self.hover = wire & f::HOVER != 0;
        self.levitating = wire & f::LEVITATING != 0;
    }
}

/// Our avatar: until `active` the camera free-flies, then we drive the body the server placed.
/// `PartialEq` serves the test that the session end resets the whole of it to `Player::default()`.
#[derive(Resource, Default, PartialEq)]
pub struct Player {
    pub active: bool,
    /// The modes the server has granted our mover.
    pub modes: MoveModes,
    /// Autorun: the reference's input bit `0x1000` at `[MOVE+4]`, flipped by `ToggleAutoRun`
    /// `0x513de0`. The emitter `0x514da0` adds it to the forward axis, but a W or S key-down clears
    /// it first ([`crate::player::autorun_cancelled`]), so it is not a held W: autorun then S walks backward.
    pub autorun: bool,
    /// Walk mode, `MOVEFLAG_WALK_MODE` (`0x100`), toggled by `ToggleRun` `0x513d50` from the word's
    /// current bit (`0x60e080`). Observers read our gait off it, and it is in the server-authored
    /// merge mask, so a server move can flip it ([`crate::player::wire_in`]); it owes no ack.
    pub walking: bool,
    /// The auto-move (`/follow` or Click to Move) holding forward this frame: the reference pushes
    /// W's own bit (`0x60e790`), so this is a [`crate::player::forward_axis`] term. Rewritten each frame by
    /// [`crate::player::follow::steer_follow`] and [`crate::player::approach::steer_approach`] just before the
    /// controller reads it.
    pub auto_forward: bool,
    /// Free-fly (`F`): the camera moves on its own and the avatar/server position is frozen.
    pub detached: bool,
    /// `SMSG_CLIENT_CONTROL_UPDATE` with `allowMove = 0` named the unit we drive: us, while someone
    /// mind-controls us, or our possessed creature when feared or confused, the possession
    /// standing. The reference zeroes its mover; vmangos neither roots a charmed player nor refuses
    /// its movement, so nothing else holds the body. Unlike a root, it stops turning too.
    pub control_lost: bool,
    /// The unit we drive in place of our body (a mind-controlled creature, Eye of Kilrogg), `None`
    /// for our own. Our body must then send nothing: outbound `MSG_MOVE_*` carry no guid, so the
    /// server applies them to whatever we last claimed.
    pub foreign_mover: Option<u64>,
    /// Our own body's modes while [`Self::foreign_mover`] is driven, restored with control.
    pub parked_modes: Option<MoveModes>,
    /// The driven body changed, and we drive nothing until its streamed pose is adopted, since
    /// `pos` and the facing still describe the last one. Set when the mover guid changes, cleared
    /// at the take-control edge.
    pub reseat: bool,
    /// A native transport worldport was ACKed; wait for the new self and its deck before driving.
    pub transport_worldport: bool,
    /// Feet position in Bevy coords, converted to WoW coords only for the wire.
    pub pos: Vec3,
    /// Vertical velocity (yd/s, Bevy +Y up), zeroed while grounded.
    pub vel_y: f32,
    /// Horizontal velocity (yd/s): from input while grounded, the frozen take-off momentum while
    /// airborne, bar one [`crate::player::AIR_NUDGE_SPEED`] steer after a standstill jump.
    pub horiz_vel: Vec3,
    /// The movement flags we last streamed; this frame's are diffed against them to emit a
    /// `MSG_MOVE_*` per axis transition.
    pub move_flags: u32,
    /// Last frame's facing: any change off the turn axis streams `MSG_MOVE_SET_FACING`, moving or
    /// not (the reference's detector `0x617170`, exact equality).
    pub last_facing: f32,
    /// The exact position floats we last sent (WoW coords). vmangos interrupts a cast on any change
    /// to its stored position (`Player.cpp:6092`), so the resolver's tiny settle after a reported
    /// rest is sent on its own ([`crate::player::movement_net::stream_self_movement`]) before the next
    /// facing change can carry it as a move.
    pub last_pos: [f32; 3],
    /// A stand state committed locally, sent or from `SMSG_STANDSTATE_UPDATE`, whose echo into
    /// `UNIT_FIELD_BYTES_1` has not landed: the reference's predicted cache `[player+0x1d68]`
    /// (`0x6127b0`).
    pub stand_pending: Option<u8>,
    /// Holding the body still, gravity off, after a teleport, summon or login until the
    /// destination's colliders stream in. Released by the terrain streamer on residency or at
    /// [`crate::player::SETTLE_TIMEOUT`], never by ground contact, which a flyer or a swimmer never makes.
    pub settling: bool,
    /// When to give up settling (`Time::elapsed_secs`), pushed on while the world is stale or
    /// still loading.
    pub settle_deadline: f32,
    /// When the current settle hold began (`Time::elapsed_secs`), which the `sett` trace reports.
    pub settle_since: f32,
    /// The colliders under us may still be the map we left: the snap runs a whole `WorldStage`
    /// before the streamer swaps maps. Set at every snap, cleared once the destination is resident;
    /// while set, the streamer judges no release and pushes [`Player::settle_deadline`] on.
    pub world_stale: bool,
    /// The camera pitch the login seize seats: the saved pose's `cameraPitch` when
    /// [`crate::player::camera_saved`] restored one, else the stock opening pitch.
    pub login_pitch: Option<f32>,
    /// A same-map teleport voided the self server-ride: vmangos ignores the spline-done ack while
    /// the teleport is pending (`MovementHandler.cpp:819`), so `drive_self_ride` drops the ride
    /// without mirroring its stale pose and sends no `CMSG_MOVE_SPLINE_DONE`.
    pub ride_abort: bool,
    /// A `MSG_MOVE_WORLDPORT_ACK` owed at the settle release: the reference sends it only after its
    /// blocking load returns (`0x401cae`, after `0x66fbe0`), and vmangos has no load timeout. A
    /// riding crossing never settles and acks at once.
    pub owes_worldport_ack: bool,
    pub native_worldport_ack: Option<wrathplus_protocol::wrath_transfer::Token>,
    /// `Time::elapsed_secs` when we last sent a heartbeat.
    pub last_heartbeat: f32,
    /// When the current airborne phase began (`Time::elapsed_secs`): the wire `fall_time`, and the
    /// take-off and landing edges that emit `MSG_MOVE_JUMP` and `MSG_MOVE_FALL_LAND`.
    pub airborne_since: Option<f32>,
    /// At rest wedged between steep faces: standing while a close down-probe still finds support,
    /// until real ground, a jump or walking off into open air.
    pub wedged: bool,
    /// Consecutive stalled airborne frames (see [`crate::player::WEDGE_STALL_RATIO`]).
    pub wedge_still: u8,
    /// On a certified steep contact rather than a walkable floor, the reference's `0x4000000`:
    /// riding a low edge's foot cone up, or following a surface down off a ledge. Counts as
    /// standing, since the straight-down ground probe sees only the steep face; re-earned each
    /// frame.
    pub steep_support: bool,
    /// A hover grant owes a jump: the reference's handler `0x61a620` calls `CMovement::Jump(0)`
    /// (`0x61a630`) before setting the flag, and its disable arm calls `StartFalling` (`0x61a637`).
    /// Latched for the take-off site, since our mover re-derives ground contact every frame and
    /// would cancel a jump committed here.
    pub hover_launch: bool,
    /// The knockback waiting to be flown, armed by `SMSG_MOVE_KNOCK_BACK` and taken at the take-off
    /// site with the jump and the hover launch.
    pub knockback: Option<PendingKnockback>,
    /// The arc's take-off vertical speed (yd/s, WoW +Z up), the client's `StartFalling` argument
    /// (`+0xa0`) and the jump tail's `zspeed`: [`crate::player::JUMP_SPEED`] for a jump, exactly 0 for a step-off.
    pub jump_zspeed: f32,
    /// The launch vertical speed, recorded before this frame's gravity: [`Self::jump_zspeed`] is
    /// seeded from this, since `vel_y` is already a step down the arc when [`crate::player::arc`] runs.
    pub launch_vz: f32,
    /// Whether the arc's direction nibble (`[CMovement+0x40] & 0xf`) is set: air control opens only
    /// while it is clear (`0x7c6afc`), so a knockback, which plants FORWARD (`0x617a18`), never
    /// steers. Seeded at take-off, set by the nudge, cleared when the arc ends.
    pub arc_dirs_set: bool,
    /// This arc is a knockback's, so its planted FORWARD rides the wire the whole arc (`0x617a18`;
    /// the send mask `0x75a07dff` keeps bit 0).
    pub knock_arc: bool,
    /// Airborne at the end of the last mover step: the mover's own record, since
    /// [`Self::airborne_since`] is written by [`crate::player::flags`], which runs after the mover.
    pub airborne_prev: bool,
    /// The direction bits the current arc launched with: mid-air they are the actual motion, so the
    /// live flags read them instead of the keys; re-seeded by the air nudge, stale while grounded.
    pub airborne_dirs: u32,
    /// Launch height (Bevy Y), the client's `StartFalling` z snapshot (`+0x7c`): the FALLINGFAR
    /// distance leg measures descent below it.
    pub fall_start_y: f32,
    /// `MOVEFLAG_FALLINGFAR` latched for this arc ([`crate::player::FALL_FAR_DROP`], [`crate::player::FALL_FAR_TIME`]); only
    /// landing clears it, as the client's `StopFalling` does.
    pub fall_far: bool,
    /// The facing (Bevy yaw, radians): the aim sent as orientation and the basis WASD move in.
    /// Right-drag and movement sync it to the camera, left-drag does not; the body is `model_yaw`.
    pub face_yaw: f32,
    /// Swimming (`MOVEFLAG_SWIMMING`): the water over the feet crossed the swim-enter depth,
    /// latched with hysteresis ([`crate::player::swim::update_swimming`]).
    pub swimming: bool,
    /// Build 12340's FLYING: with CAN_FLY granted, the ascend key held in the air took off
    /// (`Wow.exe` `0x989890`), and ground contact, water, a knockback or the grant's loss lands
    /// (`0x98c240`). The mover flies gravity-free along its pitched facing ([`crate::player::flight`]).
    pub flying: bool,
    /// Our body's collision height, of which every swim depth line is a fraction. The component's
    /// own type, so `Player::default()` gives [`crate::player::DEFAULT_COLLISION_HEIGHT`] rather than 0, which
    /// would swim on dry land.
    pub collision_height: crate::app_core::state::CollisionHeight,
    /// The liquid surface over our feet as the last movement tick stored it (Bevy Y). The camera's
    /// water corridor reads this a frame late, as the reference's camera reads the tick's cache
    /// through the field accessor `0x670630`; the lag keeps its depth off a band edge.
    pub liquid_surface: Option<f32>,
    /// The mover pitch (radians, +up), `CMovement+0x20`, live in every mode: mouse-look sets it
    /// straight to the camera pitch within ±89° (`SetPitch` `0x7c6f70`, which stores before any
    /// swim test), and it holds when unsteered; only stop-swim and teleport zero it (`0x7c6e80`).
    /// Swimming reads it for travel, pose and the wire; on land, water walking does (`0x63161e`).
    pub mover_pitch: f32,
    /// The camera pitch the last pitch push carried: the reference pushes only on mouse motion
    /// (`0x514400`), so a push here needs the aim to differ, and a still mouse leaves the drunk
    /// wobble and the stop-swim levelling (`0x7c6e80`) in place.
    pub aim_pitch_seen: f32,
    /// This frame's flag-derived swim speed (yd/s), 0 without swim input: the stroke's playback
    /// numerator, as `0x5fe2f0` divides `GetCurrentSpeed` by the clip's speed. Stale on land.
    pub swim_stroke_speed: f32,
    /// The rendered body heading (Bevy yaw): eased to the strafe offset while strafing
    /// ([`crate::creature_anim::strafe_body_offset`]), snapped to `face_yaw` while otherwise
    /// moving, and chasing at [`crate::player::STATIONARY_CHASE_RATE`] while standing.
    pub model_yaw: f32,
    /// A server spline drives the body (`SMSG_MONSTER_MOVE` for our guid: a charge, a taxi):
    /// input, physics and the movement stream yield, and its end sends `CMSG_MOVE_SPLINE_DONE`.
    pub server_riding: bool,
    /// The `splineId` of the ride in progress (echoed in `CMSG_MOVE_SPLINE_DONE` when it ends).
    pub ride_spline_id: u32,
    /// Whether the ride is a ground path (no `FLYING`), kept because its last frame has lost the
    /// spline and must still be grounded; a taxi's endpoint keeps its altitude.
    pub ride_grounded: bool,
    /// Standing on a transport, the mover in its frame: attached on a
    /// [`crate::transport::Transport`] support, kept through deck jumps, detached on world ground,
    /// in water or when the transport despawns.
    pub ride: Option<PlayerRide>,
}

impl Player {
    /// The mover pitch (radians, +up), the aiming unit's angle the vehicle aim getters read.
    pub fn mover_pitch(&self) -> f32 {
        self.mover_pitch
    }

    /// A player whose last streamed movement word is `flags`, for the gates that read it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_move_flags(flags: u32) -> Self {
        Self {
            move_flags: flags,
            ..Self::default()
        }
    }

    /// Take the jump a hover grant owes, clearing the latch: `Jump(0)` refuses only ROOT and
    /// FALLING (`0x7c625c`), skipping the hover test (`0x7c6236`), and a refused jump is dropped,
    /// not retried.
    /// A vehicle aim request (`VehicleAimRequestNormAngle`): the driven unit's mover pitch, within
    /// the mouse-look clamp.
    pub fn set_aim_pitch(&mut self, pitch: f32) {
        if pitch.is_finite() {
            self.mover_pitch =
                pitch.clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
        }
    }

    pub fn take_wire_jump(&mut self) -> bool {
        let fire = self.hover_launch && !self.modes.rooted && self.airborne_since.is_none();
        self.hover_launch = false;
        fire
    }

    /// Take the knockback, clearing the latch; the mover may still decline it (the settle hold, a
    /// root), and then it is dropped, not deferred, and no ack is sent.
    pub fn take_knockback(&mut self) -> Option<PendingKnockback> {
        self.knockback.take()
    }

    /// Turn the aim by `radians`, the scripted mouse turn (`capture::probe_look`): the same
    /// `face_yaw` a real mouse turn writes and the facing stream diffs.
    pub fn turn_aim(&mut self, radians: f32) {
        self.face_yaw += radians;
    }

    /// Set the mover pitch, the scripted dive (`capture::probe_pitch`), under `SetPitch`'s ±89°
    /// clamp (`0x7c6f70`) but without its 2^-20 deadband (`[0x8026bc]`); returns the value stored.
    pub fn aim_pitch(&mut self, radians: f32) -> f32 {
        self.mover_pitch = radians.clamp(-MOUSELOOK_PITCH_CLAMP, MOUSELOOK_PITCH_CLAMP);
        // Mouse-look pushes on a camera-pitch edge; parking the edge here keeps a still camera
        // from overwriting the script next frame.
        self.aim_pitch_seen = self.mover_pitch;
        self.mover_pitch
    }

    /// The facing (Bevy yaw) sent as orientation, not the rendered `model_yaw`; the 3D-audio
    /// listener faces it, not the camera (`0x483430`).
    pub fn facing(&self) -> f32 {
        self.face_yaw
    }

    /// Build 12340's `IsFalling` (`0x612430`): `FALLING` (`0x1000`) without `ROOT` (`0x800`), our
    /// mover airborne and not rooted.
    pub fn is_falling(&self) -> bool {
        self.airborne_since.is_some() && !self.modes.rooted
    }

    /// The movement flags as last streamed; the water foam tests `& 0xf` (translating) and `& 0x30`
    /// (turning) on them, as the reference's ripple driver `0x5fa760` does.
    pub fn move_flags(&self) -> u32 {
        self.move_flags
    }

    /// The states on this resource that stop WASD, comma-joined, or `"none"`: an instrument for the
    /// log after a session boundary, not a gate. A stun lives on the descriptor and does not show.
    pub fn movement_suppressors(&self) -> String {
        let named = [
            (!self.active, "inactive"),
            (self.detached, "free-fly"),
            (self.modes.rooted, "root"),
            (self.control_lost, "control-lost"),
            (self.server_riding, "server-ride"),
            (self.foreign_mover.is_some(), "possession"),
            (self.reseat, "reseat-pending"),
        ];
        let list: Vec<&str> = named
            .into_iter()
            .filter_map(|(set, name)| set.then_some(name))
            .collect();
        if list.is_empty() {
            "none".to_string()
        } else {
            list.join(",")
        }
    }

    /// The commanded planar speed (yd/s), the reference's `GetCurrentSpeed` (`0x987570`; 1.12 read
    /// `[[player+0x118]+0x84]`):
    /// exactly 0 with no direction bit set, and live, not averaged like the weather wind's
    /// (`0x67c150`). The precipitation slab's tilt reads it (`0x67bf8b`).
    pub fn planar_speed(&self) -> f32 {
        if self.move_flags & 0xf == 0 {
            return 0.0;
        }
        self.horiz_vel.with_y(0.0).length()
    }

    /// The guid of the transport we stand on, for instruments.
    pub fn riding(&self) -> Option<u64> {
        self.ride.as_ref().map(|r| r.guid)
    }

    /// The transport we stand on as its entity, whose transform is the deck frame a rider's effects
    /// are stored in (`wrathplus_world::ride_frame`).
    pub fn ride_entity(&self) -> Option<Entity> {
        self.ride.as_ref().map(|r| r.entity)
    }

    /// Whether a server spline drives the avatar ([`super::server_ride`]), for instruments. Not
    /// `UnitOnTaxi`, which reads [`crate::player::UNIT_FLAG_TAXI_FLIGHT`] (`0x517a86`).
    pub fn server_riding(&self) -> bool {
        self.server_riding
    }
}

impl Player {}

/// The player's attachment to a transport's frame ([`Player::ride`]).
#[derive(PartialEq)]
pub struct PlayerRide {
    /// The transport's entity, whose collider is the support that attached us.
    pub entity: Entity,
    /// The transport's guid, which the wire's transport tail carries.
    pub guid: u64,
    /// Feet in the transport's frame (Bevy axes), snapshotted at frame end; the next frame's carry
    /// recomposes the world position from it before input integrates.
    pub local_pos: Vec3,
    /// The transport's yaw at the snapshot: the carry turns `face_yaw` by its per-frame delta, and
    /// the wire's local orientation is `face_yaw − boat_yaw`.
    pub boat_yaw: f32,
}

/// A knockback aimed at our mover (`SMSG_MOVE_KNOCK_BACK`), latched until the mover flies it. The
/// ack is owed only if the launch happened and echoes `launch` as its jump tail: vmangos matches
/// all four floats within 0.01 (`Unit.cpp:7096-7100`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingKnockback {
    /// Our mover guid, echoed in the ack as a full u64 (it arrives packed).
    pub guid: u64,
    /// The server's movement counter, echoed or the server rejects the ack.
    pub counter: u32,
    /// The launch: world-XY direction, horizontal speed and vertical speed, down-positive
    /// (negative is up).
    pub launch: JumpInfo,
}

/// The mouse-look pitch clamp, ±89° (`0x8089d8`, on the camera-to-`SetPitch` path); the ±π/2 clamp
/// belongs to the pitch-key integrator `0x7c4f80`, whose keys are unbound by default.
pub const MOUSELOOK_PITCH_CLAMP: f32 = 1.553_343;

/// The FALLINGFAR distance leg (yd): a jump arc (launch vz ≠ 0) latches it once this far below
/// its launch height (`0x633240`, `[0x80dff8]`), and the anim swaps to Fall(40); a flat jump never
/// does.
pub const FALL_FAR_DROP: f32 = 0.111_11;

/// The FALLINGFAR timer leg (s): a step-off fall (launch vz = 0) latches it after 500 ms airborne
/// (`0x633240`, `0x1f4`), about 2.41 yd of free fall from rest.
pub const FALL_FAR_TIME: f32 = 0.5;
