//! Build 12340's unit and player readers, over seeded snapshots.

use crate::script::{party::PartyState, party::RaidMemberInfo, UiScript, UnitState};

#[test]
fn the_flag_readers_read_their_descriptor_bits() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            has_object: true,
            is_player: true,
            player_flags: 0x2 | 0x0200_0000,
            class_file: Some("PRIEST".into()),
            guid: 1,
            ..Default::default()
        }),
    );
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            has_object: true,
            dynamic_flags: 0x80,
            flags: 0x0100_0000,
            guid: 2,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(
            "return UnitIsAFK('player') == 1 and UnitIsDND('player') == nil \
             and IsXPUserDisabled() == true and UnitIsTappedByAllThreatList('target') == 1 \
             and UnitIsTappedByAllThreatList('player') == nil \
             and IsReferAFriendLinked('target') == nil and UnitIsSameServer('player', 'target') == 1 \
             and UnitIsPossessed('target') == 1 and UnitIsPossessed('player') == nil"
        )
        .unwrap());
    assert!(s
        .eval::<bool>(
            "local t, h, d = GetAvailableRoles() return t == false and h == true and d == true"
        )
        .unwrap());
    assert!(s.run("UnitIsAFK()").is_err());
    // An attackable AFK player reads nil (`0x60cccd`); DND has no such gate.
    let enemy = UnitState {
        exists: true,
        has_object: true,
        is_player: true,
        can_attack: true,
        player_flags: 0x2 | 0x4,
        guid: 2,
        ..Default::default()
    };
    s.set_unit("target", Some(enemy));
    assert!(s
        .eval::<bool>("return UnitIsAFK('target') == nil and UnitIsDND('target') == 1")
        .unwrap());
    s.set_party(PartyState {
        raid: vec![RaidMemberInfo {
            guid: 2,
            rank: 1,
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return UnitIsRaidOfficer('target') == 1")
        .unwrap());
    assert!(s
        .eval::<bool>("return UnitInBattleground('target') == nil")
        .unwrap());
}

#[test]
fn focus_unit_asks_for_the_named_units_guid_and_clear_focus_for_none() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            guid: 0x42,
            ..Default::default()
        }),
    );
    s.run("FocusUnit('mouseover') FocusUnit()").unwrap();
    assert_eq!(
        s.take_focus_request(),
        None,
        "a token naming no unit asks nothing"
    );
    s.run("FocusUnit('target')").unwrap();
    assert_eq!(s.take_focus_request(), Some(0x42));
    assert_eq!(s.take_focus_request(), None, "taken once");
    s.run("ClearFocus()").unwrap();
    assert_eq!(s.take_focus_request(), Some(0));
}

#[test]
fn selection_colour_friends_and_class_names_follow_build_12340() {
    use crate::script::social::{FriendInfo, SocialState};
    let mut s = UiScript::new().unwrap();
    let unit = |reaction, player_controlled, can_attack| UnitState {
        exists: true,
        reaction,
        player_controlled,
        can_attack,
        ..Default::default()
    };
    s.set_unit("target", Some(unit(2, false, true)));
    s.set_unit("focus", Some(unit(5, true, false)));
    s.set_unit("mouseover", Some(unit(5, true, true)));
    for check in [
        "local r, g, b, a = UnitSelectionColor('target') return r == 1 and g == 0 and b == 0 and a == 1",
        "local r, g, b = UnitSelectionColor('focus') return r == 0 and g == 0 and b == 1",
        "local r, g, b = UnitSelectionColor('mouseover') return r == 1 and g == 0 and b == 0",
        "local r, g, b = UnitSelectionColor('party4') return r == 0 and g == 0 and b == 1",
    ] {
        assert!(s.eval::<bool>(check).unwrap(), "{check}");
    }
    assert!(s.run("UnitSelectionColor()").is_err());
    let friend = |name: &str, connected| FriendInfo {
        name: name.into(),
        connected,
        ..Default::default()
    };
    s.set_social(SocialState {
        friends: vec![friend("Alia", true), friend("Brog", false)],
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("local n, on = GetNumFriends() return n == 2 and on == 1")
        .unwrap());
    s.set_localized_class_names(vec![("PRIEST".into(), "Priest".into(), "Priestess".into())]);
    assert!(s
        .eval::<bool>(
            "local m, f = FillLocalizedClassList({}), FillLocalizedClassList({}, true) \
             return m.PRIEST == 'Priest' and f.PRIEST == 'Priestess'"
        )
        .unwrap());
    assert!(s.run("FillLocalizedClassList(1)").is_err());
}

#[test]
fn the_lfg_timers_answer_their_aura_expiry_or_nothing() {
    let mut s = UiScript::new().unwrap();
    assert!(s
        .eval::<bool>("return select('#', GetLFGDeserterExpiration()) == 0 and select('#', GetLFGRandomCooldownExpiration()) == 0")
        .unwrap());
    s.set_player_auras(vec![crate::script::AuraState {
        spell_id: 71041,
        expiration_time: 950.5,
        ..Default::default()
    }]);
    assert!(s
        .eval::<bool>("return GetLFGDeserterExpiration() == 950.5 and select('#', GetLFGRandomCooldownExpiration()) == 0")
        .unwrap());
}

/// The friends list asks the link queries by name (`FriendsFrame.lua:99`, `:117`): a held unit of
/// that name answers, any other name is nil, and neither raises as a bad token would.
#[test]
fn the_link_queries_take_a_friends_name() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            has_object: true,
            name: Some("Onerogue".into()),
            dynamic_flags: 0x40,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(
            "return IsReferAFriendLinked('onerogue') == 1 and IsReferAFriendLinked('target') == 1 \
             and IsReferAFriendLinked('Stranger') == nil and CanSummonFriend('Stranger') == nil"
        )
        .unwrap());
}

/// `PlayerIsPVPInactive` (`0x612e20`) resolves a token or a raid member's name (`0x60abf0`), as the
/// stock battle maps pass a teammate's name (`Blizzard_BattlefieldMinimap.lua:579`), and answers
/// false for nobody rather than raising.
#[test]
fn pvp_inactive_takes_a_token_or_a_raid_members_name() {
    use crate::script::AuraState;
    let mut s = UiScript::new().unwrap();
    s.set_party(PartyState {
        raid: vec![RaidMemberInfo {
            name: "Idler".into(),
            guid: 7,
            ..Default::default()
        }],
        ..Default::default()
    });
    s.set_unit_auras(
        7,
        Some(vec![AuraState {
            spell_id: 43681,
            ..Default::default()
        }]),
    );
    let inactive = |s: &UiScript, who: &str| -> bool {
        s.eval(&format!("return PlayerIsPVPInactive({who:?})"))
            .unwrap()
    };
    assert!(inactive(&s, "Idler"), "a raid member by name");
    assert!(inactive(&s, "idler"), "case-insensitively");
    assert!(!inactive(&s, "Stranger"), "nobody is false, not a raise");
    assert!(!inactive(&s, "target"), "an empty token too");
    assert!(
        s.run("PlayerIsPVPInactive()").is_err(),
        "the usage gate stays"
    );
}
