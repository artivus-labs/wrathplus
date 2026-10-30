//! A server-moved unit's swim state is the liquid over its own feet against its own collision
//! height (12340 `0x730d10`), not a flag the spline carries. These predicate fixtures do not
//! establish whether a server surface pose represents wading: they have no lakebed or live
//! per-unit liquid/height evidence. A unit past the line swims; a `CANNOT_SWIM` unit never.

use super::spline::creature_swim_state;
use crate::app_core::movement::{swim_enter_depth, CANNOT_SWIM};

/// The orc male's collision height, as the session's own swim entries put it (0.75·h ≈ 1.55).
const ORC_MALE: f32 = 2.07;
const PLAYER_CONTROLLED: u32 = 0x8;

#[test]
fn a_unit_walked_just_under_the_surface_wades_and_one_past_the_line_swims() {
    let line = swim_enter_depth(ORC_MALE);
    let bot = 0.0 - (-1.2028_f32);
    assert!(
        !creature_swim_state(false, bot, line, PLAYER_CONTROLLED),
        "the bot wades"
    );
    assert!(creature_swim_state(
        false,
        line + 0.01,
        line,
        PLAYER_CONTROLLED
    ));
    // Inside the exit band a swimmer stays; below it, it stops.
    assert!(creature_swim_state(
        true,
        line - 0.02,
        line,
        PLAYER_CONTROLLED
    ));
    assert!(!creature_swim_state(
        true,
        line - 0.03,
        line,
        PLAYER_CONTROLLED
    ));
}

#[test]
fn a_cannot_swim_unit_walks_the_lakebed_at_any_depth() {
    let line = swim_enter_depth(ORC_MALE);
    let flags = PLAYER_CONTROLLED | 0x8000 | CANNOT_SWIM;
    assert!(!creature_swim_state(false, 20.0, line, flags));
    assert!(
        !creature_swim_state(true, 20.0, line, flags),
        "and is stopped mid-swim"
    );
}
