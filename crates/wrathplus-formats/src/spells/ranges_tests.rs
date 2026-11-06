use super::*;

/// The two inputs, as a call passes them: `target` the unit argument, `attack` the caster's
/// auto-attack target that the melee arm looks up itself.
fn reaches(target: Option<f32>, attack: Option<f32>) -> RangeTargets {
    RangeTargets {
        target: target.map(RangeUnit::still),
        attack_target: attack.map(RangeUnit::still),
    }
}

const FORWARD: u32 = 0x1;
const BACKWARD: u32 = 0x2;
/// A unit running forward: the speed `0x987570` answers for it is the run speed, above twice
/// the walk speed.
const RUNNING: UnitMotion = UnitMotion {
    flags: FORWARD,
    speed: 7.0,
    walk_speed: 2.5,
};

fn runner(reach: f32) -> RangeUnit {
    RangeUnit {
        motion: RUNNING,
        ..RangeUnit::still(reach)
    }
}

fn spell(range_index: u32, attributes: u32) -> crate::spells::SpellDisplay {
    crate::spells::SpellDisplay {
        range_index,
        attributes,
        ..Default::default()
    }
}

const MELEE: SpellRange = SpellRange {
    min: 0.0,
    max: 5.0,
    flags: 1,
};
const CHARGE: SpellRange = SpellRange {
    min: 8.0,
    max: 25.0,
    flags: 0,
};
const FIREBALL: SpellRange = SpellRange {
    min: 0.0,
    max: 35.0,
    flags: 0,
};

#[test]
fn min_max_range_follows_the_byte_law() {
    // Two 1.5-reach units (4.333) floor at 5.0,
    let d = spell(2, 0);
    assert_eq!(
        min_max_range(
            &d,
            Some(&MELEE),
            RangeUnit::still(1.5),
            reaches(Some(1.5), None)
        ),
        Some((0.0, MELEE_RANGE_FLOOR))
    );
    // a 4.0 pair (9.333) clears it.
    let (_, max) = min_max_range(
        &d,
        Some(&MELEE),
        RangeUnit::still(4.0),
        reaches(Some(4.0), None),
    )
    .unwrap();
    assert!((max - 9.3333).abs() < 1e-3);
    // With no target the caster's reach counts twice, as in the client: 9.333 alone too.
    let (_, max) = min_max_range(
        &d,
        Some(&MELEE),
        RangeUnit::still(4.0),
        RangeTargets::default(),
    )
    .unwrap();
    assert!((max - 9.3333).abs() < 1e-3);

    // Charge's 8-25 row pads both bounds by the bare reach sum; the 1.3333 is melee-only.
    let (min, max) = min_max_range(
        &d,
        Some(&CHARGE),
        RangeUnit::still(1.5),
        reaches(Some(1.5), None),
    )
    .unwrap();
    assert!((min - (8.0 + 3.0)).abs() < 1e-3);
    assert!((max - (25.0 + 3.0)).abs() < 1e-3);

    // Fireball's 0-35 row pads the max only: the fcomp-vs-0.0 guard keeps the min at zero.
    let (min, max) = min_max_range(
        &d,
        Some(&FIREBALL),
        RangeUnit::still(1.5),
        reaches(Some(1.5), None),
    )
    .unwrap();
    assert_eq!(min, 0.0);
    assert!((max - 38.0).abs() < 1e-3);

    // No target, as the tooltip calls it (`target = NULL`, `0x52e9c2`): the raw bounds.
    assert_eq!(
        min_max_range(
            &d,
            Some(&CHARGE),
            RangeUnit::still(1.5),
            RangeTargets::default()
        ),
        Some((8.0, 25.0))
    );

    // The on-next-swing attribute short-circuits to 100 without reading the row.
    assert_eq!(
        min_max_range(
            &spell(1, 0x400),
            None,
            RangeUnit::still(1.5),
            RangeTargets::default()
        ),
        Some((0.0, ON_NEXT_SWING_RANGE))
    );

    let self_row = SpellRange {
        min: 0.0,
        max: 0.0,
        flags: 0,
    };
    assert_eq!(
        min_max_range(
            &d,
            Some(&self_row),
            RangeUnit::still(1.5),
            RangeTargets::default()
        ),
        None
    );
}

/// The ranged arm pads only for the target passed in (`0x6e35d8 test ecx,ecx`, `0x6e35ec je`),
/// never for the caster's auto-attack target, which the ranged arm does not read.
#[test]
fn the_ranged_arm_pads_only_for_the_explicit_target() {
    let d = spell(4, 0);
    // Fireball's 35 with no target passed but an auto-attack target of reach 1.5: 35, not 38.
    assert_eq!(
        min_max_range(
            &d,
            Some(&FIREBALL),
            RangeUnit::still(1.5),
            reaches(None, Some(1.5))
        ),
        Some((0.0, 35.0))
    );
    // The same row with the target passed, and the caster's reach 1.5: 35 + 1.5 + 1.5.
    assert_eq!(
        min_max_range(
            &d,
            Some(&FIREBALL),
            RangeUnit::still(1.5),
            reaches(Some(1.5), None)
        ),
        Some((0.0, 38.0))
    );
    // An auto-attack target beside it pads nothing: the target passed is the pad's second reach.
    assert_eq!(
        min_max_range(
            &d,
            Some(&FIREBALL),
            RangeUnit::still(1.5),
            reaches(Some(1.5), Some(4.0))
        ),
        Some((0.0, 38.0))
    );
    // A bounded row's min stays unpadded without a target too: Charge's 8-25.
    assert_eq!(
        min_max_range(
            &d,
            Some(&CHARGE),
            RangeUnit::still(1.5),
            reaches(None, Some(1.5))
        ),
        Some((8.0, 25.0))
    );
}

/// The melee arm takes the unit target's reach, else the auto-attack target's, else the
/// caster's own (`0x6e3552`-`0x6e3584`, `0x6e3594`), and adds the caster's.
#[test]
fn the_melee_arm_falls_back_from_the_target_to_the_attack_target_to_the_caster() {
    let d = spell(2, 0);
    let max = |target, attack| {
        min_max_range(
            &d,
            Some(&MELEE),
            RangeUnit::still(4.0),
            reaches(target, attack),
        )
        .unwrap()
        .1
    };
    let sum = |reach: f32| reach + 4.0 + COMBAT_REACH_ADD;
    // With no target passed the auto-attack target's 6.0 counts: 6 + 4 + 1.3333.
    assert!((max(None, Some(6.0)) - sum(6.0)).abs() < 1e-3);
    // A target passed wins over it.
    assert!((max(Some(2.0), Some(6.0)) - sum(2.0)).abs() < 1e-3);
    // With neither, the caster's reach counts twice.
    assert!((max(None, None) - sum(4.0)).abs() < 1e-3);
}

/// The predicate is exact for the tooltip's call (`0x52e9c2`, no unit target passed): it is
/// true for a spell and row exactly when some change of the caster's reach or motion, or of the
/// auto-attack target's reach, motion or presence, moves [`min_max_range`]'s answer.
#[test]
fn the_units_predicate_matches_what_min_max_range_reads_with_no_target() {
    let row = |min, max, flags| Some(SpellRange { min, max, flags });
    let rows = [
        None,
        row(0.0, 0.0, 0),
        row(0.0, 0.0, 1),
        row(0.0, 35.0, 0),
        row(8.0, 25.0, 0),
        row(0.0, 5.0, 1),
    ];
    let still = RangeUnit::still;
    let runner = |reach| RangeUnit {
        motion: RUNNING,
        ..still(reach)
    };
    // (caster, auto-attack target) before and after.
    let moves = [
        ((still(1.5), None), (still(4.0), None)),
        (
            (still(1.5), Some(still(1.5))),
            (still(1.5), Some(still(4.0))),
        ),
        (
            (still(1.5), Some(still(1.5))),
            (still(4.0), Some(still(1.5))),
        ),
        ((still(1.5), None), (still(1.5), Some(still(4.0)))),
        ((still(1.5), Some(still(1.5))), (still(1.5), None)),
        // Motion alone: the caster's, then the auto-attack target's.
        (
            (still(1.5), Some(runner(1.5))),
            (runner(1.5), Some(runner(1.5))),
        ),
        (
            (runner(1.5), Some(still(1.5))),
            (runner(1.5), Some(runner(1.5))),
        ),
    ];
    for attributes in [0, 0x4, 0x400, 0x404] {
        let spell = crate::spells::SpellDisplay {
            attributes,
            ..Default::default()
        };
        for row in &rows {
            let answer = |caster, attack_target| {
                let targets = RangeTargets {
                    target: None,
                    attack_target,
                };
                min_max_range(&spell, row.as_ref(), caster, targets)
            };
            let moved = moves
                .iter()
                .any(|&((a, a_attack), (b, b_attack))| answer(a, a_attack) != answer(b, b_attack));
            assert_eq!(
                min_max_range_reads_units(&spell, row.as_ref()),
                moved,
                "attributes {attributes:#x}, row {row:?}"
            );
        }
    }
}

/// `0x5fc350` answers 0 for a current speed above twice the walk speed (`fcompp`, then C0),
/// and 1 for an equal pair; an unordered pair sets C0 too.
#[test]
fn running_is_a_speed_above_twice_the_walk_speed() {
    let at = |speed, walk_speed| UnitMotion {
        flags: FORWARD,
        speed,
        walk_speed,
    };
    assert!(at(7.0, 2.5).is_running(), "a run");
    assert!(!at(2.5, 2.5).is_running(), "a walk");
    assert!(
        !at(5.0, 2.5).is_running(),
        "equal to twice the walk speed is walking"
    );
    assert!(at(5.001, 2.5).is_running());
    assert!(!at(0.0, 2.5).is_running(), "no direction bit reads speed 0");
    assert!(at(f32::NAN, 2.5).is_running(), "unordered sets C0");
    assert!(at(7.0, f32::NAN).is_running());
    // A unit whose speeds never streamed (all zero) is not running.
    assert!(!UnitMotion::default().is_running());
}

/// The moving bonus (`0x6e3648`-`0x6e36a2`) on the melee row: two running units of reach 1.5
/// sum 4.333, which the melee floor raises to 5.0 first, so the bonus makes 7.667.
#[test]
fn two_running_units_earn_the_moving_bonus_on_the_melee_row_after_the_floor() {
    let d = spell(2, 0);
    let max = |caster: RangeUnit, target: RangeUnit| {
        min_max_range(
            &d,
            Some(&MELEE),
            caster,
            RangeTargets {
                target: Some(target),
                attack_target: None,
            },
        )
        .unwrap()
    };
    let running = runner(1.5);
    let (min, both) = max(running, running);
    assert_eq!(min, 0.0, "the min never takes the bonus");
    assert!((both - (MELEE_RANGE_FLOOR + MOVING_RANGE_BONUS)).abs() < 1e-6);
    assert!((both - 7.6667).abs() < 1e-3);
    // The bonus is added after the floor, not folded into the reach sum: a 4.0 pair
    // (9.333) takes it on top.
    let (_, wide) = max(runner(4.0), runner(4.0));
    assert!((wide - (9.3333 + 2.6667)).abs() < 1e-3);
    // One unit walking, or standing, or running backward: none.
    let walking = UnitMotion {
        flags: FORWARD,
        speed: 2.5,
        walk_speed: 2.5,
    };
    let backward = UnitMotion {
        flags: BACKWARD,
        ..RUNNING
    };
    for (name, motion) in [
        ("walking", walking),
        ("standing", UnitMotion::default()),
        ("backward", backward),
        (
            "turning",
            UnitMotion {
                flags: 0x10,
                ..RUNNING
            },
        ),
        (
            "swimming",
            UnitMotion {
                flags: 0x20_0000,
                ..RUNNING
            },
        ),
    ] {
        let slow = RangeUnit {
            motion,
            ..runner(1.5)
        };
        assert_eq!(max(running, slow).1, MELEE_RANGE_FLOOR, "the target {name}");
        assert_eq!(max(slow, running).1, MELEE_RANGE_FLOOR, "the caster {name}");
    }
}

/// The flag test is `0x200d`: forward, either strafe, or jumping, each alone.
#[test]
fn the_moving_test_takes_forward_either_strafe_and_jumping() {
    for (name, flags, bonus) in [
        ("forward", FORWARD, true),
        ("strafe left", 0x4, true),
        ("strafe right", 0x8, true),
        ("jumping", 0x2000, true),
        ("forward and jumping", FORWARD | 0x2000, true),
        ("backward", BACKWARD, false),
        ("root", 0x1000, false),
        ("falling far", 0x4000, false),
        ("still", 0, false),
    ] {
        let unit = RangeUnit {
            motion: UnitMotion { flags, ..RUNNING },
            ..RangeUnit::still(1.5)
        };
        let got = min_max_range(
            &spell(2, 0),
            Some(&MELEE),
            unit,
            RangeTargets {
                target: Some(unit),
                attack_target: None,
            },
        )
        .unwrap()
        .1;
        let want = if bonus {
            MELEE_RANGE_FLOOR + MOVING_RANGE_BONUS
        } else {
            MELEE_RANGE_FLOOR
        };
        assert!((got - want).abs() < 1e-6, "{name}: {got}");
    }
}

/// The bonus reads the unit `esi` holds after both arms: the melee arm's auto-attack target
/// when no target is passed (`0x6e3580`), never that target beside one passed in.
#[test]
fn the_melee_bonus_reads_the_attack_target_when_no_target_is_passed() {
    let d = spell(2, 0);
    let bonus = MELEE_RANGE_FLOOR + MOVING_RANGE_BONUS;
    let call = |target: Option<RangeUnit>, attack_target: Option<RangeUnit>| {
        min_max_range(
            &d,
            Some(&MELEE),
            runner(1.5),
            RangeTargets {
                target,
                attack_target,
            },
        )
        .unwrap()
        .1
    };
    let still = RangeUnit::still(1.5);
    assert!((call(None, Some(runner(1.5))) - bonus).abs() < 1e-6);
    assert_eq!(call(None, Some(still)), MELEE_RANGE_FLOOR);
    // No unit at all: the caster's own reach twice, and no unit to move.
    assert_eq!(call(None, None), MELEE_RANGE_FLOOR);
    // A target passed in takes the place of the auto-attack target for the bonus too.
    assert_eq!(call(Some(still), Some(runner(1.5))), MELEE_RANGE_FLOOR);
    assert!((call(Some(runner(1.5)), Some(still)) - bonus).abs() < 1e-6);
}

/// On a ranged row the bonus needs a target unit passed in, and a player: a creature target
/// earns none (`0x6e3683`-`0x6e3695`), nor does the auto-attack target, which that arm never
/// reads.
#[test]
fn the_ranged_bonus_needs_a_player_target_passed_in() {
    let d = spell(4, 0);
    let player = RangeUnit {
        player: true,
        ..runner(1.5)
    };
    let creature = runner(1.5);
    let ranged = |row: &SpellRange, target: Option<RangeUnit>, attack_target| {
        min_max_range(
            &d,
            Some(row),
            runner(1.5),
            RangeTargets {
                target,
                attack_target,
            },
        )
        .unwrap()
    };
    // Fireball's 35 with the 1.5 + 1.5 pad, and the bonus for a running player only.
    let (min, max) = ranged(&FIREBALL, Some(player), None);
    assert_eq!(min, 0.0);
    assert!((max - (38.0 + MOVING_RANGE_BONUS)).abs() < 1e-4);
    assert_eq!(ranged(&FIREBALL, Some(creature), None), (0.0, 38.0));
    // Charge's min pads and takes no bonus; its max does.
    let (min, max) = ranged(&CHARGE, Some(player), None);
    assert!((min - 11.0).abs() < 1e-4);
    assert!((max - (28.0 + MOVING_RANGE_BONUS)).abs() < 1e-4);
    // A standing player target, or a running player as the auto-attack target alone: none.
    let standing_player = RangeUnit {
        player: true,
        ..RangeUnit::still(1.5)
    };
    assert_eq!(ranged(&FIREBALL, Some(standing_player), None), (0.0, 38.0));
    assert_eq!(ranged(&FIREBALL, None, Some(player)), (0.0, 35.0));
    // The caster must run too.
    let caster = RangeUnit::still(1.5);
    let targets = RangeTargets {
        target: Some(player),
        attack_target: None,
    };
    assert_eq!(
        min_max_range(&d, Some(&FIREBALL), caster, targets),
        Some((0.0, 38.0))
    );
}

/// The on-next-swing short-circuit returns before the bonus (`0x6e350f`).
#[test]
fn the_on_next_swing_range_takes_no_bonus() {
    let targets = RangeTargets {
        target: Some(runner(1.5)),
        attack_target: None,
    };
    assert_eq!(
        min_max_range(&spell(2, 0x400), Some(&MELEE), runner(1.5), targets),
        Some((0.0, ON_NEXT_SWING_RANGE))
    );
}
