use super::super::validator::{ERR_OUT_OF_RANGE, ERR_TOO_CLOSE};
use super::*;

#[test]
fn press_minimum_first_including_negative_minimum_and_inclusive_endpoints() {
    assert_eq!(refusal((-10.0, 2.0), Some(25.0)), Some(ERR_TOO_CLOSE));
    assert_eq!(refusal((2.0, 10.0), Some(4.0)), None);
    assert_eq!(refusal((2.0, 10.0), Some(100.0)), None);
    assert_eq!(refusal((2.0, 10.0), Some(101.0)), Some(ERR_OUT_OF_RANGE));
}

#[test]
fn press_unordered_passes_while_hover_rejects() {
    for (bounds, d2) in [
        ((2.0, 10.0), f32::NAN),
        ((f32::NAN, 10.0), 25.0),
        ((2.0, f32::NAN), 25.0),
    ] {
        assert_eq!(refusal(bounds, Some(d2)), None);
        assert!(!in_interval(bounds, d2));
    }
    assert!(in_interval((2.0, 10.0), 4.0));
    assert!(in_interval((2.0, 10.0), 100.0));
}

#[test]
fn corpse_columns_follow_spell_polarity_not_corpse_relation() {
    let row = Some(RangeRow {
        flags: 0,
        min: [2.0, 4.0],
        max: [20.0, 40.0],
    });
    for helpful in [false, true] {
        let d = SpellDisplay {
            targets: if helpful { 0x100 } else { 0x80 },
            ..Default::default()
        };
        for relation in [false, true] {
            let inputs = RangeInputs {
                target: RangeTarget::Corpse,
                friendly: relation,
                ..Default::default()
            };
            assert_eq!(
                limits(&d, row, &inputs, &SpellModifiers::default()),
                if helpful { (7.0, 43.0) } else { (5.0, 23.0) }
            );
        }
    }
}

#[test]
fn corpse_melee_resolves_autoattack_but_nonmelee_does_not() {
    let d = SpellDisplay::default();
    let mods = SpellModifiers::default();
    let inputs = RangeInputs {
        target: RangeTarget::Corpse,
        attack_target: Some(RangeUnit::still(10.0)),
        ..Default::default()
    };
    let melee = Some(RangeRow {
        flags: 1,
        min: [0.0; 2],
        max: [0.0; 2],
    });
    assert!(limits(&d, melee, &inputs, &mods).1 > 12.8);
    let nonmelee = Some(RangeRow {
        flags: 0,
        min: [0.0; 2],
        max: [30.0; 2],
    });
    assert_eq!(limits(&d, nonmelee, &inputs, &mods), (0.0, 33.0));
    assert_eq!(
        limits(
            &d,
            nonmelee,
            &RangeInputs {
                target: RangeTarget::None,
                ..inputs
            },
            &mods
        ),
        (0.0, 30.0)
    );
}

#[test]
fn missing_row_differs_from_zero_row_and_next_swing_bypasses_both() {
    let mut d = SpellDisplay::default();
    let mods = SpellModifiers::default();
    let inputs = RangeInputs {
        target: RangeTarget::Unit(RangeUnit::still(1.5)),
        ..Default::default()
    };
    let zero = Some(RangeRow {
        flags: 0,
        min: [0.0; 2],
        max: [0.0; 2],
    });
    assert_eq!(limits(&d, None, &inputs, &mods), (0.0, 0.0));
    assert_eq!(limits(&d, zero, &inputs, &mods), (0.0, 3.0));
    for attr in [4, 0x400] {
        d.attributes = attr;
        assert_eq!(limits(&d, None, &inputs, &mods), (0.0, 100.0));
        assert_eq!(limits(&d, zero, &inputs, &mods), (0.0, 100.0));
        let positioned = RangeInputs {
            self_pos: Some(Vec3::ZERO),
            target_pos: Some(Vec3::X * 101.0),
            ..inputs
        };
        assert_eq!(
            positioned.refusal(&d, &SpellRangeCatalog::default(), &mods),
            Some(ERR_OUT_OF_RANGE)
        );
    }
}

#[test]
fn moving_bonus_uses_12340_flags_not_the_legacy_mask() {
    let row = Some(RangeRow {
        flags: 1,
        ..Default::default()
    });
    for (flags, bonus) in [(0x1000, true), (0x2000, false), (1, true), (2, false)] {
        let runner = RangeUnit {
            reach: 1.5,
            player: true,
            motion: wrathplus_formats::UnitMotion {
                flags,
                speed: 7.0,
                walk_speed: 2.5,
            },
        };
        assert_eq!(unit(runner).running, bonus);
        for target in [
            RangeTarget::Unit(runner),
            RangeTarget::None,
            RangeTarget::Corpse,
        ] {
            let inputs = RangeInputs {
                caster: runner,
                target,
                attack_target: Some(runner),
                ..Default::default()
            };
            let (_, max) = limits(
                &SpellDisplay::default(),
                row,
                &inputs,
                &SpellModifiers::default(),
            );
            assert_eq!(max, if bonus { 5.0 + 8.0 / 3.0 } else { 5.0 });
        }
    }
}
