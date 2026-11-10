use std::collections::HashMap;

use crate::script::{
    PartyMemberInfo, PartyState, QuestGreetingInfo, QuestState, RaidMemberInfo, ScriptCall,
    SocialState, UiScript, UnitGuids, UnitState, UnitVehicleView, WrathCharState, WrathVehicles,
};

use super::NearestMode;

fn vm() -> UiScript {
    UiScript::new().unwrap()
}

fn unit(guid: u64, name: &str) -> UnitState {
    UnitState {
        exists: true,
        has_object: true,
        guid,
        name: Some(name.into()),
        level: 80,
        ..Default::default()
    }
}

#[test]
fn vehicle_control_seat_is_a_boolean_and_using_controls_a_flag() {
    let mut s = vm();
    s.set_unit("player", Some(unit(1, "Me")));
    s.set_unit("target", Some(unit(2, "Other")));
    s.set_unit_guids(&UnitGuids {
        player: 1,
        target: 2,
        ..Default::default()
    });
    let seated = |flags| UnitVehicleView {
        seat: Some((flags, 0)),
        ..Default::default()
    };
    s.set_wrath_vehicles(WrathVehicles {
        units: HashMap::from([(1, seated(0x800)), (2, seated(0x2000_0000))]),
        player_seat_flags: Some(0x0080_0800),
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "return UnitInVehicleControlSeat('player') == true \
             and UnitInVehicleControlSeat('target') == false \
             and UnitInVehicleControlSeat('focus') == false \
             and UnitInVehicleControlSeat() == false and IsUsingVehicleControls() == 1"
        )
        .unwrap());
    s.set_wrath_vehicles(WrathVehicles {
        player_seat_flags: Some(0x800),
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return IsUsingVehicleControls() == nil")
        .unwrap());
}

#[test]
fn target_nearest_and_target_totem_queue_their_calls() {
    let mut s = vm();
    s.set_wrath_char_state(WrathCharState {
        totems: [0, 0x77, 0, 0],
        ..Default::default()
    });
    s.run("TargetNearest() TargetNearest(1) TargetTotem(1) TargetTotem('2')")
        .unwrap();
    assert_eq!(
        s.take_script_calls(),
        vec![
            ScriptCall::TargetNearest {
                mode: NearestMode::Any,
                reverse: false
            },
            ScriptCall::TargetNearest {
                mode: NearestMode::Any,
                reverse: true
            },
            ScriptCall::TargetGuid(0x77),
        ]
    );
    let err = s.run("TargetTotem(5)").unwrap_err().to_string();
    assert!(
        err.contains("Totem slot must be in range of 1 to 4"),
        "{err}"
    );
    assert!(s.run("TargetTotem(0)").is_err());
    let err = s.run("TargetTotem()").unwrap_err().to_string();
    assert!(err.contains("Usage: TargetTotem(slot)"), "{err}");
}

#[test]
fn difficulty_and_real_leaders() {
    let mut s = vm();
    assert_eq!(
        s.eval::<f64>("return GetInstanceDifficulty()").unwrap(),
        1.0
    );
    s.set_wrath_char_state(WrathCharState {
        instance_difficulty: 2,
        ..Default::default()
    });
    assert_eq!(
        s.eval::<f64>("return GetInstanceDifficulty()").unwrap(),
        3.0
    );

    // Out of world, both guids zero: only the raid twin, with no zero guard, answers 1.
    assert!(s
        .eval::<bool>("return IsRealPartyLeader() == nil and IsRealRaidLeader() == 1")
        .unwrap());
    s.set_party(PartyState {
        own_guid: 7,
        leader_guid: 7,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return IsRealPartyLeader() == 1 and IsRealRaidLeader() == 1")
        .unwrap());
    s.set_party(PartyState {
        own_guid: 7,
        leader_guid: 8,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return IsRealPartyLeader() == nil and IsRealRaidLeader() == nil")
        .unwrap());
}

#[test]
fn ignore_list_by_name_or_unit() {
    let mut s = vm();
    s.set_social(SocialState {
        ignores: vec!["Spammer".into()],
        ..Default::default()
    });
    s.set_wrath_char_state(WrathCharState {
        ignore_guids: vec![5, 7],
        ..Default::default()
    });
    s.set_unit("target", Some(unit(5, "Spammer")));
    s.set_unit("focus", Some(unit(6, "Friend")));
    // Listed by guid, its name not yet resolved into the names list.
    s.set_unit("mouseover", Some(unit(7, "Unnamed")));
    s.set_unit_guids(&UnitGuids {
        target: 5,
        focus: 6,
        mouseover: 7,
        held: HashMap::from([(7, 0)]),
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "return IsIgnored('spammer') == 1 and IsIgnored('target') == 1 \
             and IsIgnored('focus') == nil and IsIgnored() == nil \
             and IsIgnored('mouseover') == 1 and IsIgnored('Unnamed') == nil \
             and IsIgnoredOrMuted('Spammer') == 1 and IsIgnoredOrMuted('Friend') == nil"
        )
        .unwrap());
}

#[test]
fn ready_check_time_left_counts_whole_seconds_in_a_group() {
    let mut s = vm();
    s.set_now(100.0);
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        0.0
    );
    s.set_party(PartyState {
        own_guid: 1,
        leader_guid: 2,
        members: vec![PartyMemberInfo {
            guid: 2,
            ..Default::default()
        }],
        ..Default::default()
    });
    s.wrath_ready_check_start(2);
    s.set_now(110.5);
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        19.0
    );
    s.set_now(131.0);
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        0.0
    );
    s.set_now(105.0);
    s.set_party(PartyState::default());
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        0.0
    );
    s.set_party(PartyState {
        raid: vec![RaidMemberInfo {
            guid: 1,
            ..Default::default()
        }],
        ..Default::default()
    });
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        25.0
    );
    // The last answer ends the timer.
    s.wrath_ready_check_confirm(1, 1);
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        0.0
    );
    s.wrath_ready_check_start(2);
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        30.0
    );
    s.wrath_ready_check_finish();
    assert_eq!(
        s.eval::<f64>("return GetReadyCheckTimeLeft()").unwrap(),
        0.0
    );
}

#[test]
fn quest_panel_flags_and_greeting_levels() {
    let mut s = vm();
    assert!(s
        .eval::<bool>("return QuestIsDaily() == nil and QuestIsWeekly() == nil")
        .unwrap());
    s.set_wrath_quest_panel_flags(0x1000);
    assert!(s
        .eval::<bool>("return QuestIsDaily() == 1 and QuestIsWeekly() == nil")
        .unwrap());
    s.set_wrath_quest_panel_flags(0x8000);
    assert!(s
        .eval::<bool>("return QuestIsDaily() == nil and QuestIsWeekly() == 1")
        .unwrap());

    s.set_unit("player", Some(unit(1, "Me")));
    let row = |level| QuestGreetingInfo {
        level,
        ..Default::default()
    };
    s.set_quest(Some(QuestState {
        active_info: vec![row(75), row(-1)],
        available_info: vec![row(80), row(60), row(-1)],
        ..Default::default()
    }));
    // A level -1 quest scales with the player: the row answers the signed -1 (`0x58c072`).
    assert_eq!(
        s.eval::<(f64, f64, f64, f64, f64)>(
            "return GetActiveLevel(1), GetActiveLevel(2), GetAvailableLevel(2.9), \
             GetAvailableLevel('1'), GetAvailableLevel(0)"
        )
        .unwrap(),
        (75.0, -1.0, 60.0, 80.0, 0.0)
    );
    // Level 80's band is 8: 60 is trivial, 80 is not, -1 never is (`0x58aba7`), and a missing
    // row reads level 0.
    assert!(s
        .eval::<bool>(
            "return IsAvailableQuestTrivial(1) == nil and IsAvailableQuestTrivial(2) == 1 \
             and IsAvailableQuestTrivial(3) == nil and IsAvailableQuestTrivial(4) == 1 \
             and IsActiveQuestTrivial(1) == nil and IsActiveQuestTrivial(2) == nil \
             and IsActiveQuestTrivial(3) == 1 and GetAvailableQuestInfo(2) == 1"
        )
        .unwrap());
    let err = s.run("GetActiveLevel()").unwrap_err().to_string();
    assert!(err.contains("Usage: GetGetActiveLevel(index)"), "{err}");
    let err = s.run("GetAvailableLevel('x')").unwrap_err().to_string();
    assert!(err.contains("Usage: GetGetAvailableLevel(index)"), "{err}");
    let err = s.run("IsAvailableQuestTrivial()").unwrap_err().to_string();
    assert!(
        err.contains("Usage: IsAvailableQuestTrivial(index)"),
        "{err}"
    );
}
