//! The Lua side of the world snapshot (`world`): `AgentNearby`, `AgentFindNpc`, `AgentPosition`,
//! `AgentCorpse`, `AgentCinematic`, `AgentFace`, `AgentTarget` and `AgentInteract`.

use super::{guid_text, parse_guid, Handles, Order, Seen, CINEMATIC_NOTE};
use crate::npc::SERVICES;
use mlua::{Lua, Table};

fn seen_table(lua: &Lua, s: &Seen) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("guid", guid_text(s.guid))?;
    t.set("kind", s.kind)?;
    t.set("name", s.name.clone())?;
    t.set("title", s.title.clone())?;
    t.set("creature_type", s.creature_type)?;
    t.set("entry", s.entry)?;
    t.set("x", s.pos[0])?;
    t.set("y", s.pos[1])?;
    t.set("z", s.pos[2])?;
    t.set("distance", (s.distance * 10.0).round() / 10.0)?;
    t.set("level", s.level)?;
    t.set("health", s.health)?;
    t.set("health_max", s.health_max)?;
    t.set("dead", s.dead)?;
    t.set("reaction", s.reaction)?;
    t.set("in_combat", s.in_combat)?;
    t.set("lootable", s.lootable)?;
    t.set("tapped", s.tapped)?;
    t.set("tapped_by_me", s.tapped_by_me)?;
    t.set("target", s.target.map(guid_text))?;
    if !s.npc.is_empty() {
        t.set("npc", s.npc.clone())?;
    }
    t.set("quest", s.quest)?;
    t.set("classification", s.classification)?;
    t.set("critter", s.critter.then_some(true))?;
    t.set("go_type", s.go_type)?;
    t.set("lock", s.lock)?;
    // A flight master: "known" or "unknown" (its node not learned yet: talk to it).
    t.set("taxi", s.taxi.map(|k| if k { "known" } else { "unknown" }))?;
    if !s.quests.is_empty() {
        let quests = lua.create_table()?;
        for &(id, complete) in &s.quests {
            let q = lua.create_table()?;
            q.set("id", id)?;
            q.set("complete", complete)?;
            quests.push(q)?;
        }
        t.set("quests", quests)?;
    }
    Ok(t)
}

/// Install the world functions into this VM.
pub(crate) fn install(lua: &Lua, handles: &Handles) -> mlua::Result<()> {
    let g = lua.globals();
    let snap = handles.snapshot.clone();
    g.set(
        "AgentCorpse",
        lua.create_function(move |lua, ()| {
            let Some((corpse, me)) = snap.lock().ok().and_then(|s| Some((s.corpse?, s.me))) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("x", corpse.pos[0])?;
            t.set("y", corpse.pos[1])?;
            t.set("z", corpse.pos[2])?;
            t.set("map", corpse.map)?;
            if let Some(me) = me.filter(|m| m.map as i32 == corpse.map) {
                let d = (me.pos[0] - corpse.pos[0]).hypot(me.pos[1] - corpse.pos[1]);
                t.set("distance", (d * 10.0).round() / 10.0)?;
            }
            Ok(Some(t))
        })?,
    )?;
    // `AgentCinematic()`: why nothing is near or walkable while a cinematic holds the client
    // ([`CINEMATIC_NOTE`]), else nil. The built-in modules wait on it instead of failing.
    let snap = handles.snapshot.clone();
    g.set(
        "AgentCinematic",
        lua.create_function(move |_, ()| {
            Ok(snap
                .lock()
                .is_ok_and(|s| s.cinematic)
                .then_some(CINEMATIC_NOTE))
        })?,
    )?;
    let snap = handles.snapshot.clone();
    g.set(
        "AgentPosition",
        lua.create_function(move |lua, ()| {
            let Some(me) = snap.lock().ok().and_then(|s| s.me) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("x", me.pos[0])?;
            t.set("y", me.pos[1])?;
            t.set("z", me.pos[2])?;
            t.set("facing", me.facing)?;
            t.set("map", me.map)?;
            Ok(Some(t))
        })?,
    )?;
    let snap = handles.snapshot.clone();
    g.set(
        "AgentNearby",
        lua.create_function(move |lua, max: Option<f32>| {
            let list = lua.create_table()?;
            let mut note = None;
            if let Ok(s) = snap.lock() {
                for seen in s
                    .objects
                    .iter()
                    .filter(|o| max.is_none_or(|m| o.distance <= m))
                {
                    list.push(seen_table(lua, seen)?)?;
                }
                // A second value, so `ipairs(AgentNearby())` is unchanged and a reader of the
                // reply sees why the list is empty or far away.
                note = s.cinematic.then_some(CINEMATIC_NOTE);
            }
            Ok((list, note))
        })?,
    )?;
    let snap = handles.snapshot.clone();
    g.set(
        "AgentFindNpc",
        lua.create_function(move |lua, (service, max): (String, Option<f32>)| {
            if !SERVICES.iter().any(|(name, _, _)| *name == service) {
                let known: Vec<&str> = SERVICES.iter().map(|(name, _, _)| *name).collect();
                return Err(mlua::Error::runtime(format!(
                    "AgentFindNpc: no service {service:?}; one of {}",
                    known.join(", ")
                )));
            }
            let Ok(s) = snap.lock() else {
                return Ok(None);
            };
            // The nearest living one the client knows of; none in sight is nil.
            s.objects
                .iter()
                .filter(|o| !o.dead && o.npc.contains(&service.as_str()))
                .filter(|o| max.is_none_or(|m| o.distance <= m))
                .min_by(|a, b| a.distance.total_cmp(&b.distance))
                .map(|o| seen_table(lua, o))
                .transpose()
        })?,
    )?;
    let snap = handles.snapshot.clone();
    let orders = handles.orders.clone();
    g.set(
        "AgentFace",
        lua.create_function(move |_, guid: String| {
            let Some(g) = parse_guid(&guid) else {
                return Ok(false);
            };
            let pos = snap
                .lock()
                .ok()
                .and_then(|s| s.objects.iter().find(|o| o.guid == g).map(|o| o.pos));
            Ok(pos.is_some_and(|p| orders.send(Order::Face(p)).is_ok()))
        })?,
    )?;
    for (name, make) in [
        ("AgentTarget", Order::Target as fn(u64) -> Order),
        ("AgentInteract", Order::Interact as fn(u64) -> Order),
    ] {
        let orders = handles.orders.clone();
        g.set(
            name,
            lua.create_function(move |lua, guid: String| {
                let guid = parse_guid(&guid).ok_or_else(|| {
                    mlua::Error::runtime(format!("{name}: expected a \"0x…\" guid, got {guid:?}"))
                })?;
                if matches!(name, "AgentInteract") {
                    // Click-to-move, so the character walks to what it interacts with (stock
                    // `autoInteract`; an agent client has its own settings).
                    if let Ok(set) = lua.globals().get::<mlua::Function>("SetCVar") {
                        let _ = set.call::<()>(("autoInteract", "1"));
                    }
                }
                Ok(orders.send(make(guid)).is_ok())
            })?,
        )?;
    }
    Ok(())
}
