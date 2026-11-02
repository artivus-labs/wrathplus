//! Client-visible world state for agents (#64): every object the client has streamed in (units,
//! players, game objects, corpses), read from the object mirror a few times a second, and orders
//! that act on one of them by GUID. Nothing here knows more than the client: the list is what the
//! server sent this client, and reactions, quest markers and names are the client's own reading.
//!
//! Lua (installed by `env`): `AgentPosition()`, `AgentNearby([max_yards])`, `AgentTarget(guid)`,
//! `AgentInteract(guid)`. GUIDs travel as `"0x…"` hex strings. `AgentInteract` turns on click-to-move
//! (`autoInteract`), so the character walks to the object first, as a player's right-click would.

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use mlua::{Lua, Table};
use std::sync::{Arc, Mutex};
use wrathplus_app_core::app_core::session::{ObjectStore, SelfPlayer};
use wrathplus_app_core::app_core::state::QuestGiver;
use wrathplus_app_net::go_templates::GameObjectTemplates;
use wrathplus_app_net::names::NameCache;
use wrathplus_app_net::net::{Guid, GuidIndex, NetEntity};
use wrathplus_app_world::target::click::Interact;
use wrathplus_app_world::target::{RelationInputs, SelectCommit};
use wrathplus_protocol::EntityKind;

/// How often the snapshot is rebuilt (seconds).
const REFRESH: f32 = 0.25;

/// One object as the client knows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Seen {
    pub guid: u64,
    pub kind: &'static str,
    pub name: Option<String>,
    pub entry: Option<u32>,
    /// World coordinates (x, y, z), as the server and maps use them.
    pub pos: [f32; 3],
    pub distance: f32,
    pub level: Option<u32>,
    pub health: Option<u32>,
    pub health_max: Option<u32>,
    pub dead: bool,
    /// The client's reaction ring: "hostile", "neutral" or "friendly".
    pub reaction: Option<&'static str>,
    pub in_combat: bool,
    pub lootable: bool,
    pub tapped: bool,
    pub tapped_by_me: bool,
    pub target: Option<u64>,
    /// NPC services: "questgiver", "vendor", "trainer", "flightmaster", "repair", "innkeeper"…
    pub npc: Vec<&'static str>,
    /// The quest marker over it: "available", "incomplete" or "reward".
    pub quest: Option<&'static str>,
    pub go_type: Option<i32>,
}

/// The player's own place.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Me {
    pub pos: [f32; 3],
    /// Radians, 0 = north (+x), counter-clockwise, as `GetPlayerFacing`.
    pub facing: f32,
    pub map: u32,
}

#[derive(Default)]
pub(crate) struct Snapshot {
    pub me: Option<Me>,
    pub objects: Vec<Seen>,
}

pub(crate) enum Order {
    Target(u64),
    Interact(u64),
}

/// What the Lua side holds: the latest snapshot and the way to send orders.
#[derive(Clone)]
pub(crate) struct Handles {
    pub snapshot: Arc<Mutex<Snapshot>>,
    pub orders: Sender<Order>,
}

#[derive(Resource)]
pub(crate) struct WorldLink {
    pub snapshot: Arc<Mutex<Snapshot>>,
    pub orders: Receiver<Order>,
    pub since: f32,
}

pub(crate) fn link() -> (Handles, WorldLink) {
    let (orders, queue) = crossbeam_channel::unbounded();
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    (
        Handles {
            snapshot: snapshot.clone(),
            orders,
        },
        WorldLink {
            snapshot,
            orders: queue,
            since: REFRESH,
        },
    )
}

fn kind_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Player => "player",
        EntityKind::Unit => "unit",
        EntityKind::GameObject => "gameobject",
        EntityKind::DynamicObject => "dynamicobject",
        EntityKind::Corpse => "corpse",
        _ => "other",
    }
}

/// The 3.3.5 questgiver status byte as a marker: 7/8 available, 5 incomplete, 3/6/9/10 reward.
pub(crate) fn quest_marker(status: u32) -> Option<&'static str> {
    match status {
        7 | 8 => Some("available"),
        5 => Some("incomplete"),
        3 | 6 | 9 | 10 => Some("reward"),
        _ => None,
    }
}

fn reaction_name(ring: u8) -> Option<&'static str> {
    match ring {
        0 => None,
        1 | 2 => Some("hostile"),
        3 => Some("neutral"),
        _ => Some("friendly"),
    }
}

/// Bevy (x right, y up, z back) to world coordinates, as `wrathplus_assets::coords::bevy_to_wow`.
fn to_world(v: Vec3) -> [f32; 3] {
    [-v.z, -v.x, v.y]
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn snapshot(
    time: Res<Time>,
    mut link: ResMut<WorldLink>,
    objects: Query<(&Guid, &NetEntity, &ObjectStore, &GlobalTransform), Without<SelfPlayer>>,
    me: Query<(&ObjectStore, &GlobalTransform), With<SelfPlayer>>,
    player: Option<Res<wrathplus_app_core::app_core::player::Player>>,
    map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    names: Option<Res<NameCache>>,
    templates: Option<Res<GameObjectTemplates>>,
    quests: Option<Res<QuestGiver>>,
    relations: RelationInputs,
) {
    link.since += time.delta_secs();
    if link.since < REFRESH {
        return;
    }
    link.since = 0.0;
    let my_store = me.single().ok().map(|(s, _)| s);
    let my_pos = me
        .single()
        .ok()
        .map(|(_, t)| to_world(t.translation()))
        .unwrap_or_default();
    let mut seen: Vec<Seen> = objects
        .iter()
        .map(|(guid, net, store, at)| {
            let pos = to_world(at.translation());
            let fields = &store.0;
            let unit = store.is_unit();
            let mut s = Seen {
                guid: guid.0,
                kind: kind_name(net.kind),
                entry: fields.object_entry(),
                pos,
                distance: Vec3::from(pos).distance(Vec3::from(my_pos)),
                quest: quests
                    .as_ref()
                    .and_then(|q| q.wrath_statuses().get(&guid.0).copied())
                    .and_then(quest_marker),
                ..default()
            };
            if unit {
                s.name = names
                    .as_ref()
                    .and_then(|n| n.peek_unit(guid.0, Some(store)).map(str::to_owned));
                s.level = fields.unit_level();
                s.health = fields.unit_shown_health();
                s.health_max = fields.unit_max_health();
                s.dead = fields.unit_is_dead();
                s.reaction = reaction_name(relations.ring_reaction(Some(store), my_store));
                s.in_combat = fields.unit_flags()
                    & wrathplus_app_core::app_core::state::UNIT_FLAG_IN_COMBAT
                    != 0;
                s.lootable = fields.unit_lootable();
                s.tapped = fields.unit_tapped();
                s.tapped_by_me = fields.unit_tapped_by_player();
                s.target = fields.unit_target().filter(|t| *t != 0);
                let npc = fields.unit_npc_services();
                for (on, label) in [
                    (npc.is_quest_giver(), "questgiver"),
                    (npc.is_vendor(), "vendor"),
                    (npc.is_trainer(), "trainer"),
                    (npc.can_repair(), "repair"),
                ] {
                    if on {
                        s.npc.push(label);
                    }
                }
            } else if net.kind == EntityKind::GameObject {
                s.name = templates
                    .as_ref()
                    .and_then(|t| t.get(guid.0).map(|g| g.name.clone()));
                s.go_type = Some(fields.gameobject_type_id());
            }
            s
        })
        .collect();
    seen.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    let me = player.map(|p| Me {
        pos: to_world(p.pos),
        facing: p.facing().rem_euclid(std::f32::consts::TAU),
        map: map.map_or(0, |m| m.0),
    });
    if let Ok(mut shared) = link.snapshot.lock() {
        shared.me = me;
        shared.objects = seen;
    }
}

/// Carry out targeting and interaction orders from Lua.
pub(crate) fn apply_orders(
    link: Res<WorldLink>,
    index: Option<Res<GuidIndex>>,
    mut select: SelectCommit,
    mut interact: MessageWriter<Interact>,
) {
    for order in link.orders.try_iter() {
        match order {
            Order::Target(guid) => select.select_unit(guid),
            Order::Interact(guid) => {
                if let Some(entity) = index.as_ref().and_then(|i| i.0.get(&guid).copied()) {
                    interact.write(Interact(entity));
                }
            }
        }
    }
}

fn guid_text(guid: u64) -> String {
    format!("0x{guid:016X}")
}

pub(crate) fn parse_guid(text: &str) -> Option<u64> {
    let hex = text
        .trim()
        .strip_prefix("0x")
        .or_else(|| text.trim().strip_prefix("0X"))?;
    u64::from_str_radix(hex, 16).ok()
}

fn seen_table(lua: &Lua, s: &Seen) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("guid", guid_text(s.guid))?;
    t.set("kind", s.kind)?;
    t.set("name", s.name.clone())?;
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
    t.set("go_type", s.go_type)?;
    Ok(t)
}

/// Install the world functions into this VM.
pub(crate) fn install(lua: &Lua, handles: &Handles) -> mlua::Result<()> {
    let g = lua.globals();
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
            if let Ok(s) = snap.lock() {
                for seen in s
                    .objects
                    .iter()
                    .filter(|o| max.is_none_or(|m| o.distance <= m))
                {
                    list.push(seen_table(lua, seen)?)?;
                }
            }
            Ok(list)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_round_trip_as_hex_text() {
        assert_eq!(guid_text(0xF130_0000_4E00_079A), "0xF13000004E00079A");
        assert_eq!(
            parse_guid("0xF13000004E00079A"),
            Some(0xF130_0000_4E00_079A)
        );
        assert_eq!(parse_guid("F130"), None);
        assert_eq!(parse_guid("0xZZ"), None);
    }

    #[test]
    fn quest_status_bytes_read_as_markers() {
        assert_eq!(quest_marker(8), Some("available"));
        assert_eq!(quest_marker(5), Some("incomplete"));
        assert_eq!(quest_marker(10), Some("reward"));
        assert_eq!(quest_marker(0), None);
    }

    #[test]
    fn lua_reads_the_snapshot_and_sends_orders() {
        let (handles, link) = link();
        {
            let mut s = handles.snapshot.lock().unwrap();
            s.me = Some(Me {
                pos: [1.0, 2.0, 3.0],
                facing: 1.5,
                map: 0,
            });
            s.objects = vec![
                Seen {
                    guid: 0x10,
                    kind: "unit",
                    name: Some("Deputy Willem".into()),
                    distance: 12.0,
                    npc: vec!["questgiver"],
                    quest: Some("available"),
                    ..Default::default()
                },
                Seen {
                    guid: 0x20,
                    kind: "unit",
                    distance: 80.0,
                    ..Default::default()
                },
            ];
        }
        let lua = Lua::new();
        install(&lua, &handles).unwrap();
        let (name, quest, count_near, facing): (String, String, usize, f32) = lua
            .load(
                "local n = AgentNearby() local near = AgentNearby(50) \
                 return n[1].name, n[1].quest, #near, AgentPosition().facing",
            )
            .eval()
            .unwrap();
        assert_eq!(
            (name.as_str(), quest.as_str(), count_near),
            ("Deputy Willem", "available", 1)
        );
        assert_eq!(facing, 1.5);
        let sent: bool = lua
            .load("return AgentInteract('0x0000000000000010')")
            .eval()
            .unwrap();
        assert!(sent);
        assert!(matches!(link.orders.try_recv(), Ok(Order::Interact(0x10))));
        assert!(lua.load("AgentTarget('Deputy')").exec().is_err());
    }
}
