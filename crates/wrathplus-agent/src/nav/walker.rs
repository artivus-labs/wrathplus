//! The walker: feeds a route to click-to-move one corner at a time, and what it does when a
//! corner is not reached (a push, a route round the bump, then `stuck`) or a leg of a long walk
//! ends (the next leg). A fresh route comes at once from local mmaps, or is asked of the realm
//! (the walk is `routing` until it lands).

use super::{level, route_in, to_bevy, to_wow, Handles, Mesh, NavLink, Plan, Walk};
use crate::nav::realm::Purpose;
use crate::world;
use bevy::prelude::*;
use std::sync::Mutex;
use wrathplus_app_world::player::AutoMove;

/// How often a stuck walk is routed again from where the player stands before it gives up.
pub(super) const REPLANS: u8 = 2;
/// How close (yards, level) a corner counts as reached: click-to-move stops half a yard short.
const REACHED: f32 = 2.0;
/// A leg of a long walk must end at least this far (yards) from where the last one did, or the
/// walk is not getting anywhere.
const LEG_PROGRESS: f32 = 15.0;

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

/// A stuck walk's fresh route from `me` to `goal`, round everything it has bumped into so far,
/// this bump included: from local mmaps, or asked of the realm for `purpose`.
pub(super) fn route_round(
    mesh: &Mutex<Mesh>,
    bumps: &mut Vec<[f32; 3]>,
    me: world::Me,
    corner: [f32; 3],
    goal: [f32; 3],
    purpose: Purpose,
    now: f64,
) -> Result<Plan, String> {
    bumps.push(bump(me.pos, corner));
    route_in(mesh, me, goal, bumps, false, purpose, now)
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

/// What the walker drives: click-to-move (`AutoMove`) and the UI's Lua for the push, or a
/// stand-in under test.
pub(crate) trait Mover {
    fn walking(&self) -> bool;
    fn position(&self) -> Vec3;
    fn walk_to_ground(&mut self, to: Vec3) -> bool;
    /// Run a key verb chunk ([`PUSH`], [`PUSH_END`]); false when there is no VM to run it in.
    fn keys(&mut self, chunk: &str) -> bool;
}

struct Click<'a, 'w, 's> {
    auto: &'a mut AutoMove<'w, 's>,
    script: Option<&'a wrathplus_ui::script::UiScript>,
}

impl Mover for Click<'_, '_, '_> {
    fn walking(&self) -> bool {
        self.auto.walking()
    }
    fn position(&self) -> Vec3 {
        self.auto.position()
    }
    fn walk_to_ground(&mut self, to: Vec3) -> bool {
        self.auto.walk_to_ground(to)
    }
    fn keys(&mut self, chunk: &str) -> bool {
        self.script.is_some_and(|s| s.run(chunk).is_ok())
    }
}

pub(crate) fn walk(
    link: Res<NavLink>,
    mut auto: AutoMove,
    script: Option<NonSend<wrathplus_ui::script::UiScript>>,
    cinematic: Option<Res<wrathplus_app_world::cinematic::Cinematic>>,
) {
    let mut click = Click {
        auto: &mut auto,
        script: script.as_deref(),
    };
    let held = cinematic.is_some_and(|c| c.is_active());
    step(&link.0, &mut click, held);
}

/// End a walk short of its goal, saying why.
pub(super) fn end(walk: &mut Walk, status: &'static str, note: String) {
    walk.status = status;
    walk.note = Some(note);
    walk.queue.clear();
    walk.corner = None;
    walk.then_interact = None;
    walk.leg = false;
}

/// Take a fresh route into the walk: at once (local mmaps), or `routing` while the realm is asked.
fn take(walk: &mut Walk, plan: Plan) {
    match plan {
        Plan::Ready(route) => {
            walk.queue = route.points.iter().skip(1).copied().collect();
            walk.leg = false;
        }
        Plan::Asked => {
            walk.queue.clear();
            walk.status = "routing";
            walk.rerouting = true;
        }
    }
    walk.corner = None;
    walk.retried = false;
}

/// Where the player stands, on the walk's map.
fn me_at(walk: &Walk, at: Vec3) -> world::Me {
    world::Me {
        pos: to_wow(at),
        facing: 0.0,
        map: walk.map,
    }
}

/// Feed the route to click-to-move: when one walk ends, check it reached its corner, then start
/// the next. Click-to-move gives up when it stops closing (a slope too steep, a wall), so a corner
/// it fell short of is tried once more after a push ([`PUSH`]), then routed round
/// ([`route_round`]), then the walk ends as `stuck`.
/// `held`: a cinematic holds the client. Refusals are still announced and a stop still applies,
/// but the walk takes no step (a route that arrives meanwhile included) until it ends.
pub(crate) fn step(nav: &Handles, mover: &mut impl Mover, held: bool) {
    let Ok(mut walk) = nav.walk.lock() else {
        return;
    };
    let walk = &mut *walk;
    if std::mem::take(&mut walk.failed) {
        nav.walk_done("failed");
    }
    if let Some(until) = walk.pushing {
        // The push ends when its time is up, or with the walk.
        if !walk.stop && nav.now() < until {
            return;
        }
        if !mover.keys(PUSH_END) {
            return;
        }
        walk.pushing = None;
    }
    if walk.stop {
        walk.stop = false;
        walk.queue.clear();
        walk.corner = None;
        walk.then_interact = None;
        if matches!(walk.status, "walking" | "routing") {
            walk.status = "stopped";
            walk.leg = false;
            // A route still on its way from the realm is no longer wanted.
            walk.ticket = 0;
        }
        return;
    }
    if held || walk.status != "walking" || mover.walking() {
        return;
    }
    let at = mover.position();
    if let Some(corner) = walk.corner {
        let short = to_bevy(corner).xz().distance(at.xz());
        if short > REACHED {
            if !walk.retried {
                walk.retried = true;
                walk.queue.push_front(corner);
                if mover.keys(PUSH) {
                    walk.pushing = Some(nav.now() + f64::from(push_secs(short)));
                    // The second try starts from wherever the push ends.
                    walk.corner = None;
                    return;
                }
            } else if let Some(plan) = (walk.replans < REPLANS)
                .then_some(walk.goal)
                .flatten()
                .and_then(|goal| {
                    let purpose = Purpose::Walk {
                        ticket: walk.ticket,
                    };
                    let me = me_at(walk, at);
                    route_round(
                        &nav.mesh,
                        &mut walk.bumps,
                        me,
                        corner,
                        goal,
                        purpose,
                        nav.now(),
                    )
                    .ok()
                })
            {
                // As a player clicks again after bumping a doorframe: a fresh route from here.
                walk.replans += 1;
                take(walk, plan);
                return;
            } else {
                let note = format!(
                    "click-to-move stopped {short:.0} yd short of the corner at ({:.0}, {:.0}, {:.0}), twice, after {} fresh routes",
                    corner[0], corner[1], corner[2], walk.replans
                );
                end(walk, "stuck", note);
                nav.walk_done("stuck");
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
            if mover.walk_to_ground(to_bevy(point)) {
                walk.settle = 0;
            } else if walk.settle > 0 {
                // Not taken yet: the same step again next frame.
                walk.settle -= 1;
                walk.queue.push_front(point);
                walk.corner = before;
            } else {
                let note = "click-to-move refused the walk (dead, controlled, or autoInteract off)";
                end(walk, "failed", note.into());
                nav.walk_done("failed");
            }
        }
        // The end of a leg: on toward the goal, as long as legs get nearer.
        None if walk.leg => {
            let here = to_wow(at);
            let moved = walk
                .leg_from
                .is_none_or(|from| level(from, here) > LEG_PROGRESS);
            walk.leg_from = Some(here);
            let next = walk.goal.filter(|_| moved).and_then(|goal| {
                let purpose = Purpose::Walk {
                    ticket: walk.ticket,
                };
                let me = me_at(walk, at);
                route_in(
                    &nav.mesh,
                    me,
                    goal,
                    &walk.bumps,
                    walk.any_height,
                    purpose,
                    nav.now(),
                )
                .ok()
            });
            match next {
                Some(plan) => take(walk, plan),
                None => {
                    let note = "the realm's routes stopped getting nearer the goal";
                    end(walk, "stuck", note.into());
                    nav.walk_done("stuck");
                }
            }
        }
        None => {
            walk.status = "arrived";
            if let Some(guid) = walk.then_interact.take() {
                let _ = nav.orders.send(world::Order::Interact(guid));
            }
            nav.walk_done("arrived");
        }
    }
}
