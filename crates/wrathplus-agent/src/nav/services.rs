//! Where services are, asked of the realm (`mod-agent-nav`): the vendors, repairers, the
//! character's class trainer, profession trainers, flight masters, innkeepers, bankers and quest
//! hubs on its map, nearest first, as a player knows a town. Not a route, so it works with local
//! mmaps too, as long as the realm runs the module.
//!
//! Lua: `AgentServices([kinds [, radius]])` (kinds: a list or comma string of
//! [`services::KINDS`], none for all; radius in yards, none for the realm's) → `{pending = true}`
//! while the realm is asked; `AGENT_SERVICES_READY` (`true, count` or `false, why`) fires when it
//! answers, and the next call for the same kinds returns the places,
//! `{ { kind, entry, name, x, y, z, map, zone, members, profession }, ... }`: `zone` from the
//! client's own world map (`zones::Handles::name_at`), `members` the quest givers in a hub,
//! `profession` the skill a profession trainer teaches. `nil, why` when it cannot be asked.

use super::realm::{Ask, Purpose, Realm, NO_SERVICES};
use super::{level, Handles};
use crate::{world, zones};
use mlua::{Lua, Table, Value};
use wrathplus_nav::remote::services::{self, Place};

/// How long an answer is given back for the same kinds, and how far (yards) the character may
/// have moved since it was asked.
const FRESH: f64 = 120.0;
const NEAR: f32 = 100.0;

struct Answer {
    kinds: Vec<String>,
    from: [f32; 3],
    map: u32,
    at: f64,
    result: Result<Vec<Place>, String>,
}

/// The question out (its kinds and map), and the last answer.
#[derive(Default)]
pub(super) struct State {
    asked: Option<(Vec<String>, u32)>,
    answer: Option<Answer>,
}

/// The kinds from Lua: nil (all), a comma string, or a list of strings; sorted, without repeats,
/// so the same question is recognised however it is written.
fn kinds(value: Value) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = match value {
        Value::Nil => Vec::new(),
        Value::String(s) => s
            .to_string_lossy()
            .split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect(),
        Value::Table(t) => t
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()
            .map_err(|e| format!("kinds: {e}"))?,
        _ => return Err("kinds: a list of names or a comma string".into()),
    };
    if let Some(unknown) = out.iter().find(|k| !services::KINDS.contains(&k.as_str())) {
        return Err(format!(
            "unknown service kind {unknown:?} (known: {})",
            services::KINDS.join(", ")
        ));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The places for `kinds` when a fresh answer is in (`Some`), else the realm is asked (`None`).
fn ask(
    nav: &Handles,
    me: world::Me,
    kinds: Vec<String>,
    radius: Option<u32>,
) -> Result<Option<Vec<Place>>, String> {
    let now = nav.now();
    let mut mesh = nav.mesh.lock().map_err(|_| "navmesh lock".to_string())?;
    match mesh.remote.realm {
        Realm::Available(_) | Realm::Probing { .. } => {}
        Realm::Unknown => return Err("the realm is only asked once in the world".into()),
        Realm::Absent => return Err(NO_SERVICES.into()),
    }
    let state = &mut mesh.services;
    let fits = |a: &Answer| {
        a.kinds == kinds && a.map == me.map && now - a.at < FRESH && level(a.from, me.pos) < NEAR
    };
    // A refusal is told once; the next call asks again.
    if let Some(Answer {
        result: Err(why), ..
    }) = state.answer.take_if(|a| fits(a) && a.result.is_err())
    {
        return Err(why);
    }
    if let Some(Answer {
        result: Ok(places), ..
    }) = state.answer.as_ref().filter(|a| fits(a))
    {
        return Ok(Some(places.clone()));
    }
    if state.asked.as_ref() == Some(&(kinds.clone(), me.map)) {
        return Ok(None);
    }
    state.asked = Some((kinds.clone(), me.map));
    mesh.remote.ask(
        Ask {
            purpose: Purpose::Services,
            from: me.pos,
            to: me.pos,
            avoid: Vec::new(),
            any_height: false,
            kinds,
            radius,
        },
        now,
    );
    Ok(None)
}

/// The realm's answer (or why there is none), kept for the next `AgentServices` call.
pub(super) fn answered(nav: &Handles, ask: Ask, result: Result<Vec<Place>, String>) {
    let said = match &result {
        Ok(places) => serde_json::json!([true, places.len()]),
        Err(note) => serde_json::json!([false, note]),
    };
    if let Ok(mut mesh) = nav.mesh.lock() {
        let map = mesh.services.asked.take().map_or(0, |(_, map)| map);
        mesh.services.answer = Some(Answer {
            kinds: ask.kinds,
            from: ask.from,
            map,
            at: nav.now(),
            result,
        });
    }
    nav.announce("AGENT_SERVICES_READY", said);
}

fn place_table(lua: &Lua, p: &Place, map: u32, zones: &zones::Handles) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("kind", p.kind.as_str())?;
    t.set("entry", p.entry)?;
    t.set("name", p.name.as_str())?;
    t.set("x", p.pos[0])?;
    t.set("y", p.pos[1])?;
    t.set("z", p.pos[2])?;
    t.set("map", map)?;
    t.set("zone", zones.name_at(map, p.pos[0], p.pos[1]))?;
    t.set("members", p.members)?;
    t.set("profession", p.profession.clone())?;
    Ok(t)
}

pub(crate) fn install(
    lua: &Lua,
    nav: &Handles,
    world: &world::Handles,
    zones: &zones::Handles,
) -> mlua::Result<()> {
    let (n, w, z) = (nav.clone(), world.clone(), zones.clone());
    lua.globals().set(
        "AgentServices",
        lua.create_function(move |lua, (list, radius): (Value, Option<f64>)| {
            let me = w.snapshot.lock().ok().and_then(|s| s.me);
            let asked = me
                .ok_or_else(|| "not in the world yet".to_string())
                .and_then(|me| {
                    let radius = radius.filter(|r| *r > 0.0).map(|r| r.round() as u32);
                    ask(&n, me, kinds(list)?, radius).map(|places| (places, me.map))
                });
            match asked {
                Ok((Some(places), map)) => {
                    let out = lua.create_table()?;
                    for p in &places {
                        out.push(place_table(lua, p, map, &z)?)?;
                    }
                    Ok((Value::Table(out), None))
                }
                Ok((None, _)) => {
                    let pending = lua.create_table()?;
                    pending.set("pending", true)?;
                    Ok((Value::Table(pending), None))
                }
                Err(why) => Ok((Value::Nil, Some(why))),
            }
        })?,
    )
}
