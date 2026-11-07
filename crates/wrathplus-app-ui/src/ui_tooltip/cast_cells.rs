//! The spell tooltip's range, cast and cooldown cells as build 12340's builder `006238a0` writes
//! them (`0x623ee0`-`0x6245d0`), on `GetMinMaxRange 007ff480`, `GetCastTime 007ff180` (argument 1,
//! unclamped) and the category recovery `007fef60`, each with no unit target: the tooltip passes
//! none.
//!
//! Not applied, labelled: `007ff480`'s moving bonus (its `0x100d` movement test and `00716fa0` are
//! not traced here), which reaches only a melee row whose range a spell modifier moves.

use wrathplus_formats::MELEE_RANGE_FLOOR;
use wrathplus_formats::{RangeUnit, SpellDisplay, SpellRangeCatalog, COMBAT_REACH_ADD};
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::strings::Arg;

use super::keyed;
use crate::app_core::state::Spells;
use crate::spell::{SpellModifiers, SpellsCastTime, OP_COOLDOWN, OP_RANGE};

/// `[0x00a258d8]`: a max at or past it reads `SPELL_RANGE_UNLIMITED`.
const UNLIMITED_YARDS: f32 = 50_000.0;
/// `[0x009ea27c]`, `00482900`'s tolerance: two values closer than it are the same.
const SAME: f32 = f32::from_bits(0x3480_0000);
/// `SpellRange.dbc` `Flags` bit 1: a ranged row, whose tooltip minimum is 5 yd past the column's
/// (`007ff400`, `[0x009ebf34]`).
const RANGE_FLAG_RANGED: u32 = 2;
const ATTR_RANGED: u32 = 0x2;
/// `Attributes & 0x30` (ability, trade spell) and `AttributesEx3 & 0x20000000` skip the haste.
const ATTR_NO_HASTE: u32 = 0x30;
const ATTR_EX3_NO_HASTE: u32 = 0x2000_0000;
const ATTR_EX_CHANNELED: u32 = 0x44;
const ATTR_TRADESPELL: u32 = 0x20;
const ATTR_EX6_NO_COOLDOWN_CELL: u32 = 0x1;
const ATTR_EX6_NO_CATEGORY_MOD: u32 = 0x8000_0000;
const EFFECT_TRADE_SKILL: u32 = 47;
const EFFECT_ATTACK: u32 = 78;
/// The crafting effects that make a trade spell a recipe tooltip, which has no cast line
/// (`0x623a8b`): enchant, create item, create item 2, create random item.
const RECIPE_EFFECTS: [u32; 4] = [53, 24, 157, 59];

/// The worn weapons the cells read, by the template the cache holds.
#[derive(Clone, Copy, Default)]
pub(super) struct Weapons {
    /// The ranged slot's (`17`) delay and `RangedModRange`.
    pub(super) ranged: Option<(u32, f32)>,
    /// The main hand's (`15`) delay.
    pub(super) main_hand_delay: Option<u32>,
}

/// The range cell: none, one cell, or the enemy and friendly cells of `SPELL_RANGE_DUAL`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum RangeCell {
    #[default]
    None,
    One(String),
    Dual {
        enemy: Option<String>,
        friendly: Option<String>,
    },
}

impl RangeCell {
    /// The cost/range double lines, in order, when this cell needs more than the one line the
    /// view's `cost`/`range` pair draws: line A (`0x624230`) puts the enemy cell right of a lone
    /// space, or alone with no cost; line B pairs the cost with the friendly cell. With no cost the
    /// enemy text stays in the left buffer and repeats on line B, as the reference's does.
    pub(super) fn lines(&self, cost: Option<&str>) -> Option<Vec<(String, Option<String>)>> {
        let RangeCell::Dual { enemy, friendly } = self else {
            return None;
        };
        let enemy = enemy.clone().unwrap_or_default();
        let (a, left) = match cost {
            Some(cost) => ((" ".to_string(), Some(enemy)), cost.to_string()),
            None => ((enemy.clone(), None), enemy),
        };
        Some(vec![a, (left, friendly.clone())])
    }

    /// The single cell, for the view's `range`.
    pub(super) fn single(&self) -> Option<String> {
        match self {
            RangeCell::One(text) => Some(text.clone()),
            _ => None,
        }
    }
}

/// The integer yards: `"%d"` (`0x9e289c`) or `"%d-%d"` (`0x854fb4`), `fistp` rounding to nearest.
/// Which form is picked by `pick_min`, the enemy minimum on both cells (`0x62429b` reads it).
fn yards(pick_min: f32, min: f32, max: f32) -> String {
    let yd = |v: f32| f64::from(v).round_ties_even() as i64;
    if pick_min <= 0.0 {
        format!("{}", yd(max))
    } else {
        format!("{}-{}", yd(min), yd(max))
    }
}

/// Whether a range modifier (op 5) reaches `d` (`00800770`): one moves 0 (a flat term) or 1 (a
/// percentage), so the two probes find any.
fn range_modified(d: &SpellDisplay, mods: &SpellModifiers) -> bool {
    let moved = |v: f32| mods.apply_float(d, OP_RANGE, v) != v;
    moved(0.0) || moved(1.0)
}

/// Whether the range cell reads the caster's and auto-attack target's reaches: only a melee row
/// a range modifier reaches; every other melee row is `MELEE_RANGE`.
pub(super) fn range_reads_units(
    d: &SpellDisplay,
    ranges: &SpellRangeCatalog,
    mods: &SpellModifiers,
) -> bool {
    !d.tooltip_omits_range_line()
        && ranges.get(d.range_index).is_some_and(|r| r.is_melee())
        && range_modified(d, mods)
}

/// The range cell (`0x623ef7`-`0x62438a`).
#[allow(clippy::too_many_arguments)]
pub(super) fn range_cell(
    d: &SpellDisplay,
    ranges: &SpellRangeCatalog,
    mods: &SpellModifiers,
    caster: RangeUnit,
    attack_target: Option<RangeUnit>,
    ranged_mod_range: Option<f32>,
    get: &dyn Fn(&str) -> Option<String>,
) -> RangeCell {
    if d.tooltip_omits_range_line() {
        return RangeCell::None;
    }
    let Some(row) = ranges.get(d.range_index) else {
        return RangeCell::None;
    };
    // `007ff380` and `00800770(spell, 5)`: a melee row the range modifier leaves alone.
    if row.is_melee() && !range_modified(d, mods) {
        return keyed(get, "MELEE_RANGE", &[]).map_or(RangeCell::None, RangeCell::One);
    }
    let friendly_pair = ranges.friendly(d.range_index).unwrap_or((row.min, row.max));
    let pair = |friendly: bool| {
        if friendly {
            friendly_pair
        } else {
            (row.min, row.max)
        }
    };
    let column = |friendly: bool| -> (f32, f32) {
        let (row_min, row_max) = pair(friendly);
        let (min, mut max, pad) = if row.is_melee() {
            let reach = attack_target.map_or(caster.reach, |u| u.reach);
            let sum = f64::from(reach) + f64::from(caster.reach) + f64::from(COMBAT_REACH_ADD);
            (0.0, 0.0, (sum as f32).max(MELEE_RANGE_FLOOR))
        } else {
            (row_min, row_max, 0.0)
        };
        if d.attributes & ATTR_RANGED != 0 {
            if let Some(scale) = ranged_mod_range {
                max *= scale * 0.01;
            }
        }
        (min, mods.apply_float(d, OP_RANGE, max) + pad)
    };
    let (mut enemy, mut friend) = (column(false), column(true));
    // `007ff3c0`/`007ff400`: both minimums read 5 yd past the column `0x7fe1b0` picks;
    // its caster-target arm reads EffectApplyAuraName (+0x17c), never EffectRadiusIndex.
    if row.flags & RANGE_FLAG_RANGED != 0 && !row.is_melee() {
        let min = MELEE_RANGE_FLOOR + pair(d.is_helpful()).0;
        enemy.0 = min;
        friend.0 = min;
    }
    let cell = |(min, max): (f32, f32), who: Option<&str>| -> Option<String> {
        if max <= 0.0 {
            return None;
        }
        if max >= UNLIMITED_YARDS {
            return keyed(get, "SPELL_RANGE_UNLIMITED", &[]);
        }
        let text = yards(enemy.0, min, max);
        match who {
            None => keyed(get, "SPELL_RANGE", &[Arg::S(&text)]),
            Some(key) => {
                let who = get(key).unwrap_or_else(|| key.to_string());
                keyed(get, "SPELL_RANGE_DUAL", &[Arg::S(&who), Arg::S(&text)])
            }
        }
    };
    if (enemy.0 - friend.0).abs() < SAME && (enemy.1 - friend.1).abs() < SAME {
        return cell(enemy, None).map_or(RangeCell::None, RangeCell::One);
    }
    RangeCell::Dual {
        enemy: cell(enemy, Some("ENEMY")),
        friendly: cell(friend, Some("FRIENDLY")),
    }
}

/// Whether the cast and cooldown line is written (`0x624357`-`0x62437f`): not a recipe, not a
/// passive or trade-skill spell, not the melee attack.
pub(super) fn has_cast_line(d: &SpellDisplay) -> bool {
    let recipe =
        d.attributes & ATTR_TRADESPELL != 0 && d.effects.iter().any(|e| RECIPE_EFFECTS.contains(e));
    !recipe && !d.passive && !matches!(d.effects[0], EFFECT_TRADE_SKILL | EFFECT_ATTACK)
}

/// `UNIT_MOD_CAST_SPEED`, 1.0 unhasted (the descriptor default).
pub(super) fn cast_speed(unit: &ObjectFields) -> f32 {
    use wrathplus_protocol::types::Field;
    unit.get_float(Field::UnitModCastSpeed, 0).unwrap_or(1.0)
}

/// `GetCastTime 007ff180` with argument 1 (no clamp): the level-scaled row and op 10, the haste
/// of `haste_unit` (the player, or a pet view's pet), then a ranged spell's 500 ms and, for the
/// player, its weapon's `rangedAttackTime / delay` scale (`0x7ff343`-`0x7ff354`, truncated).
pub(super) fn cast_time_ms(
    d: &SpellDisplay,
    spells: &Spells,
    mods: &SpellModifiers,
    level: u32,
    haste_unit: Option<&ObjectFields>,
    player_ranged: Option<(u32, u32)>,
) -> i32 {
    if spells.cast_times.get(d.casting_time_index).is_none() {
        return 0;
    }
    let mut t = spells.cast_time_unclamped_ms(d, level, mods);
    if d.attributes & ATTR_NO_HASTE == 0 && d.attributes_ex3 & ATTR_EX3_NO_HASTE == 0 && t > 0 {
        if let Some(unit) = haste_unit {
            let haste = cast_speed(unit);
            if (haste - 1.0).abs() >= SAME {
                t = (t as f32 * haste).round_ties_even() as i32;
            }
        }
    }
    if d.attributes & ATTR_RANGED != 0 {
        t += 500;
        if let Some((delay, ranged_time)) = player_ranged.filter(|&(d, r)| d != 0 && r != 0) {
            t = (f64::from(ranged_time) / f64::from(delay as i32) * f64::from(t)) as i32;
        }
    }
    t
}

/// The cast cell (`0x62436c`-`0x6244a6`): a positive time in seconds or minutes, else the first of
/// channeled, a negative time's "Instant", next swing, next ranged, then mana or not.
pub(super) fn cast_cell(
    d: &SpellDisplay,
    ms: i32,
    resolved_cost: u32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if ms > 0 {
        let (key, unit) = if ms > 59_999 {
            ("SPELL_CAST_TIME_MIN", 60_000.0)
        } else {
            ("SPELL_CAST_TIME_SEC", 1000.0)
        };
        return keyed(get, key, &[Arg::F(f64::from(ms as f32 / unit))]);
    }
    let key = if d.attributes_ex & ATTR_EX_CHANNELED != 0 {
        "SPELL_CAST_CHANNELED"
    } else if ms < 0 {
        "SPELL_CAST_TIME_INSTANT"
    } else if d.on_next_swing() {
        "SPELL_ON_NEXT_SWING"
    } else if d.attributes & ATTR_RANGED != 0 {
        "SPELL_ON_NEXT_RANGED"
    } else if d.power_type == 0 && resolved_cost > 0 {
        "SPELL_CAST_TIME_INSTANT"
    } else {
        "SPELL_CAST_TIME_INSTANT_NO_MANA"
    };
    keyed(get, key, &[])
}

/// The cooldown in ms (`0x6243a0`-`0x6243c0`): the larger of op 11 over `RecoveryTime` and
/// `007fef60`'s category recovery. That scales by the main hand's delay for a weapon-speed
/// category (integer, per 1000), adds the player's ranged attack time for a ranged spell, then
/// takes op 11 unless `AttributesEx6 & 0x80000000`. The builder bails with no active player
/// (`006238a0`'s entry), so `007fef60`'s own no-player exit is never reached here.
pub(super) fn cooldown_ms(
    d: &SpellDisplay,
    mods: &SpellModifiers,
    main_hand_delay: Option<u32>,
    ranged_time: u32,
) -> i32 {
    let recovery = mods.apply(d, OP_COOLDOWN, d.recovery_ms as i32);
    let mut category = d.category_recovery_ms as i32;
    if d.category_weapon_speed {
        if let Some(delay) = main_hand_delay {
            category = (delay as i32).wrapping_mul(category) / 1000;
        }
    }
    if d.ranged_speed_cooldown() {
        category = category.wrapping_add(ranged_time as i32);
    }
    if d.attributes_ex6 & ATTR_EX6_NO_CATEGORY_MOD == 0 {
        category = mods.apply(d, OP_COOLDOWN, category);
    }
    recovery.max(category)
}

/// The cooldown cell: none below 1 ms or with `AttributesEx6 & 1` (`0x6243c6`).
pub(super) fn cooldown_cell(
    d: &SpellDisplay,
    ms: i32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if ms < 1 || d.attributes_ex6 & ATTR_EX6_NO_COOLDOWN_CELL != 0 {
        return None;
    }
    let (key, unit) = if ms > 59_999 {
        ("SPELL_RECAST_TIME_MIN", 60_000.0)
    } else {
        ("SPELL_RECAST_TIME_SEC", 1000.0)
    };
    keyed(get, key, &[Arg::F(f64::from(ms as f32 / unit))])
}

#[cfg(test)]
#[path = "cast_cells_tests.rs"]
mod tests;
