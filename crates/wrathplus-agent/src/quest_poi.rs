//! Where a logged quest's objectives are, and where it is handed in: the quest POIs the client
//! already asks the server for on every quest-log rebuild (`SMSG_QUEST_POI_QUERY_RESPONSE`, the
//! world map's quest areas), for agents.
//!
//! Lua: `AgentQuestPOI(questId)` → `{ { objective, map, x, y, points = { {x, y}, ... } }, ... }`
//! or nil before the answer. `objective` is the quest's objective index, and -1 marks where it is
//! handed in. `x, y` is the area's centre; points are its outline, in world coordinates.

use bevy::prelude::*;
use mlua::Lua;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use wrathplus_app_net::net::wrath_quest_pois::QuestPois;
use wrathplus_protocol::wrath_quest::Poi;

type Known = Arc<Mutex<BTreeMap<u32, Vec<Poi>>>>;

/// The Lua side: the last copy of the client's POIs.
#[derive(Clone, Default)]
pub(crate) struct Handles {
    known: Known,
}

#[derive(Resource)]
pub(crate) struct PoiLink {
    known: Known,
}

pub(crate) fn link() -> (Handles, PoiLink) {
    let known = Known::default();
    (
        Handles {
            known: known.clone(),
        },
        PoiLink { known },
    )
}

/// Copy the client's POIs whenever an answer lands.
pub(crate) fn mirror(link: Res<PoiLink>, pois: Option<Res<QuestPois>>) {
    let Some(pois) = pois.filter(|p| p.is_changed()) else {
        return;
    };
    if let Ok(mut known) = link.known.lock() {
        known.clone_from(&pois.known);
    }
}

/// An area's centre: the mean of its outline.
fn centre(points: &[(i32, i32)]) -> (f32, f32) {
    let n = points.len().max(1) as f32;
    let (x, y) = points.iter().fold((0.0, 0.0), |(x, y), &(px, py)| {
        (x + px as f32, y + py as f32)
    });
    (x / n, y / n)
}

pub(crate) fn install(lua: &Lua, handles: &Handles) -> mlua::Result<()> {
    let known = handles.known.clone();
    lua.globals().set(
        "AgentQuestPOI",
        lua.create_function(move |lua, quest: u32| {
            let Some(pois) = known.lock().ok().and_then(|k| k.get(&quest).cloned()) else {
                return Ok(None);
            };
            let out = lua.create_table()?;
            for poi in pois {
                let t = lua.create_table()?;
                let (x, y) = centre(&poi.points);
                t.set("objective", poi.objective)?;
                t.set("map", poi.map)?;
                t.set("x", x)?;
                t.set("y", y)?;
                let points = lua.create_table()?;
                for (px, py) in poi.points {
                    points.push(vec![px, py])?;
                }
                t.set("points", points)?;
                out.push(t)?;
            }
            Ok(Some(out))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quests_areas_come_back_with_their_centres_and_the_hand_in_marked() {
        let (handles, link) = link();
        link.known.lock().unwrap().insert(
            85,
            vec![Poi {
                id: 0,
                objective: -1,
                map: 0,
                area: 30,
                floor: 0,
                raw_unknown: [0, 0],
                points: vec![(-9900, 300), (-9890, 310), (-9910, 320)],
            }],
        );
        let lua = Lua::new();
        install(&lua, &handles).unwrap();
        let (objective, x, y, missing): (i32, f32, f32, bool) = lua
            .load(
                "local p = AgentQuestPOI(85)[1] return p.objective, p.x, p.y, AgentQuestPOI(1) == nil",
            )
            .eval()
            .unwrap();
        assert_eq!((objective, x, y, missing), (-1, -9900.0, 310.0, true));
    }
}
