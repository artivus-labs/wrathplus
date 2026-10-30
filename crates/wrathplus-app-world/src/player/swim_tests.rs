use super::*;

/// `CreatureModelData.collisionHeight × displayScale` for three shipped bodies, as
/// `wrathplus_formats`' `collision_height_is_the_m2_collision_box` pins them.
const HUMAN_MALE: f32 = 2.031;
const GNOME_FEMALE: f32 = 1.150; // 1.000 column × 1.15 display scale
const NIGHT_ELF_MALE: f32 = 2.438;

fn player_at(y: f32) -> Player {
    player_of_height(y, HUMAN_MALE)
}

fn player_of_height(y: f32, h: f32) -> Player {
    Player {
        pos: Vec3::new(0.0, y, 0.0),
        collision_height: crate::app_core::state::CollisionHeight(h),
        ..Default::default()
    }
}

/// The production controller calls the depth latch before the mover and caps rising strokes
/// at 0.75*h. Surface swimming must retain SWIMMING, not merely render underwater correctly.
#[test]
fn deep_water_entry_underwater_surface_exit_and_waterwalk_are_distinct() {
    const SURFACE: f32 = 297.6025;
    let mut p = player_at(SURFACE - 0.5);
    assert!(!update_swimming(&mut p, Some(SURFACE), 0.0), "wading");
    p.pos.y = SURFACE - HUMAN_MALE - 1.0;
    assert!(
        update_swimming(&mut p, Some(SURFACE), 0.1),
        "deep-water entry"
    );
    let (_, pitch) = cap_redirect(Vec3::new(3.0, 3.0, 0.0), 0.0);
    assert_eq!(pitch, Some(0.0), "rising travel levels at the cap");
    p.pos.y = SURFACE - rest_cap(HUMAN_MALE);
    for frame in 0..10 {
        assert!(
            update_swimming(&mut p, Some(SURFACE), 0.2 + frame as f32 * 0.01),
            "a capped surface swimmer must not become a runner"
        );
    }
    p.pos.y = SURFACE - HUMAN_MALE - 1.0;
    assert!(
        update_swimming(&mut p, Some(SURFACE), 0.4),
        "dive below surface"
    );
    p.pos.y = SURFACE - 0.5;
    assert!(
        !update_swimming(&mut p, Some(SURFACE), 0.5),
        "shallow-bank exit"
    );
    assert_eq!(
        super::super::mover::water_floor(false, false, 0.0, Some(SURFACE)),
        None,
        "ordinary water is not a walking floor, even over a deep lakebed"
    );
    assert_eq!(
        super::super::mover::water_floor(true, false, 0.0, Some(SURFACE)),
        Some(SURFACE),
        "the water-walking grant supplies the distinct surface floor"
    );
    assert_eq!(
        super::super::mover::water_floor(true, true, 0.0, Some(SURFACE)),
        None,
        "an already submerged swimmer does not stand on the granted floor"
    );
    assert!(!update_swimming(&mut p, None, 0.6), "dry exit");
}

#[test]
fn the_surface_is_the_rest_cap_and_not_below_it() {
    let player = player_at(0.0);
    let cap = rest_cap(HUMAN_MALE);
    assert!(at_surface(&player, Some(cap)));
    assert!(at_surface(&player, Some(cap + 0.04)));
    assert!(!at_surface(&player, Some(cap + 0.5)), "a diver is below it");
    assert!(!at_surface(&player, None));
}

#[test]
fn leaving_the_water_by_depth_levels_the_pitch_and_a_breach_does_not() {
    let deep = swim_enter_depth(HUMAN_MALE) + 1.0;
    let mut player = player_at(0.0);
    player.mover_pitch = -0.9;
    assert!(update_swimming(&mut player, Some(deep), 0.0), "must swim");
    assert_eq!(player.mover_pitch, -0.9, "swimming holds the aim");
    let band = swim_exit_depth(HUMAN_MALE) + 0.001;
    assert!(update_swimming(&mut player, Some(band), 0.1));
    assert_eq!(player.mover_pitch, -0.9);
    // Out through the bottom of the band.
    assert!(!update_swimming(&mut player, Some(0.5), 0.2));
    assert_eq!(
        player.mover_pitch, 0.0,
        "`0x7c6e80` zeroes +0x20 on StopSwim"
    );
    // No liquid at all is the same stop.
    player.mover_pitch = -0.9;
    assert!(update_swimming(&mut player, Some(deep), 1.0));
    assert!(!update_swimming(&mut player, None, 1.1));
    assert_eq!(player.mover_pitch, 0.0);
    // A breach clears the latch itself (`0x7c61f0`), so the next tick sees no stop.
    player.mover_pitch = -0.9;
    assert!(update_swimming(&mut player, Some(deep), 2.0));
    player.swimming = false; // as the controller's breach arm does
    breach_pitch_survives(&mut player, deep);
    assert_eq!(player.mover_pitch, -0.9, "a dolphin-hop keeps its aim");
}

/// One latch tick after a breach has cleared it.
fn breach_pitch_survives(player: &mut Player, depth: f32) {
    player.vel_y = 0.0;
    update_swimming(player, Some(depth), 2.1);
}

/// 12340 `0x4f53d0`: any one of four bits admits, and `CANNOT_SWIM` (0x4000) vetoes them all.
#[test]
fn the_swim_gate_is_four_unit_flags_unless_cannot_swim() {
    const PLAYER_CONTROLLED: u32 = 0x8;
    const RENAME: u32 = 0x10;
    const PET_IN_COMBAT: u32 = 0x800;
    const USE_SWIM_ANIMATION: u32 = 0x8000;
    assert_eq!(
        SWIM_UNIT_FLAGS,
        PLAYER_CONTROLLED | RENAME | PET_IN_COMBAT | USE_SWIM_ANIMATION
    );
    for bit in [PLAYER_CONTROLLED, RENAME, PET_IN_COMBAT, USE_SWIM_ANIMATION] {
        assert!(may_swim(bit), "any ONE admits ({bit:#x})");
        assert!(!may_swim(bit | CANNOT_SWIM), "CANNOT_SWIM vetoes {bit:#x}");
    }
    assert!(!may_swim(0), "no flags at all → walks the lakebed");
}

#[test]
fn swim_entry_and_exit_hysteresis() {
    for h in [HUMAN_MALE, GNOME_FEMALE, NIGHT_ELF_MALE] {
        assert!((swim_enter_depth(h) - swim_exit_depth(h) - 1.0 / 36.0).abs() < 1e-6);
        assert!((swim_enter_depth(h) - 0.75 * h).abs() < 1e-6);
    }

    let (enter, exit) = (swim_enter_depth(HUMAN_MALE), swim_exit_depth(HUMAN_MALE));
    let mut p = player_at(0.0); // feet at 0, so the surface height is the depth
    let mid_band = (enter + exit) * 0.5;
    assert!(
        !update_swimming(&mut p, Some(enter), 0.0),
        "exactly at the enter depth does not yet swim (strict >)"
    );
    assert!(
        !update_swimming(&mut p, Some(mid_band), 0.0),
        "a depth in the band, entered from walking, stays walking"
    );
    assert!(
        update_swimming(&mut p, Some(enter + 0.01), 0.0),
        "past the enter depth starts swimming"
    );
    assert!(
        update_swimming(&mut p, Some(mid_band), 0.0),
        "the same in-band depth, now entered from swimming, keeps swimming (hysteresis)"
    );
    assert!(
        !update_swimming(&mut p, Some(exit - 0.01), 0.0),
        "dropping below the leave threshold returns to walking"
    );
    assert!(update_swimming(&mut p, Some(enter + 0.5), 0.0));
    assert!(
        !update_swimming(&mut p, None, 0.0),
        "no liquid over the feet stops swimming (the binary's inLiquid == 0 → STOP)"
    );
}

#[test]
fn levitating_bails_the_water_decision_in_both_directions() {
    // `.cheat fly on` sets SWIMMING and LEVITATING (`Player.cpp:4595`); on dry ground.
    let mut flying = player_at(0.0);
    flying.swimming = true;
    flying.modes.levitating = true;
    assert!(
        update_swimming(&mut flying, None, 0.0),
        "no liquid at all must NOT stop a levitating swimmer — this is the whole of GM flight"
    );
    assert!(
        update_swimming(&mut flying, Some(-50.0), 0.0),
        "nor may a surface far below the feet, which is what flying over a lake looks like"
    );

    let mut dry = player_at(0.0);
    dry.modes.levitating = true;
    assert!(
        !update_swimming(&mut dry, Some(swim_enter_depth(HUMAN_MALE) + 1.0), 0.0),
        "the ENTER arm is skipped too — the bail is before the branch, not inside it"
    );

    // `.cheat fly off` sends flags 0.
    flying.modes.levitating = false;
    assert!(
        !update_swimming(&mut flying, None, 0.0),
        "with the mode gone, no liquid stops swimming again"
    );
}

#[test]
fn the_latch_leaves_the_settle_hold_alone() {
    for (surface, name) in [
        (Some(swim_enter_depth(HUMAN_MALE) + 1.0), "swim depth"),
        (Some(swim_exit_depth(HUMAN_MALE) - 0.5), "wading depth"),
        (None, "dry land"),
    ] {
        let mut p = player_at(0.0);
        p.settling = true;
        update_swimming(&mut p, surface, 0.0);
        assert!(p.settling, "{name} must leave the hold to the streamer");
    }
}

#[test]
fn the_hop_relatches_at_half_launch_velocity() {
    let half_decay = SWIM_JUMP_SPEED / (2.0 * GRAVITY);
    let deep = Some(swim_enter_depth(HUMAN_MALE) + 1.0);
    let mut p = player_at(0.0);
    p.airborne_since = Some(0.0);
    p.jump_zspeed = SWIM_JUMP_SPEED;
    p.vel_y = SWIM_JUMP_SPEED;
    assert!(
        !update_swimming(&mut p, deep, half_decay * 0.5),
        "young launch, still fast — the hop keeps rising"
    );
    p.vel_y = SWIM_JUMP_SPEED * 0.49;
    assert!(
        update_swimming(&mut p, deep, half_decay + 1e-3),
        "velocity decayed to half — swim re-latches while STILL rising (the ~1.6 yd hop top)"
    );
    let mut q = player_at(0.0);
    q.airborne_since = Some(0.0);
    q.jump_zspeed = SWIM_JUMP_SPEED;
    q.vel_y = -0.1;
    assert!(
        update_swimming(&mut q, deep, 0.01),
        "descending into depth enters swim — the gate only guards a rising launch"
    );
}

#[test]
fn the_rest_line_is_satisfied_from_above_and_never_pulls_up() {
    for h in [HUMAN_MALE, GNOME_FEMALE, NIGHT_ELF_MALE] {
        let cap = rest_cap(h);
        assert_eq!(settle_to_rest(10.0 - cap, 10.0, h), 0.0);
        assert_eq!(settle_to_rest(10.0 - cap - 0.01, 10.0, h), 0.0);
        assert_eq!(settle_to_rest(10.0 - cap - 30.0, 10.0, h), 0.0);
        // Above it, as when a river's surface comes down to the feet: sink exactly the excess.
        let excess = 0.25;
        assert!((settle_to_rest(10.0 - cap + excess, 10.0, h) - excess).abs() < 1e-6);
    }
}

#[test]
fn gm_flight_hands_the_water_no_authority_over_the_vertical() {
    let lake_surface = 40.0;

    // 300 yd above a lake in GM flight: the liquid query still answers `Some` under us.
    let mut flying = player_at(lake_surface + 300.0);
    flying.swimming = true;
    flying.modes.levitating = true;
    assert_eq!(
        rest_line(&flying, Some(lake_surface)),
        None,
        "no rest line while levitating — this is the gate BOTH arms take, not just the cap"
    );
    assert!(
        settle_to_rest(flying.pos.y, lake_surface, flying.collision_height.0) > 299.0,
        "the ungated settle's drop is the whole altitude — that was the yank"
    );

    flying.modes.levitating = false;
    assert_eq!(rest_line(&flying, Some(lake_surface)), Some(lake_surface));

    let swimmer = player_at(lake_surface - 2.0);
    assert_eq!(
        rest_line(&swimmer, None),
        Some(swimmer.pos.y),
        "a chunk-seam miss must stay a Some, or it is indistinguishable from GM flight"
    );
}

#[test]
fn every_race_floats_with_its_head_out_of_the_water() {
    for (label, h) in [
        ("human male", HUMAN_MALE),
        ("gnome female", GNOME_FEMALE),
        ("night elf male", NIGHT_ELF_MALE),
    ] {
        let submerged = rest_cap(h);
        assert!(
            submerged < h,
            "{label}: rest line {submerged} is at or below the top of a {h}-yd body"
        );
        assert!(((h - submerged) / h - 0.25).abs() < 1e-6, "{label}");
    }

    // One shared height would put a gnome's rest line above her head.
    let shared_rest = rest_cap(crate::app_core::movement::DEFAULT_COLLISION_HEIGHT);
    assert!(
        shared_rest > GNOME_FEMALE,
        "the old constant rest line ({shared_rest}) must sit above a {GNOME_FEMALE}-yd gnome — \
         that was the bug, and if this ever fails the test below is no longer testing it"
    );
    assert!(
        shared_rest - GNOME_FEMALE > 0.3,
        "…by a third of a yard of water over her head, not a rounding error"
    );
}

/// `SLOPE` is Felwood's Felfire Hill channel, as wrathplus-world's
/// `liquid::real_data::the_felfire_channel_falls_about_a_tenth_of_a_yard_per_yard` measures it.
#[test]
fn a_descending_surface_does_not_flap_the_swim_latch() {
    const SLOPE: f32 = 0.099;
    const DT: f32 = 1.0 / 60.0;
    const H: f32 = HUMAN_MALE;
    let cap = rest_cap(H);
    let surface_after = |secs: f32| 100.0 - SLOPE * SWIM_SPEED * secs;

    // Frozen feet: the water leaves them behind.
    let mut frozen = player_of_height(surface_after(0.0) - cap, H);
    frozen.swimming = true;
    let mut left_at = None;
    for i in 0..600 {
        let t = i as f32 * DT;
        if !update_swimming(&mut frozen, Some(surface_after(t)), t) {
            left_at = Some(t);
            break;
        }
    }
    let left_at = left_at.expect("frozen feet cannot hold the latch on a slope");
    assert!(
        left_at < 0.1,
        "the whole 1/36-yd band is spent in {left_at:.3}s of downhill swimming"
    );

    // Settled after each stroke, on the surface where the stroke landed, as `swim_step` does.
    let mut held = player_of_height(surface_after(0.0) - cap, H);
    held.swimming = true;
    for i in 0..600 {
        let t = i as f32 * DT;
        let surface = surface_after(t);
        held.pos.y -= settle_to_rest(held.pos.y, surface, H);
        assert!(
            update_swimming(&mut held, Some(surface), t),
            "left swim at frame {i} ({t:.2}s)"
        );
        assert!(
            (surface - held.pos.y - cap).abs() < 1e-4,
            "frame {i}: depth {} is off the rest line",
            surface - held.pos.y
        );
    }
}

#[test]
fn the_rest_line_redirects_the_stroke_level_at_full_speed() {
    let free = Vec3::new(0.1, 3.0, 0.2);
    assert_eq!(cap_redirect(free, 100.0), (free, None));
    assert_eq!(cap_redirect(free, f32::INFINITY), (free, None));
    let dive = Vec3::new(1.0, -2.0, 0.0);
    assert_eq!(cap_redirect(dive, 0.0), (dive, None));

    let steep = Vec3::new(0.08, 4.72, 0.0); // ~89° aim, forward at swim speed
    let (vel, pitch) = cap_redirect(steep, 0.0);
    assert!((vel.length() - steep.length()).abs() < 1e-4, "speed kept");
    assert_eq!(vel.y, 0.0, "level at the line");
    assert!(vel.x > 4.7, "the whole speed went forward");
    assert_eq!(pitch, Some(0.0), "the presented pitch levels out");

    // A partial cap, approaching the line.
    let (vel2, pitch2) = cap_redirect(steep, 2.0);
    assert!((vel2.length() - steep.length()).abs() < 1e-4);
    assert_eq!(vel2.y, 2.0);
    let aim = steep.y.atan2(steep.x);
    let eased = pitch2.expect("the cap bit");
    assert!(eased > 0.0 && eased < aim, "between level and the raw aim");
}
