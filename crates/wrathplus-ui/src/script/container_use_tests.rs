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
