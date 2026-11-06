//! Wrath equipment-manager item queries, reading the app's existing instance snapshots.
//! Installed EquipmentManager.lua:25,271-287 supplies the signatures, including the
//! caller-owned empty-slot work table (not GetContainerNumFreeSlots's bag-family result).
use super::Model;
use mlua::{Lua, MultiValue, Value};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "GetInventoryItemID",
        lua.create_function(|lua, (unit, slot): (String, i64)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(usize::try_from(slot)
                .ok()
                .and_then(|s| model.inv_slot(&unit, s))
                .map(|s| s.item_id)
                .filter(|&id| id != 0))
        })?,
    )?;
    g.set(
        "GetContainerItemID",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(model
                .containers
                .get(&bag)
                .and_then(|b| b.slots.get(&slot))
                .map(|s| s.item_id)
                .filter(|&id| id != 0))
        })?,
    )?;
    g.set(
        "GetInventoryItemDurability",
        lua.create_function(|lua, slot: i64| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let durability = usize::try_from(slot)
                .ok()
                .and_then(|s| model.inv_slot("player", s))
                .and_then(|s| s.durability)
                .filter(|&(_, max)| max != 0);
            Ok((durability.map(|d| d.0), durability.map(|d| d.1)))
        })?,
    )?;
    g.set(
        "GetContainerItemDurability",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let durability = model
                .containers
                .get(&bag)
                .and_then(|b| b.slots.get(&slot))
                .and_then(|s| s.durability)
                .filter(|&(_, max)| max != 0);
            Ok((durability.map(|d| d.0), durability.map(|d| d.1)))
        })?,
    )?;
    // GetContainerFreeSlots(index [, returnTable]) (`0x5d7820`): the empty slots' 1-based numbers
    // appended to the table (a new one when argument 2 is not a table), which it returns; the
    // backpack, the vault with the banker, and a bag 1..11 (a bank bag with the banker), else
    // zero values.
    g.set(
        "GetContainerFreeSlots",
        lua.create_function(|lua, (bag, work): (Value, Value)| {
            const USAGE: &str = "Usage: GetContainerFreeSlots(index [, returnTable])";
            let bag = i64::from(super::binding_abi::ftol_arg(lua, bag, USAGE)?);
            // Release the model borrow before invoking table operations in Lua.
            let free = {
                let model = lua.app_data_ref::<Model>().expect("model");
                let open = match bag {
                    0..=4 => true,
                    -1 | 5..=11 => model.bank.is_some(),
                    _ => false,
                };
                let c = model.containers.get(&bag).filter(|_| open);
                c.map(|b| {
                    (1..=b.num_slots)
                        .filter(|s| !b.slots.contains_key(s))
                        .collect::<Vec<_>>()
                })
            };
            let Some(free) = free else {
                return Ok(MultiValue::new());
            };
            let work = match work {
                Value::Table(t) => t,
                _ => lua.create_table()?,
            };
            for (i, slot) in free.into_iter().enumerate() {
                work.set(i + 1, slot)?;
            }
            Ok(MultiValue::from_vec(vec![Value::Table(work)]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{ContainerSlot, ContainerState, InvSlotView, UiScript};

    fn script() -> UiScript {
        UiScript::new().unwrap()
    }

    #[test]
    fn instance_queries_work_before_template_and_keep_broken_durability() {
        let mut ui = script();
        let mut slots = Default::default();
        // No icon, link, or name: item descriptors do not depend on the template cache.
        let slots: &mut super::super::InventorySlots = &mut slots;
        slots[5] = Some(InvSlotView {
            item_id: 123,
            durability: Some((0, 40)),
            ..Default::default()
        });
        ui.set_inventory_slots(slots.clone());
        ui.set_container(
            0,
            Some(ContainerState {
                num_slots: 3,
                slots: [(
                    2,
                    ContainerSlot {
                        item_id: 456,
                        durability: Some((7, 20)),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }),
        );
        ui.run(
            r#"
            assert(GetInventoryItemID('player', 5) == 123)
            assert(GetInventoryItemID('target', 5) == nil)
            assert(GetInventoryItemID('player', -1) == nil)
            local n, max = GetInventoryItemDurability(5)
            assert(n == 0 and max == 40)
            assert(GetContainerItemID(0, 2) == 456)
            n, max = GetContainerItemDurability(0, 2)
            assert(n == 7 and max == 20)
            n, max = GetContainerItemDurability(0, 1)
            assert(n == nil and max == nil)
            assert(GetInventoryItemID('player', 1) == nil)
        "#,
        )
        .unwrap();
    }

    #[test]
    fn equipment_manager_work_table_contains_only_empty_slot_indices() {
        let mut ui = script();
        ui.set_container(
            0,
            Some(ContainerState {
                num_slots: 3,
                // Even an unresolved occupied item must not be treated as empty.
                slots: [(2, ContainerSlot::default())].into(),
                ..Default::default()
            }),
        );
        ui.run(
            r#"
            local work = {}
            assert(GetContainerFreeSlots(0, work))
            assert(#work == 2 and work[1] == 1 and work[2] == 3)
            assert(not GetContainerFreeSlots(99, {}))
            assert(GetContainerItemID(0, 2) == nil)
        "#,
        )
        .unwrap();
    }
}
