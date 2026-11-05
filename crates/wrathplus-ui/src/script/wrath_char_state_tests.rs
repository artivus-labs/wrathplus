use std::collections::HashMap;

use super::*;
use crate::script::{PartyMemberInfo, PartyState};

fn vm() -> UiScript {
    UiScript::new().unwrap()
}

fn unit(guid: u64, level: u32) -> UnitState {
    UnitState {
        exists: true,
        has_object: true,
        guid,
        level,
        ..Default::default()
    }
}

#[test]
fn player_predicates_answer_one_or_nil_and_nil_without_a_player() {
    let mut s = vm();
    s.set_wrath_char_state(WrathCharState {
        mount_display: 2402,
        swimming: true,
        flyable: true,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "return IsMounted() == nil and IsSwimming() == nil and IsIndoors() == nil \
             and IsOutdoors() == nil and IsFlyableArea() == 1"
        )
        .unwrap());
    s.set_wrath_char_state(WrathCharState {
        has_player: true,
        mount_display: 2402,
        flying: true,
        stealthed: true,
        outdoors: true,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "return IsMounted() == 1 and IsFlying() == 1 and IsSwimming() == nil \
             and IsStealthed() == 1 and IsOutdoors() == 1 and IsIndoors() == nil \
             and IsFlyableArea() == nil"
        )
        .unwrap());
    s.set_wrath_char_state(WrathCharState {
        has_player: true,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return IsIndoors() == 1 and IsOutdoors() == nil")
        .unwrap());
}

#[test]
fn unit_speed_and_pitch_read_the_extras_and_zero_for_no_unit() {
    let mut s = vm();
    s.set_unit("target", Some(unit(0x10, 80)));
    s.set_wrath_unit_extras(HashMap::from([(
        0x10,
        WrathUnitExtra {
            speed: 7.0,
            pitch: -0.5,
            sanctuary: true,
        },
    )]));
    assert_eq!(
        s.eval::<(f64, f64)>("return GetUnitSpeed('target'), GetUnitPitch('target')")
            .unwrap(),
        (7.0, -0.5)
    );
    assert_eq!(
        s.eval::<(f64, f64)>("return GetUnitSpeed('focus'), GetUnitPitch('nosuchunit')")
            .unwrap(),
        (0.0, 0.0)
    );
    assert!(s.run("GetUnitSpeed()").is_err());
    assert!(s.run("GetUnitPitch({})").is_err());
    assert!(s
        .eval::<bool>(
            "return UnitIsPVPSanctuary('target') == 1 and UnitIsPVPSanctuary('focus') == nil \
             and UnitIsPVPSanctuary() == nil"
        )
        .unwrap());
    assert_eq!(s.pushed_unit_guids(), vec![0x10]);
}

#[test]
fn a_group_members_name_resolves_as_its_token() {
    let mut s = vm();
    let named = |guid, name: &str| UnitState {
        name: Some(name.into()),
        ..unit(guid, 80)
    };
    s.set_unit("player", Some(named(1, "Me")));
    s.set_unit("party1", Some(named(0x21, "Bob")));
    s.set_unit("target", Some(named(0x99, "Stranger")));
    s.set_party(PartyState {
        own_guid: 1,
        members: vec![PartyMemberInfo {
            guid: 0x21,
            ..Default::default()
        }],
        ..Default::default()
    });
    s.set_wrath_unit_extras(HashMap::from([
        (
            1,
            WrathUnitExtra {
                speed: 7.0,
                ..Default::default()
            },
        ),
        (
            0x21,
            WrathUnitExtra {
                speed: 4.5,
                ..Default::default()
            },
        ),
        (
            0x99,
            WrathUnitExtra {
                speed: 2.5,
                ..Default::default()
            },
        ),
    ]));
    assert_eq!(
        s.eval::<(f64, f64, f64, f64)>(
            "return GetUnitSpeed('me'), GetUnitSpeed('BOB'), GetUnitSpeed('Stranger'), \
             GetUnitSpeed('Nobody')"
        )
        .unwrap(),
        (7.0, 4.5, 0.0, 0.0)
    );
}

#[test]
fn feign_death_shows_only_for_our_own_group() {
    let mut s = vm();
    let feigning = |guid| UnitState {
        dynamic_flags: 0x20,
        ..unit(guid, 80)
    };
    s.set_unit("party1", Some(feigning(0x21)));
    s.set_unit("target", Some(feigning(0x99)));
    s.set_unit("focus", Some(unit(0x22, 80)));
    s.set_party(PartyState {
        own_guid: 1,
        members: vec![
            PartyMemberInfo {
                guid: 0x21,
                ..Default::default()
            },
            PartyMemberInfo {
                guid: 0x22,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    assert!(s
        .eval::<bool>(
            "return UnitIsFeignDeath('party1') == 1 and UnitIsFeignDeath('target') == nil \
             and UnitIsFeignDeath('focus') == nil"
        )
        .unwrap());
    let err = s.run("UnitIsFeignDeath()").unwrap_err().to_string();
    assert!(err.contains("Usage: UnitIsFeignDeath(\"unit\")"), "{err}");
}

#[test]
fn trivial_is_a_creature_past_the_grey_band() {
    let mut s = vm();
    s.set_unit("player", Some(unit(1, 80)));
    // Level 80: band 8 (past the table), so 71 is grey and 72 is not.
    s.set_unit("target", Some(unit(2, 71)));
    s.set_unit("focus", Some(unit(3, 72)));
    s.set_unit(
        "mouseover",
        Some(UnitState {
            is_player: true,
            ..unit(4, 10)
        }),
    );
    assert!(s
        .eval::<bool>(
            "return UnitIsTrivial('target') == 1 and UnitIsTrivial('focus') == nil \
             and UnitIsTrivial('mouseover') == nil and UnitIsTrivial('pet') == nil"
        )
        .unwrap());
    // Level 12: band 5 (index 12 / 5 = 2); 6 is grey, 7 is not.
    s.set_unit("player", Some(unit(1, 12)));
    s.set_unit("target", Some(unit(2, 6)));
    s.set_unit("focus", Some(unit(3, 7)));
    assert!(s
        .eval::<bool>("return UnitIsTrivial('target') == 1 and UnitIsTrivial('focus') == nil")
        .unwrap());
    assert!(s.run("UnitIsTrivial(nil)").is_err());
}

#[test]
fn power_regen_adds_the_base_rate_and_the_nudge() {
    let (base, casting) = power_regen(1, 0.0, 0.0);
    assert!((base - (-12.499)).abs() < 1e-6 && (casting - 0.001).abs() < 1e-6);
    let (base, casting) = power_regen(0, 2.5, 0.5);
    assert!((base - 2.501).abs() < 1e-6 && (casting - 0.501).abs() < 1e-6);
    assert!((power_regen(3, 0.0, 0.0).0 - 10.001).abs() < 1e-6);
    assert!((power_regen(9, 1.0, 0.0).0 - 1.001).abs() < 1e-6);

    let mut s = vm();
    s.set_wrath_char_state(WrathCharState {
        power_regen: (3.0, 1.0),
        ..Default::default()
    });
    assert_eq!(
        s.eval::<(f64, f64)>("return GetPowerRegen()").unwrap(),
        (0.0, 0.0)
    );
    s.set_wrath_char_state(WrathCharState {
        has_player: true,
        power_regen: (3.0, 1.0),
        ..Default::default()
    });
    assert_eq!(
        s.eval::<(f64, f64)>("return GetPowerRegen()").unwrap(),
        (3.0, 1.0)
    );
}

#[test]
fn offerer_hearth_and_skill_ups() {
    let mut s = vm();
    assert!(s
        .eval::<bool>("return ResurrectGetOfferer() == nil")
        .unwrap());
    s.set_wrath_char_state(WrathCharState {
        resurrect_offerer: Some("Anna".into()),
        ..Default::default()
    });
    assert_eq!(
        s.eval::<String>("return ResurrectGetOfferer()").unwrap(),
        "Anna"
    );
    assert_eq!(
        s.eval::<i64>(
            "HearthAndResurrectFromArea() HearthAndResurrectFromArea() \
             return select('#', AcceptSkillUps())"
        )
        .unwrap(),
        0
    );
    assert_eq!(s.take_wrath_hearth_and_resurrect(), 2);
    assert_eq!(s.take_wrath_hearth_and_resurrect(), 0);
}

#[test]
fn dismount_clears_the_mount_locally_until_the_display_moves() {
    let mut s = vm();
    s.model_mut().unit_guids.player = 1;
    s.set_unit("player", Some(unit(1, 80)));
    let mounted = |display| WrathCharState {
        has_player: true,
        mount_display: display,
        ..Default::default()
    };
    s.set_wrath_char_state(mounted(2402));
    assert!(s
        .eval::<bool>("local a = IsMounted() Dismount() return a == 1 and IsMounted() == nil")
        .unwrap());
    assert_eq!(
        s.take_script_calls(),
        vec![crate::script::ScriptCall::Dismount]
    );
    // The server has not answered yet: still the same display, still dismounted.
    s.set_wrath_char_state(mounted(2402));
    assert!(s.eval::<bool>("return IsMounted() == nil").unwrap());
    s.set_wrath_char_state(mounted(0));
    s.set_wrath_char_state(mounted(2402));
    assert!(s.eval::<bool>("return IsMounted() == 1").unwrap());
    // On a flight path the dismount sends nothing locally and the mount stays.
    s.set_unit(
        "player",
        Some(UnitState {
            flags: 0x10_0000,
            ..unit(1, 80)
        }),
    );
    assert!(s
        .eval::<bool>("Dismount() return IsMounted() == 1")
        .unwrap());
}

#[test]
fn a_name_splits_at_target_and_pet_suffixes_only() {
    assert_eq!(split_name("Bob-target"), ("Bob", "-target"));
    assert_eq!(split_name("Bob-PET"), ("Bob", "-PET"));
    assert_eq!(split_name("Bob-pet-target"), ("Bob", "-pet-target"));
    assert_eq!(split_name("Bob-Realm-target-"), ("Bob-Realm", "-target-"));
    // `-targettarget` is neither the whole `-target` nor `-target-`, so it stays in the name.
    assert_eq!(split_name("Bob-targettarget"), ("Bob-targettarget", ""));
    let long = "x".repeat(60);
    assert_eq!(split_name(&long), (&long[..48], &long[48..]));
    assert_eq!(
        suffix_hops("-target-pettarget"),
        Some(vec![Hop::Target, Hop::Pet, Hop::Target])
    );
    assert_eq!(suffix_hops("TARGETpet"), Some(vec![Hop::Target, Hop::Pet]));
    assert_eq!(suffix_hops(""), Some(vec![]));
    assert_eq!(suffix_hops("-"), None);
    assert_eq!(suffix_hops("-focus"), None);
}

#[test]
fn a_pet_hop_names_our_or_a_group_members_pet_only() {
    let mut s = vm();
    let named = |guid, name: &str| UnitState {
        name: Some(name.into()),
        ..unit(guid, 80)
    };
    s.set_unit("player", Some(named(1, "Me")));
    s.set_unit("pet", Some(named(2, "Fido")));
    s.set_unit("party1", Some(named(0x21, "Bob")));
    s.set_unit("partypet1", Some(named(0x22, "Rex")));
    s.set_unit("party2", Some(named(0x31, "Ann-Realm")));
    s.set_party(PartyState {
        own_guid: 1,
        members: [0x21, 0x31]
            .map(|guid| PartyMemberInfo {
                guid,
                ..Default::default()
            })
            .to_vec(),
        ..Default::default()
    });
    let speed = |speed| WrathUnitExtra {
        speed,
        ..Default::default()
    };
    s.set_wrath_unit_extras(HashMap::from([
        (2, speed(9.0)),
        (0x22, speed(6.0)),
        (0x31, speed(3.0)),
    ]));
    assert_eq!(
        s.eval::<(f64, f64, f64, f64, f64, f64)>(
            "return GetUnitSpeed('me-pet'), GetUnitSpeed('BOB-PET'), GetUnitSpeed('ann-realm'), \
             GetUnitSpeed('Fido-pet'), GetUnitSpeed('Bob-pets'), GetUnitSpeed('Ann-Realm-pet')"
        )
        .unwrap(),
        // A pet has no pet hop (not a group member), `-pets` is no hop, and Ann has no pet.
        (9.0, 6.0, 3.0, 0.0, 0.0, 0.0)
    );
}
