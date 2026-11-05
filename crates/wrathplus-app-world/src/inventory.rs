//! The player's inventory as the reference's walker (`0x622420`) reads it: the bag, bank and
//! keyring geometry, the item a Lua-space slot holds, and the walks that count, find and list
//! carried items. Pure over the descriptor and the object manager, so world code (death,
//! spell, target) and the UI feeds share it; `crate::ui_items` re-exports it for its own users.

use wrathplus_protocol::types::{BAG_PLAYER_INVENTORY, SLOT_PACK_FIRST};
use wrathplus_protocol::ObjectFields;

use crate::app_core::session::ObjectStore;
use crate::items::Items;
use crate::net::{NetCommands, Objects};

/// `wrathplus_ui`'s `EQUIPMENT_BAG`, the paper doll's pseudo-bag; a world module names it here
/// without the Lua crate, and a `ui_items` test pins the two together.
pub const EQUIPMENT_BAG: i64 = -100;

/// The backpack's capacity (`PLAYER_FIELD_PACK_SLOT_1..`).
pub const PACK_SLOTS: u8 = 16;

/// Worn equipment, `INV_SLOT` 0..18 (head through tabard): the first region of the walker
/// `0x622420`.
pub(crate) const EQUIPMENT_SLOTS: u8 = 19;

/// The first equipped-bag inventory slot (`INV_SLOT` 19..22 hold bags 1..4).
pub const BAG_SLOT_FIRST: u8 = 19;

/// Equipped bag count (live-API bag ids 1..=4).
pub const BAGS: u8 = 4;

/// The bank's 24 generic slots, wire 39..62, streamed at login like the backpack.
pub const BANK_SLOTS: u8 = 24;

/// The first bank generic slot in the player array (vmangos `BANK_SLOT_ITEM_START`).
pub const BANK_SLOT_FIRST: u8 = 39;

/// Bank bag count (live-API bag ids [`BANK_BAG_ID_FIRST`]..=10).
pub const BANK_BAGS: u8 = 6;

/// `BANK_CONTAINER` (`BankFrame.lua:1`).
pub const BANK_CONTAINER: i64 = -1;

/// Bank bags are containers 5..=10 (`NUM_BAG_SLOTS + 1..`).
pub const BANK_BAG_ID_FIRST: i64 = 5;

/// `KEYRING_CONTAINER` (`MainMenuBarBagButtons.lua:1`).
pub const KEYRING_CONTAINER: i64 = -2;

/// `BAG_FAMILY_MASK_KEYS`: the `BagFamily` bit the server routes into the keyring
/// (`Player::_CanStoreItem`) and build 12340's `HasKey` searches for (`0x51aea4`, mask `0x100`).
pub const BAG_FAMILY_KEYS: u32 = 0x100;

/// Keyring slots usable at `level`, the ladder `GetKeyRingSize` (`ContainerFrame.lua:773`) and the
/// server's `Player::GetMaxKeyringSize` (`Player.h:985`) share: 4 below 40, 8 at 40, 12 at 50,
/// 16 above 60. The feed sizes the keyring container with it.
pub fn keyring_size(level: u32) -> u32 {
    match level {
        61.. => 16,
        50..=60 => 12,
        40..=49 => 8,
        _ => 4,
    }
}

/// Lua `(bag, 1-based slot)` to the wire `(bag_index, slot)`, the one mapping every drain shares.
/// The backpack, the bank's generic slots, the keyring and the doll ([`EQUIPMENT_BAG`]: live id
/// minus one, 1..=23 and the bank-bag buttons 64..=69) all land on [`BAG_PLAYER_INVENTORY`], so a
/// move between them is `CMSG_SWAP_INV_ITEM`; an equipped or bank bag is addressed by its own
/// player-array slot, with a 0-based inner slot. The keyring spans the wire's 16 positions, not
/// [`keyring_size`]: a click past the unlocked count is the server's to refuse.
/// Build 12340's storage geometry. AzerothCore `Player.h:698-718` defines bank 39..67, bags
/// 67..74 and keyring 86..118 (exclusive ends); `Player.h:1379` exposes all 32 keyring slots.
#[derive(Clone, Copy)]
pub struct StorageSlots {
    pub bank_slots: u8,
    pub bank_bags: u8,
    pub bank_bag_first: u8,
    pub keyring_first: u8,
    pub keyring_slots: u8,
}

impl StorageSlots {
    /// The geometry.
    pub const GEOMETRY: Self = Self {
        bank_slots: 28,
        bank_bags: 7,
        bank_bag_first: 67,
        keyring_first: 86,
        keyring_slots: 32,
    };

    pub fn keyring_size(self, level: u32) -> u32 {
        if self.keyring_slots == 32 {
            32
        } else {
            keyring_size(level)
        }
    }

    pub fn last_bag(self) -> i64 {
        BANK_BAG_ID_FIRST + i64::from(self.bank_bags) - 1
    }
}

/// The item guid in a Lua-space bag slot; for [`EQUIPMENT_BAG`], `slot0` is the wire slot, as in
/// [`wire_pos`].
pub fn slot_guid(store: &ObjectFields, bag: i64, slot0: u8, objects: &Objects) -> Option<u64> {
    match bag {
        0 => store.player_pack_slot(slot0).filter(|g| *g != 0),
        1..=4 => {
            let bag_guid = store
                .player_inv_slot(BAG_SLOT_FIRST + bag as u8 - 1)
                .filter(|g| *g != 0)?;
            objects
                .object(bag_guid)?
                .container_slot(slot0)
                .filter(|g| *g != 0)
        }
        BANK_CONTAINER => store.player_bank_slot(slot0).filter(|g| *g != 0),
        KEYRING_CONTAINER => store.player_keyring_slot(slot0).filter(|g| *g != 0),
        b if (BANK_BAG_ID_FIRST..=StorageSlots::GEOMETRY.last_bag()).contains(&b) => {
            let bag_guid = store
                .player_bank_bag_slot((bag - BANK_BAG_ID_FIRST) as u8)
                .filter(|g| *g != 0)?;
            objects
                .object(bag_guid)?
                .container_slot(slot0)
                .filter(|g| *g != 0)
        }
        // The doll: the bank-bag buttons (wire 63..68) read their own array, since the INV
        // array's accessor caps at 23.
        EQUIPMENT_BAG
            if (StorageSlots::GEOMETRY.bank_bag_first
                ..StorageSlots::GEOMETRY.bank_bag_first + StorageSlots::GEOMETRY.bank_bags)
                .contains(&slot0) =>
        {
            store
                .player_bank_bag_slot(slot0 - StorageSlots::GEOMETRY.bank_bag_first)
                .filter(|g| *g != 0)
        }
        EQUIPMENT_BAG => store.player_inv_slot(slot0).filter(|g| *g != 0),
        _ => None,
    }
}

/// `(item guid, stack count)` at a Lua-space `(bag, 1-based slot)`, `(0, 0)` when empty:
/// [`PendingItemOps`]'s baseline, since a partial split or destroy changes only the count.
pub fn slot_guid_count(
    store: Option<&ObjectStore>,
    bag: i64,
    slot1: u32,
    objects: &Objects,
) -> (u64, u32) {
    let Some(store) = store else {
        return (0, 0);
    };
    let slot0 = slot1.saturating_sub(1) as u8;
    match slot_guid(&store.0, bag, slot0, objects) {
        Some(guid) => {
            let count = objects
                .object(guid)
                .and_then(|f| f.item_stack_count())
                .unwrap_or(1);
            (guid, count)
        }
        None => (0, 0),
    }
}

/// The walker's section mask (`0x622420`'s `ebx`) over the player's 113-guid slot array. A
/// container in an enabled section is always recursed into: sections gate only at the player's
/// root, and no caller sets the recursion bit `0x10`. Buyback (69-80) has no bit and is never
/// walked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InventoryScope {
    /// `0x01`: worn equipment, slots 0-18.
    equipment: bool,
    /// `0x02`: the four equipped bag slots 19-22, and their contents.
    bags: bool,
    /// `0x04`: the backpack, slots 23-38.
    backpack: bool,
    /// `0x08`: bank items (39-62) and the six bank-bag slots (63-68) with their contents.
    bank: bool,
    /// `0x40`: the keyring band.
    keyring: bool,
}

impl InventoryScope {
    /// `0x47`, what mask 0 becomes: `0x622420` ORs it in at the local player's root
    /// (`0x622434`-`0x622439`; the `CGPlayer_C` ctor sets that flag, `0x5dd44d`). No bank.
    pub const DEFAULT: Self = Self {
        equipment: true,
        bags: true,
        backpack: true,
        bank: false,
        keyring: true,
    };
    /// `0x01`: the equip-vs-use fork's first stage, "is a copy already worn?" (`0x4e5fe7`).
    pub(crate) const EQUIPMENT_ONLY: Self = Self {
        equipment: true,
        bags: false,
        backpack: false,
        bank: false,
        keyring: false,
    };
    /// `0x4F`: [`Self::DEFAULT`] plus the bank, what mask `8` becomes. The quest surfaces pass it
    /// to `0x622130`: `GetQuestLogLeaderBoard` (`0x4e0579`, `0x4e0592`), the ADD_ITEM toast
    /// (`0x5dd0f5`), the turn-in predicate (`0x4df778`) and `GetAbandonQuestItems` (`0x4dfc8a`).
    pub const QUEST_ITEMS: Self = Self {
        bank: true,
        ..Self::DEFAULT
    };
    /// Bags and backpack only, narrower than any reference mask: the action-bar and reagent counts
    /// use it, where the reference's `0x47` also counts worn gear and the keyring.
    pub const CARRIED: Self = Self {
        equipment: false,
        bags: true,
        backpack: true,
        bank: false,
        keyring: false,
    };
}

/// The reference's inventory walk (`0x622420`): one ascending pass over the player's slot array,
/// recursing into each container as it is passed, `scope` gating the sections. `visit` gets the
/// wire `(bag_index, slot)` and the guid, and returns `Some` to stop. The order is the reference's
/// and load-bearing: equipment 0-18, each bag slot 19-22 then its contents, backpack 23-38, bank
/// 39-62, each bank bag 63-68 then its contents, keyring.
fn walk_inventory<T>(
    store: &ObjectFields,
    objects: &Objects,
    scope: InventoryScope,
    mut visit: impl FnMut(u8, u8, u64) -> Option<T>,
) -> Option<T> {
    // A container's contents, addressed by its player-array slot as the wire bag byte.
    let contents =
        |bag_slot: u8, bag_guid: u64, visit: &mut dyn FnMut(u8, u8, u64) -> Option<T>| {
            let bag_fields = objects.object(bag_guid)?;
            let num_slots = bag_fields.container_num_slots().unwrap_or(0).min(36) as u8;
            (0..num_slots).find_map(|j| {
                let guid = bag_fields.container_slot(j).unwrap_or(0);
                (guid != 0).then(|| visit(bag_slot, j, guid)).flatten()
            })
        };

    if scope.equipment {
        for i in 0..EQUIPMENT_SLOTS {
            let guid = store.player_inv_slot(i).unwrap_or(0);
            if guid != 0 {
                if let Some(hit) = visit(BAG_PLAYER_INVENTORY, i, guid) {
                    return Some(hit);
                }
            }
        }
    }
    if scope.bags {
        for bag in 0..BAGS {
            let bag_slot = BAG_SLOT_FIRST + bag;
            let bag_guid = store.player_inv_slot(bag_slot).unwrap_or(0);
            if bag_guid == 0 {
                continue;
            }
            // The bag object is a candidate itself, before its contents.
            if let Some(hit) = visit(BAG_PLAYER_INVENTORY, bag_slot, bag_guid) {
                return Some(hit);
            }
            if let Some(hit) = contents(bag_slot, bag_guid, &mut visit) {
                return Some(hit);
            }
        }
    }
    if scope.backpack {
        for i in 0..PACK_SLOTS {
            let guid = store.player_pack_slot(i).unwrap_or(0);
            if guid != 0 {
                if let Some(hit) = visit(BAG_PLAYER_INVENTORY, SLOT_PACK_FIRST + i, guid) {
                    return Some(hit);
                }
            }
        }
    }
    if scope.bank {
        for i in 0..StorageSlots::GEOMETRY.bank_slots {
            let guid = store.player_bank_slot(i).unwrap_or(0);
            if guid != 0 {
                if let Some(hit) = visit(BAG_PLAYER_INVENTORY, BANK_SLOT_FIRST + i, guid) {
                    return Some(hit);
                }
            }
        }
        for bag in 0..StorageSlots::GEOMETRY.bank_bags {
            let bag_slot = StorageSlots::GEOMETRY.bank_bag_first + bag;
            let bag_guid = store.player_bank_bag_slot(bag).unwrap_or(0);
            if bag_guid == 0 {
                continue;
            }
            if let Some(hit) = visit(BAG_PLAYER_INVENTORY, bag_slot, bag_guid) {
                return Some(hit);
            }
            if let Some(hit) = contents(bag_slot, bag_guid, &mut visit) {
                return Some(hit);
            }
        }
    }
    if scope.keyring {
        // The addressable 16; the reference walks all 32, but the rest are always empty.
        for i in 0..StorageSlots::GEOMETRY.keyring_slots {
            let guid = store.player_keyring_slot(i).unwrap_or(0);
            if guid != 0 {
                if let Some(hit) = visit(
                    BAG_PLAYER_INVENTORY,
                    StorageSlots::GEOMETRY.keyring_first + i,
                    guid,
                ) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// How many of `entry` the player holds within `scope`, summing each copy's
/// `ITEM_FIELD_STACK_COUNT`: the reference's `0x622130(itemId, mask)`, whose per-item predicate
/// `0x622160` tests `OBJECT_FIELD_ENTRY` (`0x622166`) and adds `[+0x20]` (`0x622177`). A quest
/// objective counts banked copies ([`InventoryScope::QUEST_ITEMS`]); an action-button or reagent
/// count does not.
pub fn count_of(store: &ObjectFields, objects: &Objects, entry: u32, scope: InventoryScope) -> u32 {
    let mut total = 0u32;
    walk_inventory::<()>(store, objects, scope, |_, _, guid| {
        if let Some(fields) = objects.object(guid) {
            if fields.object_entry() == Some(entry) {
                total += fields.item_stack_count().unwrap_or(1);
            }
        }
        None
    });
    total
}

/// Every carried entry's count in one walk ([`InventoryScope::CARRIED`]): [`count_of`] for a
/// caller asking about many entries in a frame.
pub fn carried_counts(
    store: &ObjectFields,
    objects: &Objects,
) -> std::collections::HashMap<u32, u32> {
    let mut counts = std::collections::HashMap::new();
    walk_inventory::<()>(store, objects, InventoryScope::CARRIED, |_, _, guid| {
        if let Some(fields) = objects.object(guid) {
            if let Some(entry) = fields.object_entry() {
                *counts.entry(entry).or_insert(0) += fields.item_stack_count().unwrap_or(1);
            }
        }
        None
    });
    counts
}

/// How far [`find_item`] looks and which copies count: the two mode bits the reference's callers
/// pass the walker `0x622420`.
#[derive(Clone, Copy, Default)]
pub struct ItemSearch {
    /// Mode `1` alone: equipment slots 0-18 only, the equip-vs-use fork's "already worn?" stage
    /// (`0x4e5fe7`).
    pub equipment_only: bool,
    /// Mode bit `0x20`: skip a copy whose live `ITEM_FIELD_SPELL_CHARGES[0]` is 0, set by the use
    /// leg for a template with finite charges. Containers are never skipped.
    pub live_charges_only: bool,
}

/// Where a copy of `entry` is: the wire `(bag_index, slot)` and the instance guid, the first hit
/// of [`walk_inventory`] (predicate `OBJECT_FIELD_ENTRY` equality). The bank is not in scope.
pub fn find_item(
    store: &ObjectFields,
    objects: &Objects,
    entry: u32,
    search: ItemSearch,
) -> Option<(u8, u8, u64)> {
    let scope = if search.equipment_only {
        InventoryScope::EQUIPMENT_ONLY
    } else {
        InventoryScope::DEFAULT
    };
    walk_inventory(store, objects, scope, |bag, slot, guid| {
        let f = objects.object(guid)?;
        if f.object_entry() != Some(entry) {
            return None;
        }
        // Under the charges filter the instance needs uses left; a container is exempt.
        if search.live_charges_only
            && f.container_num_slots().is_none_or(|n| n == 0)
            && f.item_spell_charges(0).is_some_and(|c| c == 0)
        {
            return None;
        }
        Some((bag, slot, guid))
    })
}

/// Every occupied slot `scope` reaches, in the walker's order, for a predicate judged afterwards
/// against the template cache.
pub fn collect_inventory(
    store: &ObjectFields,
    objects: &Objects,
    scope: InventoryScope,
) -> Vec<(u8, u8, u64)> {
    let mut out = Vec::new();
    walk_inventory(store, objects, scope, |bag, slot, guid| {
        out.push((bag, slot, guid));
        None::<()>
    });
    out
}

/// Build 12340's `HasKey()` (`0x51ae80`), the gate on the keyring's existence in the UI: the
/// finder `0x754ac0` with predicate `0x753b00`, `BagFamily & 0x100` (`0x7072b0`), at mode `0x4f`,
/// so a key in the bank counts. A slot whose template is still in flight reads as no key until the
/// answer lands.
pub fn has_key(
    store: &ObjectFields,
    objects: &Objects,
    items: &Items,
    commands: &NetCommands,
) -> bool {
    // Every guid mode `0x4f` reaches, in the walker's order: each bag's contents follow its slot.
    fn contents(bag_guid: u64, objects: &Objects, out: &mut Vec<u64>) {
        let Some(f) = objects.object(bag_guid) else {
            return;
        };
        let n = f.container_num_slots().unwrap_or(0).min(36) as u8;
        out.extend((0..n).map(|j| f.container_slot(j).unwrap_or(0)));
    }
    let mut guids = Vec::new();
    for i in 0..EQUIPMENT_SLOTS {
        guids.push(store.player_inv_slot(i).unwrap_or(0));
    }
    for bag in 0..BAGS {
        let bag_guid = store.player_inv_slot(BAG_SLOT_FIRST + bag).unwrap_or(0);
        guids.push(bag_guid);
        contents(bag_guid, objects, &mut guids);
    }
    for i in 0..PACK_SLOTS {
        guids.push(store.player_pack_slot(i).unwrap_or(0));
    }
    for i in 0..StorageSlots::GEOMETRY.bank_slots {
        guids.push(store.player_bank_slot(i).unwrap_or(0));
    }
    for bag in 0..StorageSlots::GEOMETRY.bank_bags {
        let bag_guid = store.player_bank_bag_slot(bag).unwrap_or(0);
        guids.push(bag_guid);
        contents(bag_guid, objects, &mut guids);
    }
    for i in 0..StorageSlots::GEOMETRY.keyring_slots {
        guids.push(store.player_keyring_slot(i).unwrap_or(0));
    }
    guids.into_iter().any(|guid| {
        if guid == 0 {
            return false;
        }
        let Some(entry) = objects.object(guid).and_then(|f| f.object_entry()) else {
            return false;
        };
        items
            .template(entry, guid, commands)
            .is_some_and(|t| t.bag_family & BAG_FAMILY_KEYS != 0)
    })
}
