//! The Lua side of `nav`: `AgentPath`, `AgentMoveTo`, `AgentInteract` (the far case), `AgentAvoid`,
//! `AgentStop`, `AgentWalk`.

use super::realm::{self, Purpose, Source};
use super::{apply, begin_routing, route_to, routing_to, start, Handles, Plan};
use crate::world;
use mlua::{Lua, MultiValue, Table, Value};
use wrathplus_nav::Route;

/// The target point from Lua arguments: `x, y[, z]` (no height: the player's, and the route search
/// finds the ground: see [`height_given`]) or a `"0x…"` guid from `AgentNearby`.
pub(super) fn target(args: &MultiValue, snapshot: &world::Snapshot) -> Result<[f32; 3], String> {
    let num = |i: usize| match args.get(i) {
        Some(Value::Number(n)) => Some(*n as f32),
        Some(Value::Integer(n)) => Some(*n as f32),
        _ => None,
    };
    if let (Some(x), Some(y)) = (num(0), num(1)) {
        let z = num(2).or(snapshot.me.map(|m| m.pos[2])).unwrap_or(0.0);
        return Ok([x, y, z]);
    }
    let Some(Value::String(text)) = args.front() else {
        return Err("expected x, y[, z] or a guid".into());
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

/// What a caller gets while the realm's route is on its way: truthy like a route, and marked.
fn pending_table(lua: &Lua, goal: [f32; 3]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("pending", true)?;
    t.set("goal", vec![goal[0], goal[1], goal[2]])?;
    Ok(t)
}

/// Within this many yards `AgentInteract` uses the stock right-click approach, a straight line;
/// farther, it walks the navmesh route first (a straight line stalls on walls and counters).
const INTERACT_DIRECT: f32 = 3.0;

/// Click-to-move carries the walk; an agent client has its own settings.
fn click_to_move(lua: &Lua) {
    if let Ok(set) = lua.globals().get::<mlua::Function>("SetCVar") {
        let _ = set.call::<()>(("autoInteract", "1"));
    }
}

/// Whether the target's height is known: a guid's is, and so is `x, y, z`; `x, y` alone borrows
/// the player's, and the route search may look on other levels (`heights`).
pub(super) fn height_given(args: &MultiValue) -> bool {
    !matches!(
        (args.front(), args.get(2)),
        (
            Some(Value::Number(_) | Value::Integer(_)),
            None | Some(Value::Nil)
        )
    )
}

/// Whether a cinematic holds the client: nothing is walkable until it ends.
fn in_cinematic(world: &world::Handles) -> bool {
    world.snapshot.lock().is_ok_and(|s| s.cinematic)
}

/// Where we are and where the arguments point.
fn aim(args: &MultiValue, world: &world::Handles) -> Result<(world::Me, [f32; 3]), String> {
    let snapshot = world
        .snapshot
        .lock()
        .map_err(|_| "snapshot lock".to_string())?;
    let me = snapshot.me.ok_or("not in the world yet")?;
    Ok((me, target(args, &snapshot)?))
}

/// Walk to the arguments' target: at once over a local route, or `routing` while the realm is
/// asked. `Ok(None)`: the realm is already being asked for this very walk.
fn go(
    lua: &Lua,
    nav: &Handles,
    world: &world::Handles,
    args: &MultiValue,
    then_interact: Option<u64>,
) -> Result<(Option<Route>, [f32; 3]), String> {
    let (me, to) = aim(args, world)?;
    if routing_to(nav, to, then_interact) {
        return Ok((None, to));
    }
    let ticket = nav.ticket();
    let any_height = !height_given(args);
    let plan = route_to(nav, me, to, any_height, Purpose::Walk { ticket })?;
    click_to_move(lua);
    match plan {
        Plan::Ready(route) => {
            start(lua, nav, &route, then_interact, ticket);
            Ok((Some(route), to))
        }
        Plan::Asked => {
            begin_routing(lua, nav, to, any_height, then_interact, ticket);
            Ok((None, to))
        }
    }
}

pub(crate) fn install(lua: &Lua, nav: &Handles, world: &world::Handles) -> mlua::Result<()> {
    let g = lua.globals();
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentPath",
        lua.create_function(move |lua, args: MultiValue| {
            let any_height = !height_given(&args);
            let plan = aim(&args, &w).and_then(|(me, to)| {
                route_to(&n, me, to, any_height, Purpose::Path).map(|plan| (plan, to))
            });
            match plan {
                Ok((Plan::Ready(route), _)) => Ok((Some(route_table(lua, &route)?), None)),
                // The realm was asked: AGENT_PATH_READY fires when it answers, and the next call
                // for the same place returns the route.
                Ok((Plan::Asked, to)) => Ok((Some(pending_table(lua, to)?), None)),
                Err(e) => Ok((None, Some(e))),
            }
        })?,
    )?;
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentMoveTo",
        lua.create_function(move |lua, args: MultiValue| {
            // No walking under a cinematic (click-to-move stalls, ending `stuck` with no reason):
            // refuse, and say what to do about it.
            let went = if in_cinematic(&w) {
                Err(world::CINEMATIC_NOTE.to_string())
            } else {
                go(lua, &n, &w, &args, None)
            };
            match went {
                Ok((Some(route), _)) => Ok((Some(route_table(lua, &route)?), None)),
                Ok((None, to)) => Ok((Some(pending_table(lua, to)?), None)),
                Err(e) => {
                    // Announced like any walk's end, so whoever waits on it is woken; a cinematic's
                    // refusal once, not on every try of a loop that keeps asking until it is over.
                    if let Ok(mut walk) = n.walk.lock() {
                        let again = e == world::CINEMATIC_NOTE
                            && walk.status == "failed"
                            && walk.note.as_deref() == Some(world::CINEMATIC_NOTE);
                        walk.queue.clear();
                        walk.corner = None;
                        walk.then_interact = None;
                        walk.status = "failed";
                        walk.note = Some(e.clone());
                        // Never clear an announcement still pending from earlier this frame.
                        walk.failed |= !again;
                    }
                    Ok((None, Some(e)))
                }
            }
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
            // Under a cinematic no route is walked: the straight approach, as without a navmesh.
            if let Some(object) = far.filter(|_| !in_cinematic(&w)) {
                let args = MultiValue::from_iter([Value::String(lua.create_string(&guid)?)]);
                // No navmesh, or no route: fall back to the straight approach.
                if go(lua, &n, &w, &args, Some(object)).is_ok() {
                    return Ok(true);
                }
            }
            direct.call::<bool>(guid)
        })?,
    )?;
    let n = nav.clone();
    g.set(
        "AgentAvoid",
        lua.create_function(move |_, zones: Option<Vec<Table>>| {
            let zones: Vec<[f32; 3]> = zones
                .unwrap_or_default()
                .iter()
                .filter_map(|z| {
                    let get = |i: i64, k: &str| -> Option<f32> {
                        z.get::<Option<f32>>(k)
                            .ok()
                            .flatten()
                            .or_else(|| z.get(i).ok())
                    };
                    Some([get(1, "x")?, get(2, "y")?, get(3, "r")?])
                })
                .collect();
            let count = zones.len();
            if let Ok(mut mesh) = n.mesh.lock() {
                mesh.avoid = zones;
            }
            Ok(count)
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
    let (n, w) = (nav.clone(), world.clone());
    g.set(
        "AgentWalk",
        lua.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            let cinematic = in_cinematic(&w);
            if let Ok(walk) = n.walk.lock() {
                t.set("status", walk.status)?;
                t.set("remaining", walk.queue.len())?;
                t.set("goal", walk.goal.map(|g| vec![g[0], g[1], g[2]]))?;
                // A walk a cinematic holds says so, whatever else it noted.
                let note = match (&walk.note, cinematic) {
                    (Some(note), true) if note.as_str() != world::CINEMATIC_NOTE => {
                        Some(format!("{}; {note}", world::CINEMATIC_NOTE))
                    }
                    (None, true) => Some(world::CINEMATIC_NOTE.to_string()),
                    (note, _) => note.clone(),
                };
                t.set("note", note)?;
                t.set("cinematic", cinematic)?;
                let ahead = walk.corner.iter().chain(walk.queue.iter());
                t.set(
                    "route",
                    ahead.map(|p| vec![p[0], p[1], p[2]]).collect::<Vec<_>>(),
                )?;
                t.set("interact", walk.then_interact.map(world::guid_text))?;
            }
            Ok(t)
        })?,
    )?;
    Ok(())
}

/// Once a frame, with the VM at hand: run the realm exchange ([`super::pump`]) and whisper what
/// it has to send to ourselves, as `SendAddonMessage` would from an addon.
pub(crate) fn exchange(lua: &Lua, nav: &Handles, lines: Vec<(String, String)>, in_world: bool) {
    let g = lua.globals();
    let me = in_world
        .then(|| g.get::<mlua::Function>("UnitName").ok())
        .flatten()
        .and_then(|name| name.call::<Option<String>>("player").ok().flatten());
    let outbox = pump(nav, lines, me.as_deref());
    let (false, Some(me), Ok(send)) = (
        outbox.is_empty(),
        me,
        g.get::<mlua::Function>("SendAddonMessage"),
    ) else {
        return;
    };
    for line in outbox {
        let _ = send.call::<()>((wrathplus_nav::remote::PREFIX, line, "WHISPER", me.as_str()));
    }
}

/// The realm's lines among this frame's events (`CHAT_MSG_ADDON` with our prefix), taken out of
/// the stream: the message and who sent it. The client fires an addon line's prefix and body as
/// bytes (`ScriptValue::Bytes`, never decoded); a script-fired event may carry strings.
pub(crate) fn take_lines(
    events: &mut Vec<(String, Vec<wrathplus_ui::script::ScriptValue>)>,
) -> Vec<(String, String)> {
    use wrathplus_ui::script::ScriptValue::{Bytes, Str};
    let text = |v: &wrathplus_ui::script::ScriptValue| match v {
        Str(s) => Some(s.clone()),
        Bytes(b) => String::from_utf8(b.clone()).ok(),
        _ => None,
    };
    let mut lines = Vec::new();
    events.retain(|(event, args)| match (event.as_str(), args.as_slice()) {
        ("CHAT_MSG_ADDON", [prefix, message, _, Str(sender), ..])
            if text(prefix).as_deref() == Some(wrathplus_nav::remote::PREFIX) =>
        {
            match text(message) {
                Some(message) => lines.push((message, sender.clone())),
                // Not UTF-8: no line of ours (the protocol is ASCII), so not routed anywhere.
                None => return true,
            }
            false
        }
        _ => true,
    });
    lines
}

/// Once a frame: follow the client into the world (the probe), hand the realm's lines to the
/// route source, apply what came of them, and say what waits to be sent. `me` is the player's
/// name once known: lines are ours only when we whispered them, and nothing is sent before.
pub(crate) fn pump(nav: &Handles, lines: Vec<(String, String)>, me: Option<&str>) -> Vec<String> {
    let now = nav.now();
    let Ok(mut mesh) = nav.mesh.lock() else {
        return Vec::new();
    };
    let remote = &mut mesh.remote;
    let mut outcomes = remote.world(me.is_some(), now);
    for (line, sender) in lines {
        if me.is_some_and(|me| me.eq_ignore_ascii_case(&sender)) {
            outcomes.extend(remote.receive(&line, now));
        }
    }
    outcomes.extend(remote.tick(now));
    let outbox = std::mem::take(&mut remote.outbox);
    drop(mesh);
    for outcome in outcomes {
        apply(nav, outcome);
    }
    outbox
}

/// Which navmesh routes come from, for `/status`.
pub(crate) fn status(nav: &Handles) -> serde_json::Value {
    let Ok(mesh) = nav.mesh.lock() else {
        return serde_json::Value::Null;
    };
    let local = mesh.dir.as_ref().filter(|dir| dir.is_dir());
    let (source, detail) = match (realm::choose(local.is_some(), &mesh.remote.realm), local) {
        (Source::Local, Some(dir)) => (
            "local",
            format!("routes come from local mmaps ({})", dir.display()),
        ),
        (Source::Realm, _) => ("realm", mesh.remote.describe()),
        _ => ("none", mesh.remote.describe()),
    };
    serde_json::json!({"source": source, "detail": detail})
}
