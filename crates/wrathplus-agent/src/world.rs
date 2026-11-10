//! Client-visible world state for agents (#64): every object the client has streamed in (units,
//! players, game objects, corpses), read from the object mirror a few times a second, and orders
//! that act on one of them by GUID. Nothing here knows more than the client: the list is what the
//! server sent this client, and reactions, quest markers and names are the client's own reading.
//!
//! While a cinematic holds the client (a new character's intro), `AgentNearby` returns a second
//! value saying so ([`CINEMATIC_NOTE`]): the list is then what the server's camera sees.
//! `AgentCinematic()` is that reason alone, or nil.
//!
//! Lua (installed by `env`): `AgentPosition()`, `AgentNearby([max_yards])`,
//! `AgentFindNpc(service [, max_yards])`, `AgentCorpse()`, `AgentTarget(guid)`, `AgentFace(guid)`,
//! `AgentInteract(guid)`. GUIDs travel as `"0x…"` hex strings. `AgentInteract` turns on click-to-move
//! (`autoInteract`), so the character walks to the object first, as a player's right-click would.

use crate::npc::{services, SERVICES};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use mlua::{Lua, Table};
use std::sync::{Arc, Mutex};
use wrathplus_app_core::app_core::session::{ObjectStore, SelfPlayer};
use wrathplus_app_core::app_core::state::QuestGiver;
use wrathplus_app_net::go_templates::GameObjectTemplates;
use wrathplus_app_net::names::{gated_rank, NameCache};
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
    /// A creature's type from its template, once the client has it: "Beast", "Humanoid",
    /// "Critter" (no XP, no loot worth a fight) ...
    pub creature_type: Option<&'static str>,
    /// NPC services, from the unit's NPC flags ([`SERVICES`]): what it offers before it is
    /// spoken to, which the stock API cannot tell (`CanMerchantRepair` needs an open window).
    pub npc: Vec<&'static str>,
    /// The title under a creature's name ("Weapons Merchant", "Blacksmithing Trainer").
    pub title: Option<String>,
    /// The quest marker over it: "available", "incomplete" or "reward".
    pub quest: Option<&'static str>,
    /// A creature's rank once its template is cached: "normal", "elite", "rareelite", "worldboss"
    /// or "rare"; and whether it is a critter (`CreatureType.dbc` 8: a rabbit, a sheep).
    pub classification: Option<&'static str>,
    pub critter: bool,
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
    /// A cinematic holds the client (`Cinematic::is_active`): the server shows the world around
    /// its flying camera, so the list is empty or far away, and the character cannot walk.
    pub cinematic: bool,
}

/// What the helpers a cinematic blocks say (`AgentNearby`'s second value, `AgentMoveTo`'s reason,
/// `AgentWalk().note`).
pub(crate) const CINEMATIC_NOTE: &str = "a cinematic is playing: the server shows the world around its camera, not around you, and you cannot walk. StopCinematic() skips it; /status says cinematic until the world is back";

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

/// Creature ranks as `UnitClassification` words them (AzerothCore `CreatureEliteType` 0..4).
pub(crate) const RANK_WORDS: [&str; 5] = ["normal", "elite", "rareelite", "worldboss", "rare"];

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
    cinematic: Option<Res<wrathplus_app_world::cinematic::Cinematic>>,
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
                let record = names
                    .as_ref()
                    .and_then(|n| n.creature_record(fields.object_entry()?));
                s.title = record.and_then(|r| r.subname.clone());
                let rank = record.map(|r| gated_rank(Some(r), Some(store)) as usize);
                s.classification = rank.and_then(|r| RANK_WORDS.get(r).copied());
                s.critter = record.is_some_and(|r| r.creature_type == 8);
                s.creature_type = record.and_then(|r| {
                    wrathplus_app_net::creature_type::creature_type_word(r.creature_type)
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
        shared.cinematic = cinematic.is_some_and(|c| c.is_active());
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

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
