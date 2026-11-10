//! Routes and walking for agents (#68): paths over the realm's mmaps navmesh, walked through the
//! client's click-to-move one corner at a time, so the character moves as a player clicking along
//! the way would.
//!
//! The navmesh is local when the client has one: the mmaps folder in `WOW_NAV_MMAPS` (the launcher
//! passes `<server data>/mmaps`), searched by `wrathplus-nav` with the server's own Detour.
//! Without one, or for a map it lacks, the realm is asked (`realm`: `mod-agent-nav` answers route
//! requests over the game connection, probed on entering the world); the same search runs there.
//! A realm route takes a round trip, so the walk is `routing` until it arrives. `/status` says
//! which source is in use.
//!
//! Lua (`api`): `AgentPath(x, y[, z] | guid)` → `{complete, length, points = {{x, y, z}, ...}}`
//! (without `z` the floor is looked for: the player's height first, then others, see `heights`;
//! a realm asked for the route runs the same search); `AgentMoveTo(x, y[, z] | guid)` starts
//! walking; `AgentInteract(guid)` from farther than a few
//! yards walks the route, then interacts; `AgentStop()`; `AgentWalk()` → `{status,
//! remaining, goal, note, route, interact}` (`route`: the corners still ahead; `interact`: the
//! guid the walk ends talking to) with status `idle`, `routing` (the realm's route is on its way),
//! `walking`, `arrived`, `stopped`, `failed` (refused, or no route) or `stuck` (a corner not
//! reached). Each end but `stopped` is put on the event stream as `AGENT_WALK_DONE` with the
//! status. `AgentAvoid({{x, y, radius}, ...})` gives circles every later route goes round when a
//! fair detour exists (an agent's hostile mobs); `AgentAvoid({})` clears.
//!
//! While a cinematic holds the client (`world::CINEMATIC_NOTE`), `AgentMoveTo` refuses (`nil`, the
//! reason; the walk reads `failed`, announced once however often it is asked), `AgentInteract`
//! takes the straight approach, and `AgentWalk()` carries the reason in `note` with
//! `cinematic = true`. A walk already under way, or a realm route that arrives meanwhile, waits:
//! the walker takes no step until the cinematic ends, then carries on.
//!
//! A corner click-to-move falls short of is tried again after a push (forward held with a jump),
//! then the walk is routed round the spot, twice at most, before it ends as `stuck` (`walker`).

mod api;
mod realm;
mod services;
mod walker;

#[cfg(test)]
use api::pump;
pub(crate) use api::{exchange, install, status, take_lines};
pub(crate) use services::install as install_services;
pub(crate) use walker::walk;
use walker::{PUSH_END, SETTLE};

use crate::world;
use bevy::prelude::*;
use realm::{Ask, Outcome, Purpose, Remote, Source};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use wrathplus_nav::{NavMesh, Route};

#[derive(Default)]
pub(crate) struct Walk {
    queue: VecDeque<[f32; 3]>,
    goal: Option<[f32; 3]>,
    status: &'static str,
    note: Option<String>,
    stop: bool,
    /// The corner click-to-move is walking to, and whether it is the second try.
    corner: Option<[f32; 3]>,
    retried: bool,
    /// Interact with this object on arrival (`AgentInteract` from afar).
    then_interact: Option<u64>,
    /// A walk that failed in Lua (no route), to announce from the walker.
    failed: bool,
    /// The map the route is on, and how often a stuck walk has been routed again from where the
    /// player stood.
    map: u32,
    replans: u8,
    /// Which walk this is: a realm route asked for an earlier one is dropped when it arrives.
    ticket: u32,
    /// The route under way is a leg toward a goal past the realm's distance limit, begun here.
    leg: bool,
    leg_from: Option<[f32; 3]>,
    /// The route awaited replaces one the walk was already on (a stuck corner, the next leg).
    rerouting: bool,
    /// The goal's height is the player's, a guess: the route search may look on other levels.
    any_height: bool,
    /// Where this walk bumped into something the navmesh does not know (`[x, y, radius]`): each
    /// fresh route goes round them, with `AgentAvoid`'s circles.
    bumps: Vec<[f32; 3]>,
    /// Frames a new walk's first step may still be refused before the walk fails ([`walker::SETTLE`]).
    settle: u8,
    /// When the push before a second try ends ([`walker::PUSH`]), on [`Handles::now`]'s clock;
    /// forward is held until then.
    pushing: Option<f64>,
}

/// `AgentPath`'s last answer from the realm, kept for the call that reads it.
struct PathAnswer {
    from: [f32; 3],
    to: [f32; 3],
    at: f64,
    result: Result<Route, String>,
}

pub(crate) struct Mesh {
    dir: Option<PathBuf>,
    mesh: Option<NavMesh>,
    /// The map the local folder was last found not to cover, and why.
    missing: Option<(u32, String)>,
    /// `AgentAvoid`'s circles, WoW `[x, y, radius]`, applied to each route.
    avoid: Vec<[f32; 3]>,
    remote: Remote,
    path: Option<PathAnswer>,
    /// `AgentServices`: the question out and the last answer.
    services: services::State,
}

/// Everything the Lua functions, the walker and the realm pump share.
#[derive(Clone)]
pub(crate) struct Handles {
    walk: Arc<Mutex<Walk>>,
    mesh: Arc<Mutex<Mesh>>,
    tickets: Arc<AtomicU32>,
    /// Milliseconds added to the clock: tests move time on with it.
    ahead: Arc<AtomicU32>,
    /// Where a walk's end is announced (`AGENT_WALK_DONE`), to wake the tiers above.
    events: Arc<crate::events::EventLog>,
    started: std::time::Instant,
    /// The world's orders, for the interaction a walk ends in.
    orders: crossbeam_channel::Sender<world::Order>,
}

#[derive(Resource)]
pub(crate) struct NavLink(Handles);

pub(crate) fn link(
    events: Arc<crate::events::EventLog>,
    started: std::time::Instant,
    orders: crossbeam_channel::Sender<world::Order>,
) -> (Handles, NavLink) {
    let handles = Handles {
        walk: Arc::new(Mutex::new(Walk {
            status: "idle",
            ..Default::default()
        })),
        mesh: Arc::new(Mutex::new(Mesh {
            dir: std::env::var_os("WOW_NAV_MMAPS").map(PathBuf::from),
            mesh: None,
            missing: None,
            avoid: Vec::new(),
            remote: Remote::new(),
            path: None,
            services: services::State::default(),
        })),
        tickets: Arc::new(AtomicU32::new(0)),
        ahead: Arc::new(AtomicU32::new(0)),
        events,
        started,
        orders,
    };
    (handles.clone(), NavLink(handles))
}

impl Handles {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
            + f64::from(self.ahead.load(Ordering::Relaxed)) / 1000.0
    }

    fn ticket(&self) -> u32 {
        self.tickets.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Put a walk's end on the event stream.
    fn announce(&self, event: &str, args: serde_json::Value) {
        self.events.push([serde_json::json!({
            "time": self.now(),
            "event": event,
            "args": args,
        })]);
    }

    fn walk_done(&self, status: &str) {
        self.announce("AGENT_WALK_DONE", serde_json::json!([status]));
    }
}

/// World coordinates to Bevy (x right, y up, z back), as `wrathplus_assets::coords::wow_to_bevy`.
fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

fn to_wow(b: Vec3) -> [f32; 3] {
    [-b.z, -b.x, b.y]
}

fn level(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A route now (local mmaps), or asked of the realm: its [`Outcome`] comes through [`pump`].
#[derive(Debug)]
pub(crate) enum Plan {
    Ready(Route),
    Asked,
}

/// How long `AgentPath` may read the realm's answer to its last question, and how far (yards) the
/// player may have moved meanwhile.
const PATH_FRESH: f64 = 30.0;
const PATH_NEAR: f32 = 5.0;

impl Mesh {
    /// Open the local navmesh for `map`; `Err` says why there is none (`None`: no folder set).
    fn local(&mut self, map: u32) -> Result<&mut NavMesh, Option<String>> {
        if let Some((_, why)) = self.missing.as_ref().filter(|(m, _)| *m == map) {
            return Err(Some(why.clone()));
        }
        if self.mesh.as_ref().is_none_or(|m| m.map() != map) {
            let dir = self.dir.clone().ok_or(None)?;
            match NavMesh::open(&dir, map) {
                Ok(mesh) => self.mesh = Some(mesh),
                Err(why) => {
                    self.missing = Some((map, why.clone()));
                    return Err(Some(why));
                }
            }
        }
        Ok(self.mesh.as_mut().expect("opened above"))
    }
}

/// How far apart, and how many each way, the heights tried for a target given without one.
const HEIGHT_STEP: f32 = 15.0;
const HEIGHT_TRIES: u8 = 10;

/// The heights to try for a target given as `x, y` alone, after the player's own: the nearest
/// first, up then down. A quest's map area, a zone's centre and a remembered spot are flat
/// points, and in a town on several levels the floor under one is rarely at the height the walk
/// starts from: from Goldshire (z 58) the Dwarven District's smithy floor (z 95) is not what the
/// search box finds first, and the route ends 148 yards short on another level. `mod-agent-nav`
/// tries the same heights (`AgentNavRoute.h`, `Heights`) for a request marked `h`.
pub(crate) fn heights(z: f32) -> impl Iterator<Item = f32> {
    (1..=HEIGHT_TRIES).flat_map(move |i| {
        let d = HEIGHT_STEP * f32::from(i);
        [z + d, z - d]
    })
}

/// A route over the map's navmesh, round `AgentAvoid`'s circles and `bumps` (a walk's own): at
/// once from local mmaps, or asked of the realm for `purpose`. With `any_height` the target's
/// height is a guess (the player's own): when no complete route ends there, other heights are
/// tried ([`heights`]), and the first that gives one is taken; the realm does the same.
fn route_in(
    mesh: &Mutex<Mesh>,
    me: world::Me,
    to: [f32; 3],
    bumps: &[[f32; 3]],
    any_height: bool,
    purpose: Purpose,
    now: f64,
) -> Result<Plan, String> {
    let mut mesh = mesh
        .lock()
        .map_err(|_| "navmesh lock poisoned".to_string())?;
    let avoid: Vec<[f32; 3]> = mesh.avoid.iter().chain(bumps).copied().collect();
    let why = match mesh.local(me.map) {
        Ok(nav) => {
            nav.set_avoid(&avoid);
            let route = nav.route(me.pos, to);
            if !any_height || route.as_ref().is_ok_and(|r| r.complete) {
                return route.map(Plan::Ready);
            }
            return heights(to[2])
                .filter_map(|z| nav.route(me.pos, [to[0], to[1], z]).ok())
                .find(|r| r.complete)
                .map_or(route, Ok)
                .map(Plan::Ready);
        }
        Err(why) => why,
    };
    if realm::choose(false, &mesh.remote.realm) != Source::Realm {
        return Err(mesh.remote.no_navmesh(why.as_deref()));
    }
    // `AgentPath` asked before: the answer, while it is fresh and we have not moved on.
    if purpose == Purpose::Path {
        let fits = |p: &PathAnswer| {
            now - p.at < PATH_FRESH && level(p.to, to) < 1.0 && level(p.from, me.pos) < PATH_NEAR
        };
        if let Some(answer) = mesh.path.take_if(|p| fits(p)) {
            return answer.result.map(Plan::Ready);
        }
    }
    let from = me.pos;
    mesh.remote.ask(
        Ask {
            purpose,
            from,
            to,
            avoid,
            any_height,
            kinds: Vec::new(),
            radius: None,
        },
        now,
    );
    Ok(Plan::Asked)
}

/// The route to `to` for a new walk or `AgentPath`; `any_height` as [`route_in`]'s.
fn route_to(
    nav: &Handles,
    me: world::Me,
    to: [f32; 3],
    any_height: bool,
    purpose: Purpose,
) -> Result<Plan, String> {
    if let (Purpose::Walk { .. }, Ok(mut walk)) = (purpose, nav.walk.lock()) {
        walk.map = me.map;
    }
    route_in(&nav.mesh, me, to, &[], any_height, purpose, nav.now())
}

fn note_for(route: &Route, leg: bool) -> Option<String> {
    if leg {
        Some("a long way: the realm routes it a leg at a time".to_string())
    } else if !route.complete {
        Some("partial route: walking as far as the navmesh reaches".to_string())
    } else if route.steep {
        Some("the only route crosses steep ground, which may not be climbable".to_string())
    } else {
        None
    }
}

/// A new walk: what the one before it left behind is cleared, its push included (`lua` lets
/// forward go).
fn renew(lua: &mlua::Lua, walk: &mut Walk, then_interact: Option<u64>, ticket: u32) {
    if walk.pushing.take().is_some() {
        let _ = lua.load(PUSH_END).exec();
    }
    walk.stop = false;
    walk.corner = None;
    walk.retried = false;
    walk.replans = 0;
    walk.bumps.clear();
    walk.settle = SETTLE;
    walk.then_interact = then_interact;
    walk.ticket = ticket;
    (walk.leg, walk.leg_from, walk.rerouting) = (false, None, false);
}

/// Start walking a route, and interact with `then_interact` on arrival.
fn start(lua: &mlua::Lua, nav: &Handles, route: &Route, then_interact: Option<u64>, ticket: u32) {
    let Ok(mut walk) = nav.walk.lock() else {
        return;
    };
    renew(lua, &mut walk, then_interact, ticket);
    // The first corner is where we stand.
    walk.queue = route.points.iter().skip(1).copied().collect();
    walk.goal = route.points.last().copied();
    walk.any_height = false;
    walk.status = "walking";
    walk.note = note_for(route, false);
}

/// The realm has been asked for the walk's route: `routing` until [`pump`] brings it.
fn begin_routing(
    lua: &mlua::Lua,
    nav: &Handles,
    goal: [f32; 3],
    any_height: bool,
    then_interact: Option<u64>,
    ticket: u32,
) {
    let Ok(mut walk) = nav.walk.lock() else {
        return;
    };
    renew(lua, &mut walk, then_interact, ticket);
    walk.queue.clear();
    walk.goal = Some(goal);
    walk.any_height = any_height;
    walk.status = "routing";
    walk.note = None;
}

/// Whether the realm is already being asked for this very walk (callers ask again every tick a
/// walk is not `walking`).
fn routing_to(nav: &Handles, goal: [f32; 3], then_interact: Option<u64>) -> bool {
    nav.walk.lock().is_ok_and(|w| {
        w.status == "routing"
            && !w.stop
            && w.then_interact == then_interact
            && w.goal.is_some_and(|g| level(g, goal) < 1.0)
    })
}

/// Put the realm's answer where it was wanted: the waiting walk, or `AgentPath`'s next call.
fn apply(nav: &Handles, outcome: Outcome) {
    let (ask, result, leg) = match outcome {
        Outcome::Services { ask, places } => return services::answered(nav, ask, Ok(places)),
        Outcome::Failed { ask, note } if ask.purpose == Purpose::Services => {
            return services::answered(nav, ask, Err(note));
        }
        Outcome::Route { ask, route, leg } => (ask, Ok(route), leg),
        Outcome::Failed { ask, note } => (ask, Err(note), false),
    };
    let Purpose::Walk { ticket } = ask.purpose else {
        let said = match &result {
            Ok(route) => serde_json::json!([true, route.complete]),
            Err(note) => serde_json::json!([false, note]),
        };
        if let Ok(mut mesh) = nav.mesh.lock() {
            mesh.path = Some(PathAnswer {
                from: ask.from,
                to: ask.to,
                at: nav.now(),
                result,
            });
        }
        return nav.announce("AGENT_PATH_READY", said);
    };
    let Ok(mut walk) = nav.walk.lock() else {
        return;
    };
    if walk.status != "routing" || walk.ticket != ticket {
        return;
    }
    match result {
        Ok(route) => {
            walk.queue = route.points.iter().skip(1).copied().collect();
            walk.status = "walking";
            walk.note = note_for(&route, leg);
            if !leg {
                // The realm found the floor: the goal's height is known from here on.
                walk.goal = route.points.last().copied();
                walk.any_height = false;
            }
            if !std::mem::take(&mut walk.rerouting) {
                walk.replans = 0;
            }
            walk.leg = leg;
        }
        Err(note) => {
            // An interaction falls back to the straight approach, as without a navmesh.
            let interact = walk.then_interact.take();
            walker::end(&mut walk, "failed", note);
            match interact {
                Some(guid) => {
                    walk.status = "idle";
                    let _ = nav.orders.send(world::Order::Interact(guid));
                }
                None => nav.walk_done("failed"),
            }
        }
    }
}

#[cfg(test)]
#[path = "nav_rig_tests.rs"]
mod rig;
#[cfg(test)]
#[path = "nav_services_tests.rs"]
mod services_tests;
#[cfg(test)]
#[path = "nav_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "nav_walk_tests.rs"]
mod walk_tests;
