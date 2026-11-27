use super::*;
use wrathplus_protocol::wrath_calendar::{Holiday, InviteRow};

const ME: u64 = 7;

fn net() -> NetCommands {
    let (tx, _rx) = crossbeam_channel::unbounded();
    NetCommands(tx)
}

fn packed(y: i32, m: u8, d: u8, h: u8, min: u8) -> u32 {
    WrathCalendarDate::new(y, m, d, h, min).packed()
}

fn events(fired: &[(&'static str, Vec<V>)]) -> Vec<&'static str> {
    fired.iter().map(|(e, _)| *e).collect()
}

/// 15 March 2010 12:00 UTC, the server two hours ahead.
const NOW: u32 = 1_268_654_400;

fn calendar_packet() -> Packet {
    let mut holiday = Holiday {
        id: 181,
        region: 0,
        looping: 0,
        priority: 0,
        filter_type: 0,
        dates: [0; 26],
        durations: [0; 10],
        flags: [0; 10],
        texture: "Calendar_Noblegarden".into(),
    };
    holiday.dates[0] = packed(2010, 3, 14, 10, 0);
    holiday.durations[0] = 48;
    Packet::Calendar(Calendar {
        invites: vec![InviteRow {
            event_id: 100,
            invite_id: 200,
            status: 0,
            rank: 0,
            guild_event: 0,
            creator: 9,
        }],
        events: vec![EventRow {
            event_id: 100,
            title: "Naxx".into(),
            kind: 0,
            packed_time: packed(2010, 3, 16, 19, 0),
            flags: 1,
            dungeon: 160,
            creator: 9,
        }],
        server_time: NOW,
        zone_packed_time: packed(2010, 3, 15, 14, 0),
        lockouts: vec![Lockout {
            map: 533,
            difficulty: 1,
            seconds_left: 86_400,
            instance: 55,
        }],
        relation_time: 1_135_814_400,
        raid_resets: vec![RaidReset {
            map: 533,
            period: 604_800,
            offset: 0,
        }],
        holidays: vec![holiday],
    })
}

/// The calendar lists the invite, the lockout in the server's zone and each holiday day.
#[test]
fn the_calendar_lists_events_lockouts_and_holiday_days() {
    let mut state = CalendarState::default();
    let mut fired = Vec::new();
    reduce(&mut state, calendar_packet(), ME, &mut fired);
    assert_eq!(events(&fired), ["CALENDAR_UPDATE_EVENT_LIST"]);
    assert_eq!(state.zone, 7200);
    let mut catalog = CalendarCatalog::default();
    catalog.dungeons.insert(160, "LFGIcon-Naxxramas".into());
    let view = build(&state, &catalog, None, &net(), None, ME);
    let kinds: Vec<_> = view
        .entries
        .iter()
        .map(|e| (e.kind, e.date.day, e.date.hour, e.sequence))
        .collect();
    assert_eq!(
        kinds,
        [
            (WrathCalendarKind::Holiday, 14, 10, (0, 3)),
            (WrathCalendarKind::Holiday, 15, 0, (1, 3)),
            (WrathCalendarKind::Holiday, 16, 9, (2, 3)),
            (WrathCalendarKind::RaidLockout, 16, 14, (0, 0)),
            (WrathCalendarKind::Player, 16, 19, (0, 0)),
        ]
    );
    // `0x5c0fb0`: a player row's icon is its `LFGDungeons` texture; a map with no name is "".
    let titles = (
        view.entries[3].title.as_str(),
        view.raid_resets[0].name.as_str(),
    );
    assert_eq!(
        (titles, view.entries[4].texture.as_str()),
        (("", ""), "LFGIcon-Naxxramas")
    );
    let event = &view.entries[4];
    assert_eq!(
        (event.event_id, event.invite_id, event.invite),
        (100, 200, Some((0, 0)))
    );
    let now = state.local(state.now_unix());
    assert_eq!((now.day, now.hour), (15, 14));
    // The reset projects from the relation time in the server's zone.
    assert_eq!(view.raid_resets[0].first, 1_135_814_400 + 7200);
    // `0x5c2cd9`: the unanswered invite to a later, unlocked event is pending.
    assert_eq!(event.bits, 1);
    assert_eq!(
        event.alarm,
        Some(WrathCalendarDate::new(2010, 3, 16, 18, 45))
    );
    // Against the game clock (`0xd37f98`), not the calendar packet's: past the event, no pip.
    state.game_now = Some(WrathCalendarDate::new(2010, 3, 17, 0, 0));
    let view = build(&state, &catalog, None, &net(), None, ME);
    assert_eq!(view.entries[4].bits, 0);
}

/// `0x5c2cd9`-`0x5c2f8d`'s invite bits, row by row.
#[test]
fn invite_bits_follow_the_receipt() {
    let now = WrathCalendarDate::new(2010, 3, 15, 14, 0);
    let bits = |flags: u32, day: u8, creator: u64, invite: Option<(u8, u8, u8)>| {
        let row = EventRow {
            event_id: 1,
            title: String::new(),
            kind: 0,
            packed_time: packed(2010, 3, day, 19, 0),
            flags,
            dungeon: -1,
            creator,
        };
        invite_bits::invite_bits(&row, invite, ME, now)
    };
    assert_eq!(bits(1, 16, 9, Some((0, 0, 0))), Some(1));
    // Past or locked: no pending bit; any other status neither.
    assert_eq!(bits(1, 14, 9, Some((0, 0, 0))), Some(0));
    assert_eq!(bits(0x11, 16, 9, Some((0, 0, 0))), Some(0));
    assert_eq!(bits(1, 16, 9, Some((1, 1, 0))), Some(2));
    assert_eq!(bits(1, 16, ME, Some((3, 2, 0))), Some(6));
    // A sign-up invite; a guild event without one; an announcement; nothing to list.
    assert_eq!(bits(0x400, 16, 9, Some((7, 0, 1))), Some(8));
    assert_eq!(bits(0x400, 16, ME, None), Some(12));
    assert_eq!(bits(0x40, 16, ME, None), Some(4));
    assert_eq!(bits(1, 16, 9, None), None);
}

/// The asked-for event opens with its type; a resend refreshes it.
#[test]
fn an_asked_event_opens_then_refreshes() {
    // `0x5f1a40`: the open event waits until its invitees' names are all back.
    let mut held = Vec::new();
    let opened = vec![
        ("CALENDAR_OPEN_EVENT", vec![V::Str("PLAYER".into())]),
        ("CALENDAR_UPDATE_EVENT_LIST", vec![]),
    ];
    let now = release(&mut held, opened, false);
    assert_eq!(events(&now), ["CALENDAR_UPDATE_EVENT_LIST"]);
    assert_eq!(
        events(&release(&mut held, vec![], true)),
        ["CALENDAR_OPEN_EVENT"]
    );
    let mut state = CalendarState::default();
    let draft = R::NewDraft {
        flags: 1,
        arena_slot: None,
    };
    apply(&mut state, draft, ME, None);
    let sent = apply(&mut state, R::GetEvent { event_id: 100 }, ME, None);
    assert_eq!(sent, [Command::GetEvent { event_id: 100 }]);
    // The open event (here a draft) stays until the asked one arrives.
    assert_eq!(state.open.as_ref().map(|o| o.event_id), Some(0));
    let event = Event {
        send_type: 0,
        creator: 9,
        event_id: 100,
        title: "Naxx".into(),
        description: String::new(),
        kind: 0,
        repeat: 0,
        max_invites: 25,
        dungeon: -1,
        flags: 0x400,
        packed_time: packed(2010, 3, 16, 19, 0),
        timezone_time: 0,
        guild_id: 1,
        invitees: vec![EventInvitee {
            guid: ME,
            level: 80,
            status: 6,
            rank: 0,
            guild_member: 1,
            invite_id: 201,
            status_time: 0,
            text: String::new(),
        }],
    };
    let mut fired = Vec::new();
    assert!(reduce(
        &mut state,
        Packet::Event(event.clone()),
        ME,
        &mut fired
    ));
    assert_eq!(
        fired[0],
        ("CALENDAR_OPEN_EVENT", vec![V::Str("GUILD_EVENT".into())])
    );
    let open = state.open.clone().unwrap();
    assert_eq!(
        (open.event_id, open.invitees[0].signup, open.revision),
        (100, true, 0)
    );
    fired.clear();
    reduce(&mut state, Packet::Event(event), ME, &mut fired);
    assert_eq!(
        events(&fired),
        ["CALENDAR_UPDATE_EVENT", "CALENDAR_UPDATE_INVITE_LIST"]
    );
    assert_eq!(state.open.as_ref().unwrap().revision, 1);
    // The event's removal closes it.
    fired.clear();
    reduce(
        &mut state,
        Packet::RemovedAlert {
            raw_byte: 0,
            event_id: 100,
            packed_time: 0,
        },
        ME,
        &mut fired,
    );
    assert_eq!(
        events(&fired),
        ["CALENDAR_CLOSE_EVENT", "CALENDAR_UPDATE_EVENT_LIST"]
    );
    assert!(state.open.is_none());
}

/// A draft lists its creator, takes a guild filter's members and adds them; the add's echo
/// closes it.
#[test]
fn a_draft_adds_with_its_invites() {
    let mut state = CalendarState::default();
    apply(
        &mut state,
        R::NewDraft {
            flags: 1,
            arena_slot: None,
        },
        ME,
        None,
    );
    // A draft's list edits fire the list event once App holds them; a server event's wait.
    let removal = |event_id| R::RemoveInvite {
        event_id,
        invitee: 20,
        invite_id: 0,
        owner_invite_id: 0,
    };
    assert_eq!(
        [
            draft_list::redraws(&removal(0)),
            draft_list::redraws(&removal(100))
        ],
        [Some(vec![]), None]
    );
    // A new player draft fires the list event with true (`0x5bbf05`), a guild draft nothing.
    let draft = |flags| R::NewDraft {
        flags,
        arena_slot: None,
    };
    assert_eq!(
        [
            draft_list::redraws(&draft(1)),
            draft_list::redraws(&draft(0x400))
        ],
        [Some(vec![V::Bool(true)]), None]
    );
    let mut fired = Vec::new();
    reduce(
        &mut state,
        Packet::FilterGuild {
            members: vec![(20, 0), (ME, 0)],
        },
        ME,
        &mut fired,
    );
    assert_eq!(events(&fired), ["CALENDAR_UPDATE_INVITE_LIST"]);
    assert_eq!(state.open.as_ref().unwrap().invitees.len(), 2);
    let fields = WrathCalendarFields {
        title: "Raid".into(),
        event_type: 0,
        dungeon: -1,
        date: WrathCalendarDate::new(2010, 3, 20, 20, 0),
        ..Default::default()
    };
    let sent = apply(&mut state, R::AddEvent(fields.clone()), ME, None);
    let [Command::AddEvent {
        title,
        packed_time,
        invites: Some(invites),
        ..
    }] = sent.as_slice()
    else {
        panic!("{sent:?}");
    };
    assert_eq!(
        (title.as_str(), *packed_time, invites.len(), invites[0].guid),
        ("Raid", fields.date.packed(), 1, 20)
    );
    // A draft's moderator and removal stay local.
    assert!(apply(
        &mut state,
        R::RemoveInvite {
            event_id: 0,
            invitee: 20,
            invite_id: 0,
            owner_invite_id: 0,
        },
        ME,
        None,
    )
    .is_empty());
    assert_eq!(state.open.as_ref().unwrap().invitees.len(), 1);
    let mut fired = Vec::new();
    let mut echo = Event {
        send_type: SEND_ADD,
        creator: ME,
        event_id: 300,
        title: "Raid".into(),
        description: String::new(),
        kind: 0,
        repeat: 0,
        max_invites: 0,
        dungeon: -1,
        flags: 0,
        packed_time: fields.date.packed(),
        timezone_time: 0,
        guild_id: 0,
        invitees: vec![],
    };
    echo.invitees.push(EventInvitee {
        guid: ME,
        level: 80,
        status: CONFIRMED,
        rank: OWNER,
        guild_member: 0,
        invite_id: 301,
        status_time: 0,
        text: String::new(),
    });
    assert!(reduce(&mut state, Packet::Event(echo), ME, &mut fired));
    assert_eq!(
        events(&fired),
        ["CALENDAR_NEW_EVENT", "CALENDAR_UPDATE_EVENT_LIST"]
    );
    assert!(state.open.is_none());
    assert_eq!(state.events[0].invite, Some((301, CONFIRMED, OWNER, 0)));
}

/// Build 12340's switch (`0x6cd380`): its own keys for 9, 11 and 13, the limits it knows for the
/// `%d` errors, the name only where the template takes one, and `0x5b9960`'s formatting.
#[test]
fn errors_take_the_build_12340_keys_and_arguments() {
    let mut state = CalendarState::default();
    reduce(&mut state, calendar_packet(), ME, &mut Vec::new());
    let line = |state: &mut CalendarState, error: u32| {
        let mut fired = Vec::new();
        reduce(
            state,
            Packet::CommandResult {
                raw_word: 0,
                raw_byte: 0,
                param: "Bob".into(),
                error,
            },
            ME,
            &mut fired,
        );
        match &fired[..] {
            [("CALENDAR_UPDATE_ERROR", args)] => match &args[..] {
                [V::Str(packed)] => packed.clone(),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(line(&mut state, 9), "ERR_GUILD_PLAYER_NOT_IN_GUILD");
    assert_eq!(line(&mut state, 11), "PLAYER_NOT_FOUND");
    assert_eq!(line(&mut state, 13), "ERR_IGNORING_YOU_S\0Bob");
    assert_eq!(
        line(&mut state, 1),
        "CALENDAR_ERROR_GUILD_EVENTS_EXCEEDED\u{0}100"
    );
    assert_eq!(line(&mut state, 2), "CALENDAR_ERROR_EVENTS_EXCEEDED\u{0}30");

    let script = UiScript::new().unwrap();
    script
        .run(
            "CALENDAR_ERROR_EVENTS_EXCEEDED = \"limit of %d events\" \
             ERR_IGNORING_YOU_S = \"%s is ignoring you.\" \
             PLAYER_NOT_FOUND = \"Can't find that player.\"",
        )
        .unwrap();
    assert_eq!(
        error_line(&script, "CALENDAR_ERROR_EVENTS_EXCEEDED\u{0}30"),
        "limit of 30 events"
    );
    assert_eq!(
        error_line(&script, "ERR_IGNORING_YOU_S\0Bob"),
        "Bob is ignoring you."
    );
    assert_eq!(
        error_line(&script, "PLAYER_NOT_FOUND"),
        "Can't find that player."
    );
}

/// A result's error raises its line with the name; a lockout update moves its reset.
#[test]
fn errors_name_their_player_and_lockouts_update() {
    let mut state = CalendarState::default();
    reduce(&mut state, calendar_packet(), ME, &mut Vec::new());
    let mut fired = Vec::new();
    assert!(reduce(
        &mut state,
        Packet::CommandResult {
            raw_word: 0,
            raw_byte: 0,
            param: "Bob".into(),
            error: 10,
        },
        ME,
        &mut fired,
    ));
    assert_eq!(
        fired,
        [(
            "CALENDAR_UPDATE_ERROR",
            vec![V::Str(
                "CALENDAR_ERROR_ALREADY_INVITED_TO_EVENT_S\0Bob".into()
            )]
        )]
    );
    assert_eq!(error_key(15), None);
    reduce(
        &mut state,
        Packet::RaidLockoutUpdated {
            packed_time: 0,
            map: 533,
            difficulty: 1,
            old_seconds: 86_400,
            new_seconds: 2 * 86_400,
        },
        ME,
        &mut Vec::new(),
    );
    assert_eq!(state.lockouts[0].0.seconds_left, 2 * 86_400);
    // `0x5b7ce0` announces a new count only; the calendar's arrival asks for it (`0x5be30b`).
    let mut fired = Vec::new();
    for count in [3, 3] {
        reduce(&mut state, Packet::NumPending { count }, ME, &mut fired);
    }
    assert_eq!(state.pending, 3);
    assert_eq!(events(&fired), ["CALENDAR_UPDATE_PENDING_INVITES"]);
    assert_eq!(
        pending::follow_up(&calendar_packet()),
        Some(Command::GetNumPending)
    );
    // `0x5c3fe0` clears the pending bit after the calendar, never after an invite alert, after an
    // invite only on its clear byte, and after a result only with an error line (`0x5b9960`).
    let mut clears = |packet| reduce(&mut state, packet, ME, &mut Vec::new());
    let invite = |raw_not_signup| Packet::Invite {
        invitee: 20,
        event_id: 999,
        invite_id: 1,
        level: 80,
        status: 0,
        status_time: None,
        raw_not_signup,
    };
    let result = |error| Packet::CommandResult {
        raw_word: 0,
        raw_byte: 0,
        param: String::new(),
        error,
    };
    let alert = Packet::InviteRemovedAlert {
        event_id: 999,
        packed_time: 0,
        flags: 0,
        status: 0,
    };
    assert_eq!(
        [
            clears(calendar_packet()),
            clears(alert),
            clears(invite(0)),
            clears(invite(1)),
            clears(result(15)),
        ],
        [true, false, false, true, false]
    );
}
