//! Unheld member record snapshot and DND status propagation.
use super::super::pets::tests::{app, flag, frame, member, party};
use super::*;

#[test]
fn an_out_of_range_member_reads_the_record_and_divides_its_rage() {
    let m = GroupMemberEntry {
        name: "Brisca".into(),
        guid: 0x1234,
        status: member_status::ONLINE,
        flags: 0,
    };
    let record = PartyMemberStatsInfo {
        status: Some(member_status::ONLINE | member_status::DND),
        cur_hp: Some(2400),
        max_hp: Some(3000),
        level: Some(41),
        power_type: Some(1), // POWER_RAGE
        cur_power: Some(570),
        max_power: Some(1000),
        ..PartyMemberStatsInfo::default()
    };
    let s = member_unit_state(
        &m,
        Some(&record),
        None,
        &GroupState::default(),
        None,
        None,
        Default::default(),
    );
    assert_eq!(
        (s.health, s.max_health),
        (2400, 3000),
        "the bars keep their numbers"
    );
    assert_eq!(s.level, 41);
    assert_eq!(
        (s.power_type, s.power, s.max_power),
        (1, 57, 100),
        "rage reads 57/100, not 570/1000 — `UnitMana`'s record leg divides too (0x517744)"
    );
    assert!(s.exists && s.is_player && s.is_connected);
    assert_eq!(s.roster_status, member_status::ONLINE | member_status::DND);

    // No record at all (a real roster always seats one): an existing player with empty bars.
    let bare = member_unit_state(
        &m,
        None,
        None,
        &GroupState::default(),
        None,
        None,
        Default::default(),
    );
    assert_eq!((bare.health, bare.max_health, bare.power), (0, 0, 0));
    assert!(bare.exists);
}

#[test]
fn dnd_record_status_updates_and_clears_through_party_feed() {
    let mut app = app();
    party(&mut app, 1, true);
    for status in [0x81, 1, 0x80, 0] {
        app.world_mut().resource_mut::<GroupState>().apply_stats(
            member(1),
            false,
            PartyMemberStatsInfo {
                status: Some(status),
                ..Default::default()
            },
        );
        frame(&mut app);
        assert_eq!(
            flag(&mut app, "UnitIsDND('party1')"),
            if status & 0x80 != 0 { Some(1.0) } else { None }
        );
    }
}
