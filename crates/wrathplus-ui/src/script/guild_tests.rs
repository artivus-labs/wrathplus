use super::*;

/// `0x4c9330`: only an unrecognised token raises; `"player"` with no unit behind it, the paper
/// doll's case at world entry, answers `nil, nil, 0`.
#[test]
fn a_recognised_but_unresolved_unit_answers_nils_and_only_an_unknown_token_raises() {
    let s = crate::script::UiScript::new().unwrap();
    let (name, rank, idx): (Option<String>, Option<String>, i64) = s
        .eval(r#"return GetGuildInfo("player")"#)
        .expect("a recognised token must not raise");
    assert_eq!((name, rank, idx), (None, None, 0));
    // The third value is the number 0, not nil (`mov eax,3` on both legs).
    assert!(s
        .eval::<bool>(r#"local a,b,c = GetGuildInfo("party3") return c == 0"#)
        .unwrap());
    assert!(s
        .eval::<i64>(r#"return GetGuildInfo("nosuchunit")"#)
        .is_err());
    // A non-string raises its usage (`0x5eab09`); WrathPlus answered the nil triple.
    let err = s.run("GetGuildInfo()").unwrap_err().to_string();
    assert!(err.contains(r#"Usage: GetGuildInfo("unit")"#), "{err}");
}

#[test]
fn the_rank_right_table_is_not_a_shift() {
    assert_eq!(RANK_RIGHT_BITS.len(), 17, "one bit per GUILDCONTROL_OPTION");
    assert_eq!(RANK_RIGHT_BITS[right::PROMOTE - 1], 0x80);
    assert_eq!(RANK_RIGHT_BITS[right::DEMOTE - 1], 0x100);
    assert_eq!(RANK_RIGHT_BITS[right::INVITE - 1], 0x10);
    assert_eq!(RANK_RIGHT_BITS[right::REMOVE - 1], 0x20);
    assert!(
        RANK_RIGHT_BITS[right::PROMOTE - 1] > RANK_RIGHT_BITS[right::INVITE - 1],
        "promote's bit is above invite's while its index is below — the whole reason this is \
         a table and not a shift"
    );
    for (i, bit) in RANK_RIGHT_BITS.iter().enumerate() {
        assert_eq!(bit.count_ones(), 1, "flag {} is not a single bit", i + 1);
    }
    let mut seen = 0u32;
    for bit in RANK_RIGHT_BITS {
        assert_eq!(seen & bit, 0, "duplicate bit {bit:#x}");
        seen |= bit;
    }
}

/// `0x40` is none of the seventeen and never tested by the reference.
#[test]
fn saving_a_rank_preserves_bits_outside_the_seventeen_and_can_clear() {
    // Live: Invite (0x10) + Set MOTD (0x1000), plus the untouchable 0x40.
    let live = 0x0000_0040 | RANK_RIGHT_BITS[right::INVITE - 1] | RANK_RIGHT_BITS[8];
    // Staged: the user unticked Set MOTD and ticked Promote.
    let mut staged = [false; RANK_RIGHT_BITS.len()];
    staged[right::INVITE - 1] = true;
    staged[right::PROMOTE - 1] = true;

    let mut folded = live;
    for (i, bit) in RANK_RIGHT_BITS.iter().enumerate() {
        if staged[i] {
            folded |= bit;
        } else {
            folded &= !bit;
        }
    }

    assert_eq!(
        folded & 0x0000_0040,
        0x0000_0040,
        "0x40 is outside the seventeen and must ride through untouched"
    );
    assert_ne!(folded & RANK_RIGHT_BITS[right::PROMOTE - 1], 0, "ticked");
    assert_eq!(
        folded & RANK_RIGHT_BITS[8],
        0,
        "unticked Set MOTD must actually clear — a plain OR could never do this"
    );
}

fn guild_with_ranks(n: usize) -> crate::script::UiScript {
    let mut s = crate::script::UiScript::new().unwrap();
    s.set_guild(GuildState {
        in_guild: true,
        ranks: (0..n)
            .map(|i| GuildRankInfo {
                name: format!("Rank{i}"),
                rights: 0,
            })
            .collect(),
        ..Default::default()
    });
    s
}

/// `0x5cb170` is a silent no-op for an empty name, no rank loaded, or a rank past the count;
/// every other name is sanitised by `0x7e1830` and sent, never refused for its length.
#[test]
fn saving_a_rank_refuses_what_the_reference_refuses() {
    let mut s = guild_with_ranks(5);
    let saves = |s: &mut crate::script::UiScript| {
        s.take_guild_requests()
            .into_iter()
            .filter_map(|r| match r {
                GuildRequest::SaveRank { name, .. } => Some(name),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank("")"#)
        .unwrap();
    assert!(saves(&mut s).is_empty(), "an empty name is a silent no-op");

    // Fifteen characters are kept, not bytes; `|` is dropped and a line break ends the name.
    for (lua, sent) in [
        (r#"string.rep("a", 17)"#, "a".repeat(15)),
        (r#"string.rep("é", 16)"#, "é".repeat(15)),
        (
            r#""|cffffffffOfficer|r""#,
            "cffffffffOfficer"[..15].to_string(),
        ),
        (r#""Raid\nLead""#, "Raid".to_string()),
        (r#"" ""#, " ".to_string()),
        (r#""||""#, String::new()),
    ] {
        s.run(&format!(
            "GuildControlSetRank(2); GuildControlSaveRank({lua})"
        ))
        .unwrap();
        assert_eq!(saves(&mut s), [sent], "{lua}");
    }
    let err = s.run("GuildControlSaveRank()").unwrap_err().to_string();
    assert!(err.contains("Usage: GuildControlSaveRank(name)"), "{err}");

    // No rank loaded in a fresh popup (the selection persists across saves, as `[0x84966c]`
    // does), or one past the count.
    let mut fresh = guild_with_ranks(5);
    fresh.run(r#"GuildControlSaveRank("Officer")"#).unwrap();
    assert!(saves(&mut fresh).is_empty(), "nothing selected");
    s.run(r#"GuildControlSetRank(6); GuildControlSaveRank("Officer")"#)
        .unwrap();
    assert!(saves(&mut s).is_empty(), "rank index 5 >= 5 ranks");
}

/// `0x5cb330` runs the same name rule; a whitespace name passes it.
#[test]
fn adding_a_rank_runs_the_same_name_rule() {
    let mut s = guild_with_ranks(5);
    s.run(r#"GuildControlAddRank(""); GuildControlAddRank(string.rep("a", 17))"#)
        .unwrap();
    assert_eq!(
        s.take_guild_requests(),
        [GuildRequest::AddRank("a".repeat(15))]
    );
    s.run(r#"GuildControlAddRank(" ")"#).unwrap();
    assert_eq!(s.take_guild_requests().len(), 1);
}

/// `0x5caf40`: an index past the roster clears the selection rather than taking the last row.
#[test]
fn a_selection_past_the_roster_clears_it() {
    let mut s = crate::script::UiScript::new().unwrap();
    s.set_guild(GuildState {
        roster: vec![GuildMemberInfo::default(), GuildMemberInfo::default()],
        ..Default::default()
    });
    s.run("SetGuildRosterSelection(2)").unwrap();
    assert_eq!(
        s.eval::<i64>("return GetGuildRosterSelection()").unwrap(),
        2
    );
    s.run("SetGuildRosterSelection(3)").unwrap();
    assert_eq!(
        s.eval::<i64>("return GetGuildRosterSelection()").unwrap(),
        0
    );
    assert_eq!(
        s.take_guild_requests(),
        [GuildRequest::Select(2), GuildRequest::Select(0)]
    );
}

/// `0x5cc9c0` truncates its index (`fistp` under `or 0xc00`), raises its usage on a non-number and
/// answers nil for a class or zone with no DBC row.
#[test]
fn roster_indices_truncate_and_missing_names_are_nil() {
    let mut s = crate::script::UiScript::new().unwrap();
    s.set_guild(GuildState {
        roster: vec![GuildMemberInfo {
            name: "Ann".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "local n, _, _, _, class, zone = GetGuildRosterInfo(1.9) \
             return n == 'Ann' and class == nil and zone == nil"
        )
        .unwrap());
    let err = s.run("GetGuildRosterInfo()").unwrap_err().to_string();
    assert!(err.contains("Usage: GetGuildRosterInfo(index)"), "{err}");
}

/// `GuildStatus_Update` passes 0 before it tests the selection.
#[test]
fn member_lookup_is_one_based_and_tolerates_zero() {
    let roster = vec![
        GuildMemberInfo {
            name: "Alice".into(),
            ..Default::default()
        },
        GuildMemberInfo {
            name: "Bob".into(),
            ..Default::default()
        },
    ];
    assert_eq!(
        member_at(&roster, 1).map(|m| m.name.as_str()),
        Some("Alice")
    );
    assert_eq!(member_at(&roster, 2).map(|m| m.name.as_str()), Some("Bob"));
    assert!(member_at(&roster, 0).is_none(), "0 = nothing selected");
    assert!(member_at(&roster, 3).is_none(), "past the end");
    assert!(member_at(&roster, -1).is_none(), "negative");
}

/// `guildShowOffline` (`0x51e1e2`, default "1"): the getter reads it (`0x5ca1e0`), the setter
/// writes it (`0x5ca1b0`: no argument is on, nil off), and `GetNumGuildMembers` (`0x5ca130`) counts
/// the whole roster or the online tally by it.
#[test]
fn show_offline_is_the_guild_show_offline_cvar() {
    let mut s = crate::script::UiScript::new().unwrap();
    s.register_cvars([("guildShowOffline", "1")]);
    let member = |online| GuildMemberInfo {
        online,
        ..Default::default()
    };
    s.set_guild(GuildState {
        in_guild: true,
        roster: vec![member(true), member(false)],
        ..Default::default()
    });
    let read = |s: &crate::script::UiScript| {
        s.eval::<(Option<i64>, i64)>("return GetGuildRosterShowOffline(), GetNumGuildMembers()")
            .unwrap()
    };
    assert_eq!(read(&s), (Some(1), 2), "on by default");
    s.run("SetGuildRosterShowOffline(nil)").unwrap();
    assert_eq!(s.cvar("guildShowOffline").as_deref(), Some("0"));
    assert_eq!(read(&s), (None, 1));
    s.run("SetGuildRosterShowOffline()").unwrap();
    assert_eq!(s.cvar("guildShowOffline").as_deref(), Some("1"));
    s.run("SetGuildRosterShowOffline('0')").unwrap();
    assert_eq!(read(&s), (None, 1), "a \"0\" string is off");
    // The optional argument counts the offline members too (`0x5ca13d`).
    assert_eq!(s.eval::<i64>("return GetNumGuildMembers(true)").unwrap(), 2);
    assert_eq!(s.eval::<i64>("return GetNumGuildMembers(nil)").unwrap(), 1);
}
