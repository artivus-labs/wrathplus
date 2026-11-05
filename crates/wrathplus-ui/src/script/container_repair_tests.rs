//! Repair mode against the bag clicks, `ContainerIDToInventoryID` and the bag-id shapes.

use super::*;

#[test]
fn repair_cursor_wins_over_a_sellable_bag_hover() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_merchant(Some(MerchantState {
        can_repair: true,
        ..Default::default()
    }));

    s.run("ShowContainerSellCursor(0, 1)").unwrap();
    assert_eq!(s.ui_cursor(), Some(UiCursorMode::Buy));

    s.run("ShowRepairCursor()").unwrap();
    assert_eq!(
        s.ui_cursor(),
        None,
        "repair replaces the existing Buy hover"
    );
    s.run("ShowContainerSellCursor(0, 1)").unwrap();
    assert_eq!(
        s.ui_cursor(),
        None,
        "a bag hover keeps the repair base cursor"
    );
    s.run("PickupContainerItem(0, 1)").unwrap();
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(
        s.take_container_repairs(),
        vec![(0, 1), (0, 1)],
        "repair mode sticks across clicks"
    );
}

#[test]
fn right_click_repairs_bag_item_instead_of_selling_it() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_merchant(Some(MerchantState {
        can_repair: true,
        ..Default::default()
    }));
    s.run("ShowRepairCursor() UseContainerItem(0, 1) UseContainerItem(0, 1)")
        .unwrap();

    assert_eq!(s.take_container_repairs(), vec![(0, 1), (0, 1)]);
    assert!(s.take_container_uses().is_empty(), "no sell/use intent");
}

#[test]
fn right_click_in_repair_mode_puts_a_held_item_back_then_repairs() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_merchant(Some(MerchantState {
        can_repair: true,
        ..Default::default()
    }));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some());
    // A held item and repair mode together, as after a cursor-mode change.
    s.model_mut().repair_mode = true;

    s.run("UseContainerItem(0, 4)").unwrap();
    assert!(
        s.cursor_payload().is_none(),
        "the held item went back (`0x4fa198`)"
    );
    assert!(s.take_container_moves().is_empty(), "never placed");
    assert_eq!(s.take_container_repairs(), vec![(0, 4)]);

    s.model_mut().repair_mode = false;
    s.run("PickupContainerItem(0, 1)").unwrap();
    s.model_mut().repair_mode = true;
    s.run("UseContainerItem(0, 2)").unwrap();
    assert!(
        s.cursor_payload().is_none(),
        "an empty slot still clears the cursor"
    );
    assert!(
        s.take_container_repairs().is_empty(),
        "and repairs nothing (`0x4fa1a6`)"
    );
}

#[test]
fn held_vendor_row_buys_into_bag_before_repair_mode() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_merchant(Some(MerchantState {
        items: vec![MerchantItem {
            item_id: 159,
            ..Default::default()
        }],
        can_repair: true,
        ..Default::default()
    }));
    s.run("ShowRepairCursor() PickupMerchantItem(1)").unwrap();
    assert!(
        s.repair_mode(),
        "vendor grab preserves the repair base mode"
    );
    assert!(s.cursor_payload().is_some());

    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_merchant_slot_buys(), vec![(0, 1, 159)]);
    assert!(s.cursor_payload().is_none());
    assert!(s.take_container_repairs().is_empty());
    assert!(s.take_sounds().is_empty());

    // The right-click clears the vendor row with no packet and repairs instead.
    s.run("PickupMerchantItem(1) UseContainerItem(0, 1)")
        .unwrap();
    assert!(s.take_merchant_slot_buys().is_empty());
    assert!(s.cursor_payload().is_none());
    assert_eq!(s.take_container_repairs(), vec![(0, 1)]);
    assert!(s.take_container_uses().is_empty());
}

#[test]
fn held_item_places_before_repair_mode() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some());
    // A held item and repair base mode can coexist after a cursor-mode change.
    s.model_mut().repair_mode = true;

    s.run("PickupContainerItem(0, 2)").unwrap();
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 2,
            count: None,
        }]
    );
    assert!(s.cursor_payload().is_none());
    assert!(s.take_container_repairs().is_empty());
    assert!(s.take_sounds().is_empty());
}

#[test]
fn repair_mode_repairs_under_held_coins_and_keeps_them() {
    use crate::script::cursor::{CursorMoney, CursorPayload};
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.model_mut().repair_mode = true;
    let coins = CursorPayload::Money(CursorMoney { copper: 50 });
    s.model_mut().cursor = Some(coins.clone());

    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(
        s.take_container_repairs(),
        vec![(0, 1)],
        "coins pass both payload tests (`0x4f9c38`, `0x4f9c43`)"
    );
    assert_eq!(s.cursor_payload(), Some(coins), "and stay held");
    s.run("PickupContainerItem(0, 2)").unwrap();
    assert!(
        s.take_container_repairs().is_empty(),
        "an empty slot does nothing (`0x4f9c4e`)"
    );
}

#[test]
fn removing_a_bag_empties_it() {
    let mut s = UiScript::new().unwrap();
    s.set_container(2, Some(backpack()));
    s.set_container(2, None);
    assert_eq!(s.eval::<i64>("return GetContainerNumSlots(2)").unwrap(), 0);
}

#[test]
fn container_item_cooldown_reads_the_gettime_triple() {
    let mut s = UiScript::new().unwrap();
    s.tick(100.0); // an arbitrary clock epoch
    let mut state = backpack();
    // Slot 1's item is 4 s into a 10 s use-cooldown: started at GetTime 96.
    state.slots.get_mut(&1).unwrap().cooldown = Some((96_000, 10_000, true));
    s.set_container(0, Some(state));

    assert!(s
        .eval::<bool>(
            "local s, d, e = GetContainerItemCooldown(0, 1)\n\
             return s == 96 and d == 10 and e == 1"
        )
        .unwrap());
    assert!(s
        .eval::<bool>(
            "local s, d, e = GetContainerItemCooldown(0, 3)\n\
             return s == 0 and d == 0 and e == 1"
        )
        .unwrap());
    // Past expiry, the same stored triple reads cold.
    s.tick(7.0);
    assert!(s
        .eval::<bool>(
            "local s, d, e = GetContainerItemCooldown(0, 1)\n\
             return s == 0 and d == 0 and e == 1"
        )
        .unwrap());
    // A re-push without the cooldown clears the stored triple.
    s.set_container(0, Some(backpack()));
    assert!(s
        .eval::<bool>(
            "local s, d = GetContainerItemCooldown(0, 1)\n\
             return s == 0 and d == 0"
        )
        .unwrap());
}

// ── `ContainerIDToInventoryID` (`0x5d6f60`) and the bag-id shapes ─────────────────────────

#[test]
fn container_id_to_inventory_id_maps_bags_1_to_11_and_raises_elsewhere() {
    let s = UiScript::new().unwrap();
    let slot = |id: &str| s.eval::<i64>(&format!("return ContainerIDToInventoryID({id})"));
    assert_eq!(
        (slot("1").unwrap(), slot("4").unwrap(), slot("5").unwrap()),
        (20, 23, 68)
    );
    assert_eq!(
        (slot("11").unwrap(), slot("2.9").unwrap()),
        (74, 21),
        "truncated"
    );
    for id in ["0", "-1", "-2", "12", "99"] {
        let err = slot(id).unwrap_err().to_string();
        assert!(
            err.contains("ContainerIDToInventoryID(): invalid container ID"),
            "{id}: {err}"
        );
    }
}

/// The bag verbs take the resolver's (`0x5d7380`) bag ids: constants for the backpack, vault and
/// keyring, bank bags only with the banker, and the vault and keyring nameless.
#[test]
fn the_bag_verbs_answer_the_resolvers_shapes() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    let bank_bag = ContainerState {
        name: Some("Bag".into()),
        num_slots: 6,
        slots: Default::default(),
    };
    s.set_container(5, Some(bank_bag));
    let nums = "return GetContainerNumSlots(0), GetContainerNumSlots(-1), \
                GetContainerNumSlots(-2), GetContainerNumSlots(-4), GetContainerNumSlots(5)";
    assert_eq!(
        s.eval::<(i64, i64, i64, i64, i64)>(nums).unwrap(),
        (16, 28, 32, 32, 0)
    );
    // `0x88b9c0` reads math.huge and NaN as `i32::MIN`, no bag.
    let huge = "return GetContainerNumSlots(math.huge), GetContainerNumSlots(0/0)";
    assert_eq!(s.eval::<(i64, i64)>(huge).unwrap(), (0, 0));
    assert_eq!(s.arity("GetContainerNumFreeSlots(0/0)").unwrap(), 1);
    assert_eq!(
        s.arity("GetContainerNumFreeSlots(5)").unwrap(),
        1,
        "banker away"
    );
    assert_eq!(s.arity("GetContainerFreeSlots(5)").unwrap(), 0);
    let names = "return GetBagName(-1) == nil and GetBagName(-2) == nil and GetBagName(5) == nil";
    assert!(s.eval::<bool>(names).unwrap());
    s.set_bank(Some(crate::script::BankState::default()));
    assert_eq!(s.eval::<i64>("return GetContainerNumSlots(5)").unwrap(), 6);
    assert_eq!(s.eval::<String>("return GetBagName(5)").unwrap(), "Bag");
    // The slots go in through `lua_settable` (`0x84e8d0`), so a `__newindex` hears each one.
    let free = "local n = 0 local t = setmetatable({}, {__newindex = function(t, k, v) \
                n = n + 1 rawset(t, k, v) end}) GetContainerFreeSlots(5, t) \
                return #t, n, GetContainerNumFreeSlots(5)";
    assert_eq!(s.eval::<(i64, i64, i64)>(free).unwrap(), (6, 6, 6));
    // Empty and unknown slots answer (0, 0, 0); an item with no cooldown (0, 0, 1).
    let cd =
        |at: &str| s.eval::<(f64, f64, i64)>(&format!("return GetContainerItemCooldown({at})"));
    assert_eq!((cd("0, 2").unwrap().2, cd("12, 1").unwrap().2), (0, 0));
    assert_eq!(cd("0, 1").unwrap(), (0.0, 0.0, 1));
}

/// A missing or non-number argument raises: the type test's error `0x6f4940` does not return.
#[test]
fn container_id_to_inventory_id_raises_on_a_bad_argument() {
    let s = UiScript::new().unwrap();
    for call in ["ContainerIDToInventoryID()", "ContainerIDToInventoryID({})"] {
        let err = s
            .eval::<mlua::Value>(&format!("return {call}"))
            .unwrap_err();
        assert!(
            format!("{err}").contains("Usage: ContainerIDToInventoryID(containerID)"),
            "{call} must raise, got {err}"
        );
    }
}
