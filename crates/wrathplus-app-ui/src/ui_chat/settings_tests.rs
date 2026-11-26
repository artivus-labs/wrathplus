use super::*;

/// Window `n`'s (1-based) boot-init record, which differs per window, with the look fields set.
fn look(n: usize, r: u8, g: u8, b: u8, a: u8, font_size: i32) -> ChatWindowLook {
    ChatWindowLook {
        r,
        g,
        b,
        a,
        font_size,
        locked: true,
        ..ChatWindowLook::stock(n - 1)
    }
}

/// A subset of `ChatChannels.dbc` as [`parse`] takes it, with the three `INITIAL` rows.
fn rows() -> Vec<(u32, String)> {
    vec![
        (1, "General".to_string()),
        (2, "Trade".to_string()),
        (22, "LocalDefense".to_string()),
        (24, "LookingForGroup".to_string()),
    ]
}

fn colors(rows: &[(&str, [u8; 3])]) -> Vec<ChatTypeColor> {
    rows.iter()
        .map(|(n, rgb)| ChatTypeColor {
            name: (*n).to_string(),
            rgb: *rgb,
            name_by_class: n.starts_with("CHANNEL"),
        })
        .collect()
}

#[test]
fn the_file_round_trips() {
    let mut named = look(3, 9, 8, 7, 6, 12);
    named.name = "Loot & Trade".into();
    named.shown = true;
    named.messages = vec!["LOOT".into(), "MONEY".into()];
    named.uninteractable = true;
    named.channels = vec![("MyChan".into(), 0), ("Trade".into(), 2)];
    let geo = &mut named.geometry;
    (geo.saved, geo.x, geo.y, geo.width, geo.height) = (true, 0.25, -0.5, 430.0, 120.0);
    assert!(geo.set_point("bottomright"));
    let looks = vec![
        look(1, 0, 0, 0, 64, 14),
        look(2, 255, 128, 0, 255, 0),
        named,
    ];
    let colors = colors(&[("SAY", [1, 2, 3]), ("CHANNEL7", [4, 5, 6])]);
    // The mask and the custom list are passed in, not derived from the live roster.
    let text = render(&looks, &colors, &["MyChan".to_string()], 0b11, true);
    assert!(
        text.contains("\nCHANNELS\nMyChan\nEND\n\nZONECHANNELS 3\n"),
        "{text}"
    );
    assert!(text.contains("\nSAY 1 2 3 N\nCHANNEL7 4 5 6 Y\n"), "{text}");
    assert!(text.contains("\nUNINTERACTABLE 1\n"), "{text}");
    // `0x4fd870`: the geometry lines for the saved window alone.
    let geometry =
        "\nSHOWN 1\nPOSITION BOTTOMRIGHT 0.250000 -0.500000\nDIMENSIONS 430.000000 120.000000\n";
    assert!(text.contains(geometry), "{text}");
    assert_eq!(text.matches("\nPOSITION ").count(), 1, "{text}");
    let parsed = parse(&text, &rows());
    let mut expect = looks.clone();
    // The zone channel comes back by its Shortcut row, after the custom names.
    expect[2].channels = vec![("MyChan".into(), 0), ("Trade".into(), 2)];
    assert_eq!(
        parsed.looks,
        vec![
            (0, expect[0].clone()),
            (1, expect[1].clone()),
            (2, expect[2].clone())
        ],
        "0-based indices, values intact"
    );
    assert_eq!(
        parsed.colors,
        vec![
            ("SAY".to_string(), [1, 2, 3], false),
            ("CHANNEL7".to_string(), [4, 5, 6], true)
        ]
    );
    assert_eq!(parsed.joined, vec!["MyChan".to_string()]);
}

/// The stock `ChatFrame_OnEvent` matches channel lines by id (`ChatFrame.lua:1379`), so a
/// shortcut stored with id 0 would lose its channel's lines.
#[test]
fn a_window_channel_named_like_a_dbc_shortcut_regains_its_id() {
    let text = "WINDOW 1\nSIZE 0\n\nMESSAGES\nEND\n\nCHANNELS\nGeneral\nMyChan\n                    LocalDefense\nEND\n\nZONECHANNELS 2\n\nEND\n";
    let parsed = parse(text, &rows());
    assert_eq!(
        parsed.looks[0].1.channels,
        vec![
            ("General".to_string(), 1),
            ("MyChan".to_string(), 0),
            ("LocalDefense".to_string(), 22),
            ("Trade".to_string(), 2),
        ],
        "the two shortcuts come back as their DBC rows; the genuinely custom name keeps id 0, \
         and the ZONECHANNELS bit still contributes Trade"
    );

    // The next save writes them as bits, not names.
    let out = render(&[parsed.looks[0].1.clone()], &[], &[], 0x0020_0003, false);
    let window = out.split("WINDOW 1").nth(1).unwrap();
    assert!(window.contains("CHANNELS\nMyChan\nEND"), "{out}");
    assert!(window.contains("ZONECHANNELS 2097155\n"), "{out}");
}

#[test]
fn a_windows_zone_bits_are_masked_by_the_joined_set() {
    let mut w = look(1, 0, 0, 0, 0, 0);
    w.channels = vec![("General".into(), 1), ("Trade".into(), 2)];
    // The mask holds General alone: Trade was left, so its window bit is ANDed away.
    let text = render(&[w], &[], &[], 1, true);
    let windows: Vec<&str> = text.split("WINDOW 1").collect();
    assert!(windows[1].contains("ZONECHANNELS 1\n"), "{text}");
}

/// The header and window words come from the durable mask, not the live roster.
#[test]
fn an_empty_roster_does_not_erase_a_windows_zone_channels() {
    let mut w = look(1, 0, 0, 0, 0, 0);
    w.channels = vec![("General".into(), 1), ("Trade".into(), 2)];
    // In both channels by the mask, with the live roster empty, as at a session-end save.
    let text = render(&[w], &[], &[], 0b11, true);
    assert!(
        text.contains("\nZONECHANNELS 3\n"),
        "the header carries the durable mask, not the roster's shadow: {text}"
    );
    let parsed = parse(&text, &rows());
    assert_eq!(parsed.zone_mask, Some(0b11), "the header word round-trips");
    assert_eq!(
        parsed.looks[0].1.channels,
        vec![("General".to_string(), 1), ("Trade".to_string(), 2)],
        "window 1 keeps both channels, where an unmarked file's came back empty"
    );
}

/// The repair is keyed on the [`WRITER_GENERATION`] marker, not on the zone words.
#[test]
fn a_written_file_carries_the_writer_generation_and_a_damaged_one_does_not() {
    let text = render(&[ChatWindowLook::stock(0)], &[], &[], 0b11, true);
    assert!(
        text.contains(WRITER_GENERATION),
        "every file we write is stamped, so the repair fires once: {text}"
    );
    // A zeroed file: a header the reference's reader accepts, with no marker.
    let damaged = "VERSION 2\n\nCHANNELS\nEND\n\nZONECHANNELS 0\n\n\
         WINDOW 1\nSIZE 0\nSHOWN 1\n\nMESSAGES\nSYSTEM\nEND\n\n\
         CHANNELS\nEND\n\nZONECHANNELS 0\n\nEND\n";
    assert!(!damaged.contains(WRITER_GENERATION));
    let parsed = parse(damaged, &rows());
    assert_eq!(parsed.zone_mask, Some(0), "the zeroed header parses as 0");
    assert!(
        parsed.looks[0].1.channels.is_empty(),
        "and window 1 comes back with no channels — the symptom"
    );
}

#[test]
fn the_zone_mask_moves_on_join_and_explicit_leave_only() {
    let mut state = super::super::edit::ChannelState {
        channels: wrathplus_formats::ChatChannelsCatalog::from_rows(vec![
            wrathplus_formats::ChatChannelRow {
                id: 1,
                flags: 0x11,
                pattern: "General - %s".into(),
                shortcut: "General".into(),
            },
            wrathplus_formats::ChatChannelRow {
                id: 2,
                flags: 0x3b,
                pattern: "Trade - %s".into(),
                shortcut: "Trade".into(),
            },
        ]),
        ..Default::default()
    };
    // Unseated, a join is dropped.
    state.note_zone_channel_joined("General - Elwynn Forest");
    assert_eq!(state.zone_mask, None, "not seated: nothing to OR into");
    state.zone_mask = Some(0);
    state.note_zone_channel_joined("General - Elwynn Forest");
    state.note_zone_channel_joined("Trade - City");
    assert_eq!(state.zone_mask, Some(0b11));
    state.note_zone_channel_joined("MyChan");
    assert_eq!(state.zone_mask, Some(0b11), "a custom channel has no bit");
    // The clear is keyed on the slot the wire name finds and its id (`0x49f0f4`-`0x49f11a`);
    // a name with no slot clears nothing.
    state.note_zone_channel_left("Trade - City");
    assert_eq!(state.zone_mask, Some(0b11), "no slot carries it yet");
    state.claim_slot("Trade - City");
    state.note_zone_channel_left("Trade - City");
    assert_eq!(
        state.zone_mask,
        Some(0b01),
        "an explicit leave clears one bit"
    );
}

#[test]
fn the_custom_list_moves_on_join_and_explicit_leave_only() {
    let mut state = super::super::edit::ChannelState {
        channels: wrathplus_formats::ChatChannelsCatalog::from_rows(vec![
            wrathplus_formats::ChatChannelRow {
                id: 1,
                flags: 0x11,
                pattern: "General - %s".into(),
                shortcut: "General".into(),
            },
        ]),
        ..Default::default()
    };
    state.note_custom_channel_joined("General - Elwynn Forest");
    assert!(state.custom.is_empty(), "a zone channel travels as its bit");
    state.note_custom_channel_joined("MyChan");
    state.note_custom_channel_joined("mychan");
    assert_eq!(state.custom, vec!["MyChan".to_string()], "once, by name");
    state.note_custom_channel_left("MYCHAN");
    assert!(state.custom.is_empty(), "an explicit leave drops it");
    // `0x4fd43a`: only a JoinPermanentChannel's custom channel, once confirmed, is a re-join row.
    state.note_join_request("Perm", true);
    state.note_join_request("Temp", false);
    state.note_join_request("Again", true);
    state.note_join_request("again", false);
    for name in ["Temp", "Again", "Perm"] {
        state.note_join_confirmed(name);
    }
    assert_eq!(
        state.custom,
        vec!["Perm".to_string()],
        "the latest request decides"
    );
    state.note_join_request("Pending", true);
    state.end_session();
    state.note_join_confirmed("Pending");
    assert_eq!(
        state.custom.len(),
        1,
        "the session end drops a pending request"
    );
}

/// The teardown write ignores the dirty flag, as the reference's does, so a `/leave` persists.
#[test]
fn a_flush_writes_whatever_was_restored_and_the_debounce_writes_only_lua_moves() {
    // The flush: restored is the whole condition.
    assert!(owes_write(true, true, false, false));
    assert!(owes_write(true, true, true, true));
    assert!(
        !owes_write(true, false, true, true),
        "a VM that never read the file has nothing of the player's to write"
    );
    // The debounce: dirty and quiet.
    assert!(owes_write(false, true, true, true));
    assert!(!owes_write(false, true, true, false), "still being dragged");
    assert!(!owes_write(false, true, false, true), "nothing Lua moved");
}

#[test]
fn the_header_is_skipped_not_parsed() {
    assert!(render(&[ChatWindowLook::default()], &[], &[], 0, true).starts_with('#'));
    assert_eq!(parse(HEADER, &rows()), Parsed::default());
}

/// WrathPlus's older one-line rows (no `ADDEDVERSION`): the back-fill leaves the stock sets.
#[test]
fn one_line_rows_without_addedversion_still_parse() {
    let got = parse(
        "window 1  size 16  color 10 20 30 40\n\
         WINDOW 2  SHOWN 0  COLOR 1 2 3 4  DOCKED 2  BOGUS 9\n\
         WINDOW 3  SIZE 12\n",
        &rows(),
    );
    let mut w2 = look(2, 1, 2, 3, 4, 0);
    w2.shown = false;
    w2.docked = Some(2);
    assert_eq!(
        got.looks,
        vec![
            (0, look(1, 10, 20, 30, 40, 16)),
            (1, w2),
            (2, look(3, 0, 0, 0, 0x28, 12)),
        ]
    );
    assert!(got.looks[1].1.messages.iter().any(|m| m == "OPENING"));
}

/// The grammar of a real 12340-written `chat-cache.txt`.
#[test]
fn a_stock_reference_file_parses() {
    let text = "VERSION 5\n\nADDEDVERSION 13\n\nOPTION_GUILD_RECRUITMENT_CHANNEL AUTO\n\n\
                CHANNELS\nMyGuildChat\nEND\n\nZONECHANNELS 8388611\n\n\
                COLORS\nSAY 255 255 255 N\nSYSTEM 200 200 0 Y\nEND\n\n\
                WINDOW 1\nNAME General\nSIZE 0\nCOLOR 0 0 0 40\nLOCKED 1\n\
                UNINTERACTABLE 0\nDOCKED 1\nSHOWN 1\n\n\
                MESSAGES\nSYSTEM\nSAY\nACHIEVEMENT\nEND\n\nCHANNELS\nMyGuildChat\nEND\n\n\
                ZONECHANNELS 8388611\n\nEND\n\n\
                WINDOW 2\nNAME Combat Log\nSIZE 0\nCOLOR 0 0 0 40\nLOCKED 1\n\
                UNINTERACTABLE 1\nDOCKED 2\nSHOWN 0\n\n\
                MESSAGES\nOPENING\nCOMBAT_XP_GAIN\nEND\n\nCHANNELS\nEND\n\nZONECHANNELS 0\n\nEND\n";
    let got = parse(text, &rows());
    assert_eq!(
        got.colors,
        vec![
            ("SAY".to_string(), [255, 255, 255], false),
            ("SYSTEM".to_string(), [200, 200, 0], true)
        ]
    );
    assert_eq!(got.joined, vec!["MyGuildChat".to_string()]);
    assert_eq!(got.looks.len(), 2);
    let (i, w1) = &got.looks[0];
    assert_eq!(*i, 0);
    assert!(w1.shown && !w1.uninteractable);
    assert_eq!(w1.name, "General");
    assert_eq!(
        w1.messages,
        vec![
            "SYSTEM".to_string(),
            "SAY".to_string(),
            "ACHIEVEMENT".to_string()
        ],
        "a Wrath group is kept, not dropped as unknown"
    );
    assert_eq!(
        w1.channels,
        vec![
            ("MyGuildChat".to_string(), 0),
            ("General".to_string(), 1),
            ("Trade".to_string(), 2),
            ("LookingForGroup".to_string(), 24),
        ],
        "bits 0, 1 and 23 of 8388611 — the shortcut rows, in DBC order"
    );
    let (_, w2) = &got.looks[1];
    assert!(!w2.shown && w2.uninteractable);
    assert_eq!(w2.name, "Combat Log");
    assert_eq!(w2.docked, Some(2));
    assert_eq!(
        w2.messages,
        vec!["OPENING".to_string(), "COMBAT_XP_GAIN".to_string()]
    );
}

/// A file older than `ADDEDVERSION 13` gains the `defaultOn` groups added since, general ones
/// in window 1 and combat-log ones in window 2.
#[test]
fn an_older_file_is_back_filled_by_group_version() {
    let got = parse(
        "ADDEDVERSION 7\nWINDOW 1\nMESSAGES\nSAY\nEND\nEND\nWINDOW 2\nMESSAGES\nEND\nEND\n",
        &rows(),
    );
    let w1 = &got.looks[0].1.messages;
    assert!(w1.iter().any(|m| m == "ACHIEVEMENT"), "version 8");
    assert!(w1.iter().any(|m| m == "BN_WHISPER"), "version 12");
    assert!(!w1.iter().any(|m| m == "EMOTE"), "version 7 is not newer");
    assert!(!w1.iter().any(|m| m == "TARGETICONS"), "not defaultOn");
    assert!(
        got.looks[1].1.messages.is_empty(),
        "the combat-log groups are version 6 or 0"
    );
}

#[test]
fn the_lock_round_trips_and_an_absent_key_stays_locked() {
    let unlocked = ChatWindowLook {
        locked: false,
        ..look(1, 0, 0, 0, 64, 14)
    };
    let text = render(std::slice::from_ref(&unlocked), &[], &[], 0, true);
    assert!(text.contains("LOCKED 0"));
    assert_eq!(parse(&text, &rows()).looks, vec![(0, unlocked.clone())]);
    assert_eq!(
        parse("WINDOW 1\nSIZE 14\nCOLOR 0 0 0 64\nEND\n", &rows()).looks,
        vec![(0, look(1, 0, 0, 0, 64, 14))],
        "no LOCKED key = the init's LOCKED 1"
    );
}

#[test]
fn dock_positions_round_trip_and_an_absent_key_keeps_the_init() {
    let moved = ChatWindowLook {
        docked: Some(3),
        ..look(1, 0, 0, 0, 0, 0)
    };
    let text = render(std::slice::from_ref(&moved), &[], &[], 0, true);
    assert!(text.contains("DOCKED 3"));
    assert_eq!(parse(&text, &rows()).looks, vec![(0, moved.clone())]);
    // The stock records: window 1 shown at dock 1 with 39 groups, window 2 shown at dock 2
    // with the six combat-log groups, window 3 hidden.
    let got = parse(
        "WINDOW 1  SIZE 0\nWINDOW 2  SIZE 0\nWINDOW 3  SIZE 0\n",
        &rows(),
    );
    assert_eq!(
        got.looks
            .iter()
            .map(|(_, l)| (l.docked, l.shown, l.messages.len()))
            .collect::<Vec<_>>(),
        vec![(Some(1), true, 39), (Some(2), true, 6), (None, false, 0)]
    );
}

#[test]
fn junk_costs_only_its_own_line() {
    let got = parse(
        "WINDOW\nnot a window line\nWINDOW 0 SIZE 1\nWINDOW 2 SIZE 18\n",
        &rows(),
    );
    assert_eq!(got.looks, vec![(1, look(2, 0, 0, 0, 0x28, 18))]);
}

/// A `MESSAGES` block replaces the init set: the loader zeroes every flag before it reads.
#[test]
fn an_empty_messages_block_is_an_empty_set() {
    let got = parse("ADDEDVERSION 13\nWINDOW 1\nMESSAGES\nEND\nEND\n", &rows());
    assert!(got.looks[0].1.messages.is_empty());
}

/// A roster with one character picked, which keys the chat cache path.
fn probe_roster(name: &str) -> crate::app_core::state::Roster {
    crate::app_core::state::Roster::with_pending_pick(
        vec![wrathplus_protocol::Character {
            guid: 1,
            name: name.into(),
            race: 1,  // Human → Alliance
            class: 1, // Warrior
            gender: 0,
            level: 60,
            skin: 0,
            face: 0,
            hair_style: 0,
            hair_color: 0,
            facial_hair: 0,
            zone: 0,
            map: 0,
            position: wrathplus_protocol::wire::Vector3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            flags: 0,
            customize_flags: 0,
            equipment: [wrathplus_protocol::CharEnumItem::default(); 19],
            pet_display_id: 0,
            pet_level: 0,
            pet_family: 0,
        }],
        1,
    )
}

/// The disconnect clears the live roster a frame before `end_ui_session` flushes, so the
/// header's re-join list must come from the durable custom list; the same functions run here.
#[test]
fn a_custom_channel_survives_a_logout() {
    use bevy::ecs::system::RunSystemOnce;

    let _l = crate::local_state::test_env::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wp-chat-custom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("wrathplus-config")).expect("hermetic home");
    let _capture = crate::local_state::test_env::EnvGuard::unset("WOW_CAPTURE");
    let _home = crate::local_state::test_env::EnvGuard::set(
        "WRATHPLUS_HOME",
        tmp.join("wrathplus-config")
            .to_str()
            .expect("utf-8 temp path"),
    );

    let roster = probe_roster("Channelprobe");
    let id = crate::app_core::state::identity(&roster).expect("a picked character");
    let path = crate::local_state::chat_character_path(&id.0, &id.1).expect("a cache path");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the character dir");
    // A file this writer made (the marker), listing one custom channel in the header.
    let file = format!(
        "{WRITER_GENERATION}\nVERSION 2\n\nCHANNELS\nMyChan\nEND\n\nZONECHANNELS 3\n\n\
         COLORS\nEND\n\nWINDOW 1\nSIZE 0\n\nMESSAGES\nSAY\nEND\n\nCHANNELS\nEND\n\n\
         ZONECHANNELS 3\n\nEND\n"
    );
    std::fs::write(&path, file).expect("seed the file");

    let mut world = World::new();
    world.init_resource::<crate::ui_script::AddOnIdentity>();
    world.init_resource::<crate::minimap::MinimapZoom>();
    world.init_resource::<crate::ui_script::ReloadUiPending>();
    world.init_resource::<super::super::edit::ChannelState>();
    world.init_resource::<super::super::channels::ZoneChannelWalk>();
    world.init_resource::<super::super::recruitment::GuildRecruitmentCascade>();
    world.init_resource::<Messages<crate::app_core::session::DisconnectedMessage>>();
    world.init_resource::<ChatWindowFile>();
    crate::ui_script::setup_script(&mut world);
    world.insert_resource(roster);
    crate::ui_script::load_ingame_ui_on_world_entry(&mut world);

    // The server confirms the re-join (`YOU_JOINED`) through the real feed arm.
    world.resource_scope(
        |world, mut channels: Mut<super::super::edit::ChannelState>| {
            let mut script = world.non_send_resource_mut::<UiScript>();
            let mut windows = super::super::frames::ChatWindows::default();
            let mut e = super::super::event::ChatEvent::text_only(
                super::super::event::ChatEventKind::ChannelNotice,
                String::new(),
            );
            e.channel = "MyChan".into();
            e.notice = "2".into(); // YOU_JOINED
            super::super::feed::deliver(&mut script, &mut windows, &mut channels, &mut e);
        },
    );
    assert_eq!(
        world
            .resource::<super::super::edit::ChannelState>()
            .number_of("MyChan"),
        Some(1),
        "precondition: the channel is joined and numbered"
    );

    // `/logout`: the disconnect lands in `Update` first…
    world.write_message(crate::app_core::session::DisconnectedMessage::new(
        "logged out".into(),
        wrathplus_protocol::SessionEnd::LoggedOut,
    ));
    world
        .run_system_once(super::super::channels::end_session_channels_on_disconnect)
        .expect("the disconnect twin runs");
    // …and `OnExit(InWorld)` flushes the dying VM's cache a frame later.
    crate::ui_script::end_ui_session(&mut world);

    let on_disk = std::fs::read_to_string(&path).expect("the flush wrote the file");
    assert!(
        on_disk.contains("\nCHANNELS\nMyChan\nEND\n\nZONECHANNELS"),
        "the custom channel is still in the header's re-join list; file was:\n{on_disk}"
    );

    drop(world);
    let _ = std::fs::remove_dir_all(&tmp);
}
