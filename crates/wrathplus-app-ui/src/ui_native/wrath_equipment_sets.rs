//! The gear manager's saved sets from native equipment-set traffic. AzerothCore sends
//! `SMSG_EQUIPMENT_SET_LIST` only at login (`Player::SendInitialPacketsBeforeAddToMap`), so the
//! client keeps its own copy: a save or delete updates it at once, and `SMSG_EQUIPMENT_SET_SAVED`
//! brings a new set's GUID (`Player::SetEquipmentSet`). A set use's
//! `SMSG_EQUIPMENT_SET_USE_RESULT` (0 done, 4 bags full) becomes `EQUIPMENT_SWAP_FINISHED`.
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
pub(crate) use crate::net::wrath_inventory_capture::EquipmentSets;
use crate::net::wrath_inventory_capture::NativeInventoryPackets;
use crate::net::{ClientCommand, NetCommands, Objects};
use bevy::prelude::*;
use std::collections::HashMap;
use wrathplus_protocol::wrath_inventory::{Command, EquipmentSet, Packet, EQUIPMENT_SLOTS};
use wrathplus_ui::script::{
    EquipmentSetRequest, EquipmentSetView, UiScript, EQUIPMENT_SET_IGNORED_SLOT,
};

/// `MAX_EQUIPMENT_SET_INDEX` (AzerothCore Player.h).
const MAX_SETS: u32 = 10;
/// The walker's player-array bag byte, also AzerothCore's `INVENTORY_SLOT_BAG_0`.
const PLAYER_ARRAY: u8 = 255;
const IGNORED: u64 = EQUIPMENT_SET_IGNORED_SLOT as u64;

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        Update,
        feed_equipment_sets
            .after(super::wrath_item_refunds::feed_item_refunds)
            .in_set(crate::app_core::schedule::UiFeed)
            .run_if(crate::app_core::ui_state::ingame_ui_up),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn feed_equipment_sets(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<NativeInventoryPackets>,
    mut sets: ResMut<EquipmentSets>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    objects: Objects,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<Vec<EquipmentSetView>>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let (finished, deletes) = absorb(&mut packets.0, &mut sets);
    for (completed, name) in finished {
        script.equipment_swap_finished(completed, name);
    }
    for set_guid in deletes {
        let _ = commands.0.send(ClientCommand::NativeInventory(
            Command::EquipmentSetDelete { set_guid },
        ));
    }
    // Where each carried item is now: equipped slots and bag slots, by GUID.
    let carried: HashMap<u64, (u8, u8)> = self_q
        .iter()
        .next()
        .map(|store| {
            crate::inventory::collect_inventory(
                &store.0,
                &objects,
                crate::inventory::InventoryScope::DEFAULT,
            )
            .into_iter()
            .map(|(bag, slot, guid)| (guid, (bag, slot)))
            .collect()
        })
        .unwrap_or_default();
    let mut worn = [0u64; EQUIPMENT_SLOTS];
    for (&guid, &(bag, slot)) in &carried {
        if bag == PLAYER_ARRAY && usize::from(slot) < EQUIPMENT_SLOTS {
            worn[usize::from(slot)] = guid;
        }
    }
    for request in script.take_equipment_set_requests() {
        if let Some(command) = apply(&mut sets, request, &worn, &carried) {
            let _ = commands.0.send(ClientCommand::NativeInventory(command));
        }
    }
    let entry = |guid| objects.object(guid).and_then(|o| o.object_entry());
    let views: Vec<EquipmentSetView> = sets.sets.values().map(|s| set_view(s, &entry)).collect();
    let (last, reset) = last.get_reset(&script);
    if reset || last.as_ref() != Some(&views) {
        *last = Some(views.clone());
        script.set_equipment_sets(views);
    }
}

/// One set as the VM reads it: the server's index is 12340's record id (`[rec+0x10]`), and each
/// worn item is its entry, 0 while unknown.
fn set_view(set: &EquipmentSet, entry: &dyn Fn(u64) -> Option<u32>) -> EquipmentSetView {
    EquipmentSetView {
        id: set.index,
        name: set.name.clone(),
        icon: set.icon.clone(),
        items: set.items.map(|item| match item {
            0 => 0,
            IGNORED => EQUIPMENT_SET_IGNORED_SLOT,
            guid => entry(guid).unwrap_or(0),
        }),
        locked: false,
    }
}

/// Take the equipment-set receipts out of the capture into `sets`, returning each use's
/// `(completed, set name)` and the GUIDs of sets deleted before they were numbered; every other
/// receipt stays, in order, for its own consumer.
fn absorb(
    packets: &mut std::collections::VecDeque<Packet>,
    sets: &mut EquipmentSets,
) -> (Vec<(bool, Option<String>)>, Vec<u64>) {
    let mut finished = Vec::new();
    let mut deletes = Vec::new();
    let mut kept = std::collections::VecDeque::new();
    for packet in packets.drain(..) {
        match packet {
            Packet::EquipmentSetList { sets: list } => {
                sets.sets = list.into_iter().map(|s| (s.index, s)).collect();
            }
            Packet::EquipmentSetSaved { index, set_guid } => {
                if let Some(at) = sets.pending_delete.iter().position(|&i| i == index) {
                    sets.pending_delete.remove(at);
                    deletes.push(set_guid);
                } else if let Some(set) = sets.sets.get_mut(&index) {
                    set.set_guid = set_guid;
                }
            }
            Packet::EquipmentSetUseResult { result } => {
                finished.push((result == 0, sets.pending_use.take()));
            }
            other => kept.push_back(other),
        }
    }
    *packets = kept;
    (finished, deletes)
}

/// One gear-manager gesture against the local copy, and the packet it sends.
fn apply(
    sets: &mut EquipmentSets,
    request: EquipmentSetRequest,
    worn: &[u64; EQUIPMENT_SLOTS],
    carried: &HashMap<u64, (u8, u8)>,
) -> Option<Command> {
    let by_name = |sets: &EquipmentSets, name: &str| {
        sets.sets.values().find(|s| s.name == name).map(|s| s.index)
    };
    match request {
        EquipmentSetRequest::Save {
            name,
            icon,
            ignored,
        } => {
            // Saving over a name keeps its slot and GUID; a new set takes the first free index.
            let index = by_name(sets, &name)
                .or_else(|| (0..MAX_SETS).find(|i| !sets.sets.contains_key(i)))?;
            let set_guid = sets.sets.get(&index).map_or(0, |s| s.set_guid);
            let items = std::array::from_fn(|i| if ignored[i] { IGNORED } else { worn[i] });
            sets.sets.insert(
                index,
                EquipmentSet {
                    set_guid,
                    index,
                    name: name.clone(),
                    icon: icon.clone(),
                    items,
                },
            );
            Some(Command::EquipmentSetSave {
                set_guid,
                index,
                name,
                icon,
                items,
            })
        }
        EquipmentSetRequest::Delete { name } => {
            let set = sets.sets.remove(&by_name(sets, &name)?)?;
            // A set the server has not numbered yet is deleted when its GUID arrives.
            if set.set_guid == 0 {
                sets.pending_delete.push(set.index);
                return None;
            }
            Some(Command::EquipmentSetDelete {
                set_guid: set.set_guid,
            })
        }
        EquipmentSetRequest::Use { name } => {
            let set = &sets.sets[&by_name(sets, &name)?];
            // Each slot's item and where it is; AzerothCore finds the item by GUID and reads the
            // position only for its log, so an item no longer carried goes as 255/255. An ignored
            // or empty slot carries no position.
            let items = set.items.map(|item| match item {
                0 | IGNORED => (item, 0, 0),
                guid => {
                    let (bag, slot) = carried
                        .get(&guid)
                        .copied()
                        .unwrap_or((PLAYER_ARRAY, PLAYER_ARRAY));
                    (guid, bag, slot)
                }
            });
            sets.pending_use = Some(name);
            Some(Command::EquipmentSetUse { items })
        }
    }
}

#[cfg(test)]
#[path = "wrath_equipment_sets_tests.rs"]
mod tests;
