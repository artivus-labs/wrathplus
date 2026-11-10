use super::*;

#[test]
fn dnd_wins_over_afk() {
    use guild_presence::{AFK, DND, OFFLINE, ONLINE};
    assert_eq!(status_key(ONLINE | AFK | DND), Some("CHAT_FLAG_DND"));
    assert_eq!(status_key(ONLINE | DND), Some("CHAT_FLAG_DND"));
    assert_eq!(status_key(ONLINE | AFK), Some("CHAT_FLAG_AFK"));
    assert_eq!(status_key(ONLINE), None);
    assert_eq!(status_key(OFFLINE), None);
}

#[test]
fn online_is_the_whole_byte() {
    let member = |presence| GuildRosterMember {
        presence,
        ..Default::default()
    };
    assert!(member(guild_presence::ONLINE).is_online());
    assert!(member(guild_presence::AFK).is_online(), "no ONLINE bit set");
    assert!(member(0x80).is_online(), "an unknown bit reads as online");
    assert!(!member(guild_presence::OFFLINE).is_online());
}

#[test]
fn an_offline_row_keeps_its_level_class_and_zone() {
    let member = GuildRosterMember {
        guid: 5,
        presence: guild_presence::OFFLINE,
        name: "Kaplan".into(),
        rank_id: 1,
        level: 42,
        class: 8, // Mage
        zone: 12,
        last_online_days: 3.5,
        public_note: "alt".into(),
        officer_note: String::new(),
    };
    let row = row(&member, None, None, &|_| String::new());
    assert_eq!(row.info.level, 42);
    assert_eq!(row.info.class, "Mage");
    assert!(!row.info.online);
    assert_eq!(row.info.last_online.days, 3);
    assert_eq!(row.info.status, "");
    assert_eq!(row.guid, 5);
}

#[test]
fn an_online_row_reports_no_last_online() {
    let member = GuildRosterMember {
        presence: guild_presence::ONLINE,
        // Ignored for an online member, as in the reference (`0x4d14fd`).
        last_online_days: 9.0,
        ..Default::default()
    };
    assert_eq!(
        row(&member, None, None, &|_| String::new())
            .info
            .last_online,
        Default::default()
    );
}

#[test]
fn the_shown_order_maps_both_ways() {
    assert_eq!(index_of(&[11, 22, 33], 22), 2);
    assert_eq!(index_of(&[22, 11, 33], 22), 1, "same player, new row");
    assert_eq!(index_of(&[11, 33], 22), 0, "no longer listed");
    assert_eq!(index_of(&[11, 22], 0), 0, "nothing selected");
}

#[test]
fn an_over_long_rank_name_is_refused() {
    assert_eq!(
        capped_rank_name("Officer".into()).as_deref(),
        Some("Officer")
    );
    assert!(capped_rank_name("A".repeat(GUILD_RANK_MAX_LENGTH)).is_some());
    assert_eq!(
        capped_rank_name("A".repeat(GUILD_RANK_MAX_LENGTH + 1)),
        None
    );
}

/// `GuildControlSaveRank` sends the bank words of the rank it saves, the row the setters wrote
/// (`0x5cb25f`-`0x5cb2e5` and `0x5cb7cb`-`0x5cb7e1`, both `[0xad07a8]`): an officer's save must not
/// carry the guild master's row.
#[test]
fn a_rank_save_carries_the_saved_ranks_staged_bank_words() {
    use wrathplus_ui::script::WrathGuildBankView;
    let mut script = UiScript::new().unwrap();
    script.set_wrath_guild_bank(WrathGuildBankView {
        rank_rights: vec![
            WrathGuildBankRankRights {
                money_per_day: u32::MAX,
                tabs: [(0xff, u32::MAX); 6],
            },
            WrathGuildBankRankRights {
                money_per_day: 50_000,
                tabs: [(1, 10); 6],
            },
        ],
        ..Default::default()
    });
    script
        .run("GuildControlSetRank(2) SetGuildBankTabWithdraw(1, 7)")
        .unwrap();
    let bank = rank_bank(&script, &crate::ui_guild::NativeGuild::default(), 1);
    assert_eq!(bank.tabs[0], (1, 7), "rank index 1's staged edit");
}

#[test]
fn last_online_decomposes_largest_unit_first() {
    use wrathplus_ui::script::LastOnline;
    // Under an hour: every unit zero.
    assert_eq!(last_online(0.02), LastOnline::default());
    assert_eq!(last_online(0.0), LastOnline::default());
    // Just over an hour.
    assert_eq!(
        last_online(1.0 / 24.0 + 0.001),
        LastOnline {
            hours: 1,
            ..Default::default()
        }
    );
    // Just under and just over a day.
    assert_eq!(last_online(0.99).days, 0);
    assert_eq!(last_online(0.99).hours, 23);
    assert_eq!(last_online(1.01).days, 1);
    // Just under and just over a month (30 days).
    assert_eq!(last_online(29.9).months, 0);
    assert_eq!(last_online(29.9).days, 29);
    assert_eq!(
        last_online(30.0),
        LastOnline {
            months: 1,
            ..Default::default()
        }
    );
    // Just under and just over a year (365 days).
    assert_eq!(last_online(364.9).years, 0);
    assert_eq!(last_online(364.9).months, 12, "12 months, not 1 year");
    assert_eq!(
        last_online(365.0),
        LastOnline {
            years: 1,
            ..Default::default()
        }
    );
    // The full decomposition: 400.5 days is 1 year, 1 month, 5 days, 12 hours.
    assert_eq!(
        last_online(400.5),
        LastOnline {
            years: 1,
            months: 1,
            days: 5,
            hours: 12,
        }
    );
}
