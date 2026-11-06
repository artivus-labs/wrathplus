use crate::script::{UiScript, UnitPowerState, UnitState};

#[test]
fn wrath_current_power_uses_identity() {
    let mut script = UiScript::new().unwrap();
    let mut unit = UnitState {
        exists: true,
        guid: 42,
        power_type: 1,
        powers: Some(
            [UnitPowerState {
                current: 731,
                maximum: 1250,
            }; 7],
        ),
        ..Default::default()
    };
    script.set_unit("player", Some(unit.clone()));
    script.set_unit("target", Some(unit.clone()));
    assert_eq!(
        script.eval::<u32>("return UnitPower('target',0)").unwrap(),
        731
    );
    // `lua_tointeger` rounds, ties to even: 0.6 is rage (1, tenths), 0.5 mana.
    let rounded = "return UnitPower('target', 0.6), UnitPower('target', '0.5')";
    assert_eq!(script.eval::<(u32, u32)>(rounded).unwrap(), (73, 731));
    unit.guid = 43;
    script.set_unit("target", Some(unit.clone()));
    assert_eq!(
        script.eval::<u32>("return UnitPower('target',0)").unwrap(),
        0
    );
    assert_eq!(
        script.eval::<u32>("return UnitPower('target')").unwrap(),
        73
    );
    assert_eq!(
        script
            .eval::<u32>("return UnitPowerMax('target',0)")
            .unwrap(),
        1250
    );
    unit.dynamic_flags = 0x20;
    script.set_unit("target", Some(unit.clone()));
    assert_eq!(
        script.eval::<u32>("return UnitPower('target')").unwrap(),
        73
    );
    assert_eq!(
        script.eval::<u32>("return UnitPowerMax('target')").unwrap(),
        125
    );
    script.set_unit("player", Some(unit.clone()));
    assert_eq!(
        script.eval::<u32>("return UnitPower('target')").unwrap(),
        73
    );
    unit.dynamic_flags = 0;
    unit.guid = 44;
    script.set_unit("target", Some(unit));
    assert_eq!(
        script.eval::<u32>("return UnitPower('target')").unwrap(),
        73
    );
}

#[test]
fn wrath_power_readers_use_all_resources_and_native_integer_scaling() {
    let mut script = UiScript::new().unwrap();
    script.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            guid: 42,
            has_object: true,
            power_type: 1,
            powers: Some(std::array::from_fn(|slot| UnitPowerState {
                current: [721, 437, 54, 83, 1500, 6, 659][slot],
                maximum: [1000, 1000, 100, 100, 1000000, 6, 1250][slot],
            })),
            ..Default::default()
        }),
    );
    assert_eq!(
        script
            .eval::<Vec<u32>>("local t={} for i=0,6 do t[i+1]=UnitPower('player',i) end return t")
            .unwrap(),
        vec![721, 43, 54, 83, 1, 6, 65]
    );
    assert_eq!(
        script
            .eval::<u32>("return UnitPowerMax('player',6)")
            .unwrap(),
        125
    );
    assert_eq!(
        script
            .eval::<u32>("return UnitPower('player',nil,true)")
            .unwrap(),
        43
    );
    assert_eq!(
        script.eval::<u32>("return UnitPower('player',7)").unwrap(),
        43
    );
    assert_eq!(
        script
            .eval::<u32>("return UnitPower('player','notnumeric')")
            .unwrap(),
        43
    );
    assert_eq!(
        script.eval::<u32>("return UnitPower('player',-2)").unwrap(),
        0
    );
    assert_eq!(
        script.eval::<u32>("return UnitPower('player',8)").unwrap(),
        0
    );
    assert_eq!(script.eval::<u32>("return UnitPower('target')").unwrap(), 0);
    assert_eq!(
        script
            .eval::<(u8, String)>("return UnitPowerType('player')")
            .unwrap(),
        (1, "RAGE".into())
    );
    assert_eq!(
        script
            .eval::<(u8, String)>("return UnitPowerType('target')")
            .unwrap(),
        (0, "".into())
    );
    // 12340 answers `0, ""` for an index with no vehicle power-display row (`0x60f327`).
    let indexed = script.eval::<(u8, String)>("return UnitPowerType('player', 2)");
    assert_eq!(indexed.unwrap(), (0, "".into()));
    assert!(script.run("UnitPowerType()").is_err(), "the Usage gate");
}

#[test]
fn wrath_power_reads_zero_before_the_snapshot() {
    let mut wrath = UiScript::new().unwrap();
    wrath.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            has_object: true,
            power: 100,
            ..Default::default()
        }),
    );
    assert_eq!(wrath.eval::<u32>("return UnitPower('player')").unwrap(), 0);
}
