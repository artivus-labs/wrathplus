//! The joined-channel verbs `GetChannelName` and `GetChannelList` ([`crate::script::channel`])
//! and the guild-recruitment latch.

use super::common::script;

/// Two channels in join order, which is the numbering; deliberately not sorted.
fn joined() -> crate::script::UiScript {
    let mut s = script();
    let joined = |name: &str, id| {
        Some(crate::script::JoinedChannel {
            name: name.into(),
            id,
            confirmed: true,
        })
    };
    s.set_joined_channels(vec![joined("World", 0), joined("Trade - City", 2)]);
    s
}

/// The slot is the 1-based position in join order, not a DBC id or an alphabetical rank.
#[test]
fn a_joined_channel_answers_its_slot_name_and_instance() {
    let s = joined();
    let (id, name, instance): (i64, String, i64) = s.eval("return GetChannelName(2)").unwrap();
    assert_eq!(id, 2, "the 1-based slot in join order");
    assert_eq!(name, "Trade - City");
    assert_eq!(
        instance, 0,
        "instanceID is 0 on every vanilla emulator, and a NUMBER so a caller can compare it"
    );
}

/// Addons look up by name in any case: `_LazyPig` passes `"world"` for the server's `"World"`.
#[test]
fn the_name_form_resolves_case_insensitively_to_the_same_slot() {
    let s = joined();
    let by_name: i64 = s.eval("return GetChannelName('world')").unwrap();
    let by_index: i64 = s.eval("return GetChannelName(1)").unwrap();
    assert_eq!(by_name, 1);
    assert_eq!(by_name, by_index, "both directions must agree on the slot");

    let name: String = s
        .eval("local _, n = GetChannelName('TRADE - CITY') return n")
        .unwrap();
    assert_eq!(
        name, "Trade - City",
        "the answer is the JOINED spelling, not the caller's"
    );
}

/// Stock callers compare the first return numerically (`ChatFrame.lua:2114`, `:2232`).
#[test]
fn a_channel_that_is_not_joined_answers_the_number_zero_never_nil() {
    let s = joined();

    let id: i64 = s.eval("return GetChannelName('NoSuchChannel')").unwrap();
    assert_eq!(id, 0);

    let guard: bool = s
        .eval("local id = GetChannelName('NoSuchChannel') return id > 0")
        .unwrap();
    assert!(
        !guard,
        "the reference's `if ( channelNum > 0 )` must run and be false"
    );

    let out_of_range: bool = s
        .eval("local id = GetChannelName(99) return id > 0")
        .unwrap();
    assert!(!out_of_range, "an out-of-range index takes the same branch");

    let zero: bool = s
        .eval("local id = GetChannelName(0) return id > 0")
        .unwrap();
    assert!(
        !zero,
        "the client bounds-checks 1 <= n <= count, so 0 is not a slot"
    );
}

/// `ChatFrame.lua:2113` passes the `"1"` it cuts out of `/1`: `lua_isnumber` takes any numeric
/// string and the number truncates (`0x4fe85c`-`0x4fe873`); nil and other types raise (`0x9fc0a4`).
#[test]
fn a_numeric_string_resolves_as_an_index_not_as_a_name() {
    let s = joined();
    for key in ["'2'", "' 2'", "'2.0'", "'0x2'", "2.9"] {
        let id: i64 = s.eval(&format!("return GetChannelName({key})")).unwrap();
        assert_eq!(id, 2, "GetChannelName({key})");
    }
    for call in [
        "GetChannelName()",
        "GetChannelName(nil)",
        "GetChannelName({})",
    ] {
        let e = s.run(call).unwrap_err().to_string();
        assert!(
            e.contains("Usage: GetChannelName([channelIndex] or [channelName])"),
            "{e}"
        );
    }
}

/// `ui_chat::feed` appends on the server's YOU_JOINED notice, never on the request.
#[test]
fn nothing_is_joined_before_the_server_confirms_it() {
    let s = script();
    let id: i64 = s.eval("return GetChannelName('World')").unwrap();
    assert_eq!(id, 0);
}

/// The shape its consumers pin: `FCFDropDown_LoadChannels` walks `for i=1, arg.n, 2` reading
/// `arg[i+1]` as the name, and `ChatLog.lua:424` packs it and spots an id by `type == "number"`.
#[test]
fn get_channel_list_is_a_flat_slot_name_vararg_in_join_order() {
    let s = joined();

    // Two pairs for two joined channels.
    assert_eq!(s.arity("GetChannelList()").unwrap(), 4);

    // Slot 1 is the first joined; alphabetically "Trade - City" would come first.
    let (s1, n1, s2, n2) = s
        .eval::<(i64, String, i64, String)>("return GetChannelList()")
        .unwrap();
    assert_eq!((s1, n1.as_str()), (1, "World"));
    assert_eq!((s2, n2.as_str()), (2, "Trade - City"));

    assert!(s
        .eval::<bool>(
            "local t = { GetChannelList() } \
             return table.getn(t) == 4 and type(t[1]) == 'number' and type(t[2]) == 'string'"
        )
        .unwrap());

    assert!(s
        .eval::<bool>("local i = GetChannelName('Trade - City') return i == 2")
        .unwrap());

    let empty = crate::script::tests::common::script();
    assert_eq!(empty.arity("GetChannelList()").unwrap(), 0);
}

/// `0x4fe6a0`/`0x4fdf50`: a record whose DBC id names a row lists as its Shortcut, which a
/// window's `channelList` holds; `GetChannelName` keeps the record name. `GetChannelList` and the
/// numeric lookup skip a pending record (`+0x9c`), the name lookup finds it (`0x4fdfc0`).
#[test]
fn a_dbc_channel_lists_by_its_shortcut_and_a_pending_one_by_name_only() {
    let mut s = joined();
    s.set_zone_channel_catalog(vec![crate::script::ZoneChannelRow {
        id: 2,
        shortcut: "Trade".into(),
        resolved: None,
        listed: false,
    }]);
    let mut mirror = vec![None; 3];
    mirror[1] = Some(crate::script::JoinedChannel {
        name: "Trade - City".into(),
        id: 2,
        confirmed: true,
    });
    mirror[2] = Some(crate::script::JoinedChannel {
        name: "Pending".into(),
        id: 0,
        confirmed: false,
    });
    s.set_joined_channels(mirror);
    assert_eq!(
        s.eval::<(i64, String)>("return GetChannelList()").unwrap(),
        (2, "Trade".to_string())
    );
    assert_eq!(
        s.arity("GetChannelList()").unwrap(),
        2,
        "the pending record is not listed"
    );
    assert_eq!(
        s.eval::<String>("return select(2, GetChannelName(2))")
            .unwrap(),
        "Trade - City"
    );
    assert_eq!(s.eval::<i64>("return GetChannelName(3)").unwrap(), 0);
    assert_eq!(
        s.eval::<i64>("return GetChannelName('pending')").unwrap(),
        3
    );
    s.set_wrath_display_channels(vec![crate::script::WrathDisplayChannel {
        slot: 2,
        name: "Trade - City".into(),
        channel_id: 2,
        active: true,
        custom_state: 0,
    }]);
    assert_eq!(
        s.eval::<String>("return (GetChannelDisplayInfo(3))")
            .unwrap(),
        "Trade"
    );
}

/// Five 1.12 names build 12340 does not register: `ChannelModerate`, `AssistByName`,
/// `UninviteByName` and the recruitment pair, whose option is the global `0xac7a3c`, written only by
/// the join and leave handlers and the chat cache. The latch still boots AUTO (1).
#[test]
fn the_names_12340_does_not_register_are_absent() {
    let s = script();
    for name in [
        "ChannelModerate",
        "AssistByName",
        "UninviteByName",
        "GetGuildRecruitmentMode",
        "SetGuildRecruitmentMode",
    ] {
        assert!(
            s.eval::<bool>(&format!("return {name} == nil")).unwrap(),
            "{name}"
        );
    }
    assert_eq!(s.guild_recruitment_mode(), 1);
}

/// A manual join or leave of `GuildRecruitment` sets the option to 0 (`0x508260(0, 1)` at
/// `0x507be6`; `0x509513`-`0x509525`), which dirties the file.
#[test]
fn a_manual_guild_recruitment_verb_resets_the_latch() {
    let mut s = script();
    assert!(s.reset_guild_recruitment_mode(), "1 → 0 moved");
    assert_eq!(s.guild_recruitment_mode(), 0);
    assert!(s.take_guild_recruitment_change());
    assert!(
        !s.reset_guild_recruitment_mode(),
        "already 0: nothing moved"
    );
    assert!(!s.take_guild_recruitment_change());
}

/// The join and management verbs, as they queue their `CMSG_*`.
mod command_tests {
    use crate::script::{ChannelCommand, UiScript, ZoneChannelRow};

    fn catalog() -> Vec<ZoneChannelRow> {
        vec![
            ZoneChannelRow {
                id: 1,
                shortcut: "General".into(),
                resolved: Some("General - Elwynn Forest".into()),
                listed: true,
            },
            ZoneChannelRow {
                id: 2,
                shortcut: "Trade".into(),
                resolved: None,
                listed: false,
            },
        ]
    }

    /// `0x50d900` → `0x507a80`: a Shortcut answers `id, Shortcut` and joins the zone's name with
    /// no password; a custom name answers `0, nil`; a missing zone text, a space or 128 bytes
    /// answer nothing; a held channel sends nothing but still registers its window; a non-string
    /// raises the binding's own usage line.
    #[test]
    fn join_channel_by_name_answers_the_row_custom_and_nil_legs() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "A = {JoinChannelByName('General', 'pw', 1)} \
             B = {JoinChannelByName('MyChan', nil, '2', 1)} \
             C = select('#', JoinChannelByName('Trade', nil, 1)) \
             D = select('#', JoinChannelByName('two words', nil, 1)) \
             E = select('#', JoinChannelByName(string.rep('x', 128))) \
             F = select('#', JoinChannelByName('x', string.rep('x', 128)))",
        )
        .unwrap();
        assert_eq!(
            s.eval::<(i64, String)>("return A[1], A[2]").unwrap(),
            (1, "General".to_string()),
            "the Shortcut, not the composed name (`0x507ba0`)"
        );
        assert!(s
            .eval::<bool>("return B[1] == 0 and B[2] == nil and table.getn(B) == 1")
            .unwrap());
        assert_eq!(
            s.eval::<(i64, i64, i64, i64)>("return C, D, E, F").unwrap(),
            (0, 0, 0, 0)
        );
        let join = |name: &str, channel_id, voice| ChannelCommand::Join {
            name: name.into(),
            password: String::new(),
            channel_id,
            voice,
            permanent: false,
        };
        assert_eq!(
            s.take_channel_commands(),
            vec![
                join("General - Elwynn Forest", 1, false),
                join("MyChan", 0, true)
            ],
            "the row's id and no password (`0x507bcc`); the voice flag carried"
        );
        assert_eq!(
            s.chat_window_looks()[1].channels,
            vec![("MyChan".to_string(), 0)]
        );
        s.set_joined_channels(vec![Some(crate::script::JoinedChannel {
            name: "General - Elwynn Forest".into(),
            id: 1,
            confirmed: true,
        })]);
        assert_eq!(
            s.eval::<(i64, String)>("return JoinChannelByName('general', nil, 3)")
                .unwrap(),
            (1, "General".to_string())
        );
        assert!(s.take_channel_commands().is_empty(), "held: no join goes");
        assert_eq!(
            s.chat_window_looks()[2].channels,
            vec![("General".to_string(), 1)]
        );
        for (call, usage) in [
            ("JoinChannelByName()", "Usage: JoinChannelByName(\"name\""),
            (
                "JoinPermanentChannel({})",
                "Usage: JoinPermanentChannel(\"name\"",
            ),
        ] {
            let e = s.run(call).unwrap_err().to_string();
            assert!(e.contains(usage), "{call}: {e}");
        }
    }

    #[test]
    fn enumerate_server_channels_lists_the_shortcuts_the_zone_admits() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.arity("EnumerateServerChannels()").unwrap(),
            0,
            "no zone yet — 0 values"
        );
        s.set_zone_channel_catalog(catalog());
        assert_eq!(
            s.eval::<Vec<String>>("return {EnumerateServerChannels()}")
                .unwrap(),
            vec!["General".to_string()],
            "the shortcut, never the composed name; a city row only in a city"
        );
    }

    /// `0x4fe160`: a slot number names its confirmed record and an empty or unconfirmed one
    /// sends nothing; any other word goes as typed. `0x4fe330`/`0x4fe460`/`0x4fe720` raise their
    /// usage lines and `Name too long` past 48 bytes, and send the second word raw.
    #[test]
    fn the_management_verbs_resolve_slot_numbers_and_check_their_arguments() {
        let mut s = UiScript::new().unwrap();
        let rec = |name: &str, confirmed| {
            Some(crate::script::JoinedChannel {
                name: name.into(),
                id: 0,
                confirmed,
            })
        };
        s.set_joined_channels(vec![rec("Mine", true), rec("Pending", false)]);
        s.run(
            "LeaveChannelByName('General') ListChannelByName('') ListChannelByName('1') \
             ListChannelByName('2') ListChannelByName('3') ListChannels() \
             SetChannelPassword('1', '') ChannelKick('General', 'Bob') ChannelKick('1', '') \
             ChannelToggleAnnouncements(' 1') SetChannelOwner('General', ' Ann')",
        )
        .unwrap();
        let named = |name: &str| name.to_string();
        assert_eq!(
            s.take_channel_commands(),
            vec![
                ChannelCommand::Leave {
                    name: named("General")
                },
                ChannelCommand::List {
                    name: named("Mine")
                },
                ChannelCommand::ListAll,
                ChannelCommand::SetPassword {
                    name: named("Mine"),
                    password: String::new()
                },
                ChannelCommand::Kick {
                    name: named("General"),
                    player: named("Bob")
                },
                ChannelCommand::Kick {
                    name: named("Mine"),
                    player: String::new()
                },
                ChannelCommand::ToggleAnnouncements { name: named(" 1") },
                ChannelCommand::SetOwner {
                    name: named("General"),
                    player: named(" Ann")
                },
            ]
        );
        for (call, err) in [
            (
                "ListChannelByName()",
                "Usage: ListChannelByName(\"channel\")",
            ),
            (
                "ChannelKick('1')",
                "Usage: ChannelKick(\"channel\", \"name\")",
            ),
            (
                "ChannelBan({}, 'x')",
                "Usage: ChannelBan(\"channel\", \"name\")",
            ),
            (
                "SetChannelPassword('1')",
                "Usage: SetChannelPassword(\"name\", \"password\")",
            ),
            ("ChannelInvite('1', string.rep('x', 49))", "Name too long"),
        ] {
            let e = s.run(call).unwrap_err().to_string();
            assert!(e.contains(err), "{call}: {e}");
        }
        s.run("ChannelInvite('9', string.rep('x', 49)) ChannelInvite('1', string.rep('x', 48))")
            .unwrap();
        assert_eq!(
            s.take_channel_commands().len(),
            1,
            "an unresolved channel stops first"
        );
    }

    /// `frameId - 1` indexes the ten window records (`0x49ec24`); a missing, non-numeric or
    /// out-of-range frame skips the registration and nothing else.
    #[test]
    fn join_channel_by_name_registers_the_channel_in_the_named_window() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "JoinChannelByName('General', nil, 1) \
             JoinChannelByName('general', nil, 1) \
             JoinChannelByName('MyChan', nil, 2) \
             JoinChannelByName('Trade', nil, nil) \
             JoinChannelByName('LocalDefense', nil, 11) \
             JoinChannelByName('WorldDefense', nil, 0)",
        )
        .unwrap();
        let looks = s.chat_window_looks();
        assert_eq!(
            looks[0].channels,
            vec![("General".to_string(), 1)],
            "the DBC Shortcut and its id, once — the second join deduplicated by name"
        );
        assert_eq!(
            looks[1].channels,
            vec![("MyChan".to_string(), 0)],
            "a custom channel: the name and id 0"
        );
        assert!(
            looks[2..].iter().all(|l| l.channels.is_empty()),
            "nil, 11 and 0 register nowhere"
        );
        assert_eq!(
            s.take_chat_window_changes(),
            vec![0, 1],
            "…and both windows are owed a save"
        );
        // The joins went out whatever the frame argument: five, as this catalog's Trade is the
        // nil leg, which registers nothing either.
        assert_eq!(s.take_channel_commands().len(), 5);
    }

    /// `LeaveChannelByName`'s four legs and its raise (`0x4a0000` → `0x49ee70`); a Shortcut strips
    /// its entry from every window.
    #[test]
    fn leave_channel_by_name_composes_strips_and_raises_the_way_the_reference_does() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "JoinChannelByName('General', nil, 1) JoinChannelByName('MyChan', nil, 1) \
             JoinChannelByName('General', nil, 2)",
        )
        .unwrap();
        s.take_channel_commands();
        s.take_chat_window_changes();

        s.run("LeaveChannelByName('2')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave { name: "2".into() }],
            "a number goes through as typed — the app holds the slot states"
        );
        assert!(
            s.take_chat_window_changes().is_empty(),
            "…and strips no window: a slot name never equals a window entry"
        );

        s.run("LeaveChannelByName('general')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave {
                name: "General - Elwynn Forest".into()
            }],
            "a shortcut composes for the zone"
        );
        let looks = s.chat_window_looks();
        assert_eq!(
            looks[0].channels,
            vec![("MyChan".to_string(), 0)],
            "General stripped from window 1, MyChan kept"
        );
        assert!(
            looks[1].channels.is_empty(),
            "…and from window 2 — all ten are walked"
        );
        assert_eq!(s.take_chat_window_changes(), vec![0, 1]);

        s.run("LeaveChannelByName('Trade')").unwrap();
        assert!(
            s.take_channel_commands().is_empty(),
            "unresolvable here (no city word in this catalog): a complete no-op"
        );

        s.run("LeaveChannelByName('mychan')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave {
                name: "mychan".into()
            }],
            "custom: verbatim"
        );
        assert!(
            s.chat_window_looks()[0].channels.is_empty(),
            "…and stripped verbatim, case-folded"
        );

        let e = s.run("LeaveChannelByName()").unwrap_err();
        assert!(e.to_string().contains("Usage: LeaveChannelByName"), "{e}");
        assert_eq!(s.arity("LeaveChannelByName('x')").unwrap(), 0);
    }
}
