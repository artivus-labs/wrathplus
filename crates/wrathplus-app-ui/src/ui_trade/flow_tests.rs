//! An AzerothCore-shaped trade end to end: initiate at the selection, the window opens, our sword
//! dragged onto the top slot, the bot's side arriving as the one `SMSG_TRADE_STATUS_EXTENDED`
//! AzerothCore sends us (`TradeData::Update` writes a change to the trader only,
//! `TradeData.cpp:115-121`, so our own column is filled here), the bot's accept, ours, and the
//! swap. Outbound steps are the writer's bytes ([`crate::net::io::exchange_packet`]), inbound ones
//! the events the stock `TradeFrame.lua` repaints on. A bot-initiated trade needs no prompt in
//! 12340: `BEGIN_TRADE` is answered by the refusal ladder (`super::super::tests`' request tests).

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use wrathplus_protocol::types::{Field, TradeItem, TradeStatus, TradeStatusExtended};
use wrathplus_protocol::SessionEvent;
use wrathplus_ui::script::{ContainerSlot, ContainerState, UiScript};

use super::super::{drain_trade, feed_trade, TradeSession};
use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::app_core::state::Selection;
use crate::net::{ClientCommand, Guid, GuidIndex, NetCommands};

const BOT: u64 = 0x5D3;
const SWORD: u64 = 0x4000_0000_0000_0042;
const SWORD_ENTRY: u32 = 25;
const LINEN: u32 = 2589;

/// Us with the sword in backpack slot 1 (object, template and the VM's bag), the bot selected,
/// [`feed_trade`] on `Update`, and a VM that records the trade events.
fn trade_app() -> (App, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.init_resource::<TradeSession>()
        .init_resource::<crate::items::Items>()
        .init_resource::<crate::app_core::errors::UiErrorKeys>()
        .init_resource::<crate::app_core::group::GroupState>()
        .init_resource::<GuidIndex>()
        .insert_resource({
            let mut names = crate::names::NameCache::default();
            names.insert_player(BOT, "Grubbis".to_string(), None);
            names
        })
        .insert_resource(NetCommands(tx))
        .insert_resource(Selection {
            guid: Some(BOT),
            ..Default::default()
        });
    app.add_systems(Update, feed_trade);
    let world = app.world_mut();
    let pack = Field::PlayerPackSlot1.index();
    world.spawn((
        SelfPlayer,
        ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(&[
            (pack, SWORD as u32),
            (pack + 1, (SWORD >> 32) as u32),
        ])),
    ));
    let sword = world
        .spawn((
            Guid(SWORD),
            ObjectStore(wrathplus_protocol::ObjectFields::from_fields(&[
                (Field::ObjectEntry, SWORD_ENTRY),
                (Field::ItemStackCount, 1),
            ])),
        ))
        .id();
    world.resource_mut::<GuidIndex>().0.insert(SWORD, sword);
    // `InitiateTrade` needs the partner streamed as a player object (`0x522660`).
    let bot = world
        .spawn((Guid(BOT), ObjectStore(Default::default())))
        .id();
    world.resource_mut::<GuidIndex>().0.insert(BOT, bot);
    for (entry, name) in [(SWORD_ENTRY, "Worn Shortsword"), (LINEN, "Linen Cloth")] {
        world
            .resource_mut::<crate::items::Items>()
            .insert_template(entry, Some(crate::items::test_template(name)));
    }
    let mut script = UiScript::new().unwrap();
    let mut bag = ContainerState {
        num_slots: 16,
        ..Default::default()
    };
    bag.slots.insert(
        1,
        ContainerSlot {
            item_id: SWORD_ENTRY,
            count: 1,
            ..Default::default()
        },
    );
    script.set_container(0, Some(bag));
    script
        .run(
            r#"HEARD = {}
            local f = CreateFrame("Frame")
            for _, e in ipairs({ "TRADE_SHOW", "TRADE_PLAYER_ITEM_CHANGED",
                "TRADE_TARGET_ITEM_CHANGED", "TRADE_ACCEPT_UPDATE", "TRADE_CLOSED" }) do
                f:RegisterEvent(e)
            end
            f:SetScript("OnEvent", function(_, e, a, b)
                table.insert(HEARD, e .. "(" .. tostring(a) .. "," .. tostring(b) .. ")")
            end)"#,
        )
        .unwrap();
    world.insert_non_send_resource(script);
    (app, rx)
}

fn lua(app: &mut App, code: &str) {
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(code)
        .unwrap();
}

/// The events since the last call.
fn heard(app: &mut App) -> String {
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .eval("local s = table.concat(HEARD, ' ') HEARD = {} return s")
        .unwrap()
}

/// The VM's queued intents onto the wire, and the one trade packet they sent.
fn drained(app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>) -> (u32, Vec<u8>) {
    app.world_mut().run_system_once(drain_trade).unwrap();
    let sent: Vec<ClientCommand> = rx
        .try_iter()
        .filter(|c| !matches!(c, ClientCommand::ItemQuery { .. }))
        .collect();
    let [command] = <[ClientCommand; 1]>::try_from(sent).expect("one trade command");
    crate::net::io::exchange_packet(command)
}

/// A server packet through the trade's own handler, then a frame of the feed.
fn receive(app: &mut App, event: SessionEvent) {
    let world = app.world_mut();
    match event {
        SessionEvent::TradeStatus { status } => {
            let commands = world.resource::<NetCommands>().clone();
            world.resource_scope(|world, mut trade: Mut<TradeSession>| {
                let mut errors = world.resource_mut::<crate::app_core::errors::UiErrorKeys>();
                super::trade_status(status, &mut trade, &mut errors, &commands);
            });
        }
        SessionEvent::TradeStatusExtended { state } => {
            super::trade_status_extended(&state, &mut world.resource_mut::<TradeSession>());
        }
        _ => unreachable!(),
    }
    app.update();
}

fn linen(count: u32) -> TradeItem {
    TradeItem {
        entry: LINEN,
        display_id: 0,
        count,
        wrapped: false,
        gift_creator: 0,
        perm_enchant: 0,
        creator: 0,
        charges: 0,
        suffix_factor: 0,
        random_prop_id: 0,
        lock_id: 0,
        max_durability: 0,
        durability: 0,
    }
}

#[test]
fn an_azerothcore_trade_runs_from_initiate_to_complete() {
    let (mut app, rx) = trade_app();
    app.update();
    heard(&mut app);

    lua(&mut app, r#"InitiateTrade("target")"#);
    assert_eq!(
        drained(&mut app, &rx),
        (0x0116, BOT.to_le_bytes().to_vec()),
        "CMSG_INITIATE_TRADE"
    );

    receive(
        &mut app,
        SessionEvent::TradeStatus {
            status: TradeStatus::OpenWindow,
        },
    );
    assert!(heard(&mut app).contains("TRADE_SHOW"), "the window opens");

    // The sword dragged onto the top slot: trade slot 0, backpack slot 1 (wire 255/23).
    lua(&mut app, "PickupContainerItem(0, 1) ClickTradeButton(1)");
    assert_eq!(
        drained(&mut app, &rx),
        (0x011D, vec![0, 0xFF, 23]),
        "CMSG_SET_TRADE_ITEM"
    );
    app.update();
    let events = heard(&mut app);
    assert!(
        events.contains("TRADE_PLAYER_ITEM_CHANGED(1"),
        "our slot 1 repaints: {events}"
    );
    let ours: String = app
        .world_mut()
        .non_send_resource_mut::<UiScript>()
        .eval("return (GetTradePlayerItemInfo(1))")
        .unwrap();
    assert_eq!(ours, "Worn Shortsword", "our own column, filled locally");

    // The bot puts up five linen: the trader-side snapshot, their window.
    let mut slots = [None; 7];
    slots[0] = Some(linen(5));
    let state = TradeStatusExtended {
        their_window: true,
        gold: 0,
        enchant_spell_id: 0,
        slots,
    };
    receive(
        &mut app,
        SessionEvent::TradeStatusExtended {
            state: Box::new(state),
        },
    );
    let events = heard(&mut app);
    assert!(
        events.contains("TRADE_TARGET_ITEM_CHANGED(1"),
        "their slot 1 repaints: {events}"
    );
    let (theirs, count): (String, i64) = app
        .world_mut()
        .non_send_resource_mut::<UiScript>()
        .eval("local n, _, c = GetTradeTargetItemInfo(1) return n, c")
        .unwrap();
    assert_eq!((theirs.as_str(), count), ("Linen Cloth", 5));

    // The bot accepts, then we do.
    receive(
        &mut app,
        SessionEvent::TradeStatus {
            status: TradeStatus::Accept,
        },
    );
    let events = heard(&mut app);
    assert!(
        events.contains("TRADE_ACCEPT_UPDATE(0,1)"),
        "their column glows: {events}"
    );
    lua(&mut app, "AcceptTrade()");
    let (opcode, body) = drained(&mut app, &rx);
    assert_eq!((opcode, body.len()), (0x011A, 4), "CMSG_ACCEPT_TRADE");

    receive(
        &mut app,
        SessionEvent::TradeStatus {
            status: TradeStatus::Complete,
        },
    );
    assert!(
        heard(&mut app).contains("TRADE_CLOSED"),
        "the swap closes the window"
    );
    assert!(!app.world().resource::<TradeSession>().is_open());
}

/// A right-click on a bag item with the window open offers it in the first free slot.
#[test]
fn a_right_click_offers_the_bag_item_in_the_first_free_slot() {
    let (mut app, rx) = trade_app();
    lua(&mut app, r#"InitiateTrade("target")"#);
    drained(&mut app, &rx);
    receive(
        &mut app,
        SessionEvent::TradeStatus {
            status: TradeStatus::OpenWindow,
        },
    );
    lua(&mut app, "UseContainerItem(0, 1)");
    assert_eq!(
        drained(&mut app, &rx),
        (0x011D, vec![0, 0xFF, 23]),
        "into trade slot 0"
    );
}

/// `InitiateTrade`'s local gates (`0x522600`) and `CloseTrade`'s unconditional cancel
/// (`0x5879c4`): a refusal raises its error and sends nothing.
#[test]
fn initiate_refuses_locally_and_close_always_cancels() {
    let (mut app, rx) = trade_app();
    let errors = |app: &mut App| {
        let keys = std::mem::take(
            &mut app
                .world_mut()
                .resource_mut::<crate::app_core::errors::UiErrorKeys>()
                .0,
        );
        keys.iter().map(|e| e.key).collect::<Vec<_>>()
    };
    let sent = |app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>| {
        app.world_mut().run_system_once(drain_trade).unwrap();
        rx.try_iter()
            .filter(|c| !matches!(c, ClientCommand::ItemQuery { .. }))
            .count()
    };
    app.insert_resource(crate::app_core::session::SelfGuid(Some(0x42)));
    lua(&mut app, r#"InitiateTrade("player")"#);
    assert_eq!(
        (sent(&mut app, &rx), errors(&mut app)),
        (0, vec!["ERR_TRADE_SELF"])
    );
    // The bot is selected but has not streamed: nothing at all.
    let bot = app
        .world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .remove(&BOT)
        .unwrap();
    lua(&mut app, r#"InitiateTrade("target")"#);
    assert_eq!((sent(&mut app, &rx), errors(&mut app)), (0, vec![]));
    let fields =
        |pairs: &[(Field, u32)]| ObjectStore(wrathplus_protocol::ObjectFields::from_fields(pairs));
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(BOT, bot);
    let me = app
        .world_mut()
        .query_filtered::<Entity, With<SelfPlayer>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(me).insert(fields(&[
        (Field::UnitMaxHealth, 50),
        (Field::UnitHealth, 0),
    ]));
    lua(&mut app, r#"InitiateTrade("target")"#);
    assert_eq!(
        (sent(&mut app, &rx), errors(&mut app)),
        (0, vec!["ERR_PLAYER_DEAD"])
    );
    app.world_mut().entity_mut(me).insert(fields(&[
        (Field::UnitMaxHealth, 50),
        (Field::UnitHealth, 50),
    ]));
    lua(&mut app, r#"InitiateTrade("target")"#);
    assert_eq!(sent(&mut app, &rx), 1, "CMSG_INITIATE_TRADE");
    lua(&mut app, "CloseTrade()");
    assert_eq!(
        drained(&mut app, &rx).0,
        0x011C,
        "CMSG_CANCEL_TRADE with no window open"
    );
}
