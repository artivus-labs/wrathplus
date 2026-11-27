//! The flight paths as the client knows them, for the agent's flights (`wp.taxi`): where every
//! node is, from the client's own `TaxiNodes.dbc`, and which of them this character knows, from
//! what the server has told the client. Nothing here knows more than the client.
//!
//! - The known nodes are the `SMSG_SHOWTAXINODES` mask, read whenever a flight master's map opens
//!   (`TaxiState`); the client keeps no copy between maps, so this one is kept for the character
//!   until the client exits (a `ReloadUI` or a relog to the same character keeps it).
//! - Between maps, a flight master in sight whose `SMSG_TAXINODE_STATUS` says known (the green `!`
//!   gone over its head: a first visit just learned it) marks the node it serves, the nearest node
//!   within [`SERVES`] yards, as known too.
//!
//! Lua: `AgentTaxiNodes()` → `{ { id, name, map, x, y, z, known, alliance, horde, distance }, ... },
//! read`: the nodes on the character's map with a rider for either side, nearest first. `known` is
//! true or false; `read` says whether a map's mask has been read since the client started (until
//! then only nodes learned in sight read known). `alliance` and `horde` say which side's riders fly
//! from it.

use bevy::prelude::*;
use mlua::Lua;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use wrathplus_app_core::app_core::interaction::FlightMasterStatus;
use wrathplus_app_core::app_core::session::SelfPlayer;
use wrathplus_app_net::net::Guid;
use wrathplus_app_ui::ui_taxi::TaxiState;
use wrathplus_assets::{LockRecover, WorldAssets};
use wrathplus_protocol::types::TaxiMask;

/// A flight master serves the node this near it (yards): the server's own pick is the nearest node
/// on its map (`GetNearestTaxiNode`), and every shipped master stands within a few yards of its.
pub(crate) const SERVES: f32 = 60.0;

/// One node, as `AgentTaxiNodes` hands it out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub id: u32,
    pub name: String,
    pub map: u32,
    pub pos: [f32; 3],
    pub alliance: bool,
    pub horde: bool,
}

/// What the character knows: the last mask a map showed, and the nodes learned in sight since.
#[derive(Default)]
pub(crate) struct Known {
    /// Whose knowledge this is; another character starts afresh.
    player: u64,
    mask: Option<TaxiMask>,
    learned: BTreeSet<u32>,
}

impl Known {
    /// A map's mask: everything known now, the learned ones included.
    pub(crate) fn read(&mut self, mask: TaxiMask) {
        self.mask = Some(mask);
        self.learned.clear();
    }

    pub(crate) fn learn(&mut self, id: u32) {
        if !self.mask.is_some_and(|m| m.is_known(id)) {
            self.learned.insert(id);
        }
    }

    pub(crate) fn is_known(&self, id: u32) -> bool {
        self.learned.contains(&id) || self.mask.is_some_and(|m| m.is_known(id))
    }

    /// A new character (or none yet): forget the last one's.
    pub(crate) fn player(&mut self, guid: u64) {
        if self.player != guid {
            *self = Known {
                player: guid,
                ..Known::default()
            };
        }
    }
}

/// The node a flight master at `pos` on `map` serves: the nearest within [`SERVES`] yards.
pub(crate) fn served(nodes: &[Node], map: u32, pos: [f32; 3]) -> Option<u32> {
    nodes
        .iter()
        .filter(|n| n.map == map)
        .map(|n| (n.id, flat(n.pos, pos)))
        .filter(|(_, d)| *d <= SERVES)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The Lua side: the nodes once read, and what the character knows.
#[derive(Clone, Default)]
pub(crate) struct Handles {
    nodes: Arc<Mutex<Vec<Node>>>,
    known: Arc<Mutex<Known>>,
}

#[derive(Resource)]
pub(crate) struct TaxiLink {
    handles: Handles,
    loaded: bool,
}

pub(crate) fn link() -> (Handles, TaxiLink) {
    let handles = Handles::default();
    (
        handles.clone(),
        TaxiLink {
            handles,
            loaded: false,
        },
    )
}

/// The nodes off the patch chain, once: those with a rider for either side (the rest are unused
/// rows, transports' and quests' own).
pub(crate) fn load(mut link: ResMut<TaxiLink>, assets: Option<Res<WorldAssets>>) {
    if link.loaded {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    link.loaded = true;
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_taxi_nodes(&mut chain)
    };
    let table = match loaded {
        Ok(table) => table,
        Err(e) => {
            warn!("agent port: no flight paths for agents, TaxiNodes failed to load: {e:#}");
            return;
        }
    };
    let mut nodes: Vec<Node> = table
        .rows()
        .filter(|n| n.mount_alliance != 0 || n.mount_horde != 0)
        .map(|n| Node {
            id: n.id,
            name: n.name.clone(),
            map: n.map_id,
            pos: n.pos,
            alliance: n.mount_alliance != 0,
            horde: n.mount_horde != 0,
        })
        .collect();
    nodes.sort_by_key(|n| n.id);
    info!("agent port: {} flight path nodes for agents", nodes.len());
    if let Ok(mut shared) = link.handles.nodes.lock() {
        *shared = nodes;
    }
}

/// Follow what the client is told: a map's mask as it opens, a flight master in sight known.
pub(crate) fn mirror(
    link: Res<TaxiLink>,
    taxi: Option<Res<TaxiState>>,
    me: Query<&Guid, With<SelfPlayer>>,
    masters: Query<(&GlobalTransform, &FlightMasterStatus)>,
    map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
) {
    let Ok(guid) = me.single() else {
        return;
    };
    let Ok(mut known) = link.handles.known.lock() else {
        return;
    };
    known.player(guid.0);
    if let Some(open) = taxi.as_ref().and_then(|t| t.open.as_ref()) {
        known.read(open.known);
    }
    let Some(map) = map.map(|m| m.0) else {
        return;
    };
    let Ok(nodes) = link.handles.nodes.lock() else {
        return;
    };
    for (at, status) in &masters {
        if !status.known {
            continue;
        }
        let v = at.translation();
        // Bevy (x right, y up, z back) to world coordinates, as `world::to_world`.
        if let Some(id) = served(&nodes, map, [-v.z, -v.x, v.y]) {
            known.learn(id);
        }
    }
}

pub(crate) fn install(
    lua: &Lua,
    handles: &Handles,
    world: &crate::world::Handles,
) -> mlua::Result<()> {
    let nodes = handles.nodes.clone();
    let known = handles.known.clone();
    let snap = world.snapshot.clone();
    lua.globals().set(
        "AgentTaxiNodes",
        lua.create_function(move |lua, ()| {
            let out = lua.create_table()?;
            let me = snap.lock().ok().and_then(|s| s.me);
            let Some(me) = me else {
                return Ok((out, false));
            };
            let nodes = nodes.lock().map(|n| n.clone()).unwrap_or_default();
            let (read, marks): (bool, Vec<bool>) = match known.lock() {
                Ok(k) => (
                    k.mask.is_some(),
                    nodes.iter().map(|n| k.is_known(n.id)).collect(),
                ),
                Err(_) => (false, vec![false; nodes.len()]),
            };
            let mut here: Vec<(&Node, bool, f32)> = nodes
                .iter()
                .zip(marks)
                .filter(|(n, _)| n.map == me.map)
                .map(|(n, k)| (n, k, flat(n.pos, me.pos)))
                .collect();
            here.sort_by(|a, b| a.2.total_cmp(&b.2));
            for (n, k, d) in here {
                let t = lua.create_table()?;
                t.set("id", n.id)?;
                t.set("name", n.name.as_str())?;
                t.set("map", n.map)?;
                t.set("x", n.pos[0])?;
                t.set("y", n.pos[1])?;
                t.set("z", n.pos[2])?;
                t.set("known", k)?;
                t.set("alliance", n.alliance)?;
                t.set("horde", n.horde)?;
                t.set("distance", (d * 10.0).round() / 10.0)?;
                out.push(t)?;
            }
            Ok((out, read))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, map: u32, x: f32, y: f32) -> Node {
        Node {
            id,
            name: format!("node {id}"),
            map,
            pos: [x, y, 0.0],
            alliance: true,
            horde: false,
        }
    }

    #[test]
    fn a_mask_and_the_nodes_learned_since_make_what_is_known_until_another_character() {
        let mut k = Known::default();
        k.player(7);
        assert!(!k.is_known(8), "nothing read, nothing learned");
        // Thelsamar (8) learned in sight before any map opened.
        k.learn(8);
        assert!(k.is_known(8) && !k.is_known(6));
        // A map at Ironforge: Stormwind (2) and Ironforge (6) and Thelsamar (8).
        let mut mask = [0; 14];
        mask[0] = 0b1010_0010;
        k.read(TaxiMask(mask));
        assert!(k.is_known(2) && k.is_known(6) && k.is_known(8) && !k.is_known(4));
        k.learn(4);
        assert!(k.is_known(4));
        k.player(7);
        assert!(k.is_known(4), "the same character keeps it");
        k.player(9);
        assert!(
            !k.is_known(2) && !k.is_known(4),
            "another character starts afresh"
        );
    }

    #[test]
    fn a_flight_master_serves_the_nearest_node_on_its_map_within_reach() {
        let nodes = [
            node(6, 0, -4821.0, -1152.0),
            node(8, 0, -5424.0, -2929.0),
            node(9, 1, -4821.0, -1152.0),
        ];
        assert_eq!(served(&nodes, 0, [-4810.0, -1150.0, 500.0]), Some(6));
        assert_eq!(served(&nodes, 1, [-4810.0, -1150.0, 0.0]), Some(9));
        assert_eq!(
            served(&nodes, 0, [-5000.0, -2000.0, 0.0]),
            None,
            "none near"
        );
    }
}
