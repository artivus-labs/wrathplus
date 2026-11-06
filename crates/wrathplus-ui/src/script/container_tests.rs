use super::{ContainerMove, ContainerSlot, ContainerState, PendingWrap, UiCursorMode};
use crate::script::{MerchantItem, MerchantState, UiScript};

fn backpack() -> ContainerState {
    let mut slots = std::collections::HashMap::new();
    slots.insert(
        1,
        ContainerSlot {
            bar_placeable: true,
            texture: Some("Interface\\Icons\\INV_Misc_Food_16".into()),
            count: 5,
            quality: Some(1),
            item_id: 117,
            link: Some("|cffffffff|Hitem:117|h[Tough Jerky]|h|r".into()),
            ..Default::default()
        },
    );
    // An in-flight slot: the create arrived, the template answer hasn't.
    slots.insert(3, ContainerSlot::default());
    // A readable letter, a mail copy with item text.
    slots.insert(
        4,
        ContainerSlot {
            item_id: 8383,
            count: 1,
            readable: true,
            ..Default::default()
        },
    );
    ContainerState {
        name: Some("Backpack".into()),
        num_slots: 16,
        slots,
    }
}

#[test]
fn an_armed_gift_wrap_spends_on_the_next_container_click() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.arm_gift_wrap(0, 1);
    assert_eq!(s.gift_wrap_armed(), Some(PendingWrap { bag: 0, slot: 1 }));
    assert!(s.take_container_wraps().is_empty(), "arming sends nothing");
    assert_eq!(s.take_cursor_write(), Some(Some(UiCursorMode::Cast)));
    assert!(
        s.eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
            .unwrap(),
        "the paper dims while the wrap stands"
    );
    assert!(
        !s.eval::<bool>("return CursorHasItem()").unwrap(),
        "the wrap writes no payload, so every stock CursorHasItem() gate stays shut"
    );
    // A click on an empty slot leaves the wrap armed.
    s.eval::<()>("PickupContainerItem(0, 9)").unwrap();
    assert_eq!(s.gift_wrap_armed(), Some(PendingWrap { bag: 0, slot: 1 }));
    assert!(s.take_container_wraps().is_empty());
    // A click on a real item spends it: the paper's pair leads, as the wire does.
    s.eval::<()>("PickupContainerItem(0, 4)").unwrap();
    assert_eq!(s.take_container_wraps(), vec![(0, 1, 0, 4)]);
    assert_eq!(s.gift_wrap_armed(), None);
    assert_eq!(
        s.take_cursor_write(),
        Some(None),
        "the cursor resets on the send, not on the server's answer"
    );
    assert!(
        s.cursor_payload().is_none(),
        "the completing click is spent on the wrap, not on picking the target up"
    );
}

/// `ClearCursor` (`0x495190`) tests for an armed wrap first, whatever its arguments.
#[test]
fn clear_cursor_cancels_an_armed_gift_wrap() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.arm_gift_wrap(0, 1);
    let _ = s.take_cursor_write();
    s.eval::<()>("ClearCursor()").unwrap();
    assert_eq!(s.gift_wrap_armed(), None);
    assert_eq!(s.take_cursor_write(), Some(None), "and the mode goes back");
    assert!(
        !s.eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
            .unwrap(),
        "the paper unlocks with the cancel — the lock never waited on a server"
    );
    s.eval::<()>("PickupContainerItem(0, 1)").unwrap();
    assert!(s.take_container_wraps().is_empty());
    assert!(s.cursor_payload().is_some(), "a plain pickup, as before");
}

#[test]
fn container_snapshot_reads() {
    let mut s = UiScript::new().unwrap();
    // The backpack's 16 is a constant (`0x5d7507`), pushed or not.
    assert_eq!(s.eval::<i64>("return GetContainerNumSlots(0)").unwrap(), 16);
    assert!(s
        .eval::<bool>("return GetContainerItemInfo(0, 1) == nil")
        .unwrap());

    s.set_container(0, Some(backpack()));
    // The backpack's name is the global string `BACKPACK_TOOLTIP` (`0x5d8cdc`).
    s.run("BACKPACK_TOOLTIP = 'Backpack'").unwrap();
    assert_eq!(
        s.eval::<String>("return GetBagName(0)").unwrap(),
        "Backpack"
    );
    // The five values `ContainerFrame.lua:241` reads, in its order.
    let (icon, count, quality) = s
        .eval::<(String, i64, i64)>(
            "local texture, itemCount, locked, quality = GetContainerItemInfo(0, 1)\n\
             return texture, itemCount, quality",
        )
        .unwrap();
    assert_eq!(icon, "Interface\\Icons\\INV_Misc_Food_16");
    // Food is `InventoryType` 0, which answers quality -1 (`0x5d7ba7`); a weapon its own.
    assert_eq!((count, quality), (5, -1));
    assert_eq!(
        s.eval::<i64>(
            "local _, _, id = string.find(GetContainerItemLink(0, 1), 'item:(%d+)') \
             return tonumber(id)"
        )
        .unwrap(),
        117
    );
    // readable: the letter in 4, not the jerky in 1.
    assert!(!s
        .eval::<bool>("local _, _, _, _, readable = GetContainerItemInfo(0, 1) return readable")
        .unwrap());
    assert!(s
        .eval::<bool>("local _, _, _, _, readable = GetContainerItemInfo(0, 4) return readable")
        .unwrap());

    let mut bag = backpack();
    let jerky = bag.slots.get_mut(&1).unwrap();
    (jerky.inventory_type, jerky.lootable) = (13, true);
    s.set_container(0, Some(bag));
    let read = "local _, _, _, q, _, lootable = GetContainerItemInfo(0, 1) return q, lootable";
    assert_eq!(s.eval::<(i64, i64)>(read).unwrap(), (1, 1));

    // An in-flight slot: texture nil, quality -1 (`0x5d7a90`), itemCount real; seven values.
    assert!(s
        .eval::<bool>(
            "local texture, itemCount, locked, quality = GetContainerItemInfo(0, 3)\n\
             return texture == nil and quality == -1 and itemCount ~= nil",
        )
        .unwrap());
    assert_eq!(s.arity("GetContainerItemInfo(0, 3)").unwrap(), 7);
    assert!(s
        .eval::<bool>("return GetContainerItemLink(0, 3) == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("return GetContainerItemInfo(0, 2) == nil")
        .unwrap());
}

#[test]
fn use_container_item_queues_intents() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.run("UseContainerItem(0, 1)").unwrap();
    s.run("UseContainerItem(0, 3, 'target')").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1), (0, 3)]);
    assert!(s.take_container_uses().is_empty(), "drained");
}

/// A right-click clears the cursor before it reads the slot (`0x4fa198`): a held item goes
/// back to its slot, unlocked, and the clicked item is still used.
#[test]
fn right_click_puts_a_held_item_back_then_uses_the_clicked_one() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some());

    s.run("UseContainerItem(0, 4)").unwrap();
    assert!(s.cursor_payload().is_none(), "the held item went back");
    assert!(
        !s.eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
            .unwrap(),
        "its slot unlocks"
    );
    assert!(s.take_container_moves().is_empty(), "never placed");
    assert_eq!(s.take_container_uses(), vec![(0, 4)]);
}

/// The same clear cancels an armed gift wrap (`0x495190` opens with `0x5edf10`), and the
/// clicked item is used, not wrapped.
#[test]
fn right_click_cancels_an_armed_gift_wrap_then_uses_the_clicked_one() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.arm_gift_wrap(0, 1);
    s.run("UseContainerItem(0, 4)").unwrap();
    assert_eq!(s.gift_wrap_armed(), None);
    assert!(s.take_container_wraps().is_empty(), "nothing wrapped");
    assert_eq!(s.take_container_uses(), vec![(0, 4)]);
}

#[test]
fn pickup_then_place_swaps_and_clears_cursor() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack())); // slot 1 = Tough Jerky (resolved), slot 3 = in-flight
    assert!(s.cursor_item().is_none());
    assert!(!s.eval::<bool>("return CursorHasItem()").unwrap());

    s.run("PickupContainerItem(0, 1)").unwrap();
    let held = s.cursor_item().expect("cursor holds the picked item");
    assert_eq!((held.bag, held.slot, held.item_id), (0, 1, 117));
    assert_eq!(
        held.texture.as_deref(),
        Some("Interface\\Icons\\INV_Misc_Food_16")
    );
    assert!(s.eval::<bool>("return CursorHasItem()").unwrap());
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
        .unwrap());
    assert!(s.take_container_moves().is_empty());

    assert_eq!(s.cursor_item().map(|c| c.item_id), Some(117));

    s.run("PickupContainerItem(0, 5)").unwrap();
    assert!(s.cursor_item().is_none());
    assert!(!s.eval::<bool>("return CursorHasItem()").unwrap());
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 5,
            count: None,
        }]
    );
    assert!(!s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
        .unwrap());
}

/// The reference's place runs no `SetCursorItem`: the displaced item is never held.
#[test]
fn pickup_place_onto_occupied_different_item_swaps_and_clears() {
    let mut s = UiScript::new().unwrap();
    let mut state = backpack(); // slot 1 = item 117 (A)
    state.slots.insert(
        5,
        ContainerSlot {
            bar_placeable: true,
            texture: Some("Interface\\Icons\\INV_Misc_Gem_01".into()),
            count: 1,
            quality: Some(2),
            item_id: 200, // item B, a different id
            link: Some("|Hitem:200|h[Shiny Gem]|h".into()),
            ..Default::default()
        },
    );
    s.set_container(0, Some(state));

    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(s.cursor_item().unwrap().item_id, 117);

    s.run("PickupContainerItem(0, 5)").unwrap();
    assert!(
        s.cursor_item().is_none(),
        "a swap clears the cursor — the displaced item never hops on"
    );
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 5,
            count: None,
        }]
    );
    assert!(!s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked")
        .unwrap());
}

/// A same-item placement is a plain move: the server merges the stacks.
#[test]
fn pickup_place_onto_same_item_merges_and_clears() {
    let mut s = UiScript::new().unwrap();
    let mut state = backpack();
    state.slots.insert(
        5,
        ContainerSlot {
            durability: None,
            item_id: 117, // same id as slot 1
            count: 2,
            ..Default::default()
        },
    );
    s.set_container(0, Some(state));

    s.run("PickupContainerItem(0, 1)").unwrap();
    s.run("PickupContainerItem(0, 5)").unwrap();
    assert!(
        s.cursor_item().is_none(),
        "same-item merge clears the cursor"
    );
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 5,
            count: None,
        }]
    );
}

#[test]
fn pickup_refuses_a_locked_slot() {
    let mut s = UiScript::new().unwrap();
    let mut state = backpack();
    state.slots.get_mut(&1).unwrap().locked = true;
    s.set_container(0, Some(state));

    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_none());
}

#[test]
fn split_container_item_carries_a_partial_stack() {
    let mut s = UiScript::new().unwrap();
    let mut state = backpack(); // slot 1: a 5-stack of item 117
    state.slots.insert(
        7,
        ContainerSlot {
            durability: None,
            item_id: 117,
            count: 1,
            ..Default::default()
        },
    );
    state.slots.insert(
        9,
        ContainerSlot {
            durability: None,
            item_id: 999, // a different item
            count: 1,
            ..Default::default()
        },
    );
    s.set_container(0, Some(state));

    assert!(s
        .eval::<bool>("return SplitContainerItem(0, 1, 3)")
        .unwrap());
    let held = s.cursor_item().expect("the split carry");
    assert_eq!(
        (held.bag, held.slot, held.item_id, held.count),
        (0, 1, 117, Some(3))
    );

    s.run("PickupContainerItem(0, 2)").unwrap();
    assert!(s.cursor_item().is_none());
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 2,
            count: Some(3),
        }]
    );

    s.run("SplitContainerItem(0, 1, 3)").unwrap();
    s.run("PickupContainerItem(0, 9)").unwrap();
    let held = s.cursor_item().expect("kept — can't swap a partial stack");
    assert_eq!(held.count, Some(3));
    assert!(s.take_container_moves().is_empty());

    s.run("PickupContainerItem(0, 7)").unwrap();
    assert!(s.cursor_item().is_none());
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: 0,
            dst_slot: 7,
            count: Some(3),
        }]
    );
}

#[test]
fn split_of_the_whole_stack_is_a_plain_pickup() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack())); // slot 1: a 5-stack
    assert!(s
        .eval::<bool>("return SplitContainerItem(0, 1, 5)")
        .unwrap());
    assert_eq!(s.cursor_item().unwrap().count, None);
    // A split of more than the stack is a plain pickup too.
    s.run("ClearCursor()").unwrap();
    assert!(s
        .eval::<bool>("return SplitContainerItem(0, 1, 99)")
        .unwrap());
    assert_eq!(s.cursor_item().unwrap().count, None);
}

#[test]
fn pickup_same_slot_cancels_no_move() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some());
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_none());
    assert!(s.take_container_moves().is_empty());
}

#[test]
fn pickup_empty_slot_holds_nothing() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    // Slot 2 is empty and slot 3 unresolved: neither can be picked up.
    s.run("PickupContainerItem(0, 2)").unwrap();
    assert!(s.cursor_item().is_none());
    s.run("PickupContainerItem(0, 3)").unwrap();
    assert!(s.cursor_item().is_none());
}

#[test]
fn clear_cursor_drops_the_held_item() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.cursor_item().is_some());
    s.run("ClearCursor()").unwrap();
    assert!(s.cursor_item().is_none());
    assert!(s.take_container_moves().is_empty());
}

/// With the trade window open a right-click offers the bag item in our first free traded slot,
/// skipping one queued this frame; a soulbound item stays put, and closed, the click uses.
#[test]
fn right_click_with_a_trade_open_offers_the_item_in_the_first_free_slot() {
    let mut s = UiScript::new().unwrap();
    let mut bag = backpack();
    let item = |item_id, flags| ContainerSlot {
        item_id,
        count: 1,
        flags,
        ..Default::default()
    };
    bag.slots.insert(5, item(2589, 0));
    bag.slots.insert(6, item(6948, 0x1));
    s.set_container(0, Some(bag));
    s.set_trade(Some(crate::script::TradeState::default()));
    s.run("UseContainerItem(0, 1)").unwrap();
    s.run("UseContainerItem(0, 5)").unwrap();
    s.run("UseContainerItem(0, 6)").unwrap();
    assert_eq!(s.take_trade_set_items(), vec![(1, 0, 1), (2, 0, 5)]);
    assert!(
        s.take_container_uses().is_empty(),
        "nothing is used while trading"
    );

    s.set_trade(None);
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)]);
}

#[path = "container_repair_tests.rs"]
mod repair;
