//! Build 12340's arena opponents (`BattlefieldInfo.cpp`), which the stock `Blizzard_ArenaUI`
//! frames draw: the `arenaN` tokens, `GetNumArenaOpponents` and
//! `ARENA_OPPONENT_UPDATE(unit, kind)`. Two read-only Ghidra rounds on `Wow.exe`:
//! - A player created inside an arena (instance type 4, `0xbea570`) whose `PLAYER_BYTES_3` byte 3,
//!   AzerothCore's `BattlefieldArenaFaction`, is not ours (`0xbea54a`, the status-3 byte) takes an
//!   opponent slot (the player's post-create `0x6e8280` → `0x54d100`,
//!   [`ArenaOpponents::assign`]); a new guid in it resets the slot's info cache, and the slot
//!   fires `"seen"` (`0x54b840`).
//! - An opponent leaving visibility, the update's out-of-range block, fires `"unseen"` (`0x4d7230`
//!   → `0x54abf0`); `SMSG_ARENA_UNIT_DESTROYED` (0x4C7), which AzerothCore sends ahead of every
//!   arena destroy (`Object::DestroyForPlayer`), fires `"destroyed"` (`0x54b5e0` → `0x54ad10`).
//! - A hostile pet created inside an arena (`UNIT_FIELD_PETNUMBER` set, reaction under neutral,
//!   `0x7251c0 < 2`) takes a pet slot for its owner, `UNIT_FIELD_CHARMEDBY` else
//!   `UNIT_FIELD_CREATEDBY` (the unit's post-create `0x73fcc0` → `0x54d1c0`,
//!   [`ArenaOpponents::assign_pet`]), and `arenapetN` fires `"seen"` (`0x54b920`). Its leaving and
//!   its 0x4C7 are the opponents' (`0x54abf0`, `0x54ad10`, which also frees the pet slot).
//! - Leaving the battleground clears every slot and fires `"cleared"` for all ten tokens
//!   (`0x54ca50`).
//!
//! An unseen unit keeps its slot and its last snapshot, which no longer exists: the reference
//! answers `UnitClass` from the slot's cache while `UnitExists` is false (`Blizzard_ArenaUI.lua:133`).
//!
//! Unresolved, labelled: the event's `"seen"` is the string at `0xa0c3d8`, read as the one kind the
//! stock handler takes besides the three named here; a change after creation does not assign
//! (`0x6e1d70`, and `0x7402b0` for pets); the party-member-stats path into both fillers
//! (`0x6cf9b0`) needs a leading flag AzerothCore always writes 0 (`GroupHandler.cpp:1016`).

use bevy::prelude::*;
use wrathplus_protocol::types::{Field, ObjectType};
use wrathplus_protocol::wrath_battleground::Packet;
use wrathplus_ui::script::{ScriptValue, UiScript, UnitState};

use crate::app_core::arena::ArenaOpponents;
use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::app_core::state::Factions;
use crate::app_core::vm_memo::VmMemo;
use crate::names::NameCache;
use crate::net::{Guid, GuidIndex, NetCommands, Reputations};
use crate::ui_battlefield::NativeBattlegroundRest;
use crate::ui_dialog_verbs::BattlefieldQueue;
use crate::ui_unit::held::HeldUnits;

/// `Map.dbc`'s arena instance type.
const ARENA: u32 = 4;

pub fn register(app: &mut App) {
    app.init_resource::<ArenaOpponents>()
        .init_resource::<ArenaFeed>()
        .add_systems(
            Update,
            feed_arena
                .in_set(crate::app_core::schedule::UiFeed)
                .run_if(crate::app_core::ui_state::ingame_ui_up),
        );
}

/// What the feed remembers between frames.
#[derive(Resource, Default)]
pub struct ArenaFeed {
    in_arena: bool,
    opponents: Slots,
    pets: Slots,
}

/// One kind's five slots as the feed last saw them.
#[derive(Default)]
struct Slots {
    /// The unit was streamed when last looked at, so its leaving reads as `"unseen"` once.
    streamed: [bool; 5],
    /// The slot's last snapshot, the reference's info cache (`0xbe8f90`).
    cache: [Option<UnitState>; 5],
}

/// The tables and caches a held unit's snapshot reads, as `ui_unit`'s token feed takes them.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct SnapshotInputs<'w, 's> {
    tables: crate::ui_unit::SnapshotTables<'w>,
    names: Res<'w, NameCache>,
    commands: Res<'w, NetCommands>,
    factions: Option<Res<'w, Factions>>,
    reputations: Res<'w, Reputations>,
    group: Res<'w, crate::app_core::group::GroupState>,
    me: Query<'w, 's, &'static ObjectStore, With<SelfPlayer>>,
    index: Option<Res<'w, GuidIndex>>,
    stores: Query<'w, 's, &'static ObjectStore>,
}

/// The objects streamed in since the feed last ran, other than us.
type Created<'w, 's> =
    Query<'w, 's, (&'static ObjectStore, &'static Guid), (Added<ObjectStore>, Without<SelfPlayer>)>;

/// `kind` for one token, as the event carries it.
fn event(token: String, kind: &str) -> (String, String) {
    (token, kind.to_string())
}

/// The memo of what each token last had pushed, opponents then pets.
type Pushed = [[Option<UnitState>; 5]; 2];

#[allow(clippy::too_many_arguments)]
fn feed_arena(
    script: Option<NonSendMut<UiScript>>,
    queue: Option<Res<BattlefieldQueue>>,
    mut table: ResMut<ArenaOpponents>,
    mut feed: ResMut<ArenaFeed>,
    created: Created,
    rest: Option<ResMut<NativeBattlegroundRest>>,
    inputs: SnapshotInputs,
    mut memo: Local<VmMemo<Pushed>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let (in_arena, ours) = queue.as_deref().map_or((false, 0), |q| {
        (q.active_instance_type() == ARENA, q.arena_faction)
    });
    let mut events = Vec::new();

    // `0x54ca50`: the battleground's end clears every slot, and says so for each token.
    if feed.in_arena && !in_arena {
        *table = ArenaOpponents::default();
        *feed = ArenaFeed::default();
        for n in 1..=5 {
            events.push(event(format!("arena{n}"), "cleared"));
            events.push(event(format!("arenapet{n}"), "cleared"));
        }
    }
    feed.in_arena = in_arena;

    let lookup = crate::target::GuidStores {
        index: inputs.index.as_deref(),
        stores: &inputs.stores,
    };
    let held = HeldUnits {
        names: &inputs.names,
        commands: &inputs.commands,
        factions: inputs.factions.as_deref(),
        reputations: &inputs.reputations,
        group: &inputs.group,
        self_store: inputs.me.iter().next(),
        classes: inputs.tables.classes(),
        types: inputs.tables.types(&inputs.names),
        families: inputs.tables.families(),
        relations: crate::target::RelationContext::new(Some(&inputs.names), &lookup),
    };

    if in_arena {
        for (store, guid) in &created {
            let fields = &store.0;
            match fields.object_type() {
                // `0x6e8280`: a player of the other arena faction.
                Some(ObjectType::Player) => {
                    let faction = fields
                        .get(Field::PlayerBytes3)
                        .map_or(0, |b| (b >> 24) as u8);
                    if faction == ours {
                        continue;
                    }
                    if let Some((slot, fresh)) = table.assign(guid.0) {
                        if fresh {
                            feed.opponents.cache[slot] = None;
                        }
                        feed.opponents.streamed[slot] = true;
                        events.push(event(format!("arena{}", slot + 1), "seen"));
                    }
                }
                // `0x73fcc0`: a hostile pet (`UnitReaction` 1 or 2 is `0x7251c0`'s 0 or 1).
                Some(ObjectType::Unit) if fields.unit_pet_number() != 0 => {
                    if !(1..=2).contains(&held.state(store, guid.0).reaction) {
                        continue;
                    }
                    let owner = fields
                        .unit_charmed_by()
                        .filter(|&g| g != 0)
                        .or_else(|| fields.unit_created_by())
                        .unwrap_or(0);
                    let before = table.pets;
                    if let Some(slot) = table.assign_pet(guid.0, owner) {
                        if before[slot] != guid.0 {
                            feed.pets.cache[slot] = None;
                        }
                        feed.pets.streamed[slot] = true;
                        events.push(event(format!("arenapet{}", slot + 1), "seen"));
                    }
                }
                _ => {}
            }
        }
    }

    // `0x54b5e0`: consumed everywhere, acted on only in an arena; a pet's slot is freed.
    if let Some(mut rest) = rest {
        rest.0.retain(|packet| {
            let Packet::ArenaUnitDestroyed { guid } = *packet else {
                return true;
            };
            if !in_arena {
                return false;
            }
            if let Some(slot) = table.slot_of(guid) {
                feed.opponents.streamed[slot] = false;
                events.push(event(format!("arena{}", slot + 1), "destroyed"));
            } else if let Some(slot) = table.pet_slot_of(guid) {
                table.pets[slot] = 0;
                table.pet_owners[slot] = 0;
                feed.pets.streamed[slot] = false;
                feed.pets.cache[slot] = None;
                events.push(event(format!("arenapet{}", slot + 1), "destroyed"));
            }
            false
        });
    }

    let feed = &mut *feed;
    let store_of = |guid: u64| {
        let entity = *inputs.index.as_deref()?.0.get(&guid)?;
        inputs.stores.get(entity).ok()
    };
    for (prefix, guids, slots) in [
        ("arena", table.arena, &mut feed.opponents),
        ("arenapet", table.pets, &mut feed.pets),
    ] {
        for (slot, guid) in guids.into_iter().enumerate() {
            if guid == 0 {
                slots.cache[slot] = None;
                continue;
            }
            match store_of(guid) {
                Some(store) => {
                    slots.streamed[slot] = true;
                    slots.cache[slot] = Some(held.state(store, guid));
                }
                None => {
                    // `0x4d7230`: gone without a destroy first, so out of range.
                    if std::mem::take(&mut slots.streamed[slot]) {
                        events.push(event(format!("{prefix}{}", slot + 1), "unseen"));
                    }
                    if let Some(cached) = slots.cache[slot].as_mut() {
                        cached.exists = false;
                        cached.has_object = false;
                    }
                }
            }
        }
    }

    script.set_arena_units(table.arena, table.pets);
    let [opponents, pets] = memo.get(&script);
    for (prefix, pushed, slots) in [
        ("arena", opponents, &feed.opponents),
        ("arenapet", pets, &feed.pets),
    ] {
        for (slot, (pushed, cached)) in pushed.iter_mut().zip(&slots.cache).enumerate() {
            if pushed != cached {
                script.set_unit(&format!("{prefix}{}", slot + 1), cached.clone());
                pushed.clone_from(cached);
            }
        }
    }
    for (unit, kind) in events {
        script.fire_event(
            "ARENA_OPPONENT_UPDATE",
            vec![ScriptValue::Str(unit), ScriptValue::Str(kind)],
        );
    }
}

#[cfg(test)]
#[path = "wrath_arena_tests.rs"]
mod tests;
