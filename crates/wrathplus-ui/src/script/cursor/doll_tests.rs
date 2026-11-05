use crate::script::cursor::{CursorAction, CursorPayload, CursorSpell, EQUIPMENT_BAG};
use crate::script::{ContainerMove, ContainerSlot, ContainerState, InvSlotView, UiScript};

/// Head (1), a ring in finger slot 11 that also fits 12, and Tabard (19).
fn doll_slots() -> crate::script::InventorySlots {
    let mut slots: crate::script::InventorySlots = Default::default();
    slots[1] = Some(InvSlotView {
        duration_ms: None,
        already_bound: false,
        bar_placeable: true,
        durability: None,
        flags: 0,
        item_id: 1234,
        icon: Some("Interface\\Icons\\INV_Helmet_01".into()),
        count: 1,
        contents_count: None,
        quality: 2,
        name: Some("Test Helm".into()),
        link: Some("|cff1eff00|Hitem:1234:0:0:0|h[Test Helm]|h|r".into()),
        locked: false,
        equip_slots: vec![1],
        creator: None,
        enchants: Vec::new(),
    });
    slots[11] = Some(InvSlotView {
        duration_ms: None,
        already_bound: false,
        bar_placeable: true,
        durability: None,
        flags: 0,
        item_id: 555,
        icon: Some("Interface\\Icons\\INV_Jewelry_Ring_01".into()),
        count: 1,
        contents_count: None,
        quality: 1,
        name: Some("Test Ring".into()),
        link: Some("|cffffffff|Hitem:555:0:0:0|h[Test Ring]|h|r".into()),
        locked: false,
        equip_slots: vec![11, 12], // either finger: the doll-to-doll swap
        creator: None,
        enchants: Vec::new(),
    });
    slots[19] = Some(InvSlotView {
        duration_ms: None,
        already_bound: false,
        bar_placeable: true,
        durability: None,
        flags: 0,
        item_id: 999,
        icon: Some("Interface\\Icons\\INV_Shirt_White_01".into()),
        count: 1,
        contents_count: None,
        quality: 1,
        name: Some("Test Tabard".into()),
        link: Some("|cffffffff|Hitem:999:0:0:0|h[Test Tabard]|h|r".into()),
        locked: false,
        equip_slots: vec![19],
        creator: None,
        enchants: Vec::new(),
    });
    slots
}

fn one_fitting_bag_item() -> ContainerState {
    let mut slots = std::collections::HashMap::new();
    slots.insert(
        1,
        ContainerSlot {
            bar_placeable: true,
            texture: Some("Interface\\Icons\\INV_Helmet_02".into()),
            count: 1,
            quality: Some(3),
            item_id: 2000,
            link: Some("|cff0070dd|Hitem:2000:0:0:0|h[Another Helm]|h|r".into()),
            equip_slots: vec![1], // fits HeadSlot only
            cooldown: None,
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
fn pickup_from_doll_slot_holds_and_locks() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());

    assert!(!s.eval::<bool>("return IsInventoryItemLocked(1)").unwrap());
    assert!(s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    let held = s.cursor_item().expect("picked up");
    assert_eq!(
        (held.bag, held.slot, held.item_id),
        (EQUIPMENT_BAG, 1, 1234)
    );
    assert_eq!(held.equip_slots, vec![1]);
    assert!(s.eval::<bool>("return IsInventoryItemLocked(1)").unwrap());
    assert!(s.eval::<bool>("return CursorHasItem()").unwrap());
}

#[test]
fn pickup_same_doll_slot_cancels_no_move() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    assert!(s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    assert!(s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    assert!(s.cursor_item().is_none());
    assert!(s.take_container_moves().is_empty());
    assert!(!s.eval::<bool>("return IsInventoryItemLocked(1)").unwrap());
}

#[test]
fn place_fitting_bag_item_onto_doll_slot_queues_move_and_clears() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    s.set_container(0, Some(one_fitting_bag_item()));

    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(s.cursor_item().unwrap().equip_slots, vec![1]);

    assert!(s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    assert!(s.cursor_item().is_none(), "a plain clear, no hop");
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: EQUIPMENT_BAG,
            dst_slot: 1,
            count: None,
        }]
    );
}

#[test]
fn place_non_fitting_item_is_a_no_op() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    // The bag item only fits HeadSlot (1); try placing it on NeckSlot (2).
    s.set_container(0, Some(one_fitting_bag_item()));
    s.run("PickupContainerItem(0, 1)").unwrap();

    assert!(!s.eval::<bool>("return PickupInventoryItem(2)").unwrap());
    let held = s.cursor_item().expect("kept — doesn't fit slot 2");
    assert_eq!(held.item_id, 2000);
    assert!(s.take_container_moves().is_empty());
}

#[test]
fn doll_to_doll_ring_swap_fits_via_shared_equip_slots() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());

    assert!(s.eval::<bool>("return PickupInventoryItem(11)").unwrap());
    assert_eq!(s.cursor_item().unwrap().equip_slots, vec![11, 12]);

    assert!(s.eval::<bool>("return PickupInventoryItem(12)").unwrap());
    assert!(s.cursor_item().is_none());
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: EQUIPMENT_BAG,
            src_slot: 11,
            dst_bag: EQUIPMENT_BAG,
            dst_slot: 12,
            count: None,
        }]
    );
}

#[test]
fn split_carry_refuses_to_equip() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    let mut state = one_fitting_bag_item();
    state.slots.get_mut(&1).unwrap().count = 5;
    s.set_container(0, Some(state));

    s.run("SplitContainerItem(0, 1, 2)").unwrap();
    let held = s.cursor_item().expect("a split carry");
    assert_eq!(held.count, Some(2));

    assert!(!s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    let held = s.cursor_item().expect("kept — can't equip a partial stack");
    assert_eq!(held.count, Some(2));
    assert!(s.take_container_moves().is_empty());
}

#[test]
fn pickup_inventory_item_refuses_ammo_but_takes_a_bag_slot() {
    let mut s = UiScript::new().unwrap();
    let mut slots = doll_slots();
    slots[0] = Some(InvSlotView {
        durability: None,
        flags: 0,
        item_id: 2512,
        equip_slots: Vec::new(),
        ..Default::default()
    });
    // An equipped bag in Bag0Slot (id 20): its equip_slots is the four bag slots.
    slots[20] = Some(InvSlotView {
        duration_ms: None,
        already_bound: false,
        bar_placeable: true,
        durability: None,
        flags: 0,
        item_id: 4496,
        icon: Some("Interface\\Icons\\INV_Misc_Bag_08".into()),
        count: 1,
        contents_count: None,
        quality: 1,
        name: Some("Small Brown Pouch".into()),
        link: Some("|cffffffff|Hitem:4496:0:0:0|h[Small Brown Pouch]|h|r".into()),
        locked: false,
        equip_slots: vec![20, 21, 22, 23],
        creator: None,
        enchants: Vec::new(),
    });
    s.set_inventory_slots(slots);

    // Ammo (0) is refused; Bag0Slot (20) picks up the equipped bag.
    assert!(!s.eval::<bool>("return PickupInventoryItem(0)").unwrap());
    assert!(s.eval::<bool>("return PickupInventoryItem(20)").unwrap());
    let held = s.cursor_item().expect("bag picked up from the bar");
    assert_eq!(
        (held.bag, held.slot, held.item_id),
        (EQUIPMENT_BAG, 20, 4496)
    );
}

#[test]
fn place_a_bag_onto_a_bag_slot_queues_the_equip_move() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    let mut slots = std::collections::HashMap::new();
    slots.insert(
        3,
        ContainerSlot {
            bar_placeable: true,
            texture: Some("Interface\\Icons\\INV_Misc_Bag_08".into()),
            count: 1,
            quality: Some(1),
            item_id: 4496,
            link: Some("|cffffffff|Hitem:4496:0:0:0|h[Small Brown Pouch]|h|r".into()),
            equip_slots: vec![20, 21, 22, 23], // INVTYPE_BAG → any bag slot
            cooldown: None,
            ..Default::default()
        },
    );
    s.set_container(
        0,
        Some(ContainerState {
            name: Some("Backpack".into()),
            num_slots: 16,
            slots,
        }),
    );
    s.run("PickupContainerItem(0, 3)").unwrap();
    assert!(s.eval::<bool>("return CursorCanGoInSlot(21)").unwrap());

    assert!(s.eval::<bool>("return PickupInventoryItem(21)").unwrap());
    assert!(s.cursor_item().is_none(), "a plain clear on place");
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 3,
            dst_bag: EQUIPMENT_BAG,
            dst_slot: 21,
            count: None,
        }]
    );
}

#[test]
fn cursor_can_go_in_slot_per_arm() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());

    assert!(!s.eval::<bool>("return CursorCanGoInSlot(1)").unwrap());

    s.set_container(0, Some(one_fitting_bag_item()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(s.eval::<bool>("return CursorCanGoInSlot(1)").unwrap());
    assert!(!s.eval::<bool>("return CursorCanGoInSlot(2)").unwrap());
    s.run("ClearCursor()").unwrap();

    s.set_cursor_for_test(CursorPayload::Spell(CursorSpell {
        passive: false,
        book_slot: 1,
        book_type: "spell".into(),
        spell_id: 1,
        texture: None,
    }));
    assert!(!s.eval::<bool>("return CursorCanGoInSlot(1)").unwrap());
    s.set_cursor_for_test(CursorPayload::Action(CursorAction {
        src_slot: 1,
        kind: 0,
        action: 1,
        texture: None,
    }));
    assert!(!s.eval::<bool>("return CursorCanGoInSlot(1)").unwrap());
}

#[test]
fn auto_equip_cursor_item_queues_source_and_clears() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(one_fitting_bag_item()));
    s.run("PickupContainerItem(0, 1)").unwrap();

    assert!(s.eval::<bool>("return AutoEquipCursorItem()").unwrap());
    assert!(s.cursor_item().is_none());
    assert_eq!(s.take_container_autoequips(), vec![(0, 1)]);
}

#[test]
fn auto_equip_cursor_item_is_a_no_op_from_the_doll_or_a_split_carry() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());

    s.run("PickupInventoryItem(1)").unwrap();
    assert!(!s.eval::<bool>("return AutoEquipCursorItem()").unwrap());
    assert!(s.cursor_item().is_some(), "kept");
    assert!(s.take_container_autoequips().is_empty());
    s.run("ClearCursor()").unwrap();

    let mut state = one_fitting_bag_item();
    state.slots.get_mut(&1).unwrap().count = 5;
    s.set_container(0, Some(state));
    s.run("SplitContainerItem(0, 1, 2)").unwrap();
    assert!(!s.eval::<bool>("return AutoEquipCursorItem()").unwrap());
    assert!(s.cursor_item().is_some(), "kept");
    assert!(s.take_container_autoequips().is_empty());
}

#[test]
fn use_inventory_item_queues_the_id() {
    let mut s = UiScript::new().unwrap();
    s.run("UseInventoryItem(1)").unwrap();
    s.run("UseInventoryItem(19)").unwrap();
    assert_eq!(s.take_inventory_uses(), vec![1, 19]);
    assert!(s.take_inventory_uses().is_empty(), "drained");
}

/// A right-click clears the cursor before it reads the slot (`0x4c79a9`): a held item goes
/// back, and the worn item is still used.
#[test]
fn right_click_puts_a_held_item_back_then_uses_the_worn_one() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    s.run("PickupInventoryItem(11)").unwrap();
    assert!(s.cursor_item().is_some());

    s.run("UseInventoryItem(19)").unwrap();
    assert!(s.cursor_payload().is_none(), "the ring went back");
    assert!(
        !s.eval::<bool>("return IsInventoryItemLocked(11)").unwrap(),
        "its slot unlocks"
    );
    assert!(s.take_container_moves().is_empty(), "never placed");
    assert_eq!(s.take_inventory_uses(), vec![19]);
}

#[test]
fn is_inventory_item_locked_reads_held_or_fed() {
    let mut s = UiScript::new().unwrap();
    let mut slots = doll_slots();
    slots[19].as_mut().unwrap().locked = true; // the app's own PendingItemOps feed
    s.set_inventory_slots(slots);

    assert!(
        s.eval::<bool>("return IsInventoryItemLocked(19)").unwrap(),
        "fed lock"
    );
    assert!(!s.eval::<bool>("return IsInventoryItemLocked(1)").unwrap());

    s.run("PickupInventoryItem(1)").unwrap();
    assert!(
        s.eval::<bool>("return IsInventoryItemLocked(1)").unwrap(),
        "held-here lock"
    );
}

#[test]
fn equip_cursor_item_routes_through_pickup() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    s.set_container(0, Some(one_fitting_bag_item()));
    // An empty cursor equips nothing and picks nothing up (`0x51a452`).
    s.run("EquipCursorItem(1)").unwrap();
    assert!(s.cursor_payload().is_none());
    s.run("PickupContainerItem(0, 1)").unwrap();
    // The predicates push the number 1 or nil (`0x515125`, `0x5ea880`).
    let held = "return CursorHasItem(), CursorCanGoInSlot(1), CursorCanGoInSlot(2)";
    assert_eq!(
        s.eval::<(i64, i64, Option<i64>)>(held).unwrap(),
        (1, 1, None)
    );

    assert!(s.eval::<bool>("return EquipCursorItem(1)").unwrap());
    assert!(s.cursor_item().is_none());
    assert_eq!(
        s.take_container_moves(),
        vec![ContainerMove {
            src_bag: 0,
            src_slot: 1,
            dst_bag: EQUIPMENT_BAG,
            dst_slot: 1,
            count: None,
        }]
    );
}

/// The held-payload test runs first in both seams: `0x4f9c38` before `0x4f9c54` in the bag,
/// `0x4c73af` before `0x4c76df` on the doll.
#[test]
fn the_armed_item_half_reroutes_bag_and_doll_clicks() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    s.set_container(0, Some(one_fitting_bag_item()));

    // Unarmed, both seams do their ordinary gesture and queue nothing.
    assert!(s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    assert!(s.cursor_item().is_some());
    assert!(s.take_item_picks().is_empty());

    // Armed but holding: the payload wins.
    s.set_item_pick_armed(true);
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert!(
        s.take_item_picks().is_empty(),
        "a click while carrying an item is a place, not a bind"
    );

    // Armed with an empty cursor: both seams bind and leave the cursor empty.
    s.run("ClearCursor()").unwrap();
    assert!(s.cursor_item().is_none());
    assert!(!s.eval::<bool>("return PickupInventoryItem(1)").unwrap());
    s.run("PickupContainerItem(0, 1)").unwrap();
    assert_eq!(
        s.take_item_picks(),
        vec![(EQUIPMENT_BAG, 1), (0, 1)],
        "the doll reports in the ONE bag space; the bag reports its own id"
    );
    assert!(s.cursor_item().is_none(), "a bind never stages a payload");
}

/// The use bindings bind too: stock `ContainerFrameItemButton_OnClick` calls `UseContainerItem`
/// under `SpellCanTargetItem()` (`ContainerFrame.lua:698-700`), so a Disenchant cursor over a bag
/// item must target it, never use it. An empty slot binds nothing and uses nothing.
#[test]
fn the_armed_item_half_takes_the_use_bindings_too() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    s.set_container(0, Some(one_fitting_bag_item()));
    s.set_item_pick_armed(true);
    s.run("UseContainerItem(0, 1)").unwrap();
    s.run("UseInventoryItem(1)").unwrap();
    s.run("UseContainerItem(0, 9)").unwrap();
    assert_eq!(s.take_item_picks(), vec![(0, 1), (EQUIPMENT_BAG, 1)]);
    assert!(s.take_container_uses().is_empty() && s.take_inventory_uses().is_empty());

    s.set_item_pick_armed(false);
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(
        s.take_container_uses(),
        vec![(0, 1)],
        "unarmed, the click uses the item"
    );
    assert!(s.take_item_picks().is_empty());

    // A locked item does nothing, armed or not (`0x5d8650` returns before the bind).
    let mut locked = one_fitting_bag_item();
    locked.slots.get_mut(&1).unwrap().locked = true;
    s.set_container(0, Some(locked));
    s.set_item_pick_armed(true);
    s.run("UseContainerItem(0, 1)").unwrap();
    assert!(s.take_item_picks().is_empty() && s.take_container_uses().is_empty());
}

#[path = "doll_repair_tests.rs"]
mod repair;
