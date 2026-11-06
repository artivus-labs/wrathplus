//! `UseContainerItem`'s window ladder (`0x5d8650`) and its arguments.

use super::*;
use crate::script::{AuctionState, MailState, TradeState, WrathGuildBankView};

/// With the trade window open a right-click offers the bag item in our first free traded slot,
/// skipping one queued this frame; a bound item goes to the seventh, "will not be traded" slot
/// (`0x5877b0`), and once that is taken it stays put. Closed, the click uses.
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
    bag.slots.insert(7, item(6948, 0x1));
    s.set_container(0, Some(bag));
    s.set_trade(Some(TradeState::default()));
    for slot in [1, 5, 6, 7] {
        s.run(&format!("UseContainerItem(0, {slot})")).unwrap();
    }
    assert_eq!(
        s.take_trade_set_items(),
        vec![(1, 0, 1), (2, 0, 5), (7, 0, 6)]
    );
    assert!(
        s.take_container_uses().is_empty(),
        "nothing is used while trading"
    );

    s.set_trade(None);
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)]);
}

/// The first open window takes the click, in 12340's order: a vendor or the bank (App's arms),
/// the Send Mail tab, the Auctions tab, a trade, the guild vault, else the use. The mail and
/// auction arms need their tab showing (`0xbeb1b9`, `0xc0f3f5`), not just the window.
#[test]
fn a_right_click_goes_to_the_first_open_window_in_the_reference_order() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_trade(Some(TradeState::default()));
    s.set_merchant(Some(MerchantState::default()));
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)], "the sale wins");
    assert!(s.take_trade_set_items().is_empty());
    s.set_merchant(None);

    s.set_mail(Some(MailState::default()));
    s.set_auction(Some(AuctionState::default()));
    s.run("SetSendMailShowing(true) SetAuctionsTabShowing(true) UseContainerItem(0, 1)")
        .unwrap();
    let attached = s.eval::<String>("return (GetSendMailItem(1))").unwrap();
    assert_eq!(attached, "Tough Jerky", "attached to the first send slot");
    assert_eq!(s.auction_sell_item(), None);

    // The inbox tab showing: the Auctions tab takes it, replacing the staged item.
    s.run("SetSendMailShowing(false) UseContainerItem(0, 4) UseContainerItem(0, 3)")
        .unwrap();
    assert_eq!(s.auction_sell_item(), Some((0, 3)));
    // The Browse tab showing: the trade takes it.
    s.run("SetAuctionsTabShowing(false) UseContainerItem(0, 4)")
        .unwrap();
    assert_eq!(s.take_trade_set_items(), vec![(1, 0, 4)]);
    s.set_trade(None);
    s.set_mail(None);
    s.reset_compose_tab_silently();
    s.set_auction(None);

    // The guild vault deposits into the current tab's first free slot (`0x5a56b0`).
    s.set_wrath_guild_bank(WrathGuildBankView {
        money: 1,
        ..Default::default()
    });
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_guild_bank_deposits(), vec![(0, 0, 1)]);
    s.set_wrath_guild_bank(WrathGuildBankView::default());
    s.run("UseContainerItem(0, 1)").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)], "closed, it uses");
    assert!(s.take_guild_bank_deposits().is_empty());
}

/// The target token resolves before the cursor clear (`0x60abf0` with flag 1): none is the
/// selection, one that names nothing ends the call. A locked or absent item does nothing after
/// the clear; the numbers truncate through `__ftol2_sse`, and a non-number raises the Usage line.
#[test]
fn use_container_item_reads_its_target_and_numbers_first() {
    let mut s = UiScript::new().unwrap();
    let mut bag = backpack();
    let locked = ContainerSlot {
        item_id: 117,
        locked: true,
        ..Default::default()
    };
    bag.slots.insert(8, locked);
    s.set_container(0, Some(bag));
    s.run("PickupContainerItem(0, 4) UseContainerItem(0, 1, 'nobody-here')")
        .unwrap();
    assert!(
        s.cursor_item().is_some(),
        "an unresolved token returns first"
    );
    // A bank bag with the banker away resolves no container (`0x5d7380`), before the clear.
    s.set_container(5, Some(backpack()));
    s.run("UseContainerItem(5, 1)").unwrap();
    assert!(s.cursor_item().is_some() && s.take_container_uses().is_empty());
    s.run("UseContainerItem(0, 8) UseContainerItem(0, 9)")
        .unwrap();
    assert!(s.cursor_item().is_none(), "the clear ran");
    assert!(s.take_container_uses().is_empty(), "locked, then empty");
    s.run("UseContainerItem('0', 1.9)").unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)], "truncated");
    // `__ftol2_sse` truncates toward zero: -0.5 is bag 0, not -1, the bank's own slots. NaN and
    // 1e300 read as `INT_MIN`, no bag, so the call ends silently.
    s.set_container(-1, Some(backpack()));
    s.run("UseContainerItem(-0.5, 1) UseContainerItem(0/0, 1) UseContainerItem(1e300, 1)")
        .unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)]);
    let err = s.run("UseContainerItem(0)").unwrap_err().to_string();
    assert!(err.contains("Usage: UseContainerItem(index, slot[, target])"));
}

/// Exercise the production Lua verbs and the production send-intent drain, including an App
/// snapshot replacement between clicks. The wire layer resolves exactly these source pairs.
#[test]
fn mail_source_is_reserved_until_remove_or_reset_even_across_container_refresh() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_mail(Some(MailState::default()));
    s.set_mail_stationeries(vec![crate::script::StationeryView {
        id: 41,
        name: "Default".into(),
        icon: String::new(),
        cost: Some(0),
        texture: String::new(),
    }]);
    s.run("SetSendMailShowing(true) SelectStationery(1) UseContainerItem(0, 1) UseContainerItem(0, 1)").unwrap();
    s.set_container(0, Some(backpack()));
    s.run("UseContainerItem(0, 1) PickupContainerItem(0, 1)")
        .unwrap();
    assert!(s.cursor_item().is_none());
    assert!(!s
        .eval::<bool>("return SplitContainerItem(0, 1, 2)")
        .unwrap());
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == 1")
        .unwrap());
    assert!(s.eval::<bool>("return GetSendMailItem(2) == nil").unwrap());
    // A held item cannot swap into the reserved source either.
    s.run("PickupContainerItem(0, 4) PickupContainerItem(0, 1)")
        .unwrap();
    assert_eq!(s.cursor_item().unwrap().slot, 4);
    assert!(s.take_container_moves().is_empty());
    s.run("ClearCursor() SendMail('Alt', 'one stack', '')")
        .unwrap();
    assert_eq!(s.take_mail_send().unwrap().items, vec![(0, 0, 1)]);
    assert!(s.take_container_uses().is_empty());

    s.run("ClickSendMailItemButton(1, true)").unwrap();
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == nil")
        .unwrap());
    s.run("UseContainerItem(0, 1)").unwrap();
    s.reset_compose_tab();
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == nil")
        .unwrap());
    s.run("UseContainerItem(0, 1)").unwrap();
    s.reset_compose_tab_silently();
    s.run("SetSendMailShowing(false) UseContainerItem(0, 1) PickupContainerItem(0, 1)")
        .unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)]);
    assert!(s.cursor_item().is_some());
}

#[test]
fn mail_cursor_drop_replacement_and_detach_transfer_the_source_reservation() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_mail(Some(MailState::default()));
    s.run("SetSendMailShowing(true) UseContainerItem(0, 1) PickupContainerItem(0, 4) ClickSendMailItemButton(1)").unwrap();
    assert_eq!(s.cursor_item().unwrap().slot, 1);
    s.run("ClearCursor()").unwrap();
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 4) return locked == 1")
        .unwrap());
    s.run("UseContainerItem(0, 4) ClickSendMailItemButton(1) ClearCursor()")
        .unwrap();
    assert!(s
        .eval::<bool>("return GetSendMailItem(1) == nil and GetSendMailItem(2) == nil")
        .unwrap());
    s.run("PickupContainerItem(0, 4) ClickSendMailItemButton(2)")
        .unwrap();
    s.drop_send_mail_item(1);
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 4) return locked == nil")
        .unwrap());
    s.run("UseContainerItem(0, 4)").unwrap();
    s.set_wrath_mail_send_locked(true);
    s.run("RespondMailLockSendItem(1, false)").unwrap();
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 4) return locked == nil")
        .unwrap());
}

/// The same eligibility decision precedes both right-click and cursor-drop mutations. Instance
/// binding alone must not reject the template's account-bound exception (`0x56fff0`).
#[test]
fn mail_attachment_gate_rejects_bound_and_conjured_but_preserves_account_bound() {
    for cursor_drop in [false, true] {
        for (flags, already_bound, template_flags, bonding, error) in [
            (1, false, 0, 1, Some("ERR_MAIL_BOUND_ITEM")),
            (0, true, 0, 1, Some("ERR_MAIL_BOUND_ITEM")),
            (1, false, 0x0800_0000, 4, Some("ERR_MAIL_QUEST_ITEM")),
            (0, false, 2, 0, Some("ERR_MAIL_CONJURED_ITEM")),
            (1, false, 0x0800_0000, 1, None),
        ] {
            let mut s = UiScript::new().unwrap();
            let mut bag = backpack();
            let source = bag.slots.get_mut(&1).unwrap();
            source.flags = flags;
            source.already_bound = already_bound;
            // Unlike the generic in-flight letter fixture, this attachment has its name cached.
            bag.slots.get_mut(&4).unwrap().link =
                Some("|cffffffff|Hitem:8383|h[Plain Letter]|h|r".into());
            s.set_container(0, Some(bag));
            s.set_item_template(
                117,
                crate::script::ItemTemplateView {
                    name: "Tough Jerky".into(),
                    flags: template_flags,
                    bonding,
                    ..Default::default()
                },
            );
            s.set_mail(Some(MailState::default()));
            s.run("SetSendMailShowing(true) UseContainerItem(0, 4)")
                .unwrap();
            if cursor_drop {
                s.run("PickupContainerItem(0, 1) ClickSendMailItemButton(2)")
                    .unwrap();
            } else {
                s.run("UseContainerItem(0, 1)").unwrap();
            }
            assert_eq!(s.take_ui_errors(), error.into_iter().collect::<Vec<_>>());
            assert_eq!(
                s.eval::<bool>("return GetSendMailItem(2) == nil").unwrap(),
                error.is_some()
            );
            assert!(s.eval::<bool>("return GetSendMailItem(1) ~= nil").unwrap());
            assert_eq!(s.cursor_item().is_some(), cursor_drop && error.is_some());
        }
    }
}

#[test]
fn pending_mail_confirmation_blocks_both_staging_paths_and_reset_clears_it() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_mail(Some(MailState::default()));
    s.set_wrath_mail_send_locked(true);
    s.run("SetSendMailShowing(true) UseContainerItem(0, 1) PickupContainerItem(0, 4) ClickSendMailItemButton(1)").unwrap();
    assert_eq!(
        s.take_ui_errors(),
        vec!["ERR_CLIENT_LOCKED_OUT", "ERR_CLIENT_LOCKED_OUT"]
    );
    assert!(s.eval::<bool>("return GetSendMailItem(1) == nil").unwrap());
    assert_eq!(s.cursor_item().unwrap().slot, 4);
    s.reset_compose_tab_silently();
    s.run("ClearCursor() UseContainerItem(0, 1)").unwrap();
    assert!(s.eval::<bool>("return GetSendMailItem(1) ~= nil").unwrap());
}

/// The sell slot owns its source lock across App snapshots, just like compose. Replacing or
/// clearing it releases only that reservation; taking it transfers the lock to the cursor.
#[test]
fn auction_source_reservation_survives_refresh_and_releases_on_replace_take_and_clear() {
    let mut s = UiScript::new().unwrap();
    s.set_container(0, Some(backpack()));
    s.set_auction(Some(AuctionState::default()));
    s.run("SetAuctionsTabShowing(true) UseContainerItem(0, 1)")
        .unwrap();
    s.set_container(0, Some(backpack()));
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == 1")
        .unwrap());
    s.run("UseContainerItem(0, 1) PickupContainerItem(0, 1)")
        .unwrap();
    assert!(s.cursor_item().is_none());
    assert!(!s
        .eval::<bool>("return SplitContainerItem(0, 1, 2)")
        .unwrap());
    s.run("PickupContainerItem(0, 4) PickupContainerItem(0, 1)")
        .unwrap();
    assert_eq!(s.cursor_item().unwrap().slot, 4);
    assert!(s.take_container_moves().is_empty());
    assert!(s.take_container_uses().is_empty());
    assert_eq!(s.auction_sell_item(), Some((0, 1)));
    {
        // Item reservation blocks bag verbs, but is not itself a mail eligibility error.
        let mut model = s.model_mut();
        let staged = model.auction_sell_item.as_ref().unwrap().clone();
        assert!(crate::script::mail::can_attach(&mut model, &staged));
    }

    s.run("ClearCursor() UseContainerItem(0, 4)").unwrap();
    assert_eq!(s.auction_sell_item(), Some((0, 4)));
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == nil")
        .unwrap());
    s.run("ClickAuctionSellItemButton()").unwrap();
    assert_eq!(s.auction_sell_item(), None);
    assert_eq!(s.cursor_item().unwrap().slot, 4);
    s.run("ClearCursor() UseContainerItem(0, 1)").unwrap();
    s.clear_auction_sell_item();
    assert!(s
        .eval::<bool>("local _, _, locked = GetContainerItemInfo(0, 1) return locked == nil")
        .unwrap());
    s.run("SetAuctionsTabShowing(false) UseContainerItem(0, 1) PickupContainerItem(0, 1)")
        .unwrap();
    assert_eq!(s.take_container_uses(), vec![(0, 1)]);
    assert_eq!(s.cursor_item().unwrap().slot, 1);
}
