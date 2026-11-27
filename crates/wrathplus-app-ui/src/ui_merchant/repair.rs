//! The repair-cost walk: each carried or banked item's price, the carried total and whether any
//! carried item is damaged, as `GetRepairAllCost` (`0x585990`) and the bag tooltips read them.

use wrathplus_ui::script::RepairCosts;

use super::RepairTables;
use crate::items::Items;
use crate::net::{NetCommands, Objects};

/// One item's repair cost by `0x708540`'s arithmetic ([`wrathplus_formats::DurabilityTables`]),
/// after the price `discount` (`0x584b20`); `None` unless it is below its max durability and not
/// wrapped (`ITEM_FIELD_FLAGS` `0x8`, `0x585a5b`-`0x585aad`), 0 for an unknown template.
pub(super) fn item_repair_cost(
    guid: u64,
    objects: &Objects,
    items: &Items,
    tables: &RepairTables,
    commands: &NetCommands,
    discount: f64,
) -> Option<u32> {
    let obj = objects.object(guid)?;
    let cur = obj.item_durability().unwrap_or(0);
    let max = obj.item_max_durability().unwrap_or(0);
    if max == 0 || cur >= max || obj.item_flags().unwrap_or(0) & 8 != 0 {
        return None;
    }
    let entry = obj.object_entry().unwrap_or(0);
    let Some(t) = (entry != 0)
        .then(|| items.template(entry, guid, commands))
        .flatten()
    else {
        return Some(0);
    };
    let (level, quality, class, subclass) = (t.item_level, t.quality, t.class, t.subclass);
    Some(
        tables
            .0
            .repair_cost(max - cur, level, quality, class, subclass, discount),
    )
}

/// Each damaged carried or banked item's cost, keyed as the bindings take it, and whether any
/// carried item is damaged.
pub(super) fn repair_costs(
    store: &wrathplus_protocol::ObjectFields,
    objects: &Objects,
    items: &Items,
    tables: &RepairTables,
    commands: &NetCommands,
    discount: f64,
) -> (RepairCosts, bool) {
    // `GetRepairAllCost`'s second value: a damaged carried item, priced or not (`0x585ab5`).
    let mut damaged = false;
    let mut cost = |guid: u64, carried: bool| {
        let c = (guid != 0)
            .then(|| item_repair_cost(guid, objects, items, tables, commands, discount))??;
        damaged |= carried;
        (c > 0).then_some(c)
    };
    let mut costs = RepairCosts::default();
    for i in 0..19u8 {
        if let Some(c) = cost(store.player_inv_slot(i).unwrap_or(0), true) {
            costs.equipped.insert(u32::from(i) + 1, c);
        }
    }
    for i in 0..16u8 {
        if let Some(c) = cost(store.player_pack_slot(i).unwrap_or(0), true) {
            costs.bags.insert((0, u32::from(i) + 1), c);
        }
    }
    for i in 0..crate::inventory::BANK_SLOTS {
        if let Some(c) = cost(store.player_bank_slot(i).unwrap_or(0), false) {
            costs
                .bags
                .insert((crate::inventory::BANK_CONTAINER, u32::from(i) + 1), c);
        }
    }
    // Bags 1-4 are inventory slots 19-22, bank bags 5-11 the bank bag slots.
    let bags = (1..=4u8)
        .map(|b| (i64::from(b), store.player_inv_slot(18 + b)))
        .chain((0..crate::inventory::BANK_BAGS).map(|i| {
            (
                crate::inventory::BANK_BAG_ID_FIRST + i64::from(i),
                store.player_bank_bag_slot(i),
            )
        }));
    for (bag, bag_guid) in bags {
        let bag_guid = bag_guid.unwrap_or(0);
        if bag_guid == 0 {
            continue;
        }
        let Some(b) = objects.object(bag_guid) else {
            continue;
        };
        let n = b.container_num_slots().unwrap_or(0).min(36) as u8;
        for j in 0..n {
            if let Some(c) = b.container_slot(j).and_then(|g| cost(g, bag <= 4)) {
                costs.bags.insert((bag, u32::from(j) + 1), c);
            }
        }
    }
    (costs, damaged)
}

/// `GetRepairAllCost`'s sum of the per-item costs (`0x4fbd60`): equipped and bags, never the bank.
pub(super) fn repair_all_cost(costs: &RepairCosts) -> u32 {
    let carried = costs
        .bags
        .iter()
        .filter(|((bag, _), _)| (0..=4).contains(bag))
        .map(|(_, c)| c);
    let total: u64 = costs
        .equipped
        .values()
        .chain(carried)
        .map(|&c| u64::from(c))
        .sum();
    total.min(u64::from(u32::MAX)) as u32
}
