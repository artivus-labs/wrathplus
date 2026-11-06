//! The avatar's state, [`Player`] and its resources, and the movement constants; no systems.

use bevy::prelude::*;

/// Backpedal over run, the stock 4.5 / 7.0, for the synthetic speed set used under a
/// `$WOW_MOVE_SPEED` override or before the create block brings the server's speeds.
pub(super) const RUN_BACK_RATIO: f32 = 4.5 / 7.0;

/// Walk over run, the stock 2.5 / 7.0, for the same synthetic speed set as [`RUN_BACK_RATIO`].
pub(super) const WALK_RATIO: f32 = 2.5 / 7.0;

/// Turn rate (rad/s) until the mover's own arrives: the live rate is its sixth speed,
/// `CMovement+0x9c` (set by `0x7c6ff0`, read by `GetYawRate` `0x7c5c50`), which the ctor `0x7c4850`
/// zeroes, so π is the server's value and the client keeps no default.
pub(super) const TURN_RATE: f32 = std::f32::consts::PI;

/// Water walking sees the water only while the mover pitch is strictly above −37° (`[0x80dfe8]`,
/// the third gate of the trace-mask arm `0x6315f0`); aiming lower sinks a water-walker in.
pub(super) const WATER_WALK_PITCH_FLOOR: f32 = -0.645_771_8;
/// Turn-rate scale while translating or falling (`flags & 0x200f`).
pub(super) const TURN_RATE_MOVING: f32 = 0.75;

/// Once steering stops, the rendered body closes on the aim at `turnRate × 8` rad/s, clamped to the
/// gap (`0x607ed0`); while steering, only the 90° ceiling moves it (`0x60818a`-`0x6081bf`).
pub(super) const STATIONARY_CHASE_RATE: f32 = 8.0;

// ── Character controller ─────────────────────────────────────────────────────────

pub(crate) use crate::app_core::movement::CAPSULE_HEIGHT;
pub(crate) use crate::app_core::movement::CAPSULE_RADIUS;

pub(crate) use crate::app_core::movement::GRAVITY;
/// Jump take-off speed (yd/s), the reference's.
pub(super) const JUMP_SPEED: f32 = 7.955_547;
pub(crate) use crate::app_core::movement::FEATHER_TERMINAL_VELOCITY;
pub(crate) use crate::app_core::movement::HOVER_HEIGHT;
pub(crate) use crate::app_core::movement::TERMINAL_VELOCITY;
/// Hover's rise rate (yd/s): the snap only lowers the body (`0x636e52`), so the rise is a separate
/// rate-limited pass (`0x636fa1`-`0x6370f1`) at `0x7c61b0`'s `[0x87d898]`, the feather-fall 7.0.
pub(crate) const HOVER_CLIMB_RATE: f32 = 7.0;
/// Walkable iff the surface normal is within 50° of up, the reference's limit; steeper slides back.
pub(super) const GROUND_COS: f32 = 0.642_788;
/// Downward probe distance (yd) to decide whether we're standing on ground.
pub(super) const GROUND_PROBE: f32 = 0.2;
/// The snap's slope ratio, `[0x80c740]` (atan ≈ 61.6°): the reference's step-vs-fall election
/// `0x6367b0` probes `travel · ratio + slack` below the moved body, plus the rise budget `H` only
/// on a steep support (`0x4000000`, `0x636dfc`). The foot cone's waist (`0x631c0b`) shares it.
pub(super) const STEP_SLOPE_RATIO: f32 = 1.849_399;
/// The snap reach's fixed slack (yd), `[0x7ff9d0]` = 1/36.
pub(super) const STEP_SNAP_SLACK: f32 = 0.027_777_8;
pub(crate) use crate::app_core::movement::STEP_UP_HEIGHT;
/// The step-up certify advance (yd), a floor under the frame's travel: the reference steps
/// `max(H·tan50°, radius + 1/720)` into the face, 1.1918 yd at `H` = 1.0. The raised forward sweep
/// is clipped by anything in the way, so the advance reaches only clear air.
pub(super) const STEP_UP_ADVANCE: f32 = 1.191_753_6;
/// The certify reach per yard of rise budget, `tan 50°` (`0x636147`-`0x636190`); a creature's 2.0
/// reaches 2.38 yd.
pub(super) const STEP_UP_ADVANCE_PER_YARD: f32 = STEP_UP_ADVANCE / STEP_UP_HEIGHT;
/// The foot cone's height (yd). The reference's solid (`0x631440`) is a box over a cone whose
/// bevels rise `radius · [0x80c740]` from a point at the foot, so an edge below this is ridden up
/// the 61.6° skirt (`0x635c00`) and one above meets the box and takes the step-up. With the
/// placeholder [`CAPSULE_RADIUS`] ours is 0.616 yd; a live human male's is 0.565, female's 0.385.
pub(super) const FOOT_CONE_HEIGHT: f32 = CAPSULE_RADIUS * STEP_SLOPE_RATIO;
/// The landing probe (yd): airborne, walk mode resumes only this close to the floor, so the arc
/// ends where the slide contacts rather than [`GROUND_PROBE`] early.
pub(super) const LAND_PROBE: f32 = 0.05;
/// Stalled airborne frames (see [`WEDGE_STALL_RATIO`]) that count as resting wedged between two
/// steep faces: the fall ends there, and nothing becomes walkable by it.
pub(super) const WEDGE_STILL_FRAMES: u8 = 3;
/// A frame stalls when its descent is under this fraction of gravity's intended `vel_y·dt` while
/// falling faster than [`WEDGE_MIN_FALL`]: free fall achieves ~100%, a steep slide at least 75%.
pub(super) const WEDGE_STALL_RATIO: f32 = 0.15;
/// Fall speed (yd/s) before stalled frames count: a jump apex never qualifies, a wedge soon does.
pub(super) const WEDGE_MIN_FALL: f32 = 1.0;
/// The one-shot air nudge (yd/s) a standstill jump may steer by. The reference's is the live
/// `min(walk, run)` (`0x987570(1)`, `0x98762c`); this is the default walk speed, not the live one.
pub(super) const AIR_NUDGE_SPEED: f32 = 2.5;
/// Skin width (yd) kept between the capsule and geometry on casts.
pub(super) const SKIN_WIDTH: f32 = 0.02;

/// Seconds of stalled streaming before the post-snap hold gives up: the streamer pushes the
/// deadline on while the world is stale or any load counter moves, so only a dead stream hits it.
pub(crate) const SETTLE_TIMEOUT: f32 = 6.0;

pub(crate) use crate::app_core::movement::PlayerCapsule;

/// The run-speed fallback, the stock 7.0 until the server's speeds stream in, or the
/// `$WOW_MOVE_SPEED` dev override (`env_override`), which replaces the server's speeds outright.
#[derive(Resource)]
pub(super) struct MoveSpeed {
    pub(super) value: f32,
    pub(super) env_override: bool,
}

#[cfg(test)]
pub(crate) use crate::app_core::player::MoveModes;
pub(crate) use crate::app_core::player::{PendingKnockback, Player, PlayerRide};
pub(crate) use crate::app_core::player::{FALL_FAR_DROP, FALL_FAR_TIME, MOUSELOOK_PITCH_CLAMP};

/// The precondition both movement-input predicates test first, the reference's `0x5144e0`. Of its
/// five terms, health ([`MoverInput::dead`]) and the far-sight latch
/// ([`MoverInput::view_is_out`]) are here; the mover resolving is structural (the controller
/// returns early on `control_lost` and `reseat`), and the aux gate `[[mover+0x118]+0xa4]`
/// (`0x514516`) and the Knockdown animation lockout `0x60f5b0` are not built.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MoverInput {
    /// The health term: the driven mover's health is not above 0 (`0x5144fd`, off `[mover+0x110]`).
    /// A released ghost has health 1 from the server, so it passes and keeps every input.
    pub(crate) dead: bool,
    /// The far-sight term, held true while our view is out on a far-sight object and we drive our
    /// own body: `[mover+0x1c70] & 1`, read only when the mover is the active player (`0x514537`).
    /// The latch sets only once the subject resolves (`0x5ee3f6`), so this follows the resolved
    /// [`super::view_subject::ViewSubject`], not the raw `PLAYER_FARSIGHT` field.
    pub(crate) view_is_out: bool,
}

/// The far-sight term: the view is out and the body is our own. Mind Control sets the same field,
/// so without `driving_own_body` (the active-player test `0x5fa6d0`) it would freeze the victim.
pub(crate) fn view_is_out(driving_own_body: bool, far_sight_engaged: bool) -> bool {
    driving_own_body && far_sight_engaged
}

impl MoverInput {
    /// `0x5144e0`, over the terms modelled here.
    fn ready(self) -> bool {
        !self.dead && !self.view_is_out
    }

    /// `0x514560`, the gate on the forward/back and strafe emitters (`0x5146c1`): the precondition
    /// and no root (`MOVEMENTFLAGS & 0x1200`); its stand-state-7 term is one vmangos never sets.
    pub(crate) fn may_translate(self, rooted: bool) -> bool {
        self.ready() && !rooted
    }

    /// `0x5145b0`, the gate on the turn and pitch emitters (`0x514755`): the precondition and no
    /// stun (`UNIT_FIELD_FLAGS & 0x40000`). The mouse's gate `0x5145e0` calls it first, so a corpse
    /// turns by neither keys nor mouse.
    pub(crate) fn may_turn(self, stunned: bool) -> bool {
        self.ready() && !stunned
    }

    /// `0x5145e0`, the gate on the mouse's camera-to-body hand-off (`0x514474`, `0x51495a`): the
    /// stun predicate and a standing body (`0x51460c`). A right-drag while seated turns only the
    /// camera and does not stand you up; `stand_state` is the predicted one `0x5ed570` returns.
    pub(crate) fn mouse_may_turn_body(self, stunned: bool, stand_state: u8) -> bool {
        self.may_turn(stunned) && stand_state == 0
    }

    /// The input tick's teardown leg (`0x5146d6`, ending click-to-move and `/follow`), taken only
    /// with both predicates down: death, far sight, or a root and a stun together.
    pub(crate) fn torn_down(self, rooted: bool, stunned: bool) -> bool {
        !self.may_translate(rooted) && !self.may_turn(stunned)
    }
}

/// Strip from a freshly built flag word what a gated mover cannot hold: the direction bits while
/// translation is gated (the allow-list `0x618054` drops the move commands and `SetRoot` `0x7c7340`
/// masks `0xffe07f00`, so the anim resolver's `0x5fd10c` sees no motion) and the turn bits while
/// turning is gated (`0x514755`). The reference's emitters refuse at the source; ours rebuilds the
/// word each frame. Modes and SWIMMING ride on.
pub(crate) fn incapacitated_flags(flags: u32, translate_gated: bool, turn_gated: bool) -> u32 {
    use crate::app_core::movement::move_flags as f;
    let mut out = flags;
    if translate_gated {
        out &= !f::ANY_MOVE;
    }
    if turn_gated {
        out &= !(f::TURN_LEFT | f::TURN_RIGHT);
    }
    out
}

/// The forward/back axis, summed as the reference's emitter `0x514da0` sums it: autorun, forward
/// and both-button run +1 each, backward −1; its sign picks the START, and zero is a STOP.
pub(super) fn forward_axis(
    forward: bool,
    backward: bool,
    both_buttons: bool,
    autorun: bool,
) -> i32 {
    i32::from(forward) + i32::from(both_buttons) + i32::from(autorun) - i32::from(backward)
}

/// Whether this frame's input clears autorun: the four of the reference's six writers of `0x1000`
/// with an analog here. `fwd_down` and `back_down` are key-down edges (`0x514a5a`; a release
/// restores nothing), `both_engaged` the edge into both-button run (`0x514a73`), and `lost_mover` a
/// level, the translate gate down (`0x514748`). A jump, a chat box taking focus and a zone change
/// leave autorun on; mounting's effect in the reference is untraced, and here it leaves it on.
pub(super) fn autorun_cancelled(
    fwd_down: bool,
    back_down: bool,
    both_engaged: bool,
    lost_mover: bool,
) -> bool {
    fwd_down || back_down || both_engaged || lost_mover
}

/// Whether the reference's `SetStandState` (`0x5ed430`) refuses this change, returning before the
/// packet is built. A body that reads dead is refused either way; otherwise standing up never is,
/// and any other state is refused while translating or swimming (`0x20000f`, `0x5ed4f8`), SLEEP
/// also while turning (`0x5ed4e6`, which falls through into that test). FALLING is in neither
/// mask. It is the one setter, so the X key, the posture emotes and the AFK sit all go through it.
pub(super) fn stand_state_refused(reads_dead: bool, move_flags: u32, new_state: u8) -> bool {
    use crate::app_core::movement::move_flags as f;
    // Health <= 0 (`0x5ed4ac`) or `UNIT_DYNAMIC_FLAGS & 0x20`, a feigner (`0x5ed4b2`), refuses in
    // both directions, so it comes before the stand-up exit.
    if reads_dead {
        return true;
    }
    // Standing up skips the movement word (`0x5ed4f0`).
    if new_state == 0 {
        return false;
    }
    // SLEEP's turn test falls through into the shared one (`0x5ed4ec`).
    if new_state == 3 && move_flags & (f::TURN_LEFT | f::TURN_RIGHT) != 0 {
        return true;
    }
    move_flags & (f::ANY_MOVE | f::SWIMMING) != 0
}

/// `Player`'s methods that need this module (the type lives in `app_core` for the crate split).
pub(crate) trait PlayerSettle {
    /// End the settle hold, called by the terrain streamer: `resident` is false when
    /// [`SETTLE_TIMEOUT`] fired first, which the `sett` trace records.
    fn end_settle(&mut self, resident: bool, now: f32);
}

impl PlayerSettle for Player {
    fn end_settle(&mut self, resident: bool, now: f32) {
        self.settling = false;
        let waited = now - self.settle_since;
        super::move_trace::settle(resident, waited, self.pos);
    }
}

#[cfg(test)]
mod stand_state_tests {
    use super::stand_state_refused;
    use crate::app_core::movement::move_flags as f;

    #[test]
    fn a_body_that_reads_dead_can_neither_sit_nor_stand() {
        for state in [0u8, 1, 2, 3, 8] {
            assert!(
                stand_state_refused(true, 0, state),
                "stand state {state} refused on a body that reads dead — standing up included, \
                 which is the one case the movement-word gate below would have let through"
            );
        }
        assert!(
            !stand_state_refused(false, 0, 1),
            "and the same still body, alive, is granted its sit — the guard is the death, not the \
             standing still"
        );
    }

    #[test]
    fn a_swimmer_cannot_sit_but_can_always_stand() {
        // Floating still: SWIMMING alone refuses.
        assert!(
            stand_state_refused(false, f::SWIMMING, 1),
            "sit refused mid-swim"
        );
        assert!(
            stand_state_refused(false, f::SWIMMING | f::FORWARD, 1),
            "swimming forward too"
        );
        // SIT, SIT_CHAIR and KNEEL.
        for state in [1u8, 2, 8] {
            assert!(
                stand_state_refused(false, f::SWIMMING, state),
                "state {state} refused mid-swim"
            );
        }
        // Standing up (0) skips the word.
        assert!(!stand_state_refused(false, f::SWIMMING | f::FORWARD, 0));
        assert!(!stand_state_refused(false, f::ANY_MOVE | f::SWIMMING, 0));
    }

    /// The turn bits are outside `0x20000f`.
    #[test]
    fn sitting_is_refused_while_translating_and_allowed_while_merely_turning() {
        assert!(
            !stand_state_refused(false, 0, 1),
            "standing still: sit granted"
        );
        for bit in [f::FORWARD, f::BACKWARD, f::STRAFE_LEFT, f::STRAFE_RIGHT] {
            assert!(
                stand_state_refused(false, bit, 1),
                "translating: sit refused"
            );
        }
        for bit in [f::TURN_LEFT, f::TURN_RIGHT] {
            assert!(
                !stand_state_refused(false, bit, 1),
                "turning in place: sit granted"
            );
        }
        // Mode bits are not movement.
        for bit in [f::ROOT, f::WATER_WALKING, f::FALLING, f::WALK_MODE] {
            assert!(
                !stand_state_refused(false, bit, 1),
                "mode bit {bit:#x} is not a move"
            );
        }
    }

    /// SLEEP (3) adds the `0x30` turn test and falls through into `0x20000f` (`0x5ed4ec`).
    #[test]
    fn sleep_takes_both_tests_so_it_is_the_strictest_posture() {
        // The turn test is SLEEP's alone.
        assert!(
            stand_state_refused(false, f::TURN_LEFT, 3),
            "turning: /sleep refused"
        );
        assert!(stand_state_refused(false, f::TURN_RIGHT, 3));
        assert!(
            !stand_state_refused(false, f::TURN_LEFT, 1),
            "turning blocks ONLY the sleep — a sit is granted"
        );
        // The shared test it falls into.
        assert!(
            stand_state_refused(false, f::SWIMMING, 3),
            "swimming: /sleep refused"
        );
        assert!(
            stand_state_refused(false, f::FORWARD, 3),
            "walking: /sleep refused"
        );
        // SLEEP's effective mask is the one-shot route's `0x20003f`.
        assert_eq!(
            f::TURN_LEFT | f::TURN_RIGHT | f::ANY_MOVE | f::SWIMMING,
            f::ROUTE_COMMITTED_MOVE,
            "`0x20003f`, byte-verified at 0x5fe6dc as well as 0x5ed4e6+0x5ed4f8"
        );
        // Standing up from SLEEP is still ungated.
        assert!(!stand_state_refused(false, f::ROUTE_COMMITTED_MOVE, 0));
        // FALLING is in neither mask.
        assert!(
            !stand_state_refused(false, f::FALLING, 3),
            "falling: /sleep granted"
        );
    }
}

#[cfg(test)]
mod autorun_tests {
    use super::{autorun_cancelled, forward_axis};

    /// The reference's axis table (`0x514da0`).
    #[test]
    fn the_axis_reproduces_the_verified_state_table() {
        // Autorun alone: +1.
        assert_eq!(forward_axis(false, false, false, true), 1);
        // Autorun, then W: the key-down cleared the bit, so the axis sees W alone; nothing is sent.
        assert_eq!(forward_axis(true, false, false, false), 1);
        // W, then autorun: 2 (`0x514da5`); only the sign is consumed.
        assert_eq!(forward_axis(true, false, false, true), 2);
        // Autorun, then S: the key-down cleared the bit, so -1.
        assert_eq!(forward_axis(false, true, false, false), -1);
        // S, then autorun: the toggle skips the clear and both stay live, so 0.
        assert_eq!(forward_axis(false, true, false, true), 0);
    }

    #[test]
    fn the_two_orders_differ_and_only_one_resumes() {
        // Autorun, then S.
        let mut autorun = true;
        if autorun_cancelled(false, true, false, false) {
            autorun = false;
        }
        assert_eq!(
            forward_axis(false, true, false, autorun),
            -1,
            "walks backward"
        );
        // Releasing S restores no bit.
        assert_eq!(
            forward_axis(false, false, false, autorun),
            0,
            "stops, does not resume"
        );

        // S held, then autorun: the toggle is not a directional set, so nothing clears it.
        let autorun = true;
        assert!(!autorun_cancelled(false, false, false, false));
        assert_eq!(
            forward_axis(false, true, false, autorun),
            0,
            "STOP with S still held"
        );
        // Releasing S resumes: the bit survived.
        assert_eq!(
            forward_axis(false, false, false, autorun),
            1,
            "resumes forward"
        );
    }

    #[test]
    fn both_button_run_shares_the_axis() {
        assert_eq!(forward_axis(false, false, true, false), 1);
        assert_eq!(
            forward_axis(false, true, true, false),
            0,
            "S nets the both-button run to a stop"
        );
        // Engaging both-button run clears autorun.
        assert!(autorun_cancelled(false, false, true, false));
    }

    #[test]
    fn a_jump_or_a_chat_line_is_not_in_the_cancel_set() {
        assert!(!autorun_cancelled(false, false, false, false));
        // Losing the mover clears it, as a level.
        assert!(autorun_cancelled(false, false, false, true));
    }
}

#[cfg(test)]
mod move_mode_tests {
    use super::{
        incapacitated_flags, view_is_out, MoveModes, MoverInput, Player, FEATHER_TERMINAL_VELOCITY,
        GRAVITY, HOVER_CLIMB_RATE, HOVER_HEIGHT, TERMINAL_VELOCITY,
    };
    use crate::app_core::movement::move_flags as f;
    use wrathplus_protocol::MoveMode;

    #[test]
    fn each_granted_mode_rides_the_wire_word_as_its_own_bit() {
        for (mode, bit) in [
            (MoveMode::Root, f::ROOT),
            (MoveMode::WaterWalk, f::WATER_WALKING),
            (MoveMode::FeatherFall, f::SAFE_FALL),
            (MoveMode::Hover, f::HOVER),
            (MoveMode::CanFly, f::CAN_FLY),
        ] {
            let mut modes = MoveModes::default();
            assert_eq!(modes.wire_flags(), 0);
            modes.set(mode, true);
            assert_eq!(
                modes.wire_flags(),
                bit,
                "{mode:?} must ride the wire as {bit:#x}"
            );
            assert_eq!(
                mode.flag(),
                bit,
                "{mode:?}: our bit and the protocol's agree"
            );
            modes.set(mode, false);
            assert_eq!(modes.wire_flags(), 0, "{mode:?} revokes cleanly");
        }
    }

    /// Levitate (spell 1706) grants feather fall, hover and water walk at once.
    #[test]
    fn levitate_carries_its_three_modes_at_once() {
        let mut modes = MoveModes::default();
        for mode in [MoveMode::FeatherFall, MoveMode::Hover, MoveMode::WaterWalk] {
            modes.set(mode, true);
        }
        assert_eq!(
            modes.wire_flags(),
            f::SAFE_FALL | f::HOVER | f::WATER_WALKING
        );
        modes.set(MoveMode::Hover, false);
        assert_eq!(modes.wire_flags(), f::SAFE_FALL | f::WATER_WALKING);
    }

    #[test]
    fn the_wire_merge_never_unroots_us() {
        let mut modes = MoveModes {
            rooted: true,
            ..Default::default()
        };
        modes.merge_from_wire(0); // a pose claiming no modes at all
        assert!(modes.rooted, "only the ack'd opcode may unroot");
        assert!(!modes.hover && !modes.feather_fall && !modes.water_walking && !modes.levitating);

        // The other four follow the wire both ways, which is how `.cheat fly` arrives.
        modes.merge_from_wire(f::SAFE_FALL | f::HOVER | f::WATER_WALKING | f::LEVITATING);
        assert!(modes.hover && modes.feather_fall && modes.water_walking && modes.levitating);
        assert!(modes.rooted, "still ours alone");
    }

    /// Feather fall swaps the terminal velocity and leaves gravity alone (`0x7c5d20`).
    #[test]
    fn feather_fall_caps_the_descent_without_softening_gravity() {
        let dt = 1.0 / 60.0;
        let step = |v: f32, terminal: f32| (v - GRAVITY * dt).max(-terminal);

        assert_eq!(
            step(0.0, FEATHER_TERMINAL_VELOCITY),
            step(0.0, TERMINAL_VELOCITY),
            "the first frame of a slow fall is an ordinary fall"
        );

        let settle = |terminal: f32| {
            let mut v = 0.0;
            for _ in 0..600 {
                v = step(v, terminal);
            }
            v
        };
        assert_eq!(
            settle(FEATHER_TERMINAL_VELOCITY),
            -FEATHER_TERMINAL_VELOCITY
        );
        assert_eq!(settle(TERMINAL_VELOCITY), -TERMINAL_VELOCITY);

        // The cap is reached at t = 7.0 / 19.291105 ≈ 0.363 s.
        let mut v = 0.0f32;
        let mut frames = 0;
        while v > -FEATHER_TERMINAL_VELOCITY {
            v = step(v, FEATHER_TERMINAL_VELOCITY);
            frames += 1;
        }
        assert!(
            (frames as f32 * dt - 0.363).abs() < 0.02,
            "capped after {frames} frames ({:.3} s), expected ≈0.363 s",
            frames as f32 * dt
        );
    }

    #[test]
    fn the_mode_constants_are_the_verified_ones() {
        assert_eq!(FEATHER_TERMINAL_VELOCITY, 7.0); // [0x87d898]
        assert_eq!(HOVER_HEIGHT, 1.0); // [0x7ff9d8]
        assert_eq!(TERMINAL_VELOCITY, 60.148_003); // [0x87d894]
    }

    /// Feather fall is the slower cap.
    const _: () = assert!(FEATHER_TERMINAL_VELOCITY < TERMINAL_VELOCITY);

    /// The stun is `UNIT_FIELD_FLAGS` bit 18 (vmangos `UnitDefines.h:563`, the reference's
    /// `shr eax, 0x12`), not a movement flag.
    #[test]
    fn a_stun_is_not_a_root() {
        assert_eq!(crate::player::UNIT_FLAG_STUNNED, 1 << 18);
        assert_eq!(crate::player::UNIT_FLAG_STUNNED, 0x0004_0000);
        for mode in [
            MoveMode::Root,
            MoveMode::WaterWalk,
            MoveMode::FeatherFall,
            MoveMode::Hover,
        ] {
            assert_ne!(
                mode.flag(),
                crate::player::UNIT_FLAG_STUNNED,
                "{mode:?} is a MOVEMENTFLAGS bit; the stun gate is a descriptor bit"
            );
        }
        // A root alone is only the movement bit.
        let mut modes = MoveModes::default();
        modes.set(MoveMode::Root, true);
        assert_eq!(
            modes.wire_flags(),
            crate::app_core::movement::move_flags::ROOT
        );
    }

    #[test]
    fn hover_climbs_to_its_clearance_rather_than_popping() {
        assert_eq!(HOVER_CLIMB_RATE, 7.0); // [0x87d898], via 0x7c61b0
        let dt = 1.0 / 60.0;
        let mut clearance = 0.0_f32;
        let mut frames = 0;
        while clearance < HOVER_HEIGHT {
            clearance = (clearance + HOVER_CLIMB_RATE * dt).min(HOVER_HEIGHT);
            frames += 1;
        }
        assert!(
            frames > 1,
            "a snap would arrive in one frame; this must not"
        );
        assert!(
            (frames as f32 * dt - HOVER_HEIGHT / HOVER_CLIMB_RATE).abs() < 0.02,
            "climbed in {frames} frames, expected ≈0.143 s"
        );
    }

    /// `Jump(0)` at `0x61a630` is refused by ROOT and FALLING (`0x7c625c`), never by hover
    /// (`0x7c6236`).
    #[test]
    fn a_hover_grant_owes_one_jump_and_only_root_or_falling_eats_it() {
        let granted = || Player {
            hover_launch: true,
            modes: MoveModes {
                hover: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut p = granted();
        assert!(p.take_wire_jump(), "hover does not refuse its own grant");
        assert!(
            !p.take_wire_jump(),
            "one opcode, one jump — the latch is consumed, not level-triggered"
        );

        let mut rooted = granted();
        rooted.modes.rooted = true;
        assert!(!rooted.take_wire_jump(), "ROOT — `0x7c625c test ah,0x30`");
        assert!(
            !rooted.hover_launch,
            "a refused Jump returns failure and nothing retries it (`0x7c6288 xor eax,eax`)"
        );

        let mut falling = granted();
        falling.airborne_since = Some(0.0);
        assert!(
            !falling.take_wire_jump(),
            "FALLING — the same test's other bit"
        );

        let mut ungranted = Player::default();
        assert!(!ungranted.take_wire_jump(), "no opcode, no jump");
    }

    /// Stand states 1-6: sitting, the chairs (the client's 2, the server's 4, 5 and 6) and sleep.
    #[test]
    fn a_seated_body_refuses_the_mouse_turn_and_keeps_the_keyboard_one() {
        let m = MoverInput {
            dead: false,
            view_is_out: false,
        };
        for seated in [1u8, 2, 3, 4, 5, 6] {
            assert!(
                !m.mouse_may_turn_body(false, seated),
                "stand state {seated}: the mouse hand-off is refused at `0x51460c`"
            );
            assert!(
                m.may_turn(false),
                "stand state {seated}: the keyboard turn is not refused — it stands the body"
            );
        }
        assert!(
            m.mouse_may_turn_body(false, 0),
            "standing: the hand-off runs"
        );
        assert!(
            !m.mouse_may_turn_body(true, 0),
            "stunned: refused through `0x5145b0` before the stand state is even read"
        );
        assert!(
            !MoverInput {
                dead: true,
                view_is_out: false
            }
            .mouse_may_turn_body(false, 0),
            "dead: refused through the shared precondition `0x5144e0`"
        );
    }

    #[test]
    fn death_drops_both_movement_input_predicates() {
        // (dead, rooted, stunned) -> (may_translate, may_turn)
        let gate = |dead, rooted, stunned| {
            let m = MoverInput {
                dead,
                view_is_out: false,
            };
            (m.may_translate(rooted), m.may_turn(stunned))
        };

        assert_eq!(
            gate(false, false, false),
            (true, true),
            "alive and free: both predicates pass and every input applies"
        );
        assert_eq!(
            gate(false, true, false),
            (false, true),
            "a PURE root (Frost Nova) takes translation and leaves the pivot — `0x514560` alone"
        );
        assert_eq!(
            gate(false, false, true),
            (true, false),
            "a stun takes the pivot and, by itself, nothing else — `0x5145b0` alone"
        );
        assert_eq!(
            gate(true, false, false),
            (false, false),
            "DEAD, with neither a root nor a stun granted anywhere: both predicates fail their \
             shared precondition `0x5144e0` on health at `0x5144f8`. The turn half is the bug — a \
             corpse is stunned as far as the input tick is concerned, so it cannot be spun with \
             the turn keys or a right-drag, and it does not need the server's root to say so."
        );
        assert_eq!(
            gate(true, true, true),
            (false, false),
            "and death is not additive with either: it is already both"
        );
    }

    /// The teardown leg (`0x5146d6`).
    #[test]
    fn only_both_predicates_down_tears_the_follow_down() {
        assert!(
            !MoverInput::default().torn_down(true, false),
            "a PURE root does NOT end a follow — translate is down, the turn is still up"
        );
        assert!(
            !MoverInput::default().torn_down(false, true),
            "and neither does a pure stun — the emitter never consults `0x5145b0`"
        );
        assert!(
            MoverInput {
                dead: true,
                ..Default::default()
            }
            .torn_down(false, false),
            "DEATH ends it, with nothing granted: it takes both predicates down by itself"
        );
        assert!(
            MoverInput::default().torn_down(true, true),
            "and so does Ice Block, which is root and stun at once"
        );
    }

    /// `0x5144e0`'s far-sight term, read only when the mover is the active player (`0x514537`).
    #[test]
    fn far_sight_freezes_your_own_body_but_never_a_possessed_one() {
        let driving = |driving_own_body, far_sight_engaged| {
            let m = MoverInput {
                dead: false,
                view_is_out: view_is_out(driving_own_body, far_sight_engaged),
            };
            (m.may_translate(false), m.may_turn(false))
        };

        assert_eq!(
            driving(true, false),
            (true, true),
            "my own body, no far sight: the ordinary frame, both predicates pass"
        );
        assert_eq!(
            driving(true, true),
            (false, false),
            "MIND VISION — my view is out and the body is mine, so `0x5144e0` fails and takes the \
             walk and the turn together, exactly as death does"
        );
        assert_eq!(
            driving(false, true),
            (true, true),
            "MIND CONTROL — the latch is engaged (possession sets the same field), but the mover \
             is not the active player, so `0x51453e` returns the conjunct satisfied and the victim \
             stays drivable. Dropping the `IsActivePlayer` half freezes it."
        );
        assert!(
            MoverInput {
                dead: false,
                view_is_out: true,
            }
            .torn_down(false, false),
            "and it ends a follow, with neither a root nor a stun anywhere — `0x5146d6` needs both \
             predicates down, and a term in the precondition they SHARE is both by construction. \
             Engaging Mind Vision mid-follow stops you following, for the same reason dying does."
        );
    }

    #[test]
    fn the_incapacitate_suppressions_take_exactly_their_own_bits() {
        let keys = f::FORWARD | f::STRAFE_LEFT | f::TURN_LEFT;
        let modes = f::ROOT | f::SWIMMING | f::SAFE_FALL | f::HOVER | f::WATER_WALKING;

        assert_eq!(
            incapacitated_flags(keys | modes, false, false),
            keys | modes,
            "neither gate: the word passes through untouched"
        );
        assert_eq!(
            incapacitated_flags(keys | modes, true, false),
            f::TURN_LEFT | modes,
            "a PURE root (Frost Nova) takes the direction bits and leaves the pivot — turn is on \
             the reference's allow-list on purpose"
        );
        assert_eq!(
            incapacitated_flags(keys, false, true),
            f::FORWARD | f::STRAFE_LEFT,
            "the two are keyed on different state: the stun bit alone takes only the pivot"
        );
        // A stun is both: vmangos's `HandleAuraModStun` sets `UNIT_FLAG_STUNNED` and roots
        // (`SpellAuras.cpp:3523`, `:3542`).
        let blocked = incapacitated_flags(keys | modes, true, true);
        assert_eq!(
            blocked, modes,
            "Ice Block leaves NO reported motion — which is what starves the anim resolver"
        );
        // The modes ride on, or the server's next echo clears them.
        assert_eq!(blocked & f::ROOT, f::ROOT, "the root bit itself rides on");
        assert_eq!(
            blocked & f::SWIMMING,
            f::SWIMMING,
            "a rooted swimmer is still swimming — `0xffe07f00` preserves 0x200000"
        );
    }
}
