use super::{apply_container_source, ContainerSlot, FeedMemory, SlotGuids, BANK_CONTAINER};
use crate::inventory::BANK_SLOTS;

/// No bags and an empty vault.
const NO_BAGS: SlotGuids = SlotGuids {
    bags: [0; 11],
    vault: [0; 28],
};

/// The handler's container ids reach the VM as `BAG_OPEN`'s `arg1`.
#[test]
fn a_server_opened_bag_fires_bag_open_with_its_container_id() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "SEEN = {} \
         local f = CreateFrame('Frame') \
         f:RegisterEvent('BAG_OPEN') \
         f:RegisterEvent('BAG_UPDATE') \
         f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) \
             table.insert(SEEN, event .. ' ' .. tostring(arg1)) end)",
    )
    .unwrap();
    let mut memory = FeedMemory::default();
    // A bag equipped into the second bag slot, and nothing else changed this frame.
    assert!(apply_container_source(
        &mut s,
        &mut memory,
        None,
        NO_BAGS,
        Vec::new(),
        vec![2]
    ));
    let seen = s.eval::<Vec<String>>("return SEEN").unwrap();
    assert_eq!(seen, vec!["BAG_OPEN 2"]);
    // Nothing queued: nothing fired, and the push reports nothing went in.
    s.run("SEEN = {}").unwrap();
    assert!(!apply_container_source(
        &mut s,
        &mut memory,
        None,
        NO_BAGS,
        Vec::new(),
        Vec::new()
    ));
    assert!(s.eval::<Vec<String>>("return SEEN").unwrap().is_empty());
}

/// A new guid and a restack both fire the bank slot (12340's `0x6dedd0`, `0x5e84c0`), and two
/// identical items swapped fire both slots though the views are equal.
#[test]
fn playerbankslots_changed_names_the_bank_slot() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "SEEN = {} \
         local f = CreateFrame('Frame') \
         f:RegisterEvent('PLAYERBANKSLOTS_CHANGED') \
         f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) \
             table.insert(SEEN, event .. ' ' .. tostring(arg1)) end)",
    )
    .unwrap();
    let seen = |s: &mut UiScript| {
        let out = s.eval::<Vec<String>>("return SEEN").unwrap();
        s.run("SEEN = {}").unwrap();
        out
    };
    // The vault, with `slot` holding `count` of item 1234.
    let vault = |slot: u32, count: u32| {
        HashMap::from([(
            BANK_CONTAINER,
            ContainerState {
                name: None,
                num_slots: u32::from(BANK_SLOTS),
                slots: HashMap::from([(
                    slot,
                    ContainerSlot {
                        item_id: 1234,
                        count,
                        ..Default::default()
                    },
                )]),
            },
        )])
    };
    let mut guids = NO_BAGS;
    let mut memory = FeedMemory::default();

    // An item arrives in vault slot 1: the descriptor path.
    guids.vault[0] = 0xF00D;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(vault(1, 5)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["PLAYERBANKSLOTS_CHANGED 1"]);

    // The same item restacks: the item watcher's path, the same slot.
    apply_container_source(
        &mut s,
        &mut memory,
        Some(vault(1, 9)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["PLAYERBANKSLOTS_CHANGED 1"]);

    apply_container_source(
        &mut s,
        &mut memory,
        Some(vault(1, 9)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert!(seen(&mut s).is_empty(), "an unchanged vault says nothing");

    // Two identical stacks swapped between slots 1 and 2: the guids are the only witness.
    let two = || {
        HashMap::from([(
            BANK_CONTAINER,
            ContainerState {
                name: None,
                num_slots: u32::from(BANK_SLOTS),
                slots: HashMap::from([
                    (
                        1,
                        ContainerSlot {
                            item_id: 1234,
                            count: 9,
                            ..Default::default()
                        },
                    ),
                    (
                        2,
                        ContainerSlot {
                            item_id: 1234,
                            count: 9,
                            ..Default::default()
                        },
                    ),
                ]),
            },
        )])
    };
    guids.vault[1] = 0xBEEF;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(two()),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["PLAYERBANKSLOTS_CHANGED 2"]);
    guids.vault.swap(0, 1);
    let pushed = apply_container_source(
        &mut s,
        &mut memory,
        Some(two()),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        seen(&mut s),
        vec![
            "PLAYERBANKSLOTS_CHANGED 1".to_string(),
            "PLAYERBANKSLOTS_CHANGED 2".to_string()
        ],
        "the swap the pushed containers cannot see"
    );
    assert!(pushed, "and the feed reports that it spoke to the VM");
}

/// `BAG_CLOSED` (`0x4f92b5`): a swap fires it, an unequip fires it without `BAG_UPDATE`, and
/// two identical bags swapped still close both windows.
#[test]
fn bag_closed_fires_on_the_bag_guid_moving_and_only_off_a_non_empty_slot() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "SEEN = {} \
         local f = CreateFrame('Frame') \
         f:RegisterEvent('BAG_CLOSED') \
         f:RegisterEvent('BAG_UPDATE') \
         f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) \
             table.insert(SEEN, event .. ' ' .. tostring(arg1)) end)",
    )
    .unwrap();
    let seen = |s: &mut UiScript| {
        let out = s.eval::<Vec<String>>("return SEEN").unwrap();
        s.run("SEEN = {}").unwrap();
        out
    };
    let pouch = |n: u32| {
        HashMap::from([(
            1,
            ContainerState {
                name: Some("Small Brown Pouch".into()),
                num_slots: n,
                slots: HashMap::new(),
            },
        )])
    };
    let mut guids = NO_BAGS;
    let mut memory = FeedMemory::default();

    // Equipped into an empty slot: `BAG_UPDATE` only (`0x4f9247`).
    guids.bags[0] = 0xAAAA;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(pouch(6)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["BAG_UPDATE 1"]);

    apply_container_source(
        &mut s,
        &mut memory,
        Some(pouch(6)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert!(
        seen(&mut s).is_empty(),
        "an unchanged container says nothing"
    );

    // Swapped for another bag: closed, then updated.
    guids.bags[0] = 0xBBBB;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(pouch(10)),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["BAG_CLOSED 1", "BAG_UPDATE 1"]);

    // Unequipped: closed, and no `BAG_UPDATE`.
    guids.bags[0] = 0;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(HashMap::new()),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["BAG_CLOSED 1"]);

    // Two identical empty bags swapped between containers 5 and 6: equal states, both close.
    let mut memory = FeedMemory::default();
    let two = || {
        HashMap::from([
            (
                5,
                ContainerState {
                    name: Some("Linen Bag".into()),
                    num_slots: 6,
                    slots: HashMap::new(),
                },
            ),
            (
                6,
                ContainerState {
                    name: Some("Linen Bag".into()),
                    num_slots: 6,
                    slots: HashMap::new(),
                },
            ),
        ])
    };
    let mut guids = NO_BAGS;
    guids.bags[4] = 0x1111;
    guids.bags[5] = 0x2222;
    apply_container_source(
        &mut s,
        &mut memory,
        Some(two()),
        guids,
        Vec::new(),
        Vec::new(),
    );
    let _ = seen(&mut s);
    guids.bags.swap(4, 5);
    let pushed = apply_container_source(
        &mut s,
        &mut memory,
        Some(two()),
        guids,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(seen(&mut s), vec!["BAG_CLOSED 5", "BAG_CLOSED 6"]);
    assert!(
        pushed,
        "a BAG_CLOSED with no container change still went into the VM — the gate audit reads \
         this return"
    );
}
use std::collections::HashMap;
use wrathplus_ui::script::{ContainerState, UiScript};

/// An absent self store is no source, never an empty bag burst; a present source that lost a
/// bag still announces.
#[test]
fn an_absent_self_player_is_no_source_never_an_empty_bag_burst() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "BAG_EVENTS = 0 \
         local f = CreateFrame('Frame') \
         f:RegisterEvent('BAG_UPDATE') \
         f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) BAG_EVENTS = BAG_EVENTS + 1 end)",
    )
    .unwrap();
    let bag0 = ContainerState {
        name: Some("Backpack".into()),
        num_slots: 16,
        slots: HashMap::new(),
    };
    let mut memory = FeedMemory::default();

    // In session: a new bag is pushed and announced.
    apply_container_source(
        &mut s,
        &mut memory,
        Some(HashMap::from([(0, bag0)])),
        NO_BAGS,
        Vec::new(),
        Vec::new(),
    );
    // `GetContainerNumSlots(0)` is the constant 16; the free-slot table needs the pushed bag.
    let free = "return select('#', GetContainerFreeSlots(0))";
    assert_eq!(s.eval::<i64>(free).unwrap(), 1);
    assert_eq!(s.eval::<i64>("return BAG_EVENTS").unwrap(), 1);

    // The logout despawn frame: the VM keeps its bags and nothing fires.
    apply_container_source(&mut s, &mut memory, None, NO_BAGS, Vec::new(), Vec::new());
    assert_eq!(
        s.eval::<i64>(free).unwrap(),
        1,
        "an absent source must not empty the VM's containers"
    );
    assert_eq!(
        s.eval::<i64>("return BAG_EVENTS").unwrap(),
        1,
        "an absent source must not fire a BAG_UPDATE burst"
    );

    // A present source without the bag is a real transition.
    apply_container_source(
        &mut s,
        &mut memory,
        Some(HashMap::new()),
        NO_BAGS,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(s.eval::<i64>(free).unwrap(), 0, "the backpack is gone");
    assert_eq!(s.eval::<i64>("return BAG_EVENTS").unwrap(), 2);
}

/// A stack ticking down fires an argless `ITEM_LOCK_CHANGED` before its bag's `BAG_UPDATE`
/// (`0x5d94b9`): an auto-shot timer such as Quiver's learns of a shot from it alone. A changed
/// entry is a swap and must not fire it, or the timer counts shots never fired.
#[test]
fn a_stack_ticking_down_fires_item_lock_changed_before_bag_update() {
    use wrathplus_ui::script::ContainerSlot;

    let mut s = UiScript::new().unwrap();
    s.run(
        "ORDER = {} \
         local f = CreateFrame('Frame') \
         f:RegisterEvent('ITEM_LOCK_CHANGED') \
         f:RegisterEvent('BAG_UPDATE') \
         f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) \
             table.insert(ORDER, event .. ':' .. tostring(arg1) .. ',' .. tostring(arg2)) \
         end)",
    )
    .unwrap();

    let arrows = |count: u32| ContainerSlot {
        item_id: 2512, // Rough Arrow
        count,
        ..Default::default()
    };
    let bag = |s: ContainerSlot| {
        HashMap::from([(
            0,
            ContainerState {
                name: Some("Backpack".into()),
                num_slots: 16,
                slots: HashMap::from([(1, s)]),
            },
        )])
    };
    let mut memory = FeedMemory::default();

    // The quiver arrives full: a create, no lock event.
    apply_container_source(
        &mut s,
        &mut memory,
        Some(bag(arrows(200))),
        NO_BAGS,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        s.eval::<String>("return table.concat(ORDER, ' ')").unwrap(),
        "BAG_UPDATE:0,nil",
        "a slot appearing is a create, not a stack-count field write"
    );

    // An arrow leaves the quiver.
    s.run("ORDER = {}").unwrap();
    apply_container_source(
        &mut s,
        &mut memory,
        Some(bag(arrows(199))),
        NO_BAGS,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        s.eval::<String>("return table.concat(ORDER, ' ')").unwrap(),
        "ITEM_LOCK_CHANGED:0,1 BAG_UPDATE:0,nil",
        "the shot signal names its slot, `bag, slot` as 3.3.5's ContainerFrame reads it, and \
         precedes BAG_UPDATE"
    );

    // A different item in the slot, with a different count: a swap, no lock event.
    s.run("ORDER = {}").unwrap();
    let mut other = arrows(20);
    other.item_id = 3033; // Razor Arrow
    apply_container_source(
        &mut s,
        &mut memory,
        Some(bag(other)),
        NO_BAGS,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        s.eval::<String>("return table.concat(ORDER, ' ')").unwrap(),
        "BAG_UPDATE:0,nil",
        "a changed entry is a swap, not a stack-count write"
    );
}
