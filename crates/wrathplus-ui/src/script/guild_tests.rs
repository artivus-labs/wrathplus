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

/// `0x4d20d0` refuses silently: a name outside 1 to 16 UTF-16 units (`0x65b250` counts them
/// plus the NUL), no rank loaded, or a rank past the count.
#[test]
fn saving_a_rank_refuses_what_the_reference_refuses() {
    let mut s = guild_with_ranks(5);
    let saves = |s: &mut crate::script::UiScript| {
        s.take_guild_requests()
            .into_iter()
            .filter(|r| matches!(r, GuildRequest::SaveRank { .. }))
            .count()
    };

    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank("")"#)
        .unwrap();
    assert_eq!(saves(&mut s), 0, "an empty name is a silent no-op");

    // Sixteen UTF-16 units is the ceiling; seventeen is refused.
    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank(string.rep("a", 16))"#)
        .unwrap();
    assert_eq!(saves(&mut s), 1);
    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank(string.rep("a", 17))"#)
        .unwrap();
    assert_eq!(saves(&mut s), 0, "seventeen units");
    // Units, not bytes: sixteen two-byte letters are sixteen units.
    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank(string.rep("é", 16))"#)
        .unwrap();
    assert_eq!(saves(&mut s), 1, "32 bytes, 16 units");
    // Whitespace is not empty to the reference.
    s.run(r#"GuildControlSetRank(2); GuildControlSaveRank(" ")"#)
        .unwrap();
    assert_eq!(saves(&mut s), 1);

    // No rank loaded in a fresh popup (the selection persists across saves, as `[0x84966c]`
    // does), or one past the count.
    let mut fresh = guild_with_ranks(5);
    fresh.run(r#"GuildControlSaveRank("Officer")"#).unwrap();
    assert_eq!(saves(&mut fresh), 0, "nothing selected");
    s.run(r#"GuildControlSetRank(6); GuildControlSaveRank("Officer")"#)
        .unwrap();
    assert_eq!(saves(&mut s), 0, "rank index 5 >= 5 ranks");
}

/// `0x4d2210` runs the same name gate; a whitespace name passes it.
#[test]
fn adding_a_rank_runs_the_same_name_gate() {
    let mut s = guild_with_ranks(5);
    s.run(r#"GuildControlAddRank(""); GuildControlAddRank(string.rep("a", 17))"#)
        .unwrap();
    assert!(s.take_guild_requests().is_empty());
    s.run(r#"GuildControlAddRank(" ")"#).unwrap();
    assert_eq!(s.take_guild_requests().len(), 1);
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
