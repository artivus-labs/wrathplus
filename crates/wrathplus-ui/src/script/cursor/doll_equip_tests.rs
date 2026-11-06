//! Equip-cursor branching and inventory-slot argument regressions.

use super::*;

#[test]
fn equip_cursor_item_putback_autoequip_and_argument_edges() {
    for arg in ["2", "0", "256", "-1", "2^31", "'2.9'"] {
        let mut s = UiScript::new().unwrap();
        s.set_container(0, Some(one_fitting_bag_item()));
        s.run("PickupContainerItem(0, 1)").unwrap();
        assert_eq!(s.arity(&format!("EquipCursorItem({arg})")).unwrap(), 0);
        assert!(s.cursor_payload().is_none(), "{arg}");
        assert_eq!(s.take_container_autoequips(), vec![(0, 1)], "{arg}");
        assert!(s.take_container_moves().is_empty());
    }
    for arg in ["1", "2", "0", "256", "'1.9'"] {
        let mut s = UiScript::new().unwrap();
        s.set_inventory_slots(doll_slots());
        s.run("PickupInventoryItem(1)").unwrap();
        s.run(&format!("EquipCursorItem({arg})")).unwrap();
        assert!(s.cursor_payload().is_none());
        assert!(s.take_container_moves().is_empty());
        assert!(s.take_container_autoequips().is_empty());
    }
    let mut s = UiScript::new().unwrap();
    for arg in ["", "nil", "true", "{}", "'no'"] {
        let err = s.run(&format!("EquipCursorItem({arg})")).unwrap_err();
        assert!(err.to_string().contains("Usage: EquipCursorItem(slot)"));
    }
    let mut bag = one_fitting_bag_item();
    bag.slots.get_mut(&1).unwrap().count = 5;
    s.set_container(0, Some(bag));
    s.run("SplitContainerItem(0, 1, 1); EquipCursorItem(2)")
        .unwrap();
    assert!(s.cursor_payload().is_none());
    assert_eq!(s.take_container_autoequips(), vec![(0, 1)]);
}

#[test]
fn cursor_can_go_in_slot_uses_inventory_reader_and_zero_return_rejection() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(one_fitting_bag_item()));
    s.run("PickupContainerItem(0, 1)").unwrap();
    for arg in ["1.9", "'1.9'", "{}", "nil", "true", "'unknown'"] {
        assert_eq!(
            s.eval::<i64>(&format!("return CursorCanGoInSlot({arg})"))
                .unwrap(),
            1
        );
    }
    for arg in ["24", "256", "-1", "2^31"] {
        assert_eq!(s.arity(&format!("CursorCanGoInSlot({arg})")).unwrap(), 0);
    }
    assert_eq!(s.arity("CursorCanGoInSlot(2)").unwrap(), 1);
    assert!(s
        .eval::<bool>("return CursorCanGoInSlot(2) == nil")
        .unwrap());
}

#[test]
fn equip_cursor_item_fitting_split_swaps_the_source_stack_not_the_split() {
    let mut s = UiScript::new().unwrap();
    s.set_inventory_slots(doll_slots());
    let mut bag = one_fitting_bag_item();
    bag.slots.get_mut(&1).unwrap().count = 5;
    s.set_container(0, Some(bag));
    s.run("SplitContainerItem(0, 1, 2); EquipCursorItem('1.9')")
        .unwrap();
    assert_eq!(s.cursor_item().unwrap().item_id, 1234);
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
    assert!(s.take_container_autoequips().is_empty());
}
