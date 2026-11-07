//! The `Unit*` binding tests.

use crate::script::{PartyState, PlayerRecord, UiScript, UnitGuids, UnitPowerState, UnitState};

#[path = "is_unit_tests.rs"]
mod is_unit;

fn player() -> UnitState {
    UnitState {
        exists: true,
        name: Some("WrathPlus".into()),
        health: 72,
        max_health: 100,
        level: 12,
        power_type: 1, // rage
        power: 35,
        max_power: 100,
        dead: false,
        reaction: 0,
        race: Some("Night Elf".into()),
        race_file: Some("NightElf".into()),
        class: Some("Warrior".into()),
        class_file: Some("WARRIOR".into()),
        sex: 3, // female
        ..Default::default()
    }
}

/// `0x80dc00`'s order: the `Usage:` check, the not-targeting no-op, the queue.
#[test]
fn spell_target_unit_checks_in_the_references_order() {
    let mut s = UiScript::new().unwrap();
    let usage = |s: &mut UiScript, call: &str| {
        let err = s.run(call).expect_err(call).to_string();
        assert!(
            err.contains(r#"Usage: SpellTargetUnit("unit")"#),
            "{call}: {err}"
        );
    };

    // Not targeting: the argument is still type-checked, then nothing, not even the token check.
    usage(&mut s, "SpellTargetUnit({})");
    usage(&mut s, "SpellTargetUnit()");
    s.run(r#"SpellTargetUnit("not-a-unit") SpellTargetUnit("player")"#)
        .expect("a bad token is no error while not targeting");
    assert!(s.take_spell_target_unit().is_empty(), "and nothing queues");

    // Targeting: any token queues, resolved or not; the resolver never raises (`0x60abf0`).
    s.set_spell_targeting(true);
    usage(&mut s, "SpellTargetUnit(nil)");
    s.run(r#"SpellTargetUnit("not-a-unit") SpellTargetUnit("party1")"#)
        .unwrap();
    assert_eq!(
        s.take_spell_target_unit(),
        vec!["not-a-unit".to_string(), "party1".to_string()]
    );
}

#[test]
fn spell_can_target_unit_answers_per_validated_token() {
    let mut s = UiScript::new().unwrap();
    s.set_unit_guids(&UnitGuids {
        player: 0x10,
        party: [0x21, 0, 0, 0],
        target: 0x30,
        ..Default::default()
    });
    s.set_spell_targetable_units([0x10, 0x21]);
    assert!(s
        .eval::<bool>(r#"return SpellCanTargetUnit("player") == 1"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return SpellCanTargetUnit("PARTY1") == 1"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return SpellCanTargetUnit("target") == nil"#)
        .unwrap());
}

#[test]
fn unit_reaction_reports_the_scale_value_or_nil() {
    let mut s = UiScript::new().unwrap();
    // 4 is neutral.
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            reaction: 4,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitReaction("target", "player")"#)
            .unwrap(),
        4
    );
    // 0 is unresolved, which answers nil.
    s.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            reaction: 0,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(r#"return UnitReaction("player", "target") == nil"#)
        .unwrap());
}

/// A player target branches on `UnitIsEnemy` and an NPC on `UnitReaction` (2 hostile, 5
/// friendly), so a friendly player is blue, never the reaction colour.
#[test]
fn the_name_plate_faction_branch_picks_player_red_blue_over_the_reaction_swatch() {
    let mut s = UiScript::new().unwrap();
    let plate = r#"
            local u = "target"
            if UnitIsPlayer(u) then
                if UnitIsEnemy(u, "player") then return "red" else return "blue" end
            else
                return UnitReaction(u, "player") and "reaction" or "reaction-blue"
            end
        "#;
    let decide = |s: &mut UiScript| s.eval::<String>(plate).unwrap();

    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            is_player: true,
            reaction: 5,
            ..Default::default()
        }),
    );
    assert_eq!(decide(&mut s), "blue");
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            is_player: true,
            reaction: 2,
            ..Default::default()
        }),
    );
    assert_eq!(decide(&mut s), "red");
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            is_player: false,
            reaction: 5,
            ..Default::default()
        }),
    );
    assert_eq!(decide(&mut s), "reaction");
    assert!(s
        .eval::<bool>(r#"return UnitIsPlayer("target") == nil"#)
        .unwrap());
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            is_player: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsPlayer("target")"#).unwrap(),
        1
    );
}

#[test]
fn corpse_can_attack_and_green_range_bindings() {
    use crate::script::PlayerReqState;
    let mut s = UiScript::new().unwrap();
    // `UnitIsCorpse` is an object-type check (`0x5161c0`): a dead unit is not a corpse.
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            dead: true,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(r#"return UnitIsCorpse("target") == nil"#)
        .unwrap());
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            corpse_object: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsCorpse("target")"#).unwrap(),
        1
    );
    // `UnitCanAttack` reads the non-player token's verdict in either argument order.
    assert!(s
        .eval::<bool>(r#"return UnitCanAttack("player", "target") == nil"#)
        .unwrap());
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            can_attack: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitCanAttack("player", "target")"#)
            .unwrap(),
        1
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitCanAttack("target", "player")"#)
            .unwrap(),
        1
    );
    assert!(s
        .eval::<bool>(r#"return UnitIsCorpse("target") == nil"#)
        .unwrap());
    // `GetQuestGreenRange`: the grey band for the player's level, 12 past the table.
    for (pl, want) in [(3u32, 4i64), (46, 9), (120, 12)] {
        s.set_player_req_state(PlayerReqState {
            level: pl,
            ..Default::default()
        });
        assert_eq!(
            s.eval::<i64>("return GetQuestGreenRange()").unwrap(),
            want,
            "green range at level {pl}"
        );
    }
}

#[test]
fn target_unit_queues_the_token_for_the_app_to_resolve() {
    use crate::script::SelectionRequest;
    let mut s = UiScript::new().unwrap();
    assert!(s.take_selection_requests().is_empty());
    // A nil token is not queued.
    s.eval::<()>(r#"TargetUnit("player")"#).unwrap();
    s.eval::<()>(r#"TargetUnit("target")"#).unwrap();
    s.eval::<()>(r#"TargetUnit(nil)"#).unwrap();
    assert_eq!(
        s.take_selection_requests(),
        vec![
            SelectionRequest::Unit("player".into()),
            SelectionRequest::Unit("target".into()),
        ]
    );
    assert!(s.take_selection_requests().is_empty());
}

/// A nil `AssistUnit` argument is dropped like `TargetUnit`'s.
#[test]
fn the_selection_queue_carries_all_three_verbs_in_call_order() {
    use crate::script::SelectionRequest;
    let mut s = UiScript::new().unwrap();
    s.eval::<()>(r#"TargetUnit("party1")"#).unwrap();
    s.eval::<()>(r#"AssistUnit("target")"#).unwrap();
    s.eval::<()>(r#"AssistUnit(nil)"#).unwrap();
    s.eval::<()>("TargetLastEnemy()").unwrap();
    assert_eq!(
        s.take_selection_requests(),
        vec![
            SelectionRequest::Unit("party1".into()),
            SelectionRequest::Assist("target".into()),
            SelectionRequest::LastEnemy,
        ]
    );
    assert!(s.take_selection_requests().is_empty());
}

/// The reverse flag as `0x6f1c10` reads it (default 0): absent, nil, 0 and `"0"` are forward, 1,
/// `true` and `"1"` reverse (`Bindings.xml:458`). The four shims differ only in the mode.
#[test]
fn target_nearest_queues_its_mode_and_reverse_flag() {
    use crate::script::NearestMode::{Enemy, Friend, PartyMember, RaidMember};
    let mut s = UiScript::new().unwrap();
    assert!(s.take_target_nearest_requests().is_empty());
    s.eval::<()>("TargetNearestFriend()").unwrap();
    s.eval::<()>("TargetNearestFriend(1)").unwrap();
    s.eval::<()>("TargetNearestFriend(true)").unwrap();
    s.eval::<()>("TargetNearestFriend(0)").unwrap();
    s.eval::<()>("TargetNearestFriend(nil)").unwrap();
    s.eval::<()>(r#"TargetNearestFriend("0")"#).unwrap();
    s.eval::<()>(r#"TargetNearestEnemy("1")"#).unwrap();
    s.eval::<()>("TargetNearestPartyMember()").unwrap();
    s.eval::<()>("TargetNearestRaidMember(1)").unwrap();
    assert_eq!(
        s.take_target_nearest_requests(),
        vec![
            (Friend, false),
            (Friend, true),
            (Friend, true),
            (Friend, false),
            (Friend, false),
            (Friend, false),
            (Enemy, true),
            (PartyMember, false),
            (RaidMember, true),
        ]
    );
    assert!(s.take_target_nearest_requests().is_empty());
}

#[test]
fn set_unit_is_read_by_the_bindings() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));

    assert_eq!(s.eval::<i64>(r#"return UnitExists("player")"#).unwrap(), 1);
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "WrathPlus"
    );
    assert_eq!(s.eval::<i64>(r#"return UnitHealth("player")"#).unwrap(), 72);
    assert_eq!(
        s.eval::<i64>(r#"return UnitHealthMax("player")"#).unwrap(),
        100
    );
    assert_eq!(s.eval::<i64>(r#"return UnitLevel("player")"#).unwrap(), 12);
    assert!(s
        .eval::<bool>(r#"return UnitIsDead("player") == nil"#)
        .unwrap());
}

#[test]
fn absent_token_reports_not_existing_with_zero_numbers() {
    let s = UiScript::new().unwrap();
    assert!(s
        .eval::<bool>(r#"return UnitExists("target") == nil"#)
        .unwrap());
    assert_eq!(s.eval::<i64>(r#"return UnitHealth("target")"#).unwrap(), 0);
    assert_eq!(
        s.eval::<i64>(r#"return UnitHealthMax("target")"#).unwrap(),
        0
    );
    assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), 0);
    assert!(s
        .eval::<bool>(r#"return UnitName("target") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitIsDead("target") == nil"#)
        .unwrap());
}

#[test]
fn a_dead_unit_reports_dead_and_zero_health() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            name: None,
            health: 0,
            max_health: 3200,
            level: 30,
            dead: true,
            ..Default::default()
        }),
    );
    assert_eq!(s.eval::<i64>(r#"return UnitExists("target")"#).unwrap(), 1);
    assert_eq!(s.eval::<i64>(r#"return UnitIsDead("target")"#).unwrap(), 1);
    assert_eq!(s.eval::<i64>(r#"return UnitHealth("target")"#).unwrap(), 0);
    // A resolved unit with no name yet reads `UNKNOWNOBJECT`, never nil; a bare VM has no
    // GlobalStrings, so this is the literal fallback.
    assert_eq!(
        s.eval::<String>(r#"return UnitName("target")"#).unwrap(),
        "Unknown Being"
    );

    // Feign death (`UNIT_DYNFLAG_DEAD`) over real health: a stranger reads dead with 0 health and
    // power, us and the group (`0x512a30`) read alive and real (`0x60ebe6`, `0x60ee1c`, `0x60f50d`,
    // `0x60f71a`); the maxima carry no gate.
    s.set_unit_guids(&UnitGuids {
        player: 0x10,
        party: [0x21, 0, 0, 0],
        target: 0x30,
        ..Default::default()
    });
    let mut powers = [UnitPowerState::default(); 7];
    powers[0] = UnitPowerState {
        current: 300,
        maximum: 900,
    };
    for (guid, gated) in [(0x30, true), (0x21, false), (0x10, false)] {
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                has_object: true,
                guid,
                health: 500,
                max_health: 3200,
                powers: Some(powers),
                dynamic_flags: 0x20,
                ..Default::default()
            }),
        );
        let want = if gated {
            "0 3200 0 900 1 1"
        } else {
            "500 3200 300 900 nil nil"
        };
        assert_eq!(
            s.eval::<String>(
                r#"return strjoin(" ", UnitHealth("target"), UnitHealthMax("target"),
                    UnitPower("target"), UnitPowerMax("target"),
                    tostring(UnitIsDead("target")), tostring(UnitIsDeadOrGhost("target")))"#
            )
            .unwrap(),
            want,
            "guid {guid:#x}"
        );
    }
}

/// `UnitName` (`0x60e740`) is nil only for a zero guid or an empty `"player"` record, else a
/// string: the name, or `UNKNOWNOBJECT` while it is in flight (`0x609324`). `PetStable.lua:129`
/// concatenates it on `UNIT_PET`, before a called pet's name arrives.
#[test]
fn unitname_reads_unknownobject_for_a_resolved_unit_whose_name_is_in_flight() {
    let mut s = UiScript::new().unwrap();
    let pending = UnitState {
        exists: true,
        has_object: true,
        name: None,
        level: 58,
        guid: 0xF140_0000_0000_0001,
        ..Default::default()
    };
    s.set_unit("pet", Some(pending.clone()));

    // No GlobalStrings: the literal fallback (`0x860fa4`).
    assert_eq!(
        s.eval::<String>(r#"return UnitName("pet")"#).unwrap(),
        "Unknown Being"
    );
    s.run(r#"UNKNOWNOBJECT = "Unknown""#).unwrap();
    assert_eq!(
        s.eval::<String>(r#"return UnitName("pet")"#).unwrap(),
        "Unknown"
    );
    // An empty global is the same miss as an absent one.
    s.run(r#"UNKNOWNOBJECT = """#).unwrap();
    assert_eq!(
        s.eval::<String>(r#"return UnitName("pet")"#).unwrap(),
        "Unknown Being"
    );
    // Two returns, the realm nil.
    assert_eq!(
        s.eval::<i64>(r#"local t = {UnitName("pet")}; return table.getn(t)"#)
            .unwrap(),
        1,
        "a trailing nil is not counted by getn"
    );
    assert!(s
        .eval::<bool>(r#"local _, realm = UnitName("pet"); return realm == nil"#)
        .unwrap());

    let mut named = pending.clone();
    named.name = Some("Snarl".into());
    s.set_unit("pet", Some(named));
    assert_eq!(
        s.eval::<String>(r#"return UnitName("pet")"#).unwrap(),
        "Snarl"
    );

    // `"player"` reads the record, never the snapshot: an empty record is nil.
    s.set_unit("player", Some(pending));
    assert!(s
        .eval::<bool>(r#"return UnitName("player") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitName("PLAYER") == nil"#)
        .unwrap());

    // A token that resolves nothing is guid 0: nil.
    assert!(s
        .eval::<bool>(r#"return UnitName("target") == nil"#)
        .unwrap());
}

#[test]
fn power_bindings_read_the_active_type() {
    let mut s = UiScript::new().unwrap();
    let mut rage = player(); // rage 35/100, 350/1000 on the wire
    let mut powers = [crate::script::UnitPowerState::default(); 7];
    powers[1] = crate::script::UnitPowerState {
        current: 350,
        maximum: 1000,
    };
    rage.powers = Some(powers);
    s.set_unit("player", Some(rage));
    assert_eq!(
        s.eval::<i64>(r#"return UnitPowerType("player")"#).unwrap(),
        1
    );
    assert_eq!(s.eval::<i64>(r#"return UnitMana("player")"#).unwrap(), 35);
    assert_eq!(
        s.eval::<i64>(r#"return UnitManaMax("player")"#).unwrap(),
        100
    );
    // `UnitMana` is `UnitPower` (both `0x60ed40`): the active power, here rage, divided by 10.
    assert!(s
        .eval::<bool>("return UnitMana == UnitPower and UnitManaMax == UnitPowerMax")
        .unwrap());
    // An absent unit is the number 0, never nil: `UnitFrame.lua:122` indexes `ManaBarColor` by it.
    assert_eq!(
        s.eval::<i64>(r#"return UnitPowerType("target")"#).unwrap(),
        0
    );
}

#[test]
fn get_money_reads_the_pushed_purse() {
    let mut s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return GetMoney()").unwrap(), 0);
    s.set_money(123_456);
    assert_eq!(s.eval::<i64>("return GetMoney()").unwrap(), 123_456);
}

#[test]
fn set_unit_none_removes_the_token() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    assert_eq!(s.eval::<i64>(r#"return UnitExists("player")"#).unwrap(), 1);
    s.set_unit("player", None);
    assert!(s
        .eval::<bool>(r#"return UnitExists("player") == nil"#)
        .unwrap());
    assert_eq!(s.eval::<i64>(r#"return UnitHealth("player")"#).unwrap(), 0);
}

#[test]
fn party_frame_predicates_report_1_or_nil() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "party1",
        Some(UnitState {
            exists: true,
            is_connected: true,
            pvp: true,
            is_pvp_ffa: false,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsConnected("party1")"#)
            .unwrap(),
        1
    );
    assert_eq!(s.eval::<i64>(r#"return UnitIsPVP("party1")"#).unwrap(), 1);
    assert!(s
        .eval::<bool>(r#"return UnitIsPVPFreeForAll("party1") == nil"#)
        .unwrap());

    s.set_unit(
        "party1",
        Some(UnitState {
            exists: true,
            is_connected: false,
            pvp: false,
            is_pvp_ffa: true,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(r#"return UnitIsConnected("party1") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitIsPVP("party1") == nil"#)
        .unwrap());
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsPVPFreeForAll("party1")"#)
            .unwrap(),
        1
    );

    assert!(s
        .eval::<bool>(r#"return UnitIsConnected("party2") == nil"#)
        .unwrap());
}

#[test]
fn unit_race_class_sex_report_the_snapshot_or_the_absent_shape() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    assert_eq!(
        s.eval::<(String, String)>(r#"return UnitRace("player")"#)
            .unwrap(),
        ("Night Elf".into(), "NightElf".into())
    );
    assert_eq!(
        s.eval::<(String, String)>(r#"return UnitClass("player")"#)
            .unwrap(),
        ("Warrior".into(), "WARRIOR".into())
    );
    assert_eq!(s.eval::<i64>(r#"return UnitSex("player")"#).unwrap(), 3);
    // An absent token: `nil, nil` for the pairs, but `UnitSex` answers the number 1 (`0x60f9b7`).
    assert!(s
        .eval::<bool>(r#"local a, b = UnitRace("target") return a == nil and b == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"local a, b = UnitClass("target") return a == nil and b == nil"#)
        .unwrap());
    assert_eq!(s.eval::<i64>(r#"return UnitSex("target")"#).unwrap(), 1);
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(r#"local a, b = UnitRace("target") return a == nil and b == nil"#)
        .unwrap());
    // An unstreamed sex byte reads 1 too.
    assert_eq!(s.eval::<i64>(r#"return UnitSex("target")"#).unwrap(), 1);
}

/// `UnitFactionGroup` answers (English, localized), or `nil, nil` for a unit with no side.
#[test]
fn faction_group_pair_and_the_pvp_toggle() {
    let mut s = UiScript::new().unwrap();
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            faction_group: Some("Horde".into()),
            ..Default::default()
        }),
    );
    let (english, localized) = s
        .eval::<(String, String)>(r#"return UnitFactionGroup("target")"#)
        .unwrap();
    // With no localized name fed, the second return falls back to the English one, not nil.
    assert_eq!((english.as_str(), localized.as_str()), ("Horde", "Horde"));

    // A deDE-shaped client: the halves are `InternalName` and `Name0`, and stock builds texture
    // paths from the English first (`TargetFrame.lua:198`, `BattlefieldFrame.lua:195`).
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            faction_group: Some("Horde".into()),
            faction_group_localized: Some("Horde-Allianz".into()),
            ..Default::default()
        }),
    );
    let (english, localized) = s
        .eval::<(String, String)>(r#"return UnitFactionGroup("target")"#)
        .unwrap();
    assert_eq!(
        (english.as_str(), localized.as_str()),
        ("Horde", "Horde-Allianz"),
        "the FIRST return is the English InternalName — a texture path is built from it"
    );

    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            faction_group: None,
            ..Default::default()
        }),
    );
    assert!(s
        .eval::<bool>(r#"local a, b = UnitFactionGroup("target") return a == nil and b == nil"#)
        .unwrap());
    // An absent unit, a recognised token naming nothing, answers the same.
    assert!(s
        .eval::<bool>(r#"local a = UnitFactionGroup("party4") return a == nil"#)
        .unwrap());

    assert_eq!(s.take_pvp_toggles(), 0, "nothing queued yet");
    s.run("TogglePVP() TogglePVP()").unwrap();
    assert_eq!(
        s.take_pvp_toggles(),
        2,
        "one send per call, never collapsed"
    );
    assert_eq!(s.take_pvp_toggles(), 0, "the drain empties the queue");
}

/// The word table at `0x850424` by gated rank answers a string for every input: an absent token
/// reads index 0, "normal", never nil.
#[test]
fn unit_classification_reads_the_five_word_table() {
    let mut s = UiScript::new().unwrap();

    for (rank, word) in [
        (0, "normal"),
        (1, "elite"),
        (2, "rareelite"),
        (3, "worldboss"),
        (4, "rare"),
    ] {
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                rank,
                ..UnitState::default()
            }),
        );
        assert_eq!(
            s.eval::<String>(r#"return UnitClassification("target")"#)
                .unwrap(),
            word,
            "rank {rank}"
        );
    }

    s.set_unit("target", None);
    assert_eq!(
        s.eval::<String>(r#"return UnitClassification("target")"#)
            .unwrap(),
        "normal",
        "an absent unit is normal, not nil"
    );
    assert_eq!(
        s.eval::<String>(r#"return UnitClassification()"#).unwrap(),
        "normal",
        "a missing token is normal, not nil"
    );
}

// ── `UnitAffectingCombat` and `TargetByName` ────────────────────────────────────────────────────

/// Not in combat and no such unit share one nil arm: `0x517e48` and `0x517e5c` both jump to
/// `0x517e73`.
#[test]
fn unit_affecting_combat_is_one_or_nil_and_hides_the_missing_unit() {
    let mut s = UiScript::new().unwrap();
    let mut hot = player();
    hot.in_combat = true;
    s.set_unit("player", Some(hot));
    s.set_unit("target", Some(player())); // exists, peaceful

    assert_eq!(
        s.eval::<i64>(r#"return UnitAffectingCombat("player")"#)
            .unwrap(),
        1,
        "the number 1, never a boolean"
    );
    assert!(s
        .eval::<bool>(r#"return UnitAffectingCombat("target") == nil"#)
        .unwrap());
    assert!(
        s.eval::<bool>(r#"return UnitAffectingCombat("party3") == nil"#)
            .unwrap(),
        "an unresolvable token is the SAME nil a peaceful unit gives"
    );
    // A number passes the gate (`lua_isstring`) as the token "5", which names nobody.
    assert!(s
        .eval::<bool>("return UnitAffectingCombat(5) == nil")
        .unwrap());
}

/// The usage raise is `luaL_error` (`0x6f4940`), which does not return.
#[test]
fn unit_affecting_combat_raises_on_a_bad_argument() {
    let s = UiScript::new().unwrap();
    for call in ["UnitAffectingCombat()", "UnitAffectingCombat({})"] {
        let err = s
            .eval::<mlua::Value>(&format!("return {call}"))
            .unwrap_err();
        assert!(
            format!("{err}").contains(r#"Usage: UnitAffectingCombat("unit")"#),
            "{call} must raise the usage line, got {err}"
        );
    }
}

/// `exactMatch`, which the slash command cannot pass, turns off the resolver's
/// longest-common-prefix tier.
#[test]
fn target_by_name_queues_the_name_and_the_exact_flag() {
    let mut s = UiScript::new().unwrap();
    assert!(s.take_target_by_name_requests().is_empty());

    s.run(r#"TargetByName("Rag")"#).unwrap();
    s.run(r#"TargetByName("Ragnaros", 1)"#).unwrap();
    s.run(r#"TargetByName("Ragnaros", 0)"#).unwrap();
    s.run(r#"TargetByName("Ragnaros", true)"#).unwrap();
    s.run("TargetByName(5)").unwrap(); // a number stringifies, as `0x6f3690` does
    assert_eq!(
        s.take_target_by_name_requests(),
        vec![
            ("Rag".to_string(), false),
            ("Ragnaros".to_string(), true),
            ("Ragnaros".to_string(), false),
            ("Ragnaros".to_string(), true),
            ("5".to_string(), false),
        ]
    );
    assert!(s.take_target_by_name_requests().is_empty());

    // A missing name raises: the gate call at `0x489d69` fails to `0x489de1`.
    let err = s.eval::<mlua::Value>("return TargetByName()").unwrap_err();
    assert!(
        format!("{err}").contains(r#"Usage: TargetByName("name")"#),
        "got {err}"
    );
}

/// `0x516cf0` tests `UNIT_FIELD_CHARMEDBY` (fields 10-11) non-zero, not a `UNIT_FIELD_FLAGS` bit.
#[test]
fn unit_is_charmed_answers_one_or_nil_and_only_for_the_charmed_side() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    let mut mc = player();
    mc.charmed = true;
    s.set_unit("target", Some(mc));

    assert_eq!(
        s.eval::<i64>(r#"return UnitIsCharmed("target")"#).unwrap(),
        1
    );
    assert_eq!(
        s.eval::<String>(r#"return type(UnitIsCharmed("target"))"#)
            .unwrap(),
        "number",
        "the reference pushes a number, not a boolean"
    );
    assert_eq!(
        s.arity(r#"UnitIsCharmed("target")"#).unwrap(),
        1,
        "exactly one value on the hit path"
    );

    for token in ["player", "party1"] {
        assert!(
            s.eval::<Option<i64>>(&format!(r#"return UnitIsCharmed("{token}")"#))
                .unwrap()
                .is_none(),
            "{token} is not charmed, so the answer is nil"
        );
        assert_eq!(
            s.eval::<String>(&format!(r#"return type(UnitIsCharmed("{token}"))"#))
                .unwrap(),
            "nil",
            "…nil rather than false or 0"
        );
    }

    assert!(
        s.eval::<Option<i64>>(r#"return UnitIsCharmed("player")"#)
            .unwrap()
            .is_none(),
        "a charmER is not charmed; UNIT_FIELD_CHARM is never read by this binding"
    );
}

/// `0x516d40` tests `UNIT_FLAG_PLUS_MOB` (`0x40`) and never the rank getter (`0x605620`). The
/// server sets the bit from the rank, so the two part only while the creature record is missing.
#[test]
fn unit_is_plus_mob_reads_the_flag_bit_and_not_the_rank() {
    let with = |flags: u32, rank: u32| UnitState {
        exists: true,
        has_object: true,
        flags,
        rank,
        ..Default::default()
    };
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));

    // The server sets the bit on `!IsPet() && rank > 0` (`Creature.h:186`), so rare carries it.
    for (rank, word) in [
        (1u32, "elite"),
        (2, "rareelite"),
        (3, "worldboss"),
        (4, "rare"),
    ] {
        s.set_unit("target", Some(with(0x40, rank)));
        assert_eq!(
            s.eval::<i64>(r#"return UnitIsPlusMob("target")"#).unwrap(),
            1,
            "a {word} carries UNIT_FLAG_PLUS_MOB, so the answer is the number 1"
        );
    }
    s.set_unit("target", Some(with(0, 0)));
    assert!(
        s.eval::<Option<i64>>(r#"return UnitIsPlusMob("target")"#)
            .unwrap()
            .is_none(),
        "a normal mob answers nil"
    );

    // The flag without a rank: a creature whose cache record has not arrived.
    s.set_unit("target", Some(with(0x40, 0)));
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsPlusMob("target")"#).unwrap(),
        1,
        "the bit alone decides — no creature-cache rank is consulted"
    );
    // A rank without the flag: a pet, which the server excludes.
    s.set_unit("target", Some(with(0x8, 2)));
    assert!(
        s.eval::<Option<i64>>(r#"return UnitIsPlusMob("target")"#)
            .unwrap()
            .is_none(),
        "no bit, no plus mob — a neighbouring flag (player-controlled) must not read as one"
    );

    // One of the 13 with no `lua_isstring` gate: nil is quiet, and so is any token.
    assert!(s.run("UnitIsPlusMob()").is_ok(), "nil token stays quiet");
    assert!(
        s.run("UnitIsPlusMob(nil)").is_ok(),
        "…and so does an explicit nil"
    );
    assert!(
        s.eval::<Option<i64>>(r#"return UnitIsPlusMob("party5")"#)
            .unwrap()
            .is_none(),
        "recognised but naming nothing — nil, not a raise"
    );
    assert!(s
        .eval::<bool>(r#"return UnitIsPlusMob("bogus") == nil"#)
        .unwrap());
}

/// The resolver (`0x60abf0`) compares every literal with `_strnicmp`, folding ASCII case.
#[test]
fn a_unit_token_resolves_whatever_its_case() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    let mut tgt = player();
    tgt.name = Some("Hogger".into());
    s.set_unit("target", Some(tgt));

    for spelling in ["player", "Player", "PLAYER", "PlAyEr"] {
        assert_eq!(
            s.eval::<String>(&format!(r#"return UnitName("{spelling}")"#))
                .unwrap(),
            "WrathPlus",
            "UnitName(\"{spelling}\") must resolve"
        );
        assert_eq!(
            s.eval::<i64>(&format!(r#"return UnitLevel("{spelling}")"#))
                .unwrap(),
            12
        );
        assert_eq!(
            s.eval::<i64>(&format!(r#"return UnitExists("{spelling}")"#))
                .unwrap(),
            1
        );
    }
    for spelling in ["target", "Target", "TARGET"] {
        assert_eq!(
            s.eval::<String>(&format!(r#"return UnitName("{spelling}")"#))
                .unwrap(),
            "Hogger"
        );
    }

    // `pick_unit_token`'s own `"player"` test folds too, or `"Player"` would read as the other
    // unit.
    assert!(
        s.eval::<bool>(
            r#"return UnitIsEnemy("Player", "target") == UnitIsEnemy("player", "target")"#
        )
        .unwrap(),
        "the directional pick folds case too"
    );

    for spelling in ["Party1", "PARTY1", "raid9", "MouseOver"] {
        assert!(
            !s.eval::<bool>(&format!(r#"return UnitExists("{spelling}")"#))
                .unwrap(),
            "{spelling} was never seated"
        );
    }
}

/// A `"Target"` push must not create an entry shadowing `"target"`.
#[test]
fn seating_a_token_folds_its_key_too() {
    let mut s = UiScript::new().unwrap();
    let mut a = player();
    a.name = Some("First".into());
    s.set_unit("Target", Some(a));
    assert_eq!(
        s.eval::<String>(r#"return UnitName("target")"#).unwrap(),
        "First",
        "a token seated as \"Target\" is readable as \"target\""
    );

    s.set_unit("TARGET", None);
    assert!(
        s.eval::<bool>(r#"return UnitExists("target") == nil"#)
            .unwrap(),
        "removal folds too — no shadowed entry survives"
    );
}

/// 12340's resolver (`0x60abf0`) never raises: a token nothing parses names nobody, as does a
/// recognised one with no unit. "Unknown unit name" is 1.12's string, not 12340's.
#[test]
fn no_unit_token_raises() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    for t in ["bogus", "npctarget", "NPC"] {
        assert!(s
            .eval::<bool>(&format!(r#"return UnitName("{t}") == nil"#))
            .unwrap());
    }

    for absent in [
        "party5",
        "raid17",
        "pet",
        "mouseover",
        "partypet2",
        "raidpet3",
    ] {
        assert!(
            s.eval::<Option<String>>(&format!(r#"return UnitName("{absent}")"#))
                .unwrap()
                .is_none(),
            "{absent} is recognised and absent — nil, not a raise"
        );
    }
    // A recognised prefix with a suffix is quiet too: the compares stop at either NUL.
    for junk in ["playerfoo", "raidx", "petx", "targetish"] {
        assert!(
            s.eval::<Option<String>>(&format!(r#"return UnitName("{junk}")"#))
                .unwrap()
                .is_none(),
            "{junk} matches a PREFIX, so it is a quiet nil — not a raise. This is the leg an \
             implementation that raises on \"did not resolve\" gets wrong."
        );
    }
    // Quiet nil: the empty string passes the `lua_isstring` gate and stops at the resolver's
    // empty guard.
    assert!(s.run(r#"UnitName("")"#).is_ok());

    // An absent or nil argument raises `Usage: UnitName("unit")` (`0x850ee0`) from `UnitName`'s
    // own gate (`0x517048`).
    assert!(s.run("UnitName()").is_err(), "absent argument raises");
    assert!(s.run("UnitName(nil)").is_err(), "nil argument raises");
    // A number passes the gate as "5", a token naming nobody.
    assert!(s.eval::<bool>("return UnitName(5) == nil").unwrap());

    // Three of the 13 with no gate, where nil is quiet.
    for quiet in ["UnitExists", "UnitIsVisible", "UnitClassification"] {
        assert!(
            s.run(&format!("{quiet}()")).is_ok(),
            "{quiet} is one of the 13 with no gate — nil must stay quiet"
        );
    }

    // `UnitIsUnit` resolves through the name-tolerant `0x60abf0`: an unknown token is nil, not an
    // error (12340 `0x60c3d0`).
    assert!(s
        .eval::<bool>(r#"return UnitIsUnit("player", "bogus") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitIsUnit("bogus", "player") == nil"#)
        .unwrap());
    assert!(s.run(r#"UnitIsUnit("player", "party3")"#).is_ok());
}

/// The resolver compares bytes with an ASCII fold, so a non-ASCII token matches no base and no
/// name, where slicing it as a `str` would panic mid-character.
#[test]
fn a_multibyte_unit_token_names_nobody_rather_than_panicking() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));

    // Each puts a multibyte character across a prefix length: 3, 4, 5 or 6 bytes.
    for token in ["pé", "раid", "playér", "targét", "мышь", "日本語のトークン"] {
        let chunk = format!(r#"return UnitName("{token}") == nil"#);
        assert!(s.eval::<bool>(&chunk).unwrap(), "{token}");
    }
    assert!(s
        .eval::<Option<String>>(r#"return UnitName("playerfoo")"#)
        .unwrap()
        .is_none());
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "WrathPlus"
    );
}

/// `0x516030` tests only that the client holds the object, so an out-of-range party member exists
/// but is not visible.
#[test]
fn unit_is_visible_is_object_presence_not_existence() {
    let mut s = UiScript::new().unwrap();

    s.set_unit(
        "party1",
        Some(UnitState {
            exists: true,
            has_object: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsVisible("party1")"#).unwrap(),
        1,
        "a held object is visible — and the return is the NUMBER 1, never a boolean"
    );

    // Out of range: the roster has the member, the object manager does not.
    s.set_unit(
        "party1",
        Some(UnitState {
            exists: true,
            has_object: false,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<Option<i64>>(r#"return UnitIsVisible("party1")"#)
            .unwrap(),
        None,
        "no object -> nil, even though the token still exists"
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitExists("party1")"#).unwrap(),
        1,
        "...and UnitExists must NOT have moved with it — the roster fallback is the difference"
    );

    // pfUI's portrait test (`api/unitframes.lua:1873`).
    assert!(
        s.eval::<bool>(
            r#"return (not UnitIsVisible("party1") or not UnitIsConnected("party1")) and true or false"#
        )
        .unwrap(),
        "the portrait branch fires for an out-of-range member"
    );

    assert_eq!(
        s.eval::<Option<i64>>(r#"return UnitIsVisible("party4")"#)
            .unwrap(),
        None
    );
}

/// `0x519c90` and `0x519d00` test `UNIT_DYNAMIC_FLAGS` bits `0x4` and `0x8` behind object
/// presence, and unlike `UnitIsVisible` both raise `Usage:` on a bad argument.
#[test]
fn the_tapped_pair_is_two_masks_of_one_field_and_raises_on_a_bad_argument() {
    let mut s = UiScript::new().unwrap();
    let set = |s: &mut UiScript, tapped, by_player| {
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                has_object: true,
                tapped,
                tapped_by_player: by_player,
                ..Default::default()
            }),
        );
    };

    set(&mut s, false, false);
    assert_eq!(
        s.eval::<Option<i64>>(r#"return UnitIsTapped("target")"#)
            .unwrap(),
        None
    );

    // Tapped by someone else: the grey bar (pfUI `api/unitframes.lua:2012`).
    set(&mut s, true, false);
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsTapped("target")"#).unwrap(),
        1,
        "the return is the NUMBER 1, never a boolean"
    );
    assert_eq!(
        s.eval::<Option<i64>>(r#"return UnitIsTappedByPlayer("target")"#)
            .unwrap(),
        None
    );
    assert!(
        s.eval::<bool>(
            r#"return (UnitIsTapped("target") and not UnitIsTappedByPlayer("target")) and true or false"#
        )
        .unwrap(),
        "someone else's kill — the conjunction addons actually draw"
    );

    set(&mut s, true, true);
    assert!(
        !s.eval::<bool>(
            r#"return (UnitIsTapped("target") and not UnitIsTappedByPlayer("target")) and true or false"#
        )
        .unwrap(),
        "my own tap is not greyed"
    );

    // No object with the bit set, which the feed never builds: it sets `tapped` only from a live
    // descriptor, so the binding reads the field as given.
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            has_object: false,
            tapped: true,
            tapped_by_player: false,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsTapped("target")"#).unwrap(),
        1,
        "the binding reads the field it is given; the conjunct is enforced at the FEED, and this \
         assertion records which of the two owns it"
    );

    // Shape A, nil included: the resolver never raises, so the gate is the binding's.
    for bad in ["{}", "true", "nil", "", "function() end"] {
        assert!(
            s.run(&format!("UnitIsTapped({bad})")).is_err(),
            "UnitIsTapped({bad}) must raise — shape A, not the neighbour's shape C"
        );
        assert!(
            s.run(&format!("UnitIsTappedByPlayer({bad})")).is_err(),
            "UnitIsTappedByPlayer({bad}) must raise too — the pair shares its gate"
        );
    }
}

/// `0x516210`: the player object's `PLAYER_FLAGS & 0x1`, or the resolved guid is the group
/// leader's; each leg is asserted with the other dead.
#[test]
fn unit_is_party_leader_ors_two_legs_and_answers_one_when_solo() {
    let mut s = UiScript::new().unwrap();
    const ME: u64 = 0x1111;
    const THEM: u64 = 0x2222;

    // Leg 1 alone: the flag, as on a stranger who leads their own party.
    s.set_party(PartyState {
        leader_guid: 0x9999,
        own_guid: 0,
        ..Default::default()
    });
    s.set_unit(
        "target",
        Some(UnitState {
            exists: true,
            has_object: true,
            guid: THEM,
            group_leader: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsPartyLeader("target")"#)
            .unwrap(),
        1,
        "the server's PLAYER_FLAGS bit answers on its own"
    );

    // Leg 2 alone: an out-of-range member, with no object to read, whose guid is our leader.
    s.set_party(PartyState {
        leader_guid: THEM,
        own_guid: 0,
        ..Default::default()
    });
    let party = [THEM, ME, 0, 0];
    s.set_unit_guids(&UnitGuids {
        party,
        ..Default::default()
    });
    s.set_unit(
        "party1",
        Some(UnitState {
            exists: true,
            has_object: false,
            guid: THEM,
            group_leader: false,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsPartyLeader("party1")"#)
            .unwrap(),
        1,
        "the GUID compare answers where there is no descriptor to read a flag from"
    );

    s.set_unit(
        "party2",
        Some(UnitState {
            exists: true,
            has_object: true,
            guid: ME,
            group_leader: false,
            ..Default::default()
        }),
    );
    assert_eq!(
        s.eval::<Option<i64>>(r#"return UnitIsPartyLeader("party2")"#)
            .unwrap(),
        None
    );

    // No zero guard (`0x60c7cb`; `IsPartyLeader`, `0x52ccd0`, has one): solo, the leader is 0,
    // and so is the guid of nil or of a token that does not parse (`0x60b021`), so they answer 1;
    // a recognised token naming nobody is the sentinel (`0x60b00c`) and answers nil.
    s.set_party(PartyState::default());
    s.set_unit_guids(&UnitGuids::default());
    for t in ["target", "party1", "party2"] {
        s.set_unit(t, None);
    }
    for (t, want) in [
        ("nil", Some(1)),
        ("'notatoken'", Some(1)),
        ("'party1target'", Some(1)),
    ]
    .into_iter()
    .chain([("'target'", None), ("'party3'", None)])
    {
        let got = s.eval::<Option<i64>>(&format!("return UnitIsPartyLeader({t})"));
        assert_eq!(got.unwrap(), want, "{t}");
    }
}

/// `GetDamageBonusStat()` (`0x48b520`) is the player's class row's field 2 plus one, 0 with no
/// player or no row; it takes no unit, so another unit's class never answers.
#[test]
fn damage_bonus_stat_is_the_player_class_stat_one_based() {
    let mut s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return GetDamageBonusStat()").unwrap(), 0);

    let mut rogue = player();
    rogue.damage_bonus_stat = Some(1);
    s.set_unit("target", Some(rogue.clone()));
    assert_eq!(
        s.eval::<i64>("return GetDamageBonusStat()").unwrap(),
        0,
        "the target is not the player"
    );
    s.set_unit("player", Some(rogue));
    assert_eq!(s.eval::<i64>("return GetDamageBonusStat()").unwrap(), 2);

    let mut warrior = player();
    warrior.damage_bonus_stat = Some(0);
    s.set_unit("player", Some(warrior));
    assert_eq!(s.eval::<i64>("return GetDamageBonusStat()").unwrap(), 1);

    let mut rowless = player();
    rowless.damage_bonus_stat = None;
    s.set_unit("player", Some(rowless));
    assert_eq!(s.eval::<i64>("return GetDamageBonusStat()").unwrap(), 0);
}

/// Stock calls it unconditionally (`PaperDollFrame.lua:429`, `PaperDollFrame.lua:580`), so the
/// global must exist for every class.
#[test]
fn unit_has_relic_slot_answers_one_or_nil() {
    let mut s = UiScript::new().unwrap();

    let mut druid = player();
    druid.class = Some("Druid".into());
    druid.class_file = Some("DRUID".into());
    druid.has_relic_slot = true;
    s.set_unit("player", Some(druid));

    let mut warrior = player();
    warrior.has_relic_slot = false;
    s.set_unit("target", Some(warrior));

    // The true leg is the number 1 (`0x519ec8`).
    assert_eq!(
        s.eval::<String>(r#"return tostring(UnitHasRelicSlot("player"))"#)
            .unwrap(),
        "1"
    );
    assert_eq!(
        s.eval::<String>(r#"return type(UnitHasRelicSlot("player"))"#)
            .unwrap(),
        "number"
    );
    // The false leg is nil, never false (`0x519edb`).
    assert_eq!(
        s.eval::<String>(r#"return type(UnitHasRelicSlot("target"))"#)
            .unwrap(),
        "nil"
    );
    // No `"player"` fast path: `InspectPaperDollFrame.lua:123` passes the inspected unit.
    assert_eq!(
        s.eval::<String>(r#"return type(UnitHasRelicSlot("party1"))"#)
            .unwrap(),
        "nil"
    );
    assert_eq!(
        s.eval::<String>("return type(UnitHasRelicSlot)").unwrap(),
        "function"
    );
}

/// Each of these carries two `lua_isstring` gates in the table at `0x850438`, so either argument
/// nil raises.
#[test]
fn a_two_token_predicate_raises_on_either_nil_argument() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    s.set_unit("target", Some(player()));

    for verb in [
        "UnitIsUnit",
        "UnitIsEnemy",
        "UnitIsFriend",
        "UnitCanCooperate",
        "UnitCanAttack",
        "UnitReaction",
    ] {
        assert!(
            s.run(&format!(r#"{verb}("player", "target")"#)).is_ok(),
            "{verb} with both tokens is fine"
        );
        assert!(
            s.run(&format!(r#"{verb}("player")"#)).is_err(),
            "{verb} with the SECOND argument absent must raise"
        );
        assert!(
            s.run(&format!(r#"{verb}(nil, "target")"#)).is_err(),
            "{verb} with the FIRST argument nil must raise"
        );
        assert!(s.run(&format!("{verb}()")).is_err(), "{verb} with neither");
    }

    // `GetRaidTargetIndex` takes a token and is gated.
    assert!(s.run(r#"GetRaidTargetIndex("player")"#).is_ok());
    assert!(s.run("GetRaidTargetIndex()").is_err());

    // These take no argument and have no gate; `GetTimeToWellRested` is a stub in the reference
    // that always pushes nil.
    for none in [
        "GetQuestGreenRange",
        "GetRestState",
        "GetXPExhaustion",
        "GetTimeToWellRested",
        "GetBillingTimeRested",
    ] {
        assert!(s.run(&format!("{none}()")).is_ok(), "{none} takes no token");
    }
}

// ── The predicates' return shape ────────────────────────────────────────────────────────────────

/// The reference pushes the number 1 (`lua_pushnumber`, `0x6f3810`) or nil (`0x6f37f0`), and no
/// unit binding calls `lua_pushboolean` (`0x6f39f0`). mlua reads 1 as `true`, so only `type`,
/// `== 1` and `== nil` tell the shapes apart.
#[test]
fn every_unit_predicate_is_one_or_nil_and_never_a_boolean() {
    // (binding, the call, a UnitState that makes it true)
    let table: Vec<(&str, &str, UnitState)> = vec![
        (
            "UnitExists",
            r#"UnitExists("target")"#,
            UnitState {
                exists: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsVisible",
            r#"UnitIsVisible("target")"#,
            UnitState {
                exists: true,
                has_object: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsDead",
            r#"UnitIsDead("target")"#,
            UnitState {
                exists: true,
                dead: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsGhost",
            r#"UnitIsGhost("target")"#,
            UnitState {
                exists: true,
                ghost: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsDeadOrGhost",
            r#"UnitIsDeadOrGhost("target")"#,
            UnitState {
                exists: true,
                dead: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsCorpse",
            r#"UnitIsCorpse("target")"#,
            UnitState {
                exists: true,
                corpse_object: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsPlayer",
            r#"UnitIsPlayer("target")"#,
            UnitState {
                exists: true,
                is_player: true,
                ..Default::default()
            },
        ),
        (
            "UnitPlayerControlled",
            r#"UnitPlayerControlled("target")"#,
            UnitState {
                exists: true,
                player_controlled: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsPlusMob",
            r#"UnitIsPlusMob("target")"#,
            UnitState {
                exists: true,
                flags: 0x40,
                ..Default::default()
            },
        ),
        (
            "UnitIsCharmed",
            r#"UnitIsCharmed("target")"#,
            UnitState {
                exists: true,
                charmed: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsTapped",
            r#"UnitIsTapped("target")"#,
            UnitState {
                exists: true,
                tapped: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsTappedByPlayer",
            r#"UnitIsTappedByPlayer("target")"#,
            UnitState {
                exists: true,
                tapped_by_player: true,
                ..Default::default()
            },
        ),
        (
            "UnitAffectingCombat",
            r#"UnitAffectingCombat("target")"#,
            UnitState {
                exists: true,
                in_combat: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsConnected",
            r#"UnitIsConnected("target")"#,
            UnitState {
                exists: true,
                is_connected: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsPVP",
            r#"UnitIsPVP("target")"#,
            UnitState {
                exists: true,
                pvp: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsPVPFreeForAll",
            r#"UnitIsPVPFreeForAll("target")"#,
            UnitState {
                exists: true,
                is_pvp_ffa: true,
                ..Default::default()
            },
        ),
        (
            "UnitHasRelicSlot",
            r#"UnitHasRelicSlot("target")"#,
            UnitState {
                exists: true,
                has_relic_slot: true,
                ..Default::default()
            },
        ),
        (
            "UnitCanAttack",
            r#"UnitCanAttack("player", "target")"#,
            UnitState {
                exists: true,
                can_attack: true,
                ..Default::default()
            },
        ),
        (
            "UnitIsEnemy",
            r#"UnitIsEnemy("player", "target")"#,
            UnitState {
                exists: true,
                reaction: 1,
                ..Default::default()
            },
        ),
        (
            "UnitIsFriend",
            r#"UnitIsFriend("player", "target")"#,
            UnitState {
                exists: true,
                reaction: 5,
                ..Default::default()
            },
        ),
        (
            "UnitCanCooperate",
            r#"UnitCanCooperate("player", "target")"#,
            UnitState {
                exists: true,
                is_player: true,
                reaction: 5,
                ..Default::default()
            },
        ),
    ];

    for (name, call, live) in table {
        let mut s = UiScript::new().unwrap();
        s.set_unit("player", Some(player()));
        s.set_unit("target", Some(live));

        assert_eq!(
            s.eval::<String>(&format!("return type({call})")).unwrap(),
            "number",
            "{name} pushes a boolean where the reference pushes tag 3 (lua_pushnumber)"
        );
        assert_eq!(
            s.eval::<i64>(&format!("return {call}")).unwrap(),
            1,
            "{name}'s true leg is the number 1"
        );
        assert!(
            s.eval::<bool>(&format!("return {call} == 1")).unwrap(),
            "{name}: the corpus idiom `== 1` must match on the true leg"
        );
        assert!(
            !s.eval::<bool>(&format!("return {call} == true")).unwrap(),
            "{name}: `== true` must NEVER match — the reference cannot push tag 1"
        );
        assert_eq!(
            s.arity(call).unwrap(),
            1,
            "{name} returns exactly one value on the true leg"
        );

        // The false leg: nil, one value. Clearing the token also covers the absent-unit arm.
        let mut s = UiScript::new().unwrap();
        s.set_unit("player", Some(player()));
        s.set_unit("target", None);

        assert!(
            s.eval::<bool>(&format!("return {call} == nil")).unwrap(),
            "{name}'s false leg is nil — a boolean `false` is not nil, and a caller comparing \
             against nil would read the opposite of the truth"
        );
        assert!(
            !s.eval::<bool>(&format!("return {call} == false")).unwrap(),
            "{name}: `== false` must never match"
        );
        assert_eq!(
            s.arity(call).unwrap(),
            1,
            "{name} returns exactly one value on the false leg too — nil, never zero values"
        );
    }
}

/// The addon idiom: under a boolean, neither `== 1` nor `== nil` would ever be true.
#[test]
fn the_equals_one_idiom_branches_correctly_on_a_live_and_a_dead_token() {
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    s.set_unit("target", Some(player()));

    assert_eq!(
        s.eval::<String>(
            r#"if UnitExists("target") == 1 then return "live" else return "dead" end"#
        )
        .unwrap(),
        "live"
    );

    s.set_unit("target", None);
    assert_eq!(
        s.eval::<String>(
            r#"if UnitExists("target") == 1 then return "live" else return "dead" end"#
        )
        .unwrap(),
        "dead"
    );
    assert_eq!(
        s.eval::<String>(
            r#"if UnitExists("target") == nil then return "gone" else return "here" end"#
        )
        .unwrap(),
        "gone",
        "the `== nil` half — the comparison a boolean INVERTS"
    );
}

/// `UnitOnTaxi` (`0x60f350`) answers the number 1 or nil from any held unit's flag and raises
/// `Usage:` from its gate; `SetPortraitTexture` (`0x6113e0`) answers a Lua boolean.
#[test]
fn unit_on_taxi_is_one_or_nil_and_raises_the_reference_usage() {
    use mlua::Value::{Boolean, Integer, Nil};
    let mut s = UiScript::new().unwrap();
    s.set_unit("player", Some(player()));
    let ask = |s: &UiScript, call: &str| s.eval::<mlua::Value>(&format!("return {call}")).unwrap();
    assert_eq!(ask(&s, "UnitOnTaxi('player')"), Nil);
    s.set_on_taxi(true);
    assert_eq!(ask(&s, "UnitOnTaxi('player')"), Integer(1));
    assert_eq!(ask(&s, "UnitOnTaxi('target')"), Nil);
    let flying = UnitState {
        exists: true,
        has_object: true,
        flags: 0x0010_0000,
        ..Default::default()
    };
    s.set_unit("target", Some(flying));
    assert_eq!(ask(&s, "UnitOnTaxi('target')"), Integer(1));
    let portrait =
        |t: &str| format!("SetPortraitTexture(CreateFrame('Frame'):CreateTexture(), '{t}')");
    assert_eq!(ask(&s, &portrait("target")), Boolean(true));
    assert_eq!(ask(&s, &portrait("focus")), Boolean(false));

    let err = s.eval::<mlua::Value>("return UnitOnTaxi()").unwrap_err();
    assert!(
        err.to_string().contains(r#"Usage: UnitOnTaxi("unit")"#),
        "the gate's own message, not mlua's type error: {err}"
    );
}

/// The reference's `"player"` arm reads the record (`0x6b1060` reads `0xc79d18`), never the
/// snapshot, so a nameless player push cannot blank the name.
#[test]
fn the_player_record_outlives_every_snapshot() {
    let mut s = UiScript::new().unwrap();
    // Before Enter World the record is empty: nil.
    assert!(s
        .eval::<bool>(r#"return UnitName("player") == nil"#)
        .unwrap());
    // Enter World seeds it from the character row, before any addon file runs.
    s.set_player_record(PlayerRecord {
        name: "Nelprifour".into(),
        race: Some(("Night Elf".into(), "NightElf".into())),
        class: Some(("Priest".into(), "PRIEST".into())),
        sex: 2,
        faction_group: Some(("Alliance".into(), "Alliance".into())),
    });
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "Nelprifour",
        "the buffer answers with no player snapshot at all — the reference runs addon OnUpdate \
         for many frames with no local player object and this verb still answers"
    );
    // The descriptor has landed, but the name cache missed our own guid.
    let nameless = UnitState {
        exists: true,
        has_object: true,
        name: None,
        level: 5,
        ..Default::default()
    };
    s.set_unit("player", Some(nameless));
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "Nelprifour",
        "a nameless snapshot must not reach this verb — that push is the reported bug"
    );
    assert_eq!(
        s.eval::<String>(r#"return UnitRace("player")"#).unwrap(),
        "Night Elf",
        "the same holds for the other three fields of the record"
    );
    assert_eq!(
        s.eval::<String>(r#"local _, t = UnitClass("player"); return t"#)
            .unwrap(),
        "PRIEST",
        "UnitClass's second return is the UPPERCASE token"
    );
    assert_eq!(s.eval::<i64>(r#"return UnitSex("player")"#).unwrap(), 2);
    let side = r#"return table.concat({ UnitFactionGroup("PLAYER") }, ",")"#;
    assert_eq!(s.eval::<String>(side).unwrap(), "Alliance,Alliance");
    // `UnitLevel` reads the snapshot: the record's level accessor (`0x5abe00`) has no caller.
    assert_eq!(s.eval::<i64>(r#"return UnitLevel("player")"#).unwrap(), 5);

    // Removing the token, as at logout, keeps the record.
    s.set_unit("player", None);
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "Nelprifour"
    );
    assert!(
        s.eval::<bool>(r#"return UnitExists("player") == nil"#)
            .unwrap(),
        "the name and the unit are different questions — which is exactly what the buffer buys"
    );

    // An empty seed is refused: nothing empties the reference's record.
    s.set_player_record(PlayerRecord::default());
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "Nelprifour"
    );
    assert_eq!(
        s.eval::<String>(r#"return UnitRace("player")"#).unwrap(),
        "Night Elf"
    );

    // A named push updates the record, so a push never leaves the two disagreeing.
    let mut other = player();
    other.name = Some("Onewarrior".into());
    s.set_unit("player", Some(other));
    assert_eq!(
        s.eval::<String>(r#"return UnitName("player")"#).unwrap(),
        "Onewarrior"
    );
    assert_eq!(
        s.eval::<String>(r#"local _, t = UnitClass("player"); return t"#)
            .unwrap(),
        "WARRIOR",
        "a push that carries the field updates it; one that does not leaves it standing"
    );
    assert_eq!(
        s.eval::<i64>(r#"return UnitSex("player")"#).unwrap(),
        3,
        "player() is female — the record follows the push"
    );

    // Two returns, the realm nil.
    assert_eq!(
        s.eval::<i64>(r#"local t = {UnitName("player")}; return table.getn(t)"#)
            .unwrap(),
        1
    );
    // The record path folds case too.
    assert_eq!(
        s.eval::<String>(r#"return UnitName("PLAYER")"#).unwrap(),
        "Onewarrior"
    );
}

/// An all-zero record, which a bare VM here can reach: `UnitName` nil (`0x6b1060`), `UnitRace` and
/// `UnitClass` `nil, nil` as race and class 0 have no row, and `UnitSex` 2, as `0x60f93c` indexes
/// its table (`0x9fe7ec`) unchecked.
#[test]
fn an_unseeded_player_record_answers_the_references_four_ways() {
    let s = UiScript::new().unwrap();
    assert!(s
        .eval::<bool>(r#"return UnitName("player") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitRace("player") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"local _, t = UnitRace("player"); return t == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"return UnitClass("player") == nil"#)
        .unwrap());
    assert!(s
        .eval::<bool>(r#"local _, t = UnitClass("player"); return t == nil"#)
        .unwrap());
    assert_eq!(
        s.eval::<i64>(r#"return UnitSex("player")"#).unwrap(),
        2,
        "the one of the four with no bounds check — an all-zero record reads male, not nil"
    );
    // The record arm needs the whole token: `"playerfoo"` goes to the resolver.
    assert!(s
        .eval::<bool>(r#"return UnitName("playerfoo") == nil"#)
        .unwrap());
    assert_eq!(
        s.eval::<i64>(r#"return UnitSex("playerfoo")"#).unwrap(),
        1,
        "…which for UnitSex is `[0x9fe7f4]`, 1, not the record's 2 (`0x60f9b7`)"
    );
}
