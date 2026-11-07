use super::*;
use std::collections::HashMap;
use wrathplus_formats::{SpellCastTime, SpellCastTimeCatalog, SpellRange};
use wrathplus_protocol::types::Field;

/// Build 12340's `GlobalStrings.lua` values for the keys the cells fill.
fn get(key: &str) -> Option<String> {
    Some(
        match key {
            "SPELL_RANGE" => "%s yd range",
            "SPELL_RANGE_DUAL" => "%1$s: %2$s yd range",
            "SPELL_RANGE_UNLIMITED" => "Unlimited range",
            "MELEE_RANGE" => "Melee Range",
            "ENEMY" => "Enemy",
            "FRIENDLY" => "Friendly",
            "SPELL_CAST_TIME_SEC" => "%.3g sec cast",
            "SPELL_CAST_TIME_MIN" => "%.3g min cast",
            "SPELL_CAST_TIME_INSTANT" => "Instant cast",
            "SPELL_CAST_TIME_INSTANT_NO_MANA" => "Instant",
            "SPELL_CAST_CHANNELED" => "Channeled",
            "SPELL_ON_NEXT_SWING" => "Next melee",
            "SPELL_ON_NEXT_RANGED" => "Attack speed",
            "SPELL_RECAST_TIME_SEC" => "%.3g sec cooldown",
            "SPELL_RECAST_TIME_MIN" => "%.3g min cooldown",
            _ => return None,
        }
        .to_string(),
    )
}

const ROW: u32 = 4;

fn ranges(min: f32, max: f32, flags: u32, friendly: (f32, f32)) -> SpellRangeCatalog {
    SpellRangeCatalog::from_rows(HashMap::from([(ROW, SpellRange { min, max, flags })]))
        .with_friendly(HashMap::from([(ROW, friendly)]))
}

fn spell(attributes: u32) -> SpellDisplay {
    SpellDisplay {
        range_index: ROW,
        casting_time_index: 1,
        attributes,
        ..Default::default()
    }
}

fn range(d: &SpellDisplay, catalog: &SpellRangeCatalog, scale: Option<f32>) -> RangeCell {
    let mods = SpellModifiers::default();
    range_cell(d, catalog, &mods, RangeUnit::still(1.5), None, scale, &get)
}

#[test]
fn the_range_cell_reads_melee_unlimited_and_the_ranged_minimum() {
    let d = spell(0);
    let one = |s: &str| RangeCell::One(s.to_string());
    assert_eq!(
        range(&d, &ranges(0.0, 30.0, 0, (0.0, 30.0)), None),
        one("30 yd range")
    );
    assert_eq!(
        range(&d, &ranges(8.0, 25.0, 0, (8.0, 25.0)), None),
        one("8-25 yd range")
    );
    assert_eq!(
        range(&d, &ranges(0.0, 0.0, 1, (0.0, 0.0)), None),
        one("Melee Range")
    );
    let unlimited = ranges(0.0, 50_000.0, 0, (0.0, 50_000.0));
    assert_eq!(range(&d, &unlimited, None), one("Unlimited range"));
    assert_eq!(
        range(&d, &ranges(0.0, 0.0, 0, (0.0, 0.0)), None),
        RangeCell::None
    );
    // A ranged row's minimum reads 5 yd past the row's; the ranged weapon scales the maximum.
    let shot = ranges(0.0, 35.0, 2, (0.0, 35.0));
    assert_eq!(range(&spell(0x2), &shot, None), one("5-35 yd range"));
    assert_eq!(range(&spell(0x2), &shot, Some(120.0)), one("5-42 yd range"));
    assert_eq!(
        range(&spell(0), &shot, Some(120.0)),
        one("5-35 yd range"),
        "not a ranged spell"
    );
    assert_eq!(
        range(&spell(0x4), &shot, None),
        RangeCell::None,
        "on next swing"
    );
}

#[test]
fn differing_columns_split_into_two_lines() {
    let d = spell(0);
    let dual = range(&d, &ranges(0.0, 20.0, 0, (0.0, 40.0)), None);
    assert_eq!(
        dual,
        RangeCell::Dual {
            enemy: Some("Enemy: 20 yd range".into()),
            friendly: Some("Friendly: 40 yd range".into()),
        }
    );
    assert_eq!(dual.single(), None);
    assert_eq!(
        dual.lines(Some("100 Mana")).unwrap(),
        [
            (" ".to_string(), Some("Enemy: 20 yd range".to_string())),
            (
                "100 Mana".to_string(),
                Some("Friendly: 40 yd range".to_string())
            ),
        ]
    );
    assert_eq!(
        dual.lines(None).unwrap(),
        [
            ("Enemy: 20 yd range".to_string(), None),
            (
                "Enemy: 20 yd range".to_string(),
                Some("Friendly: 40 yd range".to_string())
            ),
        ],
        "with no cost the enemy text repeats on the second line"
    );
    // The form follows the enemy minimum on both cells.
    let skewed = range(&d, &ranges(0.0, 20.0, 0, (10.0, 40.0)), None);
    assert_eq!(
        skewed,
        RangeCell::Dual {
            enemy: Some("Enemy: 20 yd range".into()),
            friendly: Some("Friendly: 40 yd range".into()),
        }
    );
    assert_eq!(RangeCell::One("x".into()).lines(None), None);
}

#[test]
fn the_ranged_minimum_column_follows_the_spells_target_kind() {
    let shot = ranges(0.0, 30.0, 2, (8.0, 30.0));
    let mut d = spell(0);
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("5-30 yd range".into())
    );
    d.effect_implicit_target_a = [21, 0, 0];
    assert!(d.is_helpful());
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("13-30 yd range".into())
    );
    d.effect_implicit_target_b = [6, 0, 0];
    assert!(d.is_harmful(), "an enemy target type wins");
}

/// `0x7fe1b0` reads the aura at `+0x17c`, not the radius at `+0x170`, for either
/// caster-target arm. Exercise the rendered range cell, with distinct column minimums.
#[test]
fn caster_target_range_uses_the_aura_name_not_the_radius_index() {
    let shot = ranges(0.0, 30.0, 2, (8.0, 30.0));
    for effect in 0..3 {
        for secondary in [false, true] {
            for (aura, radius, expected) in [(4, 0, "5-30 yd range"), (0, 4, "13-30 yd range")] {
                let mut d = spell(0);
                if secondary {
                    d.effect_implicit_target_b[effect] = 1;
                } else {
                    d.effect_implicit_target_a[effect] = 1;
                }
                d.effect_apply_aura[effect] = aura;
                d.effect_radius_index[effect] = radius;
                assert_eq!(
                    range(&d, &shot, None),
                    RangeCell::One(expected.into()),
                    "effect={effect} secondary={secondary} aura={aura} radius={radius}"
                );
            }
        }
    }
}

#[test]
fn caster_aura_range_keeps_explicit_target_and_enemy_precedence() {
    let shot = ranges(0.0, 30.0, 2, (8.0, 30.0));
    let mut d = spell(0);
    d.effect_implicit_target_a[0] = 1;
    d.effect_apply_aura[0] = 4;
    d.effect_implicit_target_b[2] = 21;
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("13-30 yd range".into())
    );
    d.effect_implicit_target_a[1] = 6;
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("5-30 yd range".into())
    );
    d.targets = 0x180;
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("13-30 yd range".into())
    );
    d.targets = 0x80;
    assert_eq!(
        range(&d, &shot, None),
        RangeCell::One("5-30 yd range".into())
    );
}

fn spells_with(row: SpellCastTime) -> Spells {
    let mut spells = Spells::empty_for_tests();
    spells.cast_times = SpellCastTimeCatalog::from_rows([(1, row)]);
    spells
}

fn cast_row(base_ms: i32) -> SpellCastTime {
    SpellCastTime {
        base_ms,
        per_level_ms: 0,
        minimum_ms: base_ms.min(0),
    }
}

fn hasted(speed: f32) -> ObjectFields {
    ObjectFields::from_fields(&[(Field::UnitModCastSpeed, speed.to_bits())])
}

#[test]
fn the_cast_time_takes_haste_then_the_ranged_pad_and_weapon_scale() {
    let mods = SpellModifiers::default();
    let spells = spells_with(cast_row(2500));
    let haste = hasted(0.8);
    let time = |d: &SpellDisplay, unit: Option<&ObjectFields>, ranged| {
        cast_time_ms(d, &spells, &mods, 80, unit, ranged)
    };
    assert_eq!(time(&spell(0), None, None), 2500);
    assert_eq!(time(&spell(0), Some(&haste), None), 2000);
    assert_eq!(
        time(&spell(0x10), Some(&haste), None),
        2500,
        "an ability is not hasted"
    );
    let instant = spells_with(cast_row(0));
    let shot = spell(0x2);
    assert_eq!(cast_time_ms(&shot, &instant, &mods, 80, None, None), 500);
    // 500 ms × ranged attack time 2400 / delay 3000, truncated.
    assert_eq!(
        cast_time_ms(&shot, &instant, &mods, 80, None, Some((3000, 2400))),
        400
    );
    let missing = SpellDisplay {
        casting_time_index: 9,
        ..spell(0x2)
    };
    assert_eq!(
        cast_time_ms(&missing, &instant, &mods, 80, None, None),
        0,
        "no row"
    );
}

#[test]
fn the_cast_ladder_puts_channeled_first() {
    let channeled = SpellDisplay {
        attributes_ex: 0x4,
        attributes: 0x4,
        ..spell(0)
    };
    let cell = |d: &SpellDisplay, ms, cost| cast_cell(d, ms, cost, &get).unwrap();
    assert_eq!(
        cell(&channeled, -1, 0),
        "Channeled",
        "before a negative time and next swing"
    );
    assert_eq!(cell(&spell(0x4), -1, 0), "Instant cast");
    assert_eq!(cell(&spell(0x4), 0, 0), "Next melee");
    assert_eq!(cell(&spell(0x2), 0, 0), "Attack speed");
    assert_eq!(cell(&spell(0), 0, 30), "Instant cast");
    assert_eq!(cell(&spell(0), 0, 0), "Instant");
    assert_eq!(cell(&spell(0), 1500, 0), "1.5 sec cast");
    assert_eq!(cell(&spell(0), 60_000, 0), "1 min cast");
    assert!(has_cast_line(&spell(0)));
    let recipe = SpellDisplay {
        attributes: 0x20,
        effects: [24, 0, 0],
        ..spell(0)
    };
    assert!(!has_cast_line(&recipe), "a crafting recipe");
    let attack = SpellDisplay {
        effects: [78, 0, 0],
        ..spell(0)
    };
    assert!(!has_cast_line(&attack));
}

#[test]
fn the_cooldown_scales_by_weapon_and_hides_on_ex6() {
    let mods = SpellModifiers::default();
    let d = SpellDisplay {
        recovery_ms: 6000,
        category_recovery_ms: 10_000,
        category_weapon_speed: true,
        ..spell(0)
    };
    assert_eq!(cooldown_ms(&d, &mods, None, 0), 10_000);
    assert_eq!(cooldown_ms(&d, &mods, Some(500), 0), 6000, "5000 < 6000");
    assert_eq!(cooldown_ms(&d, &mods, Some(2000), 0), 20_000);
    let ranged = SpellDisplay {
        category_weapon_speed: false,
        ..spell(0x2)
    };
    assert_eq!(cooldown_ms(&ranged, &mods, None, 2400), 2400);
    assert_eq!(
        cooldown_cell(&d, 6000, &get).as_deref(),
        Some("6 sec cooldown")
    );
    assert_eq!(
        cooldown_cell(&d, 60_000, &get).as_deref(),
        Some("1 min cooldown")
    );
    assert_eq!(cooldown_cell(&d, 0, &get), None);
    let hidden = SpellDisplay {
        attributes_ex6: 1,
        ..d
    };
    assert_eq!(cooldown_cell(&hidden, 6000, &get), None);
}
