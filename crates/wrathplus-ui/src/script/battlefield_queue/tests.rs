use super::*;
use crate::script::UiScript;

fn vm() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "player",
        Some(crate::script::UnitState {
            exists: true,
            name: Some("Probe".into()),
            ..Default::default()
        }),
    );
    s
}

fn ty(id: u32, min_level: u32, instance_type: u32) -> BattlegroundTypeView {
    BattlegroundTypeView {
        id,
        name: format!("Type {id}"),
        instance_type,
        groups_allowed: id != 7,
        max_group_size: 5,
        min_level,
        can_enter: true,
        is_holiday: false,
        description: Some(format!("About {id}")),
        has_map: true,
    }
}

fn types() -> Vec<BattlegroundTypeView> {
    vec![
        ty(1, 51, 3),
        ty(2, 10, 3),
        ty(3, 20, 3),
        ty(4, 10, 4),
        ty(7, 61, 3),
        ty(32, 80, 3),
    ]
}

#[test]
fn the_type_list_sorts_random_first_then_holiday_then_level() {
    let mut s = vm();
    let mut t = types();
    t[4].is_holiday = true;
    s.set_battleground_types(t);
    // The start-up sort ignores holidays: random, then minimum level, then id.
    let names = |s: &UiScript| {
        s.eval::<String>(
            "local t = {} for i = 1, GetNumBattlegroundTypes() do \
             local n, _, h, r, id = GetBattlegroundInfo(i) t[#t+1] = id end \
             return table.concat(t, ',')",
        )
        .unwrap()
    };
    assert_eq!(names(&s), "32,2,3,1,7");
    s.run("SortBGList()").unwrap();
    assert_eq!(names(&s), "32,7,2,3,1");
    let (n, enter, holiday, random, id): (String, Option<i64>, Option<i64>, Option<i64>, i64) =
        s.eval("return GetBattlegroundInfo(2.9)").unwrap();
    assert_eq!(
        (n.as_str(), enter, holiday, random, id),
        ("Type 7", Some(1), Some(1), None, 7)
    );
    assert_eq!(s.arity("GetBattlegroundInfo(9)").unwrap(), 0);
}

#[test]
fn the_listed_type_drives_info_arena_and_group_answers() {
    let mut s = vm();
    s.set_battleground_types(types());
    assert_eq!(
        s.arity("GetBattlefieldInfo()").unwrap(),
        0,
        "no listed type"
    );
    s.run("RequestBattlegroundInstanceInfo(4)").unwrap();
    assert_eq!(
        s.take_battlefield_requests(),
        [BattlefieldRequest::List { bg_type: 1 }]
    );
    let got = s
        .eval::<String>(
            "local n, d, g, e, h, r = GetBattlefieldInfo() \
             return n .. '|' .. d .. '|' .. g .. '|' .. tostring(e) .. '|' .. tostring(r)",
        )
        .unwrap();
    assert_eq!(got, "Type 1|About 1|5|1|nil");
    assert!(
        s.eval::<Option<i64>>("return CanJoinBattlefieldAsGroup()")
            .unwrap()
            == Some(1)
    );
    assert!(s
        .eval::<Option<i64>>("return IsBattlefieldArena()")
        .unwrap()
        .is_none());
    assert!(s
        .eval::<bool>("return GetBattlefieldArenaFaction() == nil")
        .unwrap());
    s.set_battlefield_arena_faction(true);
    assert_eq!(
        s.eval::<i64>("return GetBattlefieldArenaFaction()")
            .unwrap(),
        1
    );
    s.set_battlefield_list(BattlefieldListView {
        bg_type: 4,
        instances: vec![9, 3],
        holiday: BattlegroundRewardsView {
            has_win: true,
            win_honor: 30,
            win_arena: 25,
            loss_honor: 5,
        },
        random: Default::default(),
    });
    assert_eq!(
        s.eval::<Option<i64>>("return IsBattlefieldArena()")
            .unwrap(),
        Some(1)
    );
    let (w, h, a, l, z): (bool, i64, i64, i64, i64) =
        s.eval("return GetHolidayBGHonorCurrencyBonuses()").unwrap();
    assert_eq!((w, h, a, l, z), (true, 30, 25, 5, 0));
    assert!(!s
        .eval::<bool>("return (GetRandomBGHonorCurrencyBonuses())")
        .unwrap());
    s.run("SetSelectedBattlefield(2) JoinBattlefield(2, 1) JoinBattlefield(1, nil, true)")
        .unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedBattlefield()").unwrap(), 2);
    assert_eq!(
        s.eval::<i64>("return GetBattlefieldInstanceInfo(1)")
            .unwrap(),
        9
    );
    assert_eq!(
        s.take_battlefield_requests(),
        [
            BattlefieldRequest::Join {
                index: 1,
                as_group: true,
                rated: false
            },
            BattlefieldRequest::Join {
                index: 0,
                as_group: false,
                rated: true
            },
        ]
    );
}

#[test]
fn status_answers_seven_values_and_the_port_closes_the_listed_window() {
    let mut s = vm();
    let slot = BattlefieldQueueSlot {
        status: 2,
        bg_type: 3,
        name: Some("Arathi Basin".into()),
        instance_id: 12,
        min_level: 20,
        max_level: 29,
        team_size: 0,
        rated: false,
        port_expiration_secs: 80,
        estimated_wait_ms: 30_000,
        time_waited_ms: 5_000,
    };
    s.set_battlefield_queue([slot, Default::default()], 0, false, false);
    assert_eq!(s.arity("GetBattlefieldStatus(1)").unwrap(), 7);
    assert_eq!(s.arity("GetBattlefieldStatus(3)").unwrap(), 7);
    let got = s
        .eval::<String>(
            "local st, n, id, lo, hi, size, rated = GetBattlefieldStatus(1.9) \
             return st .. '|' .. n .. '|' .. id .. '|' .. lo .. '|' .. hi .. '|' .. size \
             .. '|' .. tostring(rated) .. '|' .. GetBattlefieldPortExpiration(1) \
             .. '|' .. GetBattlefieldEstimatedWaitTime(1) .. '|' .. GetBattlefieldTimeWaited(1)",
        )
        .unwrap();
    assert_eq!(got, "confirm|Arathi Basin|12|20|29|0|nil|80|30000|5000");
    assert_eq!(
        s.eval::<String>("return (GetBattlefieldStatus(2))")
            .unwrap(),
        "none"
    );
    assert!(s
        .eval::<Option<i64>>("return AcceptBattlefieldPort(3, 1)")
        .unwrap()
        .is_none());
    s.set_battlefield_list(BattlefieldListView {
        bg_type: 3,
        ..Default::default()
    });
    assert_eq!(s.eval::<i64>("return AcceptBattlefieldPort(1)").unwrap(), 1);
    assert_eq!(
        s.take_battlefield_requests(),
        [BattlefieldRequest::Port {
            slot: 0,
            accept: false
        }]
    );
    let events: Vec<_> = s
        .model_mut()
        .pending_events
        .drain(..)
        .map(|(e, _)| e)
        .filter(|e| e.starts_with("BATTLEFIELDS"))
        .collect();
    assert_eq!(events, ["BATTLEFIELDS_CLOSED"]);
    assert!(s.run("GetBattlefieldStatus('x')").is_err());
    // `0x54daf0`: an accept refused, a catalog error and nil, nothing sent; not inside a
    // battleground (`0x54a700`), nor for a decline.
    let mut player = s.model_ref().unit("player").cloned().unwrap();
    player.flags = 0x800;
    s.set_unit("player", Some(player.clone()));
    s.set_wrath_mirror_timer(0, Some(fatigue()));
    s.take_ui_errors();
    assert!(s
        .eval::<Option<i64>>("return AcceptBattlefieldPort(1, 1)")
        .unwrap()
        .is_none());
    assert_eq!(s.take_ui_errors(), ["ERR_NOT_IN_COMBAT"]);
    player.flags = 0;
    s.set_unit("player", Some(player));
    assert!(s
        .eval::<Option<i64>>("return AcceptBattlefieldPort(1, 1)")
        .unwrap()
        .is_none());
    assert_eq!(s.take_ui_errors(), ["ERR_NOT_WHILE_FATIGUED"]);
    assert_eq!(
        s.eval::<i64>("return AcceptBattlefieldPort(1, 0)").unwrap(),
        1
    );
    s.set_wrath_instance_info(Some(crate::script::WrathInstanceInfo {
        instance_type: 3,
        ..Default::default()
    }));
    assert_eq!(
        s.eval::<i64>("return AcceptBattlefieldPort(1, 1)").unwrap(),
        1
    );
    let ports: Vec<_> = s.take_battlefield_requests();
    assert_eq!(
        ports.len(),
        2,
        "the decline and the accept inside a battleground"
    );
    assert!(s.take_ui_errors().is_empty());
}

fn fatigue() -> crate::script::WrathMirrorTimer {
    crate::script::WrathMirrorTimer {
        value: 60_000,
        max_value: 60_000,
        scale: -1,
        paused: false,
        label: String::new(),
        started: 0.0,
    }
}

#[test]
fn world_pvp_reads_the_pushed_queue_and_states() {
    let mut s = vm();
    assert_eq!(s.arity("GetWintergraspWaitTime()").unwrap(), 0);
    s.set_wrath_server_time_offset(Some(0));
    let now = s.wrath_server_now().unwrap() as i32;
    s.set_world_pvp(WorldPvpView {
        status: 1,
        zone_name: Some("Wintergrasp".into()),
        battle_id: 1,
        deadline: 0,
        can_queue_wintergrasp: true,
        wintergrasp_waiting: true,
        wintergrasp_next: now + 600,
        arena_season: 8,
        previous_arena_season: 7,
        subzone_pvp_poi: true,
    });
    let got = s
        .eval::<String>(
            "local st, z, id, left = GetWorldPVPQueueStatus(1) \
             return st .. '|' .. z .. '|' .. id .. '|' .. left",
        )
        .unwrap();
    assert_eq!(got, "queued|Wintergrasp|1|0");
    assert_eq!(s.arity("GetWorldPVPQueueStatus(2)").unwrap(), 0);
    let wait = s.eval::<i64>("return GetWintergraspWaitTime()").unwrap();
    assert!((598..=600).contains(&wait), "{wait}");
    assert_eq!(
        s.eval::<String>(
            "return tostring(CanQueueForWintergrasp()) .. GetCurrentArenaSeason() \
             .. GetPreviousArenaSeason() .. tostring(IsSubZonePVPPOI())"
        )
        .unwrap(),
        "1871"
    );
    s.run("BattlefieldMgrQueueRequest(1) BattlefieldMgrExitRequest(1.6)")
        .unwrap();
    assert_eq!(
        s.take_battlefield_requests(),
        [
            BattlefieldRequest::BfQueueRequest { battle_id: 1 },
            BattlefieldRequest::BfExit { battle_id: 1 },
        ]
    );
}
