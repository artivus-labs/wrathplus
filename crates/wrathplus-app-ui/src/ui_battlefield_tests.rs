use super::*;
use wrathplus_formats::{BattlemasterType, PvpBracket};
use wrathplus_protocol::wrath_battleground::Status;

fn list(bg_type: u32, battlemaster: u64, from_where: u8) -> BattlefieldList {
    let none = Rewards {
        has_win: 0,
        win_honor: 0,
        win_arena: 0,
        loss_honor: 0,
    };
    BattlefieldList {
        battlemaster,
        from_where,
        bg_type,
        raw_bytes: [0; 2],
        rewards: Rewards {
            has_win: 1,
            win_honor: 30,
            win_arena: 25,
            loss_honor: 5,
        },
        random_rewards: Some(none),
        instances: vec![4, 9],
    }
}

#[test]
fn a_list_fires_by_its_shape_and_keeps_the_battlemaster() {
    let mut s = Battlefield::default();
    s.apply_list(list(32, 0x77, 0));
    s.apply_list(list(2, 0, 1));
    s.apply_list(list(2, 0, 0));
    s.apply_list(list(3, 0x88, 0));
    assert_eq!(
        s.events,
        [
            "NPC_PVPQUEUE_ANYWHERE",
            "PVPQUEUE_ANYWHERE_SHOW",
            "BATTLEFIELDS_CLOSED",
            "BATTLEFIELDS_SHOW",
            "BATTLEFIELDS_SHOW",
        ]
    );
    assert_eq!(s.battlemaster(), Some(0x88));
    let v = list_view(&s);
    assert_eq!((v.bg_type, v.instances.as_slice()), (3, &[4, 9][..]));
    assert_eq!(v.holiday.win_arena, 25);
    assert!(!v.random.has_win);
    assert_eq!(
        s.leashed(),
        None,
        "leashed once its window's event goes out"
    );
    assert_eq!(s.take_events().len(), 5);
    assert_eq!(s.leashed(), Some(0x88));
    s.close();
    assert_eq!((s.battlemaster(), s.leashed()), (None, None));
}

#[test]
fn verdicts_name_their_game_errors() {
    assert_eq!(
        verdict_key(-2, false, false),
        Some("ERR_GROUP_JOIN_BATTLEGROUND_DESERTERS")
    );
    assert_eq!(
        verdict_key(-12, true, false),
        Some("ERR_BATTLEGROUND_JOIN_TIMED_OUT")
    );
    assert_eq!(
        verdict_key(-11, false, false),
        Some("ERR_BATTLEGROUND_JOIN_FAILED")
    );
    assert_eq!(verdict_key(-15, false, false), Some("ERR_IN_NON_RANDOM_BG"));
    assert_eq!(verdict_key(-1, false, true), None);
    assert_eq!(
        verdict_key(2, false, true),
        Some("ERR_GROUP_JOIN_BATTLEGROUND_S")
    );
    assert_eq!(
        verdict_key(99, false, false),
        Some("ERR_GROUP_JOIN_BATTLEGROUND_FAIL")
    );
}

#[test]
fn the_world_pvp_queue_takes_over_only_when_idle() {
    let mut w = WorldPvp::default();
    let in_zone = |z: u32| z == 4197;
    w.apply(
        &Packet::BfQueueResponse {
            battle_id: 1,
            zone: 4197,
            accepted: 1,
            logging_in: 0,
            warmup: 1,
        },
        in_zone,
    );
    assert_eq!((w.battle_id, w.status, w.zone), (1, 1, 4197));
    // Another battle cannot take over a queued one.
    w.apply(
        &Packet::BfEntryInvite {
            battle_id: 2,
            zone: 9,
            deadline: 50,
        },
        in_zone,
    );
    assert_eq!((w.battle_id, w.status, w.deadline), (1, 1, 0));
    w.apply(
        &Packet::BfEntryInvite {
            battle_id: 1,
            zone: 4197,
            deadline: 50,
        },
        in_zone,
    );
    assert_eq!((w.status, w.deadline), (2, 50));
    w.apply(
        &Packet::BfEntered {
            battle_id: 1,
            raw_bytes: [1, 0],
            clear_afk: 1,
        },
        in_zone,
    );
    assert_eq!(w.status, 0);
    assert!(w.cleared_afk);
    let names: Vec<_> = w.events.iter().map(|(e, _)| *e).collect();
    assert_eq!(
        names,
        [
            "BATTLEFIELD_MGR_QUEUE_REQUEST_RESPONSE",
            "BATTLEFIELD_MGR_ENTRY_INVITE",
            "BATTLEFIELD_MGR_ENTRY_INVITE",
            "BATTLEFIELD_MGR_ENTERED",
        ]
    );
    assert_eq!(
        w.events[0].1,
        [
            ScriptValue::Int(1),
            ScriptValue::Bool(true),
            ScriptValue::Bool(true),
            ScriptValue::Bool(true),
            ScriptValue::Bool(false),
        ]
    );
}

fn status(status: u32, bg_type: u32, tail: StatusTail) -> Status {
    Status {
        arena_type: 0x82,
        raw_arena_flag: 0x0e,
        bg_type,
        raw_word: 0x1f90,
        min_level: 70,
        max_level: 79,
        client_instance: 5,
        rated: 1,
        status,
        tail,
    }
}

#[test]
fn the_queue_names_and_clocks_each_slot_like_the_status_getter() {
    let table = Battlemasters::new(
        vec![BattlemasterType {
            id: 32,
            name: "Random Battleground".into(),
            ..Default::default()
        }],
        Vec::new(),
    );
    let now = Instant::now();
    let mut q = BattlefieldQueue::default();
    q.apply(
        0,
        Some(status(
            1,
            32,
            StatusTail::Queued {
                average_wait_ms: 40_000,
                waited_ms: u32::MAX,
            },
        )),
        |_| 0,
        now,
    );
    q.apply(
        1,
        Some(status(
            2,
            32,
            StatusTail::Invited {
                map: 489,
                raw_unknown: 0,
                expires_ms: 80_500,
            },
        )),
        |_| 0,
        now,
    );
    q.apply(7, None, |_| 0, now);
    assert_eq!(q.take_events(), [1, 2]);
    let a = slot_view(q.slots()[0].as_ref(), Some(&table), None, now);
    assert_eq!(a.name.as_deref(), Some("Random Battleground"));
    assert_eq!(
        (a.team_size, a.rated, a.estimated_wait_ms),
        (2, true, 40_000)
    );
    assert_eq!(a.time_waited_ms, u32::MAX, "a long wait reads forwards");
    let b = slot_view(q.slots()[1].as_ref(), Some(&table), None, now);
    // A random invite with a zero tail word names its type; whole seconds left.
    assert_eq!(b.name.as_deref(), Some("Random Battleground"));
    assert_eq!(b.port_expiration_secs, 80);
    assert_eq!(
        q.port_command(1, true),
        Some(Bg::Port {
            arena_type: 0x82,
            raw_byte: 0x0e,
            bg_type: 32,
            raw_word: 0x1f90,
            action: 1
        })
    );
    assert_eq!(
        q.port_command(0, false)
            .map(|c| matches!(c, Bg::Port { action: 0, .. })),
        Some(true)
    );
}

#[test]
fn status_three_makes_the_slot_active_until_its_clear() {
    let now = Instant::now();
    let mut q = BattlefieldQueue::default();
    q.apply(
        1,
        Some(status(
            3,
            7,
            StatusTail::InProgress {
                map: 572,
                raw_unknown: 0,
                auto_leave_ms: 60_000,
                elapsed_ms: 2_000,
                arena_faction: 1,
            },
        )),
        |m| if m == 572 { 4 } else { 0 },
        now,
    );
    assert_eq!(q.active_map(), Some(572));
    assert_eq!(q.active_instance_type(), 4);
    assert!(q.active_rated());
    assert!(q.take_score_dirty());
    assert!(q.run_time_ms(now) >= 2_000);
    assert!(q.instance_expiration_ms(now) > 59_000);
    // The other slot's queue zeroes the instance clocks but keeps the active slot.
    q.apply(0, Some(status(1, 2, StatusTail::Other)), |_| 0, now);
    assert_eq!(q.active_map(), Some(572));
    assert_eq!(q.run_time_ms(now), 0);
    q.apply(1, None, |_| 0, now);
    assert_eq!(q.active_map(), None);
    assert_eq!(q.active_instance_type(), 0);
}

#[test]
fn type_views_resolve_level_holiday_and_side() {
    let t = Battlemasters::new(
        vec![
            BattlemasterType {
                id: 1,
                maps: [30, -1, -1, -1, -1, -1, -1, -1],
                instance_type: 3,
                holiday_world_state: 1941,
                name: "Alterac Valley".into(),
                ..Default::default()
            },
            BattlemasterType {
                id: 32,
                maps: [-1; 8],
                instance_type: 3,
                name: "Random".into(),
                ..Default::default()
            },
        ],
        vec![PvpBracket {
            map: 30,
            bracket: 0,
            min_level: 51,
            max_level: 80,
        }],
    );
    let mut states = WorldStates::default();
    states.write(&[(1941, 1)]);
    let v = type_views(&t, None, &states, 70, 0);
    assert!(v[0].can_enter && v[0].is_holiday && !v[0].has_map);
    assert!(!v[1].can_enter, "random needs level 80");
    assert!(type_views(&t, None, &WorldStates::default(), 80, -1)[1].can_enter);
}

#[test]
fn world_enter_asks_for_the_status_on_the_native_wire() {
    let (tx, rx) = crossbeam_channel::unbounded();
    request_battlefield_status(&NetCommands(tx));
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::BattlefieldStatusRequest)
    ));
    // 12340 routes it to `CMSG_BATTLEFIELD_STATUS` (0x2D3) with an empty body.
    assert_eq!(
        wrathplus_protocol::wrath_battleground::encode(&Bg::Status),
        (0x02d3, Vec::new())
    );
}
