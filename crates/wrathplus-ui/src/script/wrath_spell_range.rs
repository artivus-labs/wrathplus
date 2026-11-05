//! Build-12340's spell range test, as `IsSpellInRange` and `IsItemInRange` run it: the target
//! gate `0x809610`, then `0x803850`'s squared-distance compare against `GetMinMaxRange`
//! (`0x801650` → `0x7ff480`), over the spell's Spell.dbc and SpellRange.dbc facts
//! ([`SpellRangeFacts`], in the catalog) and the units the app pushes ([`RangeUnitFacts`]).

use mlua::Value;

use super::wrath_spell_use::SpellUse;
use super::{Model, SpellInfoView};

/// One `SpellRange.dbc` row: `flags` bit 0 melee, bit 1 ranged; `[hostile, friendly]` pairs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeRow {
    pub flags: u32,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// The Spell.dbc columns the range test reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpellRangeFacts {
    /// `Targets` (`+0x40`), the seed of the target word.
    pub targets: u32,
    /// `EffectImplicitTargetA[3]` (`+0x158`).
    pub implicit_a: [u32; 3],
    /// `TargetCreatureType` (`+0x44`): bit `type - 1` per allowed type, 0 any.
    pub creature_types: u32,
    /// `Attributes` (`+0x10`): `0x404` on next swing, `0x2` ranged-weapon scale.
    pub attributes: u32,
    /// `AttributesEx2` (`+0x18`): bit 0 a dead target is fine.
    pub attributes_ex2: u32,
    /// `AttributesEx5` (`+0x24`): bit `0x800` aims through the unit to its target (`0x7ffc40`).
    pub attributes_ex5: u32,
    /// `AttributesEx6` (`+0x28`): `0x1000000` takes an unselectable unit; bit 3 the assist mode.
    pub attributes_ex6: u32,
    /// The `rangeIndex` row (`+0xb8`); `None` when SpellRange.dbc lacks it.
    pub row: Option<RangeRow>,
}

/// One unit as the range test reads it, from the player's point of view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeUnitFacts {
    /// The position `0x803850` reads (vtable `+0x2c`).
    pub pos: [f32; 3],
    /// `UNIT_FIELD_COMBATREACH` (`+0xf0`).
    pub reach: f32,
    /// `TYPEMASK_PLAYER`.
    pub player: bool,
    /// The moving-bonus half (`0x7ff480`): a `0x100d` movement flag and not walking (`0x716fa0`).
    pub running: bool,
    /// `UNIT_FIELD_HEALTH` (`+0x48`).
    pub health: u32,
    /// `UNIT_FIELD_FLAGS` (`+0xd4`).
    pub unit_flags: u32,
    /// `UNIT_DYNAMIC_FLAGS` (`+0x124`): `0x20` dead.
    pub dynamic_flags: u32,
    /// The creature type (`0x71f300`), 0 for none.
    pub creature_type: u32,
    /// `UNIT_FIELD_PETNUMBER` (`+0x114`).
    pub pet_number: u32,
    /// `CanAssist` (`0x7293d0`) from the player, mode 0 and mode 1.
    pub assist: [bool; 2],
    /// `CanAttack` (`0x729740`) from the player.
    pub attack: bool,
    /// In the player's party (`0x718ca0`) or raid (`0x718d70`).
    pub party: bool,
    pub raid: bool,
}

/// `0x7ff480`'s constants: the reach addend `0x9f987c`, the melee floor `0x9ebf34`, the moving
/// bonus `0xa437d0` and the on-next-swing maximum `0xa42178`.
const REACH_ADD: f32 = 4.0 / 3.0;
const MELEE_FLOOR: f32 = 5.0;
const MOVING_BONUS: f32 = 8.0 / 3.0;
const NEXT_SWING_MAX: f32 = 100.0;

/// `0x7ff480`: `(min, max)` for `caster` against `unit` (`None`: no target), column 1 the
/// friendly pair. `melee_reach` is the reach the melee arm sums without a unit target, the
/// caster's auto-attack target's (`[caster+0xa20]`), else the caster's own.
pub(crate) fn min_max(
    state: &SpellUse,
    spell: &SpellInfoView,
    caster: &RangeUnitFacts,
    unit: Option<&RangeUnitFacts>,
    friendly: bool,
) -> (f32, f32) {
    let f = &spell.range;
    if f.attributes & 0x404 != 0 {
        return (0.0, NEXT_SWING_MAX);
    }
    let col = usize::from(friendly);
    let (mut min, mut max, mut pad) = (0.0f32, 0.0f32, 0.0f32);
    if let Some(row) = f.row {
        let reach_sum =
            || f64::from(unit.map_or(caster.reach, |u| u.reach)) + f64::from(caster.reach);
        if row.flags & 1 == 0 {
            min = if row.flags & 2 == 0 {
                row.min[col]
            } else {
                ((reach_sum() + f64::from(REACH_ADD)) as f32).max(MELEE_FLOOR) + row.min[col]
            };
            max = row.max[col];
            if let Some(u) = unit {
                pad = caster.reach + u.reach;
                if min != 0.0 && row.flags & 2 == 0 {
                    min += pad;
                }
            }
        } else {
            pad = ((reach_sum() + f64::from(REACH_ADD)) as f32).max(MELEE_FLOOR);
        }
        let earns =
            |u: &RangeUnitFacts| caster.running && u.running && (row.flags & 1 != 0 || u.player);
        if unit.is_some_and(earns) {
            pad += MOVING_BONUS;
        }
    }
    if f.attributes & 2 != 0 && caster.player {
        if let Some(m) = state.ranged_mod {
            max *= m * 0.01;
        }
    }
    if let Some(&(flat, pct)) = state.range_mods.get(&spell.spell_id) {
        max = (flat as f32 + max) * pct as f32 * 0.01;
    }
    (min, max + pad)
}

/// `SpellHasRange`/`ItemHasRange`: `0x801650` with no target, the friendly pair for a helpful
/// spell (`0x7fe1b0` answering 1), then `|min|` or `|max|` at least `FLT_EPSILON` (`0x9ea27c`).
pub(crate) fn has_range(state: &SpellUse, spell: &SpellInfoView, caster: u64) -> bool {
    let Some(c) = state.units.get(&caster) else {
        return false;
    };
    let (min, max) = min_max(state, spell, c, None, spell.polarity == 1);
    min.abs() >= f32::EPSILON || max.abs() >= f32::EPSILON
}

/// The unit a spell argument names (`0x60abf0` with its empty-is-target flag, pushed at
/// `0x541d37` and `0x51ca7a`): an empty or missing token is the target (`0x60b029`). A token
/// naming nobody stores the sentinel `0xffffffff_fffffffe` (`0x60b00c`), which passes
/// `0x809610`'s zero test (`0x80961c`) and finds no unit, so it is `None`, as an unknown token is.
fn unit_guid(model: &Model, unit: Option<&Value>) -> Option<u64> {
    let token = match unit {
        Some(Value::String(s)) => s.to_string_lossy(),
        Some(v @ (Value::Integer(_) | Value::Number(_))) => v.to_string().ok()?,
        _ => String::new(),
    };
    if token.is_empty() {
        return Some(model.unit_guids.target);
    }
    model.unit_guids.resolve(&token).ok()?
}

/// `0x809610`'s target word: `Targets` adjusted by each `EffectImplicitTargetA`, and whether one
/// is the pet (5).
fn target_word(f: &SpellRangeFacts) -> (u32, bool) {
    let mut w = f.targets & 0xffff;
    let mut pet = false;
    for a in f.implicit_a {
        match a {
            1 => w &= !0x400,
            5 => {
                pet = true;
                w &= !0x8000;
            }
            6 | 0x35 => w |= 0x80,
            0x15 | 0x2d => w |= 0x100,
            0x17 => w |= 0x800,
            0x19 | 0x3f..=0x47 | 0x4a | 0x4b => w |= 2,
            0x1a => w |= 0x4000,
            0x23 => w |= 8,
            0x39 | 0x3d => w |= 4,
            _ => {}
        }
    }
    (w, pet)
}

/// `0x7ffc40` for an `AttributesEx5 & 0x800` spell: an enemy-only word aims at a friendly unit's
/// target if that can be attacked; a friendly word at a hostile unit's target if that can be
/// assisted and the word's party, raid or ally bit takes it. Inferred: the unit, not the
/// player, is the one whose target is read.
fn through_target(
    model: &Model,
    state: &SpellUse,
    w: u32,
    assist_mode: usize,
    guid: u64,
) -> Option<u64> {
    let u = state.units.get(&guid)?;
    let next = || {
        let t = *model.unit_guids.held.get(&guid)?;
        Some((t, state.units.get(&t)?))
    };
    if w & 0x10c == 0 {
        if w & 0x80 != 0 && u.assist[assist_mode] {
            return next().filter(|(_, t)| t.attack).map(|(g, _)| g);
        }
        return Some(guid);
    }
    if !u.attack {
        return Some(guid);
    }
    let (g, t) = next()?;
    let takes = (w & 8 != 0 && t.party) || (w & 4 != 0 && t.raid) || w & 0x100 != 0;
    (t.assist[assist_mode] && takes).then_some(g)
}

/// `0x809610`'s unit arm: whether the spell may take `guid`; the unit it then measures.
fn gate(model: &Model, state: &SpellUse, f: &SpellRangeFacts, guid: u64) -> Option<u64> {
    let (mut w, pet) = target_word(f);
    let mut guid = guid;
    if pet && w & 2 == 0 {
        guid = model.unit_guids.pet;
        w |= 2;
    }
    let assist_mode = usize::from(f.attributes_ex6 >> 3 & 1 != 0);
    if f.attributes_ex5 & 0x800 != 0 {
        guid = through_target(model, state, w, assist_mode, guid)?;
    }
    let u = state.units.get(&guid)?;
    if u.unit_flags & 0x1_0000 != 0 && f.attributes_ex6 & 0x100_0000 == 0 {
        return None;
    }
    if f.creature_types != 0 {
        let t = u.creature_type;
        if t == 0 || f.creature_types & (1u32 << ((t - 1) & 31)) == 0 {
            return None;
        }
    }
    let dead = u.health == 0;
    if dead {
        let corpse_spell = w & 0x8600 != 0 || f.attributes_ex2 & 1 != 0;
        if u.dynamic_flags & 0x20 == 0 && !corpse_spell {
            return None;
        }
    } else if w & 0x400 != 0 {
        return None;
    }
    let takes = (w & 8 != 0 && u.party && u.assist[0])
        || (w & 4 != 0 && u.raid && u.assist[0])
        || (w & 0x100 != 0 && u.assist[assist_mode])
        || (w & 0x80 != 0 && u.attack)
        || (w & 2 != 0 && u.unit_flags & 0x100 == 0);
    if takes {
        return Some(guid);
    }
    // The corpse words: `0x8000` an ally's (a dead friendly player or pet), else `0x200` an
    // enemy's (a dead player who is not friendly).
    if w & 0x8000 == 0 || !dead || (!u.player && u.pet_number == 0) {
        let enemy_corpse = w & 0x200 != 0 && dead && u.player && !u.assist[0];
        return enemy_corpse.then_some(guid);
    }
    u.assist[0].then_some(guid)
}

/// `IsSpellInRange`/`IsItemInRange`'s answer: `None` (nil) when the token is unknown, the unit
/// or the caster is not held, or the spell cannot take the unit; else the range test, in when
/// `min² <= d² <= max²` (`0x803850`), with the friendly pair when the unit can be assisted
/// (`0x801650`).
pub(crate) fn in_range(
    model: &Model,
    state: &SpellUse,
    spell: &SpellInfoView,
    caster: u64,
    unit: Option<&Value>,
) -> Option<bool> {
    let c = state.units.get(&caster)?;
    let guid = gate(model, state, &spell.range, unit_guid(model, unit)?)?;
    let u = state.units.get(&guid)?;
    let (min, max) = min_max(state, spell, c, Some(u), u.assist[0]);
    let d2: f32 = (0..3).map(|i| (u.pos[i] - c.pos[i]).powi(2)).sum();
    Some(min * min <= d2 && d2 <= max * max)
}

#[cfg(test)]
#[path = "wrath_spell_range_tests.rs"]
mod tests;
