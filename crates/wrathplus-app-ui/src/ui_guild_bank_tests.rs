use super::*;
use wrathplus_protocol::wrath_guild_bank::Slot;
use wrathplus_protocol::wrath_guild_bank_log::Entry;
use wrathplus_protocol::wrath_guild_bank_text::Text;
use wrathplus_ui::script::ScriptValue;

fn dbg(command: Option<ClientCommand>) -> String {
    format!("{command:?}")
}

fn item(count: u32) -> Item {
    Item {
        flags: 0,
        random_property: 0,
        random_property_seed: None,
        count,
        enchantment: 0,
        charges: 0,
        sockets: Vec::new(),
    }
}

fn list(tab: u8, full_update: u8, slots: Vec<(u8, u32, u32)>) -> Packet {
    Packet::Contents(Contents {
        money: 5_000,
        tab,
        remaining_withdrawals: 3,
        full_update,
        tab_info: (tab == 0 && full_update != 0).then(|| {
            vec![TabInfo {
                name: b"Mats".to_vec(),
                icon: b"INV_Box_01".to_vec(),
            }]
        }),
        slots: slots
            .into_iter()
            .map(|(slot, item_id, count)| Slot {
                slot,
                item_id,
                item: (item_id != 0).then(|| item(count)),
            })
            .collect(),
    })
}

#[test]
fn lists_fill_tabs_and_slots_and_partial_updates_patch_them() {
    let mut bank = GuildBank::default();
    apply(&mut bank, list(0, 1, vec![(0, 100, 5), (3, 200, 1)]), false);
    assert!(!bank.open, "a list nobody asked for opens nothing");
    apply(&mut bank, list(0, 1, vec![(0, 100, 5), (3, 200, 1)]), true);
    assert!(bank.open);
    assert_eq!(bank.tabs.len(), 1);
    assert_eq!(bank.slots[0].len(), 2);
    // A partial update changes the named slots only; item 0 empties one.
    apply(&mut bank, list(0, 0, vec![(3, 0, 0), (7, 300, 2)]), true);
    assert_eq!(bank.slots[0].keys().copied().collect::<Vec<_>>(), [0, 7]);
    assert_eq!(
        bank.tabs.len(),
        1,
        "tab info only rides a full tab-0 update"
    );
    apply(&mut bank, Packet::RemainingMoney { remaining: -1 }, true);
    apply(
        &mut bank,
        Packet::Text(Text {
            tab: 0,
            text: b"Officers only".to_vec(),
        }),
        true,
    );
    apply(
        &mut bank,
        Packet::Log(Log {
            tab: MONEY_LOG_TAB,
            entries: vec![Entry {
                kind: 4,
                actor: 9,
                payload: Payload::Money { amount: 100 },
                elapsed: 60,
            }],
        }),
        true,
    );
    assert_eq!(bank.money_remaining, Some(-1));
    assert_eq!(bank.texts[&0], "Officers only");
    assert_eq!(bank.logs[&MONEY_LOG_TAB].entries.len(), 1);
}

#[test]
fn gestures_name_the_vault_and_zero_based_coordinates() {
    use wrathplus_protocol::wrath_guild_bank_query::Query;
    use wrathplus_protocol::wrath_guild_bank_text::Command as T;
    let mut bank = GuildBank::default();
    let mut open = GuildBankOpen::default();
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::QueryTab { tab: 1 },
            None
        )),
        dbg(None),
        "no vault open"
    );
    open.open(0x77);
    apply(&mut bank, list(0, 1, vec![(4, 100, 5)]), true);
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::QueryTab { tab: 1 },
            None
        )),
        dbg(Some(ClientCommand::GuildBankQuery(Query::Tab {
            banker: 0x77,
            tab: 0,
            full_update: 1
        })))
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::QueryLog { tab: 7 },
            None
        )),
        dbg(Some(ClientCommand::NativeGuildBankText(T::QueryLog {
            tab: 6
        }))),
        "tab 7 is the money log"
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::AutoStoreItem { tab: 1, slot: 5 },
            None
        )),
        dbg(Some(ClientCommand::NativeGuildBankSwap(Swap {
            banker: 0x77,
            transfer: Transfer::AutoStore {
                source: BankSlot {
                    tab: 0,
                    slot: 4,
                    item: 100
                },
                count: 5
            },
        })))
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::PickupItem { tab: 1, slot: 1 },
            None
        )),
        dbg(None),
        "picking a bank item up waits for the cursor mode"
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::PickupItem { tab: 1, slot: 1 },
            Some((255, 23, 0))
        )),
        dbg(Some(ClientCommand::NativeGuildBankSwap(Swap {
            banker: 0x77,
            transfer: Transfer::InventoryToBank {
                destination: BankSlot {
                    tab: 0,
                    slot: 0,
                    item: 0
                },
                container: 255,
                container_slot: 23,
                stack_count: 0,
            },
        })))
    );
    // A bag right-click deposits into the current tab's first free slot: slot 0xff, no item.
    let Some(ClientCommand::NativeGuildBankSwap(swap)) =
        crate::ui_items::guild_bank_deposit(open.banker, 2, 0, 1)
    else {
        panic!("a deposit");
    };
    assert_eq!(
        (swap.banker, format!("{:?}", swap.transfer)),
        (
            0x77,
            format!(
                "{:?}",
                Transfer::InventoryToBank {
                    destination: BankSlot {
                        tab: 2,
                        slot: 0xff,
                        item: 0
                    },
                    container: 255,
                    container_slot: 23,
                    stack_count: 0,
                }
            )
        )
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::Close,
            None
        )),
        dbg(None)
    );
    assert!(open.banker().is_none() && !bank.open);
    // Closing clears the bank guid only (`0x5a4270`): money and the tab count stay readable.
    assert_eq!(bank.tab_purchase(), (false, 1, 5_000));
    assert!(
        crate::ui_items::guild_bank_deposit(open.banker, 2, 0, 1).is_none(),
        "the vault closed"
    );
}

#[test]
fn the_frame_events_follow_the_view() {
    let names = |events: Vec<(&'static str, Vec<ScriptValue>)>| {
        events.into_iter().map(|(e, _)| e).collect::<Vec<_>>()
    };
    let open = WrathGuildBankView {
        open: true,
        money: 10,
        tabs: vec![WrathGuildBankTab::default()],
        slots: vec![Vec::new()],
        tab_text: vec![String::new()],
        ..Default::default()
    };
    assert_eq!(
        names(events(None, &open)),
        [
            "GUILDBANKFRAME_OPENED",
            "GUILDBANK_UPDATE_TABS",
            "GUILDBANK_UPDATE_MONEY",
            "GUILDBANKBAGSLOTS_CHANGED",
            "GUILDBANK_UPDATE_TEXT"
        ]
    );
    let mut richer = open.clone();
    richer.money = 20;
    richer.tab_text[0] = "hi".into();
    let changed = events(Some(&open), &richer);
    assert_eq!(
        names(changed.clone()),
        ["GUILDBANK_UPDATE_MONEY", "GUILDBANK_UPDATE_TEXT"]
    );
    assert_eq!(changed[1].1, [ScriptValue::Int(1)]);
    assert_eq!(
        names(events(Some(&richer), &WrathGuildBankView::default())),
        ["GUILDBANKFRAME_CLOSED"]
    );
    assert!(events(None, &WrathGuildBankView::default()).is_empty());
    // A closed vault's published state (rank rights from the roster, kept money) opens nothing.
    let closed = WrathGuildBankView {
        open: false,
        ..richer
    };
    assert!(events(None, &closed).is_empty());
}

#[test]
fn held_bank_items_move_to_bank_bag_and_worn_slots() {
    let mut bank = GuildBank::default();
    let mut open = GuildBankOpen::default();
    open.open(0x77);
    apply(&mut bank, list(0, 1, vec![(4, 100, 5)]), true);
    let source = BankSlot {
        tab: 0,
        slot: 4,
        item: 100,
    };
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::MoveItem {
                from_tab: 1,
                from_slot: 5,
                to_tab: 1,
                to_slot: 9,
                count: Some(2)
            },
            None
        )),
        dbg(Some(ClientCommand::NativeGuildBankSwap(Swap {
            banker: 0x77,
            transfer: Transfer::BankToBank {
                destination: BankSlot {
                    tab: 0,
                    slot: 8,
                    item: 0
                },
                source,
                count: 2
            },
        })))
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::DropOnBag {
                tab: 1,
                slot: 5,
                bag: 0,
                bag_slot: 1,
                count: None
            },
            None
        )),
        dbg(Some(ClientCommand::NativeGuildBankSwap(Swap {
            banker: 0x77,
            transfer: Transfer::BankToInventory {
                source,
                container: 255,
                container_slot: 23,
                stack_count: 0
            },
        }))),
        "the backpack's first slot"
    );
    assert_eq!(
        dbg(command(
            &mut bank,
            &mut open,
            WrathGuildBankRequest::DropOnEquip {
                tab: 1,
                slot: 5,
                equip_slot: 1,
                count: None
            },
            None
        )),
        dbg(Some(ClientCommand::NativeGuildBankSwap(Swap {
            banker: 0x77,
            transfer: Transfer::BankToInventory {
                source,
                container: 255,
                container_slot: 0,
                stack_count: 0
            },
        }))),
        "the head slot"
    );
}

/// `GE_BANK_TEXT_CHANGED` (`0x6d92d0` case `0x13`) queues its 0-based tab; no other event does.
#[test]
fn a_bank_text_change_event_queues_its_tab() {
    use bevy::ecs::system::RunSystemOnce;
    use wrathplus_protocol::types::{guild_event, GuildEventNotice};
    let mut world = World::new();
    world.init_resource::<GuildBank>();
    for (event, param) in [
        (guild_event::BANK_TEXT_CHANGED, "2"),
        (guild_event::MOTD, "3"),
        (guild_event::BANK_TEXT_CHANGED, "x"),
    ] {
        let notice = GuildEventNotice {
            event,
            params: vec![param.into()],
            guid: None,
        };
        world
            .run_system_once_with(text_changed, SessionEvent::GuildEvent(notice))
            .unwrap();
    }
    assert_eq!(world.resource::<GuildBank>().text_changed, [2]);
}
