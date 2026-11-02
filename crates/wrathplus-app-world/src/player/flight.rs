//! Build 12340 flight (`MOVEMENTFLAG_FLYING`, with `CAN_FLY` granted by `SMSG_MOVE_SET_CAN_FLY`).
//! The mover flies gravity-free through the swim resolver with no waterline: forward along the
//! pitched facing, strafe level, and the ascend and descend keys straight up and down (`Wow.exe`
//! `0x9898e0`), at the server's flight speeds. `https://github.com/artivus-labs/wrathplus/wiki/WRATH-FLIGHT` has the native trace.
//!
//! Take-off is `0x989890`: CAN_FLY, no pending root, and it clears FALLING. Which input queues it
//! (the drain's event `0x2d`) was not traced in the one Ghidra round; here it is the ascend key
//! held while airborne, so a jump held into the air takes off. Landing is `0x98c240`: ground
//! contact while not rising, entering the water, a knockback, or the grant's loss.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::mover::Outcome;
use super::swim::{swim_step, SwimFrame};
use super::Player;

/// Stock `MOVE_FLIGHT` and `MOVE_FLIGHT_BACK` (AzerothCore `baseMoveSpeed`, yd/s): the fallback
/// until the server's speeds stream in, and under the `$WOW_MOVE_SPEED` run override.
pub(super) const FLIGHT_SPEED: f32 = 7.0;
pub(super) const FLIGHT_BACK_SPEED: f32 = 4.5;

/// The vertical key this frame: +1 ascend, -1 descend, 0 neither or both (`0x9898e0` sets one
/// bit, and a second key replaces the first; both held cancel here).
pub(super) fn vertical(ascend: bool, descend: bool) -> f32 {
    f32::from(i8::from(ascend) - i8::from(descend))
}

/// Whether the mover takes off this frame (`0x989890`): granted, airborne, not rooted, and the
/// ascend key held. Swimming takes off nowhere: `0x989890` would clear SWIMMING, but the water's
/// own ascend is not modelled.
pub(super) fn takes_off(player: &Player, airborne: bool, ascend: bool) -> bool {
    player.modes.can_fly
        && !player.flying
        && !player.swimming
        && !player.modes.rooted
        && airborne
        && ascend
}

/// Clears flight (`0x98c240`); the pitch stays as mouse look left it, since the mover pitch is live
/// in every mode, but the wire sends none without FLYING.
pub(super) fn land(player: &mut Player) {
    player.flying = false;
}

/// One flying frame. `basis` is the level `(forward, right)` pair, `amounts` the netted
/// `(fwd, side)` translation (as swimming nets it) and `vertical` the ascend/descend key.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_step(
    player: &mut Player,
    time: &Time,
    collide: &wrathplus_world::collision::WorldCollision<'_, '_>,
    capsule: &Collider,
    basis: (Vec3, Vec3),
    amounts: (f32, f32),
    vertical: f32,
    speeds: Option<wrathplus_protocol::MoveSpeeds>,
    move_speed: &super::MoveSpeed,
) -> SwimFrame {
    let (fwd, side) = amounts;
    let pitch = player.mover_pitch;
    let (sp, cp) = pitch.sin_cos();
    let fwd_axis = basis.0 * cp + Vec3::Y * sp;
    let v = fwd_axis * fwd + basis.1 * side + Vec3::Y * vertical;
    let dir = v.normalize_or_zero();
    let (flight, flight_back) = match speeds {
        Some(s) if !move_speed.env_override => (s.flight, s.flight_back),
        _ => (FLIGHT_SPEED, FLIGHT_BACK_SPEED),
    };
    // `GetCurrentSpeed`'s flight arm, as the swim arm: backward takes the smaller.
    let speed = if fwd < 0.0 {
        flight_back.min(flight)
    } else {
        flight
    };
    let input_vel = dir * speed;
    // No waterline: the resolver's cap is open and nothing settles the flier.
    let out = swim_step(player, time, collide, capsule, input_vel, None, |_| None);
    // Ground contact lands a flier that is not rising.
    if out.grounded && input_vel.y <= 0.0 {
        land(player);
    }
    SwimFrame {
        outcome: Outcome {
            held: false,
            grounded: out.grounded,
            jumped: false,
            knocked: false,
            air_nudged: false,
            ground: None,
        },
        pitch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vertical_key_is_one_direction_or_none() {
        assert_eq!(vertical(true, false), 1.0);
        assert_eq!(vertical(false, true), -1.0);
        assert_eq!(vertical(true, true), 0.0);
        assert_eq!(vertical(false, false), 0.0);
    }

    #[test]
    fn take_off_needs_the_grant_the_air_and_the_ascend_key() {
        let mut player = Player::default();
        assert!(!takes_off(&player, true, true), "no CAN_FLY");
        player.modes.can_fly = true;
        assert!(takes_off(&player, true, true));
        assert!(
            !takes_off(&player, false, true),
            "on the ground the key jumps"
        );
        assert!(
            !takes_off(&player, true, false),
            "a plain fall stays a fall"
        );
        player.modes.rooted = true;
        assert!(
            !takes_off(&player, true, true),
            "0x989890 refuses a pending root"
        );
        player.modes.rooted = false;
        player.swimming = true;
        assert!(!takes_off(&player, true, true));
        player.swimming = false;
        player.flying = true;
        assert!(!takes_off(&player, true, true), "already flying");
    }
}
