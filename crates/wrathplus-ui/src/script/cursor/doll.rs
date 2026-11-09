//! The paper-doll cursor verbs: an equipped slot is an item source under
//! [`super::EQUIPMENT_BAG`], so the shared cursor code handles its cancel, clear and world drop.

use mlua::{Lua, MultiValue, Value};

use crate::script::binding_abi::{flag, ftol_arg, inventory_slot0};
use crate::script::container::ContainerMove;
use crate::script::Model;

use super::{queue_cursor_update, queue_lock_changed, CursorItem, CursorPayload, EQUIPMENT_BAG};

/// `PickupInventoryItem(id)` (`0x4c7300`), the doll slot's click, on the bag click's model: a
/// fitting whole item moves in and the cursor clears, with no hop; a split or a misfit stays
/// held. Returns whether to repaint.
pub(super) fn pickup_inventory_item(model: &mut Model, id: u32) -> bool {
    // Head 1 to Tabard 19, the bag slots 20-23, and the bank bag slots, which only `PutItemInBag`
    // reaches (`0x4c7c00` calls `0x4c7300`). Ammo (0) is refused; its leg is not built.
    if !(1..=23).contains(&id) && !model.bank_bag_inventory_slots().contains(&(id as usize)) {
        return false;
    }
    // Only a held item (`0x4c769a`) or vendor row (`0x4c76a5`) takes the click first. Past them
    // an empty slot or a locked item does nothing (`0x4c76af`, `0x4c76d2`), an item-targeting
    // spell binds the item, so a worn weapon can be poisoned (`0x4c76df`: `0x6e48a0` IsTargeting,
    // `0x6e6330` TargetingWantsItem, `0x495d60` bind), and repair mode repairs a worn item
    // (`0x4c7714`), any other payload staying held.
    let placing = matches!(
        model.cursor,
        Some(CursorPayload::Item(_) | CursorPayload::Merchant(_) | CursorPayload::GuildBankItem(_))
    );
    if !placing && (model.item_pick_armed || model.repair_mode) {
        let usable = model
            .inv_slot("player", id as usize)
            .is_some_and(|s| s.item_id != 0 && !s.locked);
        if usable && model.item_pick_armed {
            model.item_picks.push((EQUIPMENT_BAG, id));
        } else if usable && (1..=19).contains(&id) {
            model.inventory_repairs.push(id);
        }
        return false;
    }
    match model.cursor.take() {
        None => {
            let picked = model
                .inv_slot("player", id as usize)
                .filter(|s| s.item_id != 0 && !s.locked)
                .map(|s| CursorItem {
                    bag: EQUIPMENT_BAG,
                    slot: id,
                    item_id: s.item_id,
                    texture: s.icon.clone(),
                    link: s.link.clone(),
                    quality: Some(u32::try_from(s.quality).unwrap_or(0)),
                    count: None,
                    bar_placeable: s.bar_placeable,
                    equip_slots: s.equip_slots.clone(),
                });
            match picked {
                Some(item) => {
                    model.cursor = Some(CursorPayload::Item(item));
                    queue_cursor_update(model);
                    queue_lock_changed(model, EQUIPMENT_BAG, id);
                    true
                }
                None => false,
            }
        }
        Some(CursorPayload::Item(held)) if held.bag == EQUIPMENT_BAG && held.slot == id => {
            queue_cursor_update(model);
            queue_lock_changed(model, held.bag, held.slot);
            true
        }
        Some(CursorPayload::Item(held)) => {
            // A split or a misfit stays held, with no event: nothing changed.
            if held.count.is_some() || !super::bag_verbs::fits_slot(model, &held, id) {
                model.cursor = Some(CursorPayload::Item(held));
                return false;
            }
            model.container_moves.push(ContainerMove {
                src_bag: held.bag,
                src_slot: held.slot,
                dst_bag: EQUIPMENT_BAG,
                dst_slot: id,
                count: None,
            });
            queue_cursor_update(model);
            queue_lock_changed(model, held.bag, held.slot);
            true
        }
        // Mode 5, a buy into this slot: `0x4c78fd` sends `CMSG_BUY_ITEM_IN_SLOT` (`0x1a3`) and the
        // server equips or refuses. A stale row buys nothing: of the reference's three drop sites
        // only this one checks it; the other two (`0x4f9f35`, `0x4c7ea5`) dereference it.
        Some(CursorPayload::Merchant(held)) => {
            let entry = model
                .merchant
                .as_ref()
                .and_then(|m| m.items.get(held.row as usize))
                .filter(|it| it.item_id == held.item_id)
                .map(|it| it.item_id);
            if let Some(entry) = entry {
                model.merchant_slot_buys.push((EQUIPMENT_BAG, id, entry));
            }
            queue_cursor_update(model);
            true
        }
        // Mode 11, a guild bank item: the withdraw onto this slot, with the bank open. Labelled:
        // the reference's arm (`0x5e85d0`) is unread; this mirrors the bag drop's.
        Some(CursorPayload::GuildBankItem(held)) => {
            if model.wrath_friends.bank.tabs.is_empty() {
                model.cursor = Some(CursorPayload::GuildBankItem(held));
                return false;
            }
            model
                .wrath_friends
                .requests
                .push(crate::script::WrathGuildBankRequest::DropOnEquip {
                    tab: held.tab,
                    slot: held.slot,
                    equip_slot: id,
                    count: held.count,
                });
            queue_cursor_update(model);
            true
        }
        Some(
            other @ (CursorPayload::Spell(_)
            | CursorPayload::Action(_)
            | CursorPayload::Macro(_)
            | CursorPayload::PetAction(_)
            // Mode 10: a stabled pet stays held for the stable window.
            | CursorPayload::StablePet(_)
            // Mode 2: coins stay held for a money frame's drop. On an occupied slot the reference
            // picks the item up over either, as it tests only for a held item (`0x4c769a`) or a
            // preview row (`0x4c76a5`) before the pickup (`0x4c7838`).
            | CursorPayload::Money(_)
            // Modes 7 and 0xd, an item by id and an equipment set, are not slot payloads.
            | CursorPayload::ItemId(_)
            | CursorPayload::EquipmentSet(_)),
        ) => {
            model.cursor = Some(other);
            false
        }
    }
}

/// `EquipCursorItem` (`0x51a3d0`): put back, swap if fitting, put back a worn source,
/// otherwise autoequip; 0 and 256 skip directly to autoequip (`0x51a486`).
pub(super) fn equip_cursor_item(model: &mut Model, id: u32) {
    let Some(CursorPayload::Item(held)) = model.cursor.clone() else {
        return;
    };
    if id != 0 && id != 256 {
        if held.bag == EQUIPMENT_BAG && held.slot == id {
            super::clear_cursor(model);
            return;
        }
        if super::bag_verbs::fits_slot(model, &held, id) {
            model.container_moves.push(ContainerMove {
                src_bag: held.bag,
                src_slot: held.slot,
                dst_bag: EQUIPMENT_BAG,
                dst_slot: id,
                count: None,
            });
            // `0x513740` puts the displaced item on the cursor after the swap.
            super::clear_cursor(model);
            model.cursor = model
                .inv_slot("player", id as usize)
                .filter(|v| v.item_id != 0)
                .map(|v| {
                    CursorPayload::Item(CursorItem {
                        bag: EQUIPMENT_BAG,
                        slot: id,
                        item_id: v.item_id,
                        texture: v.icon,
                        link: v.link,
                        quality: Some(u32::try_from(v.quality).unwrap_or(0)),
                        count: None,
                        bar_placeable: v.bar_placeable,
                        equip_slots: v.equip_slots,
                    })
                });
            if model.cursor.is_some() {
                queue_lock_changed(model, EQUIPMENT_BAG, id);
            }
            return;
        }
        if held.bag == EQUIPMENT_BAG && (1..=19).contains(&held.slot) {
            super::clear_cursor(model);
            return;
        }
    }
    // `0x6dfc40(1)` equips the source, not a split quantity; worn bags are put back too.
    if held.bag != EQUIPMENT_BAG {
        model.container_autoequips.push((held.bag, held.slot));
    }
    super::clear_cursor(model);
}

/// `CursorCanGoInSlot(id)`, the doll slot's highlight on `CURSOR_UPDATE`
/// (`PaperDollFrame.lua:609-615`): `equip_slots`, which follows vmangos `Player::FindEquipSlot`,
/// where the reference's `IsValidForSlot` (`0x5da1d0`) tests a static slot mask per
/// `InventoryType`.
pub(super) fn cursor_can_go_in_slot(model: &Model, id: u32) -> bool {
    match &model.cursor {
        Some(CursorPayload::Item(item)) => super::bag_verbs::fits_slot(model, item, id),
        _ => false,
    }
}

/// `AutoEquipCursorItem()`, a click or drop on the doll's model (`PaperDollFrame.lua:31-35`): a
/// whole stack from a bag becomes `CMSG_AUTOEQUIP_ITEM`: the server picks the slot and swaps
/// the displaced piece back (`ItemHandler.cpp:138-228`); an equipped item or a split stays held.
/// Returns whether to repaint.
pub(super) fn auto_equip_cursor_item(model: &mut Model) -> bool {
    match model.cursor.take() {
        Some(CursorPayload::Item(item)) if item.bag >= 0 && item.count.is_none() => {
            model.container_autoequips.push((item.bag, item.slot));
            queue_cursor_update(model);
            queue_lock_changed(model, item.bag, item.slot);
            true
        }
        other => {
            model.cursor = other;
            false
        }
    }
}

/// `UseInventoryItem(id)`, the doll slot's right-click (`PaperDollFrame.lua:658-659`): the cursor
/// clears before the slot is read (`0x4c79a9`), so a held payload goes back, never placed; then
/// the app sends `CMSG_USE_ITEM` with bag 255 and the 0-based slot, dropping an empty slot unsent,
/// or in repair mode a worn item is repaired, locked or not (`0x4c79c4`).
pub(super) fn use_inventory_item(model: &mut Model, id: u32) {
    // An armed item-targeting spell binds the worn item, as `UseContainerItem` (`0x5d8650`) binds a
    // bag item; inferred for 12340's `UseInventoryItem` (`0x5ea630` → `0x5e8a60`, not decompiled).
    if model.item_pick_armed {
        let usable = model
            .inv_slot("player", id as usize)
            .is_some_and(|s| s.item_id != 0 && !s.locked);
        if usable {
            model.item_picks.push((EQUIPMENT_BAG, id));
        }
        return;
    }
    super::clear_cursor(model);
    if !model.repair_mode {
        model
            .script_calls
            .push(crate::script::ScriptCall::UseInventoryItem(id));
        return;
    }
    if (1..=19).contains(&id)
        && model
            .inv_slot("player", id as usize)
            .is_some_and(|s| s.item_id != 0)
    {
        model.inventory_repairs.push(id);
    }
}

/// `IsInventoryItemLocked(id)`: true while `id` is the held item's source or the app reports a
/// pending operation on it. Reads [`Model::inv_slot`], since `BankFrameItemButton_UpdateLock`
/// asks about bank slots too.
pub(super) fn is_inventory_item_locked(model: &Model, id: u32) -> bool {
    let held_here = matches!(&model.cursor, Some(CursorPayload::Item(c)) if c.bag == EQUIPMENT_BAG && c.slot == id);
    let fed = usize::try_from(id)
        .ok()
        .and_then(|i| model.inv_slot("player", i))
        .is_some_and(|s| s.locked);
    held_here || fed
}

/// Register the paper-doll verbs as globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "PickupInventoryItem",
        lua.create_function(|lua, id: u32| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            Ok(pickup_inventory_item(&mut model, id))
        })?,
    )?;
    g.set(
        "EquipCursorItem",
        lua.create_function(|lua, id: Value| {
            let id = ftol_arg(lua, id, "Usage: EquipCursorItem(slot)")? as u32;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            equip_cursor_item(&mut model, id);
            Ok(())
        })?,
    )?;
    g.set(
        "CursorCanGoInSlot",
        lua.create_function(|lua, id: Value| {
            let slot0 = inventory_slot0(lua, id)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            if !model.inventory_slot_reader_accepts(slot0) {
                return Ok(MultiValue::new());
            }
            Ok(MultiValue::from_vec(vec![flag(cursor_can_go_in_slot(
                &model,
                slot0.wrapping_add(1) as u32,
            ))]))
        })?,
    )?;
    g.set(
        "AutoEquipCursorItem",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            Ok(auto_equip_cursor_item(&mut model))
        })?,
    )?;
    g.set(
        "UseInventoryItem",
        lua.create_function(|lua, (id, target): (u32, Option<Value>)| {
            let unit = crate::script::unit::token::token_text(lua, &target.unwrap_or(Value::Nil))
                .filter(|t| !t.is_empty());
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if unit.as_ref().is_some_and(|t| !model.resolve_token(t).ok) {
                return Ok(());
            }
            let start = model.script_calls.len();
            use_inventory_item(&mut model, id);
            for call in &mut model.script_calls[start..] {
                *call = call.clone().at_unit(unit.clone());
            }
            Ok(())
        })?,
    )?;
    g.set(
        "IsInventoryItemLocked",
        lua.create_function(|lua, id: u32| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(is_inventory_item_locked(&model, id)))
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[path = "doll_tests.rs"]
mod tests;
