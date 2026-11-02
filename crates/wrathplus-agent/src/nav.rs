//! Routes and walking for agents (#68): paths over the realm's mmaps navmesh (`wrathplus-nav`,
//! the server's own Detour), walked through the client's click-to-move one corner at a time, so
//! the character moves as a player clicking along the way would.
//!
//! The mmaps folder comes from `WOW_NAV_MMAPS` (the launcher passes `<server data>/mmaps`).
//!
//! Lua: `AgentPath(x, y, z | guid)` → `{complete, length, points = {{x, y, z}, ...}}`;
//! `AgentMoveTo(x, y, z | guid)` starts walking; `AgentInteract(guid)` from farther than a few
//! yards walks the route, then interacts; `AgentStop()`; `AgentWalk()` → `{status,
//! remaining, goal, note}` with status `idle`, `walking`, `arrived`, `stopped`, `failed` (refused)
//! or `stuck` (a corner not reached). Each end but `stopped` is put on the event stream as
//! `AGENT_WALK_DONE` with the status.

use crate::world;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wrathplus_app_world::player::AutoMove;
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
}

pub(crate) struct Mesh {
    dir: Option<PathBuf>,
    mesh: Option<NavMesh>,
}

#[derive(Clone)]
pub(crate) struct Handles {
    walk: Arc<Mutex<Walk>>,
    mesh: Arc<Mutex<Mesh>>,
}

#[derive(Resource)]
pub(crate) struct NavLink {
    walk: Arc<Mutex<Walk>>,
    /// Where a walk's end is announced (`AGENT_WALK_DONE`), to wake the tiers above.
    events: Arc<crate::events::EventLog>,
    started: std::time::Instant,
    /// The world's orders, for the interaction a walk ends in.
    orders: crossbeam_channel::Sender<world::Order>,
}

pub(crate) fn link(
    events: Arc<crate::events::EventLog>,
    started: std::time::Instant,
    orders: crossbeam_channel::Sender<world::Order>,
) -> (Handles, NavLink) {
    let walk = Arc::new(Mutex::new(Walk {
        status: "idle",
        ..Default::default()
    }));
    let mesh = Arc::new(Mutex::new(Mesh {
        dir: std::env::var_os("WOW_NAV_MMAPS").map(PathBuf::from),
        mesh: None,
    }));
    (
        Handles {
            walk: walk.clone(),
            mesh,
        },
        NavLink {
            walk,
            events,
            started,
            orders,
        },
    )
}

/// World coordinates to Bevy (x right, y up, z back), as `wrathplus_assets::coords::wow_to_bevy`.
fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

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
/// it fell short of is tried once more, then the walk ends as `stuck`.
pub(crate) fn walk(link: Res<NavLink>, mut auto: AutoMove) {
    let Ok(mut walk) = link.walk.lock() else {
        return;
    };
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
            } else {
                walk.status = "stuck";
                walk.note = Some(format!(
                    "click-to-move stopped {short:.0} yd short of the corner at ({:.0}, {:.0}, {:.0}), twice",
                    corner[0], corner[1], corner[2]
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
    walk.corner = walk.queue.front().copied();
    match walk.queue.pop_front() {
        Some(point) => {
            if !auto.walk_to_ground(to_bevy(point)) {
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

fn route_to(handles: &Handles, me: world::Me, to: [f32; 3]) -> Result<Route, String> {
    let mut mesh = handles
        .mesh
        .lock()
        .map_err(|_| "navmesh lock poisoned".to_string())?;
    if mesh.mesh.as_ref().is_none_or(|m| m.map() != me.map) {
        let dir = mesh
            .dir
            .clone()
            .ok_or("no navmesh: start the client with WOW_NAV_MMAPS=<server data>/mmaps")?;
        mesh.mesh = Some(NavMesh::open(&dir, me.map)?);
    }
    mesh.mesh.as_mut().expect("opened above").route(me.pos, to)
}

/// The target point from Lua arguments: `x, y, z` or a `"0x…"` guid from `AgentNearby`.
fn target(args: &MultiValue, snapshot: &world::Snapshot) -> Result<[f32; 3], String> {
    let num = |i: usize| match args.get(i) {
        Some(Value::Number(n)) => Some(*n as f32),
        Some(Value::Integer(n)) => Some(*n as f32),
        _ => None,
    };
    if let (Some(x), Some(y), Some(z)) = (num(0), num(1), num(2)) {
        return Ok([x, y, z]);
    }
    let Some(Value::String(text)) = args.front() else {
        return Err("expected x, y, z or a guid".into());
    };
    let text = text.to_string_lossy();
    let guid = world::parse_guid(&text).ok_or_else(|| format!("not a guid: {text}"))?;
    snapshot
        .objects
        .iter()
        .find(|o| o.guid == guid)
        .map(|o| o.pos)
        .ok_or_else(|| format!("{text} is not near (not in AgentNearby)"))
}

fn route_table(lua: &Lua, route: &Route) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let points = lua.create_table()?;
    let mut length = 0.0f32;
    for (i, p) in route.points.iter().enumerate() {
        points.push(vec![p[0], p[1], p[2]])?;
        if i > 0 {
            let q = route.points[i - 1];
            length +=
                ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        }
    }
    t.set("points", points)?;
    t.set("complete", route.complete)?;
    t.set("length", (length * 10.0).round() / 10.0)?;
    Ok(t)
}

/// Within this many yards `AgentInteract` uses the stock right-click approach, a straight line;
/// farther, it walks the navmesh route first (a straight line stalls on walls and counters).
const INTERACT_DIRECT: f32 = 3.0;

/// Start walking a route, and interact with `then_interact` on arrival.
fn start(lua: &Lua, nav: &Handles, route: &Route, then_interact: Option<u64>) {
    // Click-to-move carries the walk; an agent client has its own settings.
    if let Ok(set) = lua.globals().get::<mlua::Function>("SetCVar") {
        let _ = set.call::<()>(("autoInteract", "1"));
    }
    let Ok(mut walk) = nav.walk.lock() else {
        return;
    };
    // The first corner is where we stand.
    walk.queue = route.points.iter().skip(1).copied().collect();
    walk.goal = route.points.last().copied();
    walk.status = "walking";
    walk.note = if !route.complete {
        Some("partial route: walking as far as the navmesh reaches".to_string())
    } else if route.steep {
        Some("the only route crosses steep ground, which may not be climbable".to_string())
    } else {
        None
    };
    walk.stop = false;
    walk.corner = None;
    walk.retried = false;
    walk.then_interact = then_interact;
}

pub(crate) fn install(lua: &Lua, nav: &Handles, world: &world::Handles) -> mlua::Result<()> {
    let g = lua.globals();
    let plan = |args: MultiValue, nav: &Handles, world: &world::Handles| -> Result<Route, String> {
        let snapshot = world
            .snapshot
            .lock()
            .map_err(|_| "snapshot lock".to_string())?;
        let me = snapshot.me.ok_or("not in the world yet")?;
        let to = target(&args, &snapshot)?;
        drop(snapshot);
        route_to(nav, me, to)
    };
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentPath",
        lua.create_function(move |lua, args: MultiValue| match plan(args, &n, &w) {
            Ok(route) => Ok((Some(route_table(lua, &route)?), None)),
            Err(e) => Ok((None, Some(e))),
        })?,
    )?;
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentMoveTo",
        lua.create_function(move |lua, args: MultiValue| {
            let route = match plan(args, &n, &w) {
                Ok(route) => route,
                Err(e) => return Ok((None, Some(e))),
            };
            start(lua, &n, &route, None);
            let summary = route_table(lua, &route)?;
            Ok((Some(summary), None))
        })?,
    )?;
    // `AgentInteract` (from `world`) walks the navmesh route first unless the object is close.
    let direct: mlua::Function = g.get("AgentInteract")?;
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentInteract",
        lua.create_function(move |lua, guid: String| {
            let far = world::parse_guid(&guid).filter(|g| {
                w.snapshot.lock().ok().is_some_and(|s| {
                    s.objects
                        .iter()
                        .any(|o| o.guid == *g && o.distance > INTERACT_DIRECT)
                })
            });
            if let Some(object) = far {
                let args = MultiValue::from_iter([Value::String(lua.create_string(&guid)?)]);
                // No navmesh, or no route: fall back to the straight approach.
                if let Ok(route) = plan(args, &n, &w) {
                    start(lua, &n, &route, Some(object));
                    return Ok(true);
                }
            }
            direct.call::<bool>(guid)
        })?,
    )?;
    let n = nav.clone();
    g.set(
        "AgentStop",
        lua.create_function(move |_, ()| {
            if let Ok(mut walk) = n.walk.lock() {
                walk.stop = true;
            }
            Ok(())
        })?,
    )?;
    let n = nav.clone();
    g.set(
        "AgentWalk",
        lua.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            if let Ok(walk) = n.walk.lock() {
                t.set("status", walk.status)?;
                t.set("remaining", walk.queue.len())?;
                t.set("goal", walk.goal.map(|g| vec![g[0], g[1], g[2]]))?;
                t.set("note", walk.note.clone())?;
            }
            Ok(t)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_come_from_coordinates_or_a_nearby_guid() {
        let lua = Lua::new();
        let snapshot = world::Snapshot {
            me: None,
            objects: vec![world::Seen {
                guid: 0x10,
                pos: [1.0, 2.0, 3.0],
                ..Default::default()
            }],
        };
        let args = |code: &str| lua.load(code).eval::<MultiValue>().unwrap();
        assert_eq!(
            target(&args("return 4, 5, 6"), &snapshot),
            Ok([4.0, 5.0, 6.0])
        );
        assert_eq!(
            target(&args("return '0x0000000000000010'"), &snapshot),
            Ok([1.0, 2.0, 3.0])
        );
        assert!(target(&args("return '0x99'"), &snapshot).is_err());
        assert!(target(&args("return true"), &snapshot).is_err());
    }

    #[test]
    fn without_mmaps_a_route_says_how_to_get_one() {
        let (handles, _) = link(
            Arc::new(crate::events::EventLog::new()),
            std::time::Instant::now(),
            crossbeam_channel::unbounded().0,
        );
        handles.mesh.lock().unwrap().dir = None;
        let me = world::Me {
            pos: [0.0; 3],
            facing: 0.0,
            map: 0,
        };
        assert!(route_to(&handles, me, [1.0, 1.0, 0.0])
            .unwrap_err()
            .contains("WOW_NAV_MMAPS"));
    }

    #[test]
    fn a_far_interaction_without_a_navmesh_falls_back_to_the_straight_approach() {
        let (world_handles, world_link) = world::link();
        let (handles, _) = link(
            Arc::new(crate::events::EventLog::new()),
            std::time::Instant::now(),
            world_handles.orders.clone(),
        );
        handles.mesh.lock().unwrap().dir = None;
        {
            let mut s = world_handles.snapshot.lock().unwrap();
            s.me = Some(world::Me {
                pos: [0.0; 3],
                facing: 0.0,
                map: 0,
            });
            s.objects = vec![world::Seen {
                guid: 0x10,
                distance: 24.0,
                pos: [24.0, 0.0, 0.0],
                ..Default::default()
            }];
        }
        let lua = Lua::new();
        world::install(&lua, &world_handles).unwrap();
        install(&lua, &handles, &world_handles).unwrap();
        let sent: bool = lua
            .load("return AgentInteract('0x0000000000000010')")
            .eval()
            .unwrap();
        assert!(sent);
        assert!(matches!(
            world_link.orders.try_recv(),
            Ok(world::Order::Interact(0x10))
        ));
        assert_eq!(handles.walk.lock().unwrap().then_interact, None);
    }
}
