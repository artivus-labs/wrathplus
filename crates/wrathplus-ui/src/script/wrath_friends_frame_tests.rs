use super::*;
use crate::script::UiScript;

fn vm() -> UiScript {
    let s = UiScript::new().unwrap();
    s.model_mut().unit_guids.player = 0x10;
    s
}

fn bank() -> WrathGuildBankView {
    WrathGuildBankView {
        money: 1_234_567,
        tabs: vec![
            WrathGuildBankTab {
                name: "Mats".into(),
                icon: "INV_Fabric_Linen_01".into(),
            },
            WrathGuildBankTab::default(),
        ],
        remaining_withdrawals: 3,
        rank_rights: vec![WrathGuildBankRankRights {
            money_per_day: 250_000,
            tabs: [(1 | 2, 10), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)],
        }],
        ..Default::default()
    }
}

#[test]
fn guild_bank_tabs_money_and_staged_rank_rights() {
    let mut s = vm();
    assert_eq!(s.eval::<i64>("return GetNumGuildBankTabs()").unwrap(), 0);
    s.run("GUILDBANK_TAB_NUMBER = 'Tab %d'").unwrap();
    s.set_wrath_guild_bank(bank());
    assert_eq!(s.eval::<i64>("return GetNumGuildBankTabs()").unwrap(), 2);
    assert_eq!(
        s.eval::<(String, String, Option<i64>, Option<i64>, i64, i64)>(
            "return GetGuildBankTabInfo(1)"
        )
        .unwrap(),
        (
            "Mats".into(),
            "Interface\\Icons\\INV_Fabric_Linen_01".into(),
            Some(1),
            Some(1),
            10,
            3
        )
    );
    // The guild master's unlimited item limit and allowance read -1, not 0 or 4294967295.
    {
        let mut m = s.model_mut();
        m.wrath_friends.bank.rank_rights[0].tabs[1].1 = u32::MAX;
        m.wrath_friends.bank.remaining_withdrawals = -1;
    }
    assert_eq!(
        s.eval::<(i64, i64)>("return select(5, GetGuildBankTabInfo(2))")
            .unwrap(),
        (-1, -1)
    );
    assert_eq!(
        s.eval::<String>("return (GetGuildBankTabInfo(1.5))")
            .unwrap(),
        "Mats",
        "truncated, not rounded (`0x5a6867`)"
    );
    assert_eq!(
        s.eval::<String>("return (GetGuildBankTabInfo(2))").unwrap(),
        "Tab 2"
    );
    assert_eq!(
        s.eval::<i64>("return select(5, GetGuildBankTabInfo(4))")
            .unwrap(),
        0
    );
    assert_eq!(
        s.eval::<f64>("return GetGuildBankMoney()").unwrap(),
        1_234_567.0
    );
    s.run("PickupGuildBankMoney(500) PickupGuildBankMoney(99999999)")
        .unwrap();
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![WrathGuildBankRequest::PickupMoney { copper: 500 }]
    );
    // Rank rights follow the rank being edited.
    s.model_mut().guild_control.rank = 1;
    assert_eq!(
        s.eval::<i64>("return GetGuildBankWithdrawLimit()").unwrap(),
        25
    );
    assert_eq!(
        s.eval::<(Option<i64>, Option<i64>, Option<i64>, i64)>(
            "return GetGuildBankTabPermissions(1)"
        )
        .unwrap(),
        (Some(1), Some(1), None, 10)
    );
    s.run(
        "SetGuildBankTabPermissions(1, 3, true) SetGuildBankTabPermissions(1, 1, false) \
         SetGuildBankTabWithdraw(1, 999999) SetGuildBankWithdrawLimit(5000000000)",
    )
    .unwrap();
    let staged = s.wrath_guild_bank_staged_rights(0).unwrap();
    assert_eq!(staged.tabs[0], (2 | 4, 100_000));
    assert_eq!(staged.money_per_day, 1_000_000_000);
    s.run("SetGuildBankWithdrawLimit(-5)").unwrap();
    let staged = s.wrath_guild_bank_staged_rights(0).unwrap();
    assert_eq!(
        staged.money_per_day, 1_000_000_000,
        "unsigned compare (`0x5cb6d9`)"
    );
    s.model_mut().wrath_friends.staged[0].money_per_day = u32::MAX;
    assert_eq!(
        s.eval::<i64>("return GetGuildBankWithdrawLimit()").unwrap(),
        -1,
        "the unlimited word is not divided (`0x5cb71e`)"
    );
}

#[test]
fn guild_events_coin_text_mutes_and_battle_net() {
    let mut s = vm();
    s.set_wrath_guild_events(vec![WrathGuildEvent {
        kind: 3,
        player1: Some("Ann".into()),
        player2: Some("Bob".into()),
        rank: Some("Officer".into()),
        seconds_ago: (26 * 3600) as u32,
    }]);
    assert_eq!(s.eval::<i64>("return GetNumGuildEvents()").unwrap(), 1);
    assert_eq!(
        s.eval::<(String, String, String, String, i64, i64, i64, i64)>(
            "return GetGuildEventInfo(1)"
        )
        .unwrap(),
        (
            "promote".into(),
            "Ann".into(),
            "Bob".into(),
            "Officer".into(),
            0,
            0,
            1,
            2
        )
    );
    s.run("GOLD_AMOUNT = '%d Gold' SILVER_AMOUNT = '%d Silver' COPPER_AMOUNT = '%d Copper'")
        .unwrap();
    assert_eq!(
        s.eval::<String>("return GetCoinText(1020304)").unwrap(),
        "102 Gold, 3 Silver, 4 Copper"
    );
    assert_eq!(
        s.eval::<String>("return GetCoinText(0, ' ')").unwrap(),
        "0 Copper"
    );
    assert_eq!(
        s.eval::<i64>("return GetNumMutes() + GetSelectedMute()")
            .unwrap(),
        0
    );
    s.run("UNKNOWN = 'Unknown'").unwrap();
    assert_eq!(
        s.eval::<String>("return GetMuteName(1)").unwrap(),
        "Unknown"
    );
    assert_eq!(
        s.eval::<i64>("return select('#', BNGetNumFOF(1))").unwrap(),
        0
    );
    s.run("SummonFriend('Cid')").unwrap();
    assert_eq!(
        s.take_wrath_guild_bank_requests(),
        vec![WrathGuildBankRequest::SummonFriend { name: "Cid".into() }]
    );
}
