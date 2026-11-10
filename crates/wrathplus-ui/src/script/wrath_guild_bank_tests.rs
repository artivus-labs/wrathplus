use super::*;
use crate::script::{UiScript, WrathGuildBankTab, WrathGuildBankView};

fn vm() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.model_mut().unit_guids.player = 0x10;
    s.set_wrath_guild_bank(WrathGuildBankView {
        tabs: vec![WrathGuildBankTab::default(), WrathGuildBankTab::default()],
        slots: vec![vec![WrathGuildBankSlot {
            slot: 4,
            item_id: 2589,
            count: 20,
            texture: Some("Interface\\Icons\\INV_Fabric_Linen_01".into()),
            link: Some("|Hitem:2589|h[Linen Cloth]|h".into()),
            ..Default::default()
        }]],
        money_log: vec![WrathGuildBankMoneyLog {
            kind: 5,
            player: Some("Ann".into()),
            copper: 300,
            seconds_ago: 9000 * 3600,
        }],
        tab_logs: vec![vec![WrathGuildBankItemLog {
            kind: 3,
            player: Some("Bob".into()),
            link: None,
            count: 2,
            other_tab: 2,
            seconds_ago: 25 * 3600,
        }]],
        tab_text: vec!["hello".into()],
        withdraw_money_remaining: 5000,
        ..Default::default()
    });
    s
}

#[test]
fn slots_links_and_tab_selection() {
    let s = vm();
    assert_eq!(
        s.eval::<(String, i64, bool)>(
            "local t, c, l = GetGuildBankItemInfo(1, 5) return t, c, l == nil"
        )
        .unwrap(),
        ("Interface\\Icons\\INV_Fabric_Linen_01".into(), 20, true)
    );
    assert_eq!(
        s.eval::<(bool, i64)>("local t, c = GetGuildBankItemInfo(1, 6) return t == nil, c")
            .unwrap(),
        (true, 0)
    );
    assert_eq!(
        s.eval::<String>("return GetGuildBankItemLink(1, 5)")
            .unwrap(),
        "|Hitem:2589|h[Linen Cloth]|h"
    );
    assert_eq!(
        s.eval::<i64>("return select('#', GetGuildBankItemLink(1, 6))")
            .unwrap(),
        0
    );
    s.run("SetCurrentGuildBankTab(2) SetCurrentGuildBankTab(9)")
        .unwrap();
    assert_eq!(s.eval::<i64>("return GetCurrentGuildBankTab()").unwrap(), 2);
    assert!(s.run("GetGuildBankItemInfo(1)").is_err());
}

#[test]
fn logs_text_and_gestures() {
    let mut s = vm();
    // `QueryGuildBankText` raises the log's usage string (`0x5a7129`).
    let err = s.run("QueryGuildBankText()").unwrap_err().to_string();
    assert!(err.contains("Usage: QueryGuildBankLog(tab)"), "{err}");
    assert_eq!(
        s.eval::<(String, String, i64, i64, i64, i64, i64)>(
            "return GetGuildBankMoneyTransaction(1)"
        )
        .unwrap(),
        ("withdraw".into(), "Ann".into(), 300, 1, 0, 10, 0)
    );
    assert_eq!(
        s.eval::<(String, String, String, i64, i64, i64, i64)>(
            "local k, n, l, c, a, b, y, m, d, h = GetGuildBankTransaction(1, 1) return k, n, l, c, a, b, d"
        )
        .unwrap(),
        ("move".into(), "Bob".into(), "".into(), 2, 1, 2, 1)
    );
    assert_eq!(
        s.eval::<String>("return GetGuildBankText(1)").unwrap(),
        "hello"
    );
    assert_eq!(s.eval::<String>("return GetGuildBankText(2)").unwrap(), "");
    assert!(s.eval::<bool>("return GetGuildBankText(3) == nil").unwrap());
    assert_eq!(
        s.eval::<f64>("return GetGuildBankWithdrawMoney()").unwrap(),
        5000.0
    );
    s.run(
        "QueryGuildBankTab(1) AutoStoreGuildBankItem(2, 1) SetGuildBankText(1, 'x') \
         SetGuildBankTabInfo(2, 'Mats', 'INV_Box_01') QueryGuildBankLog(7) CloseGuildBankFrame()",
    )
    .unwrap();
    use WrathGuildBankRequest as R;
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![
            R::QueryTab { tab: 1 },
            R::AutoStoreItem { tab: 2, slot: 1 },
            R::SetText {
                tab: 1,
                text: "x".into()
            },
            R::SetTabInfo {
                tab: 2,
                name: "Mats".into(),
                icon: "INV_Box_01".into()
            },
            R::QueryLog { tab: 7 },
            R::Close,
        ]
    );
}

#[test]
fn bank_items_ride_the_cursor_as_mode_11() {
    use crate::script::CursorPayload;
    use WrathGuildBankRequest as R;
    let mut s = vm();
    // An empty cursor takes slot 5 locally: locked while held, reported as an item, not by
    // CursorHasItem.
    s.run("PickupGuildBankItem(1, 5)").unwrap();
    assert!(s.take_wrath_guild_bank_requests().is_empty());
    assert!(s
        .eval::<bool>(
            "local t, c, l = GetGuildBankItemInfo(1, 5) local k, id = GetCursorInfo() \
             return l == 1 and k == 'item' and id == 2589 and not CursorHasItem()"
        )
        .unwrap());
    // Without deposit rights on tab 2 the move is refused and the item stays held.
    s.run("PickupGuildBankItem(2, 1)").unwrap();
    assert!(s.take_wrath_guild_bank_requests().is_empty());
    assert!(matches!(
        s.cursor_payload(),
        Some(CursorPayload::GuildBankItem(_))
    ));
    {
        let mut m = s.model_mut();
        m.wrath_friends.bank.rank_rights = vec![crate::script::WrathGuildBankRankRights {
            tabs: [(3, 0); 6],
            ..Default::default()
        }];
    }
    s.run("PickupGuildBankItem(2, 1)").unwrap();
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![R::MoveItem {
            from_tab: 1,
            from_slot: 5,
            to_tab: 2,
            to_slot: 1,
            count: None
        }]
    );
    assert!(s.cursor_payload().is_none());
    // A split of 7 drops on a bag slot as a withdraw; the whole stack splits as a pickup.
    s.run("SplitGuildBankItem(1, 5, 7) PickupContainerItem(0, 3)")
        .unwrap();
    s.run("SplitGuildBankItem(1, 5, 20) PickupInventoryItem(16)")
        .unwrap();
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![
            R::DropOnBag {
                tab: 1,
                slot: 5,
                bag: 0,
                bag_slot: 3,
                count: Some(7)
            },
            R::DropOnEquip {
                tab: 1,
                slot: 5,
                equip_slot: 16,
                count: None
            },
        ]
    );
    // A bag item on the cursor deposits: App reads it and clears.
    s.set_cursor_for_test(CursorPayload::Item(crate::script::CursorItem {
        bag: 0,
        slot: 1,
        item_id: 4,
        texture: None,
        link: None,
        count: None,
        quality: None,
        equip_slots: Vec::new(),
        bar_placeable: false,
    }));
    s.run("PickupGuildBankItem(1, 9)").unwrap();
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![R::PickupItem { tab: 1, slot: 9 }]
    );
}

#[test]
fn rank_rights_gate_withdraw_and_repair() {
    let s = vm();
    assert!(s
        .eval::<bool>("return CanWithdrawGuildBankMoney() == nil and CanGuildBankRepair() == nil")
        .unwrap());
    {
        let mut m = s.model_mut();
        m.guild.in_guild = true;
        m.guild.rank_index = 0;
        m.guild.rights = 0x80000;
        m.wrath_friends.bank.rank_rights = vec![crate::script::WrathGuildBankRankRights {
            money_per_day: 1000,
            ..Default::default()
        }];
    }
    assert!(s
        .eval::<bool>("return CanWithdrawGuildBankMoney() == 1 and CanGuildBankRepair() == nil")
        .unwrap());
    // Tab text editing is the own rank's flag 0x4 on that tab (`0x5cc325`), not guild master.
    s.model_mut().wrath_friends.bank.rank_rights[0].tabs[1] = (4, 0);
    assert!(s
        .eval::<bool>(
            "return CanEditGuildTabInfo(2) == 1 and CanEditGuildTabInfo(2.9) == 1 \
             and CanEditGuildTabInfo(1) == nil and CanEditGuildTabInfo(7) == nil"
        )
        .unwrap());
    let err = s.run("CanEditGuildTabInfo()").unwrap_err().to_string();
    assert!(err.contains("Usage: CanEditGuildTabInfo(tab)"), "{err}");
    // Repair tests its right alone (`0x5cc23a`): no money word needed.
    {
        let mut m = s.model_mut();
        m.guild.rights = 0x40000;
        m.wrath_friends.bank.rank_rights[0].money_per_day = 0;
    }
    assert!(s
        .eval::<bool>("return CanWithdrawGuildBankMoney() == nil and CanGuildBankRepair() == 1")
        .unwrap());
}

#[test]
fn the_bank_item_tooltip_shows_the_slots_link() {
    let s = vm();
    s.run(
        r#"
        local tip = CreateFrame("GameTooltip", "BankTip", UIParent)
        local shown = {}
        tip.SetHyperlink = function(_, link) shown[#shown + 1] = link end
        tip:SetGuildBankItem(1, 5)
        tip:SetGuildBankItem(1, 6)
        tip:SetGuildBankItem(7, 5)
        assert(#shown == 1 and shown[1] == "|Hitem:2589|h[Linen Cloth]|h", table.concat(shown, ","))
        assert(not pcall(tip.SetGuildBankItem, tip, 1))
    "#,
    )
    .unwrap();
}

#[test]
fn the_item_icon_list_answers_paths_and_empty_strings() {
    let mut s = vm();
    assert_eq!(s.eval::<i64>("return GetNumMacroItemIcons()").unwrap(), 0);
    s.set_macro_item_icons(vec!["INV_Misc_Bag_07".into(), String::new()]);
    assert!(s
        .eval::<bool>(
            "return GetNumMacroItemIcons() == 2 \
             and GetMacroItemIconInfo(1) == 'Interface\\\\Icons\\\\INV_Misc_Bag_07' \
             and GetMacroItemIconInfo(2) == '' and GetMacroItemIconInfo(3) == '' \
             and GetMacroItemIconInfo(0) == ''"
        )
        .unwrap());
    assert!(s.run("GetMacroItemIconInfo()").is_err());
}
