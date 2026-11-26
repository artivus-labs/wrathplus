use super::*;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
#[test]
fn adapter_fifo_is_lossless_repeated_ordered_and_disconnect_scoped() {
    let mut app = App::new();
    app.add_plugins(UiArenaTeamPlugin);
    let mut adapter = WrathEventAdapter::new();
    let body = b"inviter\0guild\0".to_vec();
    let list = wrathplus_protocol::wrath_arena_team::decode(0x350, &body).unwrap();
    let mut expected = Vec::new();
    for (opcode, bytes) in [
        (0x350, body.clone()),
        (0x350, vec![255, 0, 254, 0]),
        (0x350, body.clone()),
        (0x350, vec![0, 0]),
    ] {
        let WrathDispatch::Handled { session_events, .. } =
            adapter.dispatch(opcode, &bytes, 0).unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            &session_events[..],
            [SessionEvent::WrathArenaTeam { .. }]
        ));
        crate::net::handlers::dispatch(app.world_mut(), session_events);
    }
    expected.extend([
        list.clone(),
        Packet::Invite {
            inviter: vec![255],
            team: vec![254],
        },
        list,
        Packet::Invite {
            inviter: vec![],
            team: vec![],
        },
    ]);
    assert_eq!(
        app.world()
            .resource::<NativeArenaTeamPackets>()
            .0
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        expected
    );
    for end in 0..body.len() {
        assert!(matches!(
            adapter.dispatch(0x350, &body[..end], 0).unwrap(),
            WrathDispatch::Unhandled { .. }
        ));
    }
    let mut body = body;
    body.push(0);
    assert!(matches!(
        adapter.dispatch(0x350, &body, 0).unwrap(),
        WrathDispatch::Unhandled { .. }
    ));
    for bad in [vec![], vec![3, 0]] {
        assert!(matches!(
            adapter.dispatch(0x350, &bad, 0).unwrap(),
            WrathDispatch::Unhandled { .. }
        ));
    }
    crate::net::handlers::dispatch(
        app.world_mut(),
        vec![SessionEvent::Disconnected {
            reason: "test".into(),
            end: wrathplus_protocol::SessionEnd::Lost,
        }],
    );
    assert!(app
        .world()
        .resource::<NativeArenaTeamPackets>()
        .0
        .is_empty());
}
fn row(guid: u64, name: &[u8], online: u8, class: u8) -> Member {
    Member {
        guid,
        online,
        name: name.to_vec(),
        captain_flag: 1,
        level: 80,
        class,
        week_games: 3,
        week_wins: 2,
        season_games: 10,
        season_wins: 6,
        personal_rating: 1550,
        extra_bits: None,
    }
}

#[test]
fn table_follows_descriptor_and_drops_strangers() {
    let mut table = ArenaTable::default();
    let mut fx = Effects::default();
    let mut words = [[0; 7]; 3];
    words[1] = [55, 3, 0, 1, 2, 3, 1600];
    table.sync_descriptor(words, 0.0, &mut fx);
    assert_eq!(
        fx.sends,
        [
            Command::Roster { team_id: 55 },
            Command::Query { team_id: 55 }
        ]
    );
    assert_eq!(fx.events, [("ARENA_TEAM_UPDATE", vec![])]);
    // Unchanged words do nothing; a frame request inside 10 s is throttled, after it goes.
    let mut fx = Effects::default();
    table.sync_descriptor(words, 1.0, &mut fx);
    table.request_roster(1, 9.9, &mut fx);
    table.request_roster(0, 9.9, &mut fx);
    assert_eq!(fx, Effects::default());
    table.request_roster(1, 10.0, &mut fx);
    assert_eq!(fx.sends, [Command::Roster { team_id: 55 }]);

    let mut fx = Effects::default();
    for team_id in [9, 55] {
        table.apply(
            Packet::Roster {
                team_id,
                discriminator: 0,
                team_type: 3,
                members: vec![
                    Member {
                        extra_bits: Some([1.5f32.to_bits(), (-2.0f32).to_bits()]),
                        ..row(1, &[b'x'; 60], 1, 6)
                    },
                    row(2, b"Bo", 0, 10),
                ],
            },
            &mut fx,
        );
        table.apply(
            Packet::Stats {
                team_id,
                rating: 1700,
                week_games: 4,
                week_wins: 1,
                season_games: 8,
                season_wins: 5,
                rank: 12,
            },
            &mut fx,
        );
    }
    table.apply(
        Packet::Query {
            team_id: 55,
            name: b"Trio".to_vec(),
            team_type: 3,
            emblem: [1, 2, 3, 4, 5],
        },
        &mut fx,
    );
    table.apply(
        Packet::Invite {
            inviter: b"Hal".to_vec(),
            team: b"Duo".to_vec(),
        },
        &mut fx,
    );
    assert_eq!(fx.rosters, [1]);
    assert_eq!(
        fx.events,
        [
            ("ARENA_TEAM_UPDATE", vec![]),
            ("ARENA_TEAM_UPDATE", vec![]),
            (
                "ARENA_TEAM_INVITE_REQUEST",
                vec![
                    ScriptValue::Str("Hal".into()),
                    ScriptValue::Str("Duo".into())
                ]
            ),
        ]
    );
    let roster = table.roster(1);
    assert_eq!(roster[0].name.len(), 47);
    assert_eq!(roster[0].class.as_deref(), Some("Death Knight"));
    assert!(roster[0].online && !roster[1].online && roster[1].class.is_none());
    assert_eq!(roster[1].rating, 1550);
    assert_eq!((roster[0].gdf, roster[1].gdf), ([1.5, -2.0], [0.0; 2]));
    let teams = table.teams();
    assert_eq!(teams[1].stats.rank, 12);
    assert_eq!(teams[1].info.as_ref().unwrap().name, "Trio");
    assert!(teams[0].info.is_none() && teams[0].stats == ArenaTeamStatsView::default());

    // Leaving the team keeps the old members until a new roster lands; no query repeats.
    let mut fx = Effects::default();
    words[1] = [0; 7];
    words[2] = [55, 3, 1, 0, 0, 0, 0];
    table.sync_descriptor(words, 30.0, &mut fx);
    assert_eq!(fx.sends, [Command::Roster { team_id: 55 }]);
    assert_eq!(table.roster(1).len(), 2);
    assert!(table.teams()[1].info.is_none());
}

#[test]
fn an_arena_event_clears_every_throttle_and_fires_three_stale_updates() {
    let mut table = ArenaTable::default();
    let mut fx = Effects::default();
    let mut words = [[0; 7]; 3];
    words[2] = [91, 5, 0, 0, 0, 0, 0];
    table.sync_descriptor(words, 0.0, &mut fx);
    assert_eq!(fx.sends[0], Command::Roster { team_id: 91 });
    let mut fx = Effects::default();
    table.request_roster(2, 3.0, &mut fx);
    assert!(fx.sends.is_empty(), "inside the 10 s");
    table.mark_stale(&mut fx);
    assert_eq!(
        fx.events,
        vec![("ARENA_TEAM_ROSTER_UPDATE", vec![ScriptValue::Int(1)]); 3],
        "0x5a28e0(slot, 1) for each of the three slots"
    );
    table.request_roster(2, 3.0, &mut fx);
    assert_eq!(
        fx.sends,
        [Command::Roster { team_id: 91 }],
        "the throttle is cleared, so the frame's re-ask goes out at once"
    );
}
