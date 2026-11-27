use super::*;
use std::collections::VecDeque;

fn set(index: u32, guid: u64, name: &str) -> EquipmentSet {
    let mut items = [0; EQUIPMENT_SLOTS];
    items[0] = 0x4000_0000_0000_0101;
    items[4] = IGNORED;
    EquipmentSet {
        set_guid: guid,
        index,
        name: name.into(),
        icon: "INV_Shield_06".into(),
        items,
    }
}

#[test]
fn set_receipts_leave_the_capture_and_other_receipts_keep_their_order() {
    let mut sets = EquipmentSets::default();
    let mut packets = VecDeque::from([
        Packet::ReadItemOk { item_guid: 1 },
        Packet::EquipmentSetList {
            sets: vec![set(3, 30, "Tank"), set(0, 10, "Heal")],
        },
        Packet::RefundInfo {
            item_guid: 9,
            cost: wrathplus_protocol::wrath_inventory::RefundCost {
                money: 0,
                honor: 0,
                arena: 0,
                items: [(0, 0); 5],
            },
            raw_unknown: 0,
            played_seconds: 0,
        },
    ]);
    assert_eq!(absorb(&mut packets, &mut sets), (Vec::new(), Vec::new()));
    assert_eq!(packets.len(), 2, "only the set list leaves");
    assert_eq!(
        sets.sets
            .values()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Heal", "Tank"],
        "by server index"
    );
    sets.sets.insert(5, set(5, 0, "New"));
    sets.pending_use = Some("Tank".into());
    let mut packets = VecDeque::from([
        Packet::EquipmentSetSaved {
            index: 5,
            set_guid: 50,
        },
        Packet::EquipmentSetUseResult { result: 4 },
        Packet::EquipmentSetUseResult { result: 0 },
    ]);
    assert_eq!(
        absorb(&mut packets, &mut sets),
        (vec![(false, Some("Tank".into())), (true, None)], Vec::new())
    );
    assert!(packets.is_empty());
    assert_eq!(sets.sets[&5].set_guid, 50);
}

#[test]
fn saves_deletes_and_uses_update_the_local_copy_and_name_the_packet() {
    let mut sets = EquipmentSets::default();
    sets.sets.insert(0, set(0, 10, "Heal"));
    let mut worn = [0; EQUIPMENT_SLOTS];
    worn[0] = 0x4000_0000_0000_0201;
    worn[4] = 0x4000_0000_0000_0205;
    let mut ignored = [false; EQUIPMENT_SLOTS];
    ignored[4] = true;
    let save = |name: &str| EquipmentSetRequest::Save {
        name: name.into(),
        icon: "INV_Misc_QuestionMark".into(),
        ignored,
    };
    let carried = HashMap::from([(0x4000_0000_0000_0101, (19, 7))]);

    // A new name takes the first free index and no GUID yet; the ignored slot is raw 1.
    let Some(Command::EquipmentSetSave {
        set_guid,
        index,
        items,
        ..
    }) = apply(&mut sets, save("PvP"), &worn, &carried)
    else {
        panic!()
    };
    assert_eq!(
        (set_guid, index, items[0], items[4]),
        (0, 1, worn[0], IGNORED)
    );
    // Saving over a name keeps its index and GUID.
    let Some(Command::EquipmentSetSave {
        set_guid, index, ..
    }) = apply(&mut sets, save("Heal"), &worn, &carried)
    else {
        panic!()
    };
    assert_eq!((set_guid, index), (10, 0));
    assert_eq!(sets.sets.len(), 2);

    // A use names each slot's item and where it is now.
    sets.sets.insert(0, set(0, 10, "Heal"));
    let use_heal = EquipmentSetRequest::Use {
        name: "Heal".into(),
    };
    let Some(Command::EquipmentSetUse { items }) = apply(&mut sets, use_heal, &worn, &carried)
    else {
        panic!()
    };
    assert_eq!(items[0], (0x4000_0000_0000_0101, 19, 7));
    assert_eq!((items[1], items[4]), ((0, 0, 0), (IGNORED, 0, 0)));
    assert_eq!(sets.pending_use.as_deref(), Some("Heal"));

    // An unnumbered set is forgotten locally and deleted once its GUID arrives.
    let delete = |name: &str| EquipmentSetRequest::Delete { name: name.into() };
    assert_eq!(apply(&mut sets, delete("PvP"), &worn, &carried), None);
    let mut packets = VecDeque::from([Packet::EquipmentSetSaved {
        index: 1,
        set_guid: 77,
    }]);
    assert_eq!(absorb(&mut packets, &mut sets), (Vec::new(), vec![77]));
    assert!(!sets.sets.contains_key(&1) && sets.pending_delete.is_empty());
    assert_eq!(
        apply(&mut sets, delete("Heal"), &worn, &carried),
        Some(Command::EquipmentSetDelete { set_guid: 10 })
    );
    assert!(sets.sets.is_empty());
    assert_eq!(apply(&mut sets, delete("Heal"), &worn, &carried), None);
}

#[test]
fn a_full_manager_saves_nothing_new() {
    let mut sets = EquipmentSets::default();
    for i in 0..MAX_SETS {
        sets.sets
            .insert(i, set(i, u64::from(i) + 1, &format!("S{i}")));
    }
    let request = EquipmentSetRequest::Save {
        name: "Eleventh".into(),
        icon: String::new(),
        ignored: [false; EQUIPMENT_SLOTS],
    };
    assert_eq!(
        apply(&mut sets, request, &[0; EQUIPMENT_SLOTS], &HashMap::new()),
        None
    );
    // The VM's view keeps the server's index as the record id (`[rec+0x10]`).
    let view = set_view(&sets.sets[&7], &|_| Some(40_000));
    assert_eq!((view.id, view.items[0], view.items[4]), (7, 40_000, 1));
}
