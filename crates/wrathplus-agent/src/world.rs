//! Client-visible world state for agents (#64): every object the client has streamed in (units,
//! players, game objects, corpses), read from the object mirror a few times a second, and orders
//! that act on one of them by GUID. Nothing here knows more than the client: the list is what the
//! server sent this client, and reactions, quest markers and names are the client's own reading.
//!
//! Lua (installed by `env`): `AgentPosition()`, `AgentNearby([max_yards])`,
//! `AgentFindNpc(service [, max_yards])`, `AgentCorpse()`, `AgentTarget(guid)`, `AgentFace(guid)`,
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
use wrathplus_protocol::types::NpcService;
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
    /// NPC services, from the unit's NPC flags ([`SERVICES`]): what it offers before it is
    /// spoken to, which the stock API cannot tell (`CanMerchantRepair` needs an open window).
    pub npc: Vec<&'static str>,
    /// The title under a creature's name ("Weapons Merchant", "Blacksmithing Trainer").
    pub title: Option<String>,
    /// The quest marker over it: "available", "incomplete" or "reward".
    pub quest: Option<&'static str>,
    /// A player's quest log as the client has it (the server sends it for party members):
    /// quest ids, and whether each is complete.
    pub quests: Vec<(u32, bool)>,
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
    /// Our corpse (the server's `MSG_CORPSE_QUERY` answer): where to run as a ghost.
    pub corpse: Option<Corpse>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Corpse {
    /// World coordinates; inside an instance, its entrance's.
    pub pos: [f32; 3],
    pub map: i32,
}

pub(crate) enum Order {
    Target(u64),
    Interact(u64),
    /// Turn to face this point (world coordinates), without walking.
    Face([f32; 3]),
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
    death: Option<Res<wrathplus_app_core::app_core::death::DeathNet>>,
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
            if net.kind == EntityKind::Player {
                s.quests = (0..fields.player_quest_log_slots())
                    .filter_map(|slot| fields.player_quest_log(slot))
                    .filter(|q| q.quest_id != 0)
                    // QUEST_STATE_COMPLETE (AzerothCore QuestDef.h).
                    .map(|q| (q.quest_id, q.state == 1))
                    .collect();
            }
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
                s.npc = services(fields.unit_npc_flags());
                s.title = names.as_ref().and_then(|n| {
                    let entry = fields.object_entry()?;
                    n.creature_subname(entry).map(str::to_owned)
                });
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
        shared.corpse = death.and_then(|d| d.corpse).map(|c| Corpse {
            pos: c.position,
            map: c.display_map,
        });
    }
}

/// Carry out targeting and interaction orders from Lua.
pub(crate) fn apply_orders(
    link: Res<WorldLink>,
    index: Option<Res<GuidIndex>>,
    mut select: SelectCommit,
    mut interact: MessageWriter<Interact>,
    mut auto: wrathplus_app_world::player::AutoMove,
) {
    for order in link.orders.try_iter() {
        match order {
            Order::Target(guid) => select.select_unit(guid),
            Order::Face(p) => {
                // World to Bevy (x right, y up, z back), as `wrathplus_assets::coords::wow_to_bevy`.
                auto.face(Vec3::new(-p[1], p[2], -p[0]));
            }
            Order::Interact(guid) => {
                if let Some(entity) = index.as_ref().and_then(|i| i.0.get(&guid).copied()) {
                    interact.write(Interact(entity));
                }
            }
        }
    }
}

pub(crate) fn guid_text(guid: u64) -> String {
    format!("0x{guid:016X}")
}

pub(crate) fn parse_guid(text: &str) -> Option<u64> {
    let hex = text
        .trim()
        .strip_prefix("0x")
        .or_else(|| text.trim().strip_prefix("0X"))?;
    u64::from_str_radix(hex, 16).ok()
}

/// The NPC services an agent can ask for, by name: `NpcService`'s flags, with trainers also split
/// by kind (AzerothCore `UNIT_NPC_FLAG_TRAINER_CLASS` 0x20, `_PROFESSION` 0x40).
pub(crate) const SERVICES: &[(&str, Option<NpcService>, u32)] = &[
    ("gossip", Some(NpcService::Gossip), 0),
    ("questgiver", Some(NpcService::QuestGiver), 0),
    ("vendor", Some(NpcService::Vendor), 0),
    ("repair", Some(NpcService::Repair), 0),
    ("trainer", Some(NpcService::Trainer), 0),
    ("classtrainer", None, 0x20),
    ("professiontrainer", None, 0x40),
    ("flightmaster", Some(NpcService::FlightMaster), 0),
    ("innkeeper", Some(NpcService::Innkeeper), 0),
    ("banker", Some(NpcService::Banker), 0),
    ("auctioneer", Some(NpcService::Auctioneer), 0),
    ("stablemaster", Some(NpcService::StableMaster), 0),
    ("spirithealer", Some(NpcService::SpiritHealer), 0),
    ("spiritguide", Some(NpcService::SpiritGuide), 0),
    ("battlemaster", Some(NpcService::BattleMaster), 0),
    ("tabarddesigner", Some(NpcService::TabardDesigner), 0),
    ("petitioner", Some(NpcService::Petitioner), 0),
    ("guildbanker", Some(NpcService::GuildBanker), 0),
    ("mailbox", Some(NpcService::Mailbox), 0),
];

/// The service names an NPC flags word offers.
pub(crate) fn services(bits: u32) -> Vec<&'static str> {
    let flags = wrathplus_protocol::types::NpcFlags::from_bits(bits);
    SERVICES
        .iter()
        .filter(|(_, service, mask)| match service {
            Some(service) => flags.has(*service),
            None => bits & mask != 0,
        })
        .map(|(name, _, _)| *name)
        .collect()
}

fn seen_table(lua: &Lua, s: &Seen) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("guid", guid_text(s.guid))?;
    t.set("kind", s.kind)?;
    t.set("name", s.name.clone())?;
    t.set("title", s.title.clone())?;
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

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
