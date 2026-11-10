//! The walker: feeds a route to click-to-move one corner at a time, and what it does when a
//! corner is not reached: a push, a route round the bump, then `stuck`.

use super::{route_in, to_bevy, to_wow, Mesh, NavLink};
use crate::world;
use bevy::prelude::*;
use std::sync::Mutex;
use wrathplus_app_world::player::AutoMove;
use wrathplus_nav::Route;

/// How often a stuck walk is routed again from where the player stands before it gives up.
pub(super) const REPLANS: u8 = 2;

/// How far ahead of the player (yards, toward the corner fallen short of) a bump is marked, and
/// the circle a fresh route goes round. The navmesh hugs corners closer than the client's
/// collision lets a player pass (the timber foot at the corner of Goldshire's inn, #82), and a
/// route from the same spot over the same mesh is the same route, so a fresh one has to be told
/// what was in the way.
pub(super) const BUMP_AHEAD: f32 = 1.5;
pub(super) const BUMP_RADIUS: f32 = 3.0;

/// The circle (`[x, y, radius]`, WoW) marking what a player at `at` bumped into on the way to
/// `corner`: just ahead, or the corner itself when that is nearer.
pub(super) fn bump(at: [f32; 3], corner: [f32; 3]) -> [f32; 3] {
    let (dx, dy) = (corner[0] - at[0], corner[1] - at[1]);
    let far = dx.hypot(dy);
    let k = if far > f32::EPSILON {
        far.min(BUMP_AHEAD) / far
    } else {
        0.0
    };
    [at[0] + dx * k, at[1] + dy * k, BUMP_RADIUS]
}

/// A stuck walk's fresh route from `at` to `goal`, round everything it has bumped into so far,
/// this bump included.
pub(super) fn route_round(
    mesh: &Mutex<Mesh>,
    bumps: &mut Vec<[f32; 3]>,
    me: world::Me,
    corner: [f32; 3],
    goal: [f32; 3],
) -> Option<Route> {
    bumps.push(bump(me.pos, corner));
    route_in(mesh, me, goal, bumps).ok()
}

/// Before the second try at a corner, a push: forward held with a jump, as a player hops what a
/// walk bumps into, for [`push_secs`]. The server's navmesh steps over what its creatures climb (a
/// low wall, a kerb, the foot of a timber), which the client's collision stops a walking player
/// at (Goldshire's graveyard wall on the way to Stormwind, #82); and click-to-move gives up on
/// the first frame that gains no ground, where holding the key walks free (beside the same
/// graveyard's spirit healer). The keys' own verbs, as an agent's Lua presses them; the first try
/// stopped facing the corner, so forward is toward it.
pub(super) const PUSH: &str = "MoveForwardStart() JumpOrAscendStart() AscendStop()";
pub(super) const PUSH_END: &str = "MoveForwardStop()";

/// How long the push holds forward: the run to the corner (`short` yards at the 7 yd/s run), no
/// less than a jump's worth and no more than carries a few yards.
pub(super) fn push_secs(short: f32) -> f32 {
    (short / 7.0).clamp(0.3, 0.7)
}

/// How many frames a new walk waits for click-to-move to take its first step. `AgentMoveTo` turns
/// `autoInteract` on, and the option reaches click-to-move a frame or two after the CVar is set:
/// on a client that had it off (a new agent's first walk) the first step was refused and the
/// walk failed, though the same call a moment later walked.
pub(super) const SETTLE: u8 = 30;

/// Put a walk's end on the event stream.
fn announce(link: &NavLink, status: &str) {
    link.events.push([serde_json::json!({
        "time": link.started.elapsed().as_secs_f64(),
        "event": "AGENT_WALK_DONE",
        "args": [status],
    })]);
}

/// How close (yards, level) a corner counts as reached: click-to-move stops half a yard short.
const REACHED: f32 = 2.0;

/// Feed the route to click-to-move: when one walk ends, check it reached its corner, then start
/// the next. Click-to-move gives up when it stops closing (a slope too steep, a wall), so a corner
/// it fell short of is tried once more after a push ([`PUSH`]), then routed round
/// ([`route_round`]), then the walk ends as `stuck`.
pub(crate) fn walk(
    link: Res<NavLink>,
    mut auto: AutoMove,
    script: Option<NonSend<wrathplus_ui::script::UiScript>>,
) {
    let Ok(mut walk) = link.walk.lock() else {
        return;
    };
    let walk = &mut *walk;
    if std::mem::take(&mut walk.failed) {
        announce(&link, "failed");
    }
    if let Some(until) = walk.pushing {
        // The push ends when its time is up, or with the walk.
        if !walk.stop && std::time::Instant::now() < until {
            return;
        }
        let Some(script) = &script else {
            return;
        };
        let _ = script.run(PUSH_END);
        walk.pushing = None;
    }
    if walk.stop {
        walk.stop = false;
        walk.queue.clear();
        walk.corner = None;
        walk.then_interact = None;
        if walk.status == "walking" {
            walk.status = "stopped";
        }
        return;
    }
    if walk.status != "walking" || auto.walking() {
        return;
    }
    if let Some(corner) = walk.corner {
        let at = auto.position();
        let short = to_bevy(corner).xz().distance(at.xz());
        if short > REACHED {
            if !walk.retried {
                walk.retried = true;
                walk.queue.push_front(corner);
                if script.as_ref().is_some_and(|s| s.run(PUSH).is_ok()) {
                    let secs = std::time::Duration::from_secs_f32(push_secs(short));
                    walk.pushing = Some(std::time::Instant::now() + secs);
                    // The second try starts from wherever the push ends.
                    walk.corner = None;
                    return;
                }
            } else if let Some(route) = (walk.replans < REPLANS)
                .then_some(walk.goal)
                .flatten()
                .and_then(|goal| {
                    let me = world::Me {
                        pos: to_wow(at),
                        facing: 0.0,
                        map: walk.map,
                    };
                    route_round(&link.mesh, &mut walk.bumps, me, corner, goal)
                })
            {
                // As a player clicks again after bumping a doorframe: a fresh route from here.
                walk.replans += 1;
                walk.retried = false;
                walk.queue = route.points.iter().skip(1).copied().collect();
                walk.corner = None;
                return;
            } else {
                walk.status = "stuck";
                walk.note = Some(format!(
                    "click-to-move stopped {short:.0} yd short of the corner at ({:.0}, {:.0}, {:.0}), twice, after {} fresh routes",
                    corner[0], corner[1], corner[2], walk.replans
                ));
                walk.queue.clear();
                walk.corner = None;
                walk.then_interact = None;
                announce(&link, "stuck");
                return;
            }
        } else {
            walk.retried = false;
        }
    }
    let before = walk.corner;
    walk.corner = walk.queue.front().copied();
    match walk.queue.pop_front() {
        Some(point) => {
            if auto.walk_to_ground(to_bevy(point)) {
                walk.settle = 0;
            } else if walk.settle > 0 {
                // Not taken yet: the same step again next frame.
                walk.settle -= 1;
                walk.queue.push_front(point);
                walk.corner = before;
            } else {
                walk.status = "failed";
                walk.note = Some(
                    "click-to-move refused the walk (dead, controlled, or autoInteract off)".into(),
                );
                walk.queue.clear();
                walk.corner = None;
                walk.then_interact = None;
                announce(&link, "failed");
            }
        }
        None => {
            walk.status = "arrived";
            if let Some(guid) = walk.then_interact.take() {
                let _ = link.orders.send(world::Order::Interact(guid));
            }
            announce(&link, "arrived");
        }
    }
}
