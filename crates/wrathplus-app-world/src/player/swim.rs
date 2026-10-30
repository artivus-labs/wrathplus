//! Swim mode: deep enough, the avatar leaves the floor and swims in 3D along its pitched facing.
//! The reference's decision `0x6030c0` latches `MOVEFLAG_SWIMMING` (`0x00200000`) on the depth
//! over the feet against 0.75 × the unit's collision height, with a 1/36-yd hysteresis; there is
//! no wade flag (`0x40000000` is HOVER).
//!
//! The floating resolver bypasses gravity: an idle swimmer's depth is frozen, the vertical comes
//! only from the pitched stroke, and the one constraint is a top cap with the feet at
//! `surface - 0.75·h` (`0x632ba0`), satisfied again when the surface falls ([`settle_to_rest`]) and
//! never pulling a deeper swimmer up. A rise into the cap turns level at full speed
//! ([`cap_redirect`]). The way out through the top is the swim jump ([`breach_step`], `0x7c6230`);
//! the way to rise is aiming up and swimming forward (`0x7c6f70`).

use avian3d::prelude::*;
use bevy::prelude::*;

use wrathplus_world::world_point::{Subject, WorldPoint};

use super::mover::Outcome;
use super::{Player, CAPSULE_HEIGHT, GRAVITY, GROUND_COS, GROUND_PROBE, SKIN_WIDTH};
use wrathplus_assets::coords::bevy_to_wow;

/// Swim speed (yd/s), the stock `MOVE_SWIM` (vmangos `Unit.cpp:71`): the fallback until the
/// server's speeds stream in, and under the `$WOW_MOVE_SPEED` run override.
pub(super) const SWIM_SPEED: f32 = 4.722_222;

/// Backward swim speed (yd/s), the stock `MOVE_SWIM_BACK` (vmangos `Unit.cpp:72`), with the same
/// fallback role as [`SWIM_SPEED`].
pub(super) const SWIM_BACK_SPEED: f32 = 2.5;

/// Swim-jump take-off speed (yd/s): `0x7c6230`'s swim seed `0xc1118c48` (-9.096748, down-positive),
/// about 14% over the land jump's 7.955547.
const SWIM_JUMP_SPEED: f32 = 9.096_748;

/// The enter/leave band (yd, `0x7ff9d0`): enter above `0.75·h`, leave below `0.75·h - 1/36`,
/// hold between (`0x603100`, `0x6031c0`).
const SWIM_HYSTERESIS: f32 = 1.0 / 36.0;

#[cfg(test)]
use crate::app_core::movement::{may_swim, CANNOT_SWIM, SWIM_UNIT_FLAGS};

pub(crate) use crate::app_core::movement::swim_enter_depth;
/// The depth (yd) below which swimming stops, `0.75·h - 1/36`: the band is absolute, never scaled
/// by `h` (`0x6030f2` subtracts `[0x7ff9d0]`).
pub(super) fn swim_exit_depth(h: f32) -> f32 {
    swim_enter_depth(h) - SWIM_HYSTERESIS
}

/// The depth (yd) of the top cap a rising swimmer's feet stop at, the floating resolver's top
/// plane (`0x632ba0`). It equals the enter depth, so a capped swimmer never drops out of the latch.
fn rest_cap(h: f32) -> f32 {
    swim_enter_depth(h)
}

/// The Bevy-Y surface of any liquid over the feet: magma and slime are swum in like water. The
/// subject is the player, so its own interior claim picks which room's liquid answers.
pub(super) fn surface_over_feet(world: &WorldPoint, feet: Vec3) -> Option<f32> {
    let wow = bevy_to_wow(feet);
    world
        .liquid_at(Subject::Player, wow)
        .map(|hit| feet.y + (hit.surface_z - wow[2]))
}

/// The swimmer rests at the surface cap ([`rest_cap`]), within a few centimetres: where a climb
/// can only leave the water.
pub(super) fn at_surface(player: &Player, surface_y: Option<f32>) -> bool {
    const BAND: f32 = 0.05;
    surface_y
        .is_some_and(|surface| surface - player.pos.y <= rest_cap(player.collision_height.0) + BAND)
}

/// Latches [`Player::swimming`] as `0x6030c0` does and returns it: enter on a strict
/// `depth > 0.75·h`, leave on `depth < 0.75·h - 1/36` or no liquid at all. Entry also waits out a
/// rising launch until its upward speed halves (`0x7c5de0`, about 0.236 s for a swim jump), so the
/// hop re-latches still rising and its residual velocity is dropped (`0x7c6e50` → `0x7c6290`).
/// `now` is `Time::elapsed_secs`. Never touches [`Player::settling`], the terrain streamer's.
pub(super) fn update_swimming(player: &mut Player, surface_y: Option<f32>, now: f32) -> bool {
    // LEVITATING skips the whole decision (`0x6030d2` test ah,4, to `0x6031fa`): it is GM flight,
    // whose SWIMMING the dry ground would otherwise clear the next frame.
    if player.modes.levitating {
        return player.swimming;
    }
    let was = player.swimming;
    let Some(surface) = surface_y else {
        player.swimming = false; // not in liquid
        stop_pitch(player, was);
        return false;
    };
    let depth = surface - player.pos.y;
    let h = player.collision_height.0;
    player.swimming = if player.swimming {
        depth >= swim_exit_depth(h)
    } else {
        let hop_blocked = player.vel_y > 0.0
            && player
                .airborne_since
                .is_some_and(|t0| now - t0 < player.jump_zspeed / (2.0 * GRAVITY));
        depth > swim_enter_depth(h) && !hop_blocked
    };
    stop_pitch(player, was);
    player.swimming
}

/// A depth-driven stop zeroes the mover pitch, which water walking reads on land: StopSwim
/// `0x7c6e80` (via `0x6031eb` → `0x60dff0` → `0x61a070`) is the pitch's only zeroing writer. A
/// breach keeps its aim, as `CMovement::Jump` (`0x7c6230`) clears SWIMMING through `0x7c61f0`.
fn stop_pitch(player: &mut Player, was: bool) {
    if was && !player.swimming {
        player.mover_pitch = 0.0;
    }
}

/// The take-off frame of a jump while swimming (`0x7c6230`), at any depth: [`SWIM_JUMP_SPEED`],
/// then FALLING. The last swim frame's travel carries the leap, frozen as in every jump
/// (`0x7c61f0` never rewrites it), and there is no gravity tick this frame, so the jump tail
/// carries the exact seed. The caller has already cleared [`Player::swimming`].
pub(super) fn breach_step(
    player: &mut Player,
    time: &Time,
    world: &wrathplus_world::collision::WorldCollision<'_, '_>,
    capsule: &Collider,
) -> Outcome {
    player.vel_y = SWIM_JUMP_SPEED;
    // For the arc bookkeeping, or the jump tail sends `zspeed = 0` and observers see a step-off.
    player.launch_vz = SWIM_JUMP_SPEED;
    // The fresh arc's direction nibble seeds from the swim keys, as a ledge step-off's does.
    player.arc_dirs_set = player.horiz_vel.length_squared() > 0.0;
    player.knock_arc = false;
    let half_h = Vec3::Y * (CAPSULE_HEIGHT * 0.5);
    let out = world.slide_body(
        capsule,
        player.pos + half_h,
        player.horiz_vel + Vec3::Y * player.vel_y,
        time.delta(),
        &MoveAndSlideConfig::default(),
        |_hit| MoveAndSlideHitResponse::Accept,
    );
    player.pos = out.position - half_h;
    Outcome {
        held: false,
        grounded: false,
        jumped: true,
        knocked: false,
        air_nudged: false,
        ground: None,
    }
}

/// One swim step's result, for the controller's flags and animation.
pub(super) struct SwimOutcome {
    /// Walkable floor right under the feet: a shallow bottom.
    pub grounded: bool,
    /// The travel pitch after a surface redirect, for the pose and the wire; `None` when the cap
    /// did not bite. The raw aim stays in [`Player::mover_pitch`].
    pub surface_pitch: Option<f32>,
}

/// Clamps a rising stroke's upward speed to `cap`, the rise that reaches the rest line this
/// frame, and turns the rest level at the same speed; returns the velocity and, when the cap bit,
/// the travel pitch. The 1.12 client is seen swimming level at full speed along the surface; its
/// own-input resolver (`0x634640`) as read slides a steep aim along the cap at `cos(pitch)·speed`,
/// near zero, so the mechanism behind the seen behaviour is untraced.
fn cap_redirect(input_vel: Vec3, cap: f32) -> (Vec3, Option<f32>) {
    if input_vel.y <= 0.0 || input_vel.y <= cap {
        return (input_vel, None);
    }
    let speed = input_vel.length();
    let level_dir = Vec3::new(input_vel.x, 0.0, input_vel.z).normalize_or_zero();
    let level_speed = (speed * speed - cap * cap).max(0.0).sqrt();
    (
        level_dir * level_speed + Vec3::Y * cap,
        Some(cap.atan2(level_speed)),
    )
}

/// How far the feet sit above the rest line: the sink that satisfies it, which [`swim_step`]
/// sweeps so a lakebed can still hold the feet higher.
fn settle_to_rest(feet_y: f32, surface_y: f32, h: f32) -> f32 {
    (feet_y - (surface_y - rest_cap(h))).max(0.0)
}

/// The surface [`swim_step`] caps the swimmer under, or `None` in GM flight: `LEVITATING` lifts
/// the water's authority (`0x6030d2`, as in [`update_swimming`]), so an ascent is free. A
/// momentary sample miss in real water is `Some(own feet Y)`, a hold for the frame. Both arms of
/// the constraint, the rise cap and the settle, must read this one source.
pub(super) fn rest_line(player: &Player, surface_y: Option<f32>) -> Option<f32> {
    (!player.modes.levitating).then(|| surface_y.unwrap_or(player.pos.y))
}

/// One swim frame through the floating resolver (`0x634640`): no gravity, the vertical only from
/// `input_vel`, a slide against the lakebed and banks, the rise capped at the rest line
/// ([`cap_redirect`]) and a stroke left above it settled back ([`settle_to_rest`]). `surface_y`
/// is the waterline where the frame starts, which caps its climb; `surface_at` resamples it where
/// the stroke lands, since on a steep river a frame's descent can be the whole hysteresis band.
pub(super) fn swim_step(
    player: &mut Player,
    time: &Time,
    world: &wrathplus_world::collision::WorldCollision<'_, '_>,
    capsule: &Collider,
    input_vel: Vec3,
    surface_y: Option<f32>,
    surface_at: impl Fn(Vec3) -> Option<f32>,
) -> SwimOutcome {
    let dt = time.delta_secs();
    let half_h = Vec3::Y * (CAPSULE_HEIGHT * 0.5);
    let center = player.pos + half_h;

    // Cap the upward velocity, not the position after the slide: a position clamp overrides the
    // lakebed's collision, so in the shallows the depth never drops below the leave threshold and
    // the swimmer cannot walk out. `None` is GM flight, uncapped.
    let cap = match surface_y {
        Some(surface) if dt > 0.0 => {
            let rest_feet_y = surface - rest_cap(player.collision_height.0);
            ((rest_feet_y - player.pos.y) / dt).max(0.0)
        }
        _ => f32::INFINITY,
    };
    let (vel, surface_pitch) = cap_redirect(input_vel, cap);

    let out = world.slide_body(
        capsule,
        center,
        vel,
        time.delta(),
        &MoveAndSlideConfig::default(),
        |_hit| MoveAndSlideHitResponse::Accept,
    );
    let mut c = out.position;
    // Satisfy the rest line, not only guard it: a swimmer mostly ends up above it by the surface
    // falling to meet them, and a river falling a tenth of a yard per yard sheds the 1/36-yd band
    // within a few frames, flapping the latch between swim and fall. The drop is swept, so a bank
    // still holds the feet up. Only under a stroke, as the resolver's outer gate skips an idle
    // mover (`0x634100` test [esi+0x40],0x200f), and only under the rest line the cap reads, or GM
    // flight over a lake would drop onto the waterline.
    if surface_y.is_some() && input_vel != Vec3::ZERO {
        if let Some(surface_now) = surface_at(c - half_h) {
            let excess = settle_to_rest(c.y - half_h.y, surface_now, player.collision_height.0);
            if excess > 0.0 {
                let drop = world
                    .cast_body(capsule, c, Vec3::NEG_Y * excess, SKIN_WIDTH)
                    .map_or(excess, |h| h.distance.min(excess));
                c.y -= drop;
            }
        }
    }
    player.pos = c - half_h;
    // Swim owns its vertical: a clean zero, so a fall out of the water starts from rest.
    player.vel_y = 0.0;
    player.horiz_vel = Vec3::new(vel.x, 0.0, vel.z);

    let probe = world.cast_body(capsule, c, Vec3::NEG_Y * GROUND_PROBE, SKIN_WIDTH);
    let grounded = probe.is_some_and(|h| h.normal1.y >= GROUND_COS);
    SwimOutcome {
        grounded,
        surface_pitch,
    }
}

/// One swim frame's mover outcome and its presented pitch, for the pose and the wire: the raw aim,
/// or the level travel pitch after a surface redirect.
pub(super) struct SwimFrame {
    pub outcome: Outcome,
    pub pitch: f32,
}

/// One swimming frame: the drunk pitch wobble, the pitched travel basis, the directional speed,
/// the stroke rate and [`swim_step`]. `basis` is the level `(forward, right)` pair, `amounts` the
/// netted `(fwd, side)` translation ([`translate_amounts`]), and `wobble` the drunk angle, zero
/// when it must not apply.
pub(super) fn drive_step(
    player: &mut Player,
    time: &Time,
    collide: &wrathplus_world::collision::WorldCollision<'_, '_>,
    capsule: &Collider,
    world: &WorldPoint,
    surface_y: Option<f32>,
    basis: (Vec3, Vec3),
    amounts: (f32, f32),
    vertical: f32,
    speeds: Option<wrathplus_protocol::MoveSpeeds>,
    move_speed: &super::MoveSpeed,
    wobble: f32,
) -> SwimFrame {
    let (swim_fwd, swim_side) = amounts;
    // Drunk and swimming, the pitch wobbles too, by the facing veer's angle × 4.0 each frame
    // (`0x60aabc`-`0x60ab0a`, `[0x80306c]`, committed through `0x60de70`), clamped to ±π/2
    // (`0x60aba0`, `0x808acc`/`0x80c5e4`), not the mouselook's ±89°.
    if wobble != 0.0 {
        player.mover_pitch = (player.mover_pitch + wobble * super::drunk::SWIM_PITCH_WOBBLE_SCALE)
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }
    let mut pitch = player.mover_pitch;
    // The travel basis (`0x7c5880`): forward is the facing pitched by the swim pitch, strafe stays
    // level, and build 12340's ascend and descend keys (`0x9898e0`, which takes SWIMMING as it takes
    // FLYING) climb or sink straight up or down, the climb still capped at the rest line.
    let (sp, cp) = player.mover_pitch.sin_cos();
    let fwd_axis = basis.0 * cp + Vec3::Y * sp;
    let v = fwd_axis * swim_fwd + basis.1 * swim_side + Vec3::Y * vertical;
    let dir3 = v.normalize_or_zero();
    let rest_line = rest_line(player, surface_y);
    // The directional speed (`0x7c4c90`'s swim arm): backward takes `min(swimBack, swim)`,
    // forward and strafe the swim speed.
    let (swim_speed, swim_back_speed) = match speeds {
        Some(s) if !move_speed.env_override => (s.swim, s.swim_back),
        _ => (SWIM_SPEED, SWIM_BACK_SPEED),
    };
    let dir_speed = if swim_fwd < 0.0 {
        swim_back_speed.min(swim_speed)
    } else {
        swim_speed
    };
    // The stroke rate's numerator is the flag speed whatever the pitch: `0x5fe2f0` divides
    // `GetCurrentSpeed` by the clip's speed, for local and observed units alike.
    player.swim_stroke_speed = if dir3 == Vec3::ZERO { 0.0 } else { dir_speed };
    let out = swim_step(
        player,
        time,
        collide,
        capsule,
        dir3 * dir_speed,
        rest_line,
        |feet| surface_over_feet(world, feet),
    );
    // After a surface redirect the pose and the wire show the travel pitch, while the raw aim
    // stays in `mover_pitch` so a nose-down dives at once.
    if let Some(p) = out.surface_pitch {
        pitch = p;
    }
    let outcome = Outcome {
        held: false,
        grounded: out.grounded,
        jumped: false,
        // A knockback clears SWIMMING at take-off (`StartFalling`, `0x7c61f0`), so the land
        // mover flies it.
        knocked: false,
        air_nudged: false,
        ground: None, // swimming detaches from any platform frame below
    };
    SwimFrame { outcome, pitch }
}

/// The swim translation `(fwd, side)`, read by [`drive_step`] and the flag build alike so the two
/// agree; under mouselook A/D strafe.
pub(super) fn translate_amounts(
    axes: &super::input::MoveAxes,
    translate_gated: bool,
) -> (f32, f32) {
    // The translate predicate `0x514560` (a root, or death) cuts swim translation as it cuts the
    // walk; the water reads its own axes, so it needs its own cut.
    if translate_gated {
        return (0.0, 0.0);
    }
    let swim_fwd = axes.fwd.signum() as f32;
    let mut swim_side = 0.0_f32; // +right
    if axes.strafe_right {
        swim_side += 1.0;
    }
    if axes.strafe_left {
        swim_side -= 1.0;
    }
    if axes.mouselook {
        if axes.turn_right {
            swim_side += 1.0;
        }
        if axes.turn_left {
            swim_side -= 1.0;
        }
    }
    (swim_fwd, swim_side)
}

#[cfg(test)]
#[path = "swim_tests.rs"]
mod tests;
