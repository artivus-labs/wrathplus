//! `SpellRange.dbc`, by `rangeIndex` ([`crate::spells::SpellDisplay::range_index`]), and
//! `GetMinMaxRange` (`0x7ff480`): a spell's cast range in yards. `0x7ff480` and
//! `GetCurrentSpeed`'s `0x987570` are build 12340's; every other address and object offset here
//! is 1.12's.

use std::collections::HashMap;

use crate::Chain;
use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{f32_at, parse, u32_at};

/// One `SpellRange.dbc` row, in yards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpellRange {
    pub min: f32,
    pub max: f32,
    /// Bit 0: the melee range family (combat-reach based, not the authored min/max).
    pub flags: u32,
}

impl SpellRange {
    /// Flags bit 0: `GetMinMaxRange` takes the reach sum instead of the row's pair.
    pub fn is_melee(&self) -> bool {
        self.flags & 1 != 0
    }
}

/// The pad the melee arm adds to the two reaches, `[0x80b058]`; `CanLootNow` (`0x5ec110`) pads the
/// loot reach by the same value.
pub const COMBAT_REACH_ADD: f32 = f32::from_bits(0x3faa_aaab);
/// The melee arm's floor, not a cap, `[0x80a1e8]`: a lower reach sum is raised to it (`0x6e35bf`).
pub const MELEE_RANGE_FLOOR: f32 = 5.0;
/// The on-next-swing short-circuit's max, `[0x8118d4]` (`0x6e3504`).
pub const ON_NEXT_SWING_RANGE: f32 = 100.0;

/// The movement-flag bits the moving bonus tests of each unit, `0x200d` (`0x6e3658`): forward,
/// either strafe, and jumping. Backward, turning and swimming are not among them.
pub const MOVING_BONUS_FLAGS: u32 = 0x200d;
/// The moving bonus, `[0x811914]` (`0x6e369c`), 2.6667: added to the max after both arms.
pub const MOVING_RANGE_BONUS: f32 = f32::from_bits(0x402a_aaab);

/// One unit's motion as the moving bonus reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnitMotion {
    /// The movement flags word, `[[unit+0x118]+0x40]` (`0x6e364f`-`0x6e3655`, `0x6e3661`).
    pub flags: u32,
    /// The current speed, `GetCurrentSpeed` (`0x987570`; 1.12 on `[unit+0x9a8]`,
    /// `0x5fc35c`-`0x5fc365`): 0 with no direction bit, else the unit's spline speed, or the swim,
    /// walk, run or run-back speed the flags pick.
    pub speed: f32,
    /// The walk speed, `[unit+0xa30]` (`0x5fc354`).
    pub walk_speed: f32,
}

impl UnitMotion {
    /// `0x5fc350` answering 0: the current speed is above twice the walk speed. The reference
    /// doubles the walk speed (`fadd st, st`), compares it with the speed (`fcompp`) and tests
    /// C0 (`test ah, 1`, `0x5fc373`-`0x5fc376`), which is set by a doubled walk speed below the
    /// speed and by an unordered pair, so a NaN reads as running and an equal pair as walking.
    pub fn is_running(&self) -> bool {
        let doubled = 2.0 * self.walk_speed;
        self.speed > doubled || self.speed.is_nan() || doubled.is_nan()
    }

    /// One unit's half of the bonus test: a movement flag in [`MOVING_BONUS_FLAGS`]
    /// (`0x6e364c`-`0x6e366a`) and [`Self::is_running`] (`0x6e366c`-`0x6e3681`).
    pub fn moves_at_run_speed(&self) -> bool {
        self.flags & MOVING_BONUS_FLAGS != 0 && self.is_running()
    }
}

/// A unit as `GetMinMaxRange` (`0x7ff480`) reads it: its combat reach, whether it is a player, and
/// its motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeUnit {
    /// `UNIT_FIELD_COMBATREACH`, `[[unit+0x110]+0x1f0]`.
    pub reach: f32,
    /// `OBJECT_FIELD_TYPE` bit 4 (`0x6e3689`-`0x6e3695`). Only a unit target's read counts: the
    /// ranged arm's bonus asks it of the target, and the melee arm's never does. The caster's
    /// own is the gate of the `Attributes & 2` scale (`0x6e36b4`-`0x6e36bf`), which is not applied.
    pub player: bool,
    pub motion: UnitMotion,
}

impl RangeUnit {
    /// A unit that stands still: no motion, so it never earns the moving bonus.
    pub fn still(reach: f32) -> Self {
        Self {
            reach,
            player: false,
            motion: UnitMotion::default(),
        }
    }
}

/// The units `GetMinMaxRange` (`0x7ff480`) reads besides the caster, `None` where that unit is
/// absent. The two are different inputs: only the first is an argument of the call.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeTargets {
    /// The call's `target` argument (`[ebx+0x10]`), when it is a unit (typemask bit 3,
    /// `0x6e34e1`-`0x6e34f9`): a null target, and one that is no unit, read `None`. Both arms
    /// read its reach, and the ranged arm pads only for a target passed in (`0x6e35d8`).
    pub target: Option<RangeUnit>,
    /// The caster's auto-attack target, `[caster+0xc48]`, which the function looks up itself
    /// (`0x47bf60`, `0x6e356a`). Only the melee arm reads it, and only with no unit target passed
    /// (`0x6e3552`-`0x6e3584`). A caller that has no such unit to hand passes `None`.
    pub attack_target: Option<RangeUnit>,
}

/// `GetMinMaxRange` (`0x7ff480`): a spell's `{min, max}` cast range, summed in `f64` because the
/// client keeps the reach sums on the x87 stack and stores `f32` only at the end. On-next-swing
/// spells (`Attributes & 0x404`, `0x6e34fb`) short-circuit to `(0, 100)`. The melee arm sums the
/// reach of the unit target, else the auto-attack target, else the caster's own, with the
/// caster's. The ranged arm pads only for a unit target passed in, the auto-attack target never
/// pads it: the max always, and the min only when nonzero (the `fcomp`-vs-0.0 guard), so a min-0
/// spell never refuses `TOO_CLOSE`. `None`: no row, or the self row (id 1, `{0, 0}`).
///
/// After both arms the max takes the moving bonus ([`MOVING_RANGE_BONUS`], `0x6e3648`-`0x6e36a2`)
/// when there is a unit (the target, or on the melee arm the auto-attack target), the caster and
/// that unit each [`UnitMotion::moves_at_run_speed`], and the row is the melee row or the unit a
/// player. It comes after the melee floor and before the range modifier (op 5, `0x6e3746`), and
/// the min never takes it. The on-next-swing short-circuit skips it.
///
/// Not applied: for a player the `Attributes & 2` scale (`0x6e36aa`), `max *= RangedModRange ·
/// 0.01` of the ranged-slot item, which a relic (range mod 0, InventoryType 28, admitted by
/// `0x809200`) would zero, though no spell a relic class learns carries the bit.
pub fn min_max_range(
    spell: &crate::spells::SpellDisplay,
    row: Option<&SpellRange>,
    caster: RangeUnit,
    targets: RangeTargets,
) -> Option<(f32, f32)> {
    if spell.on_next_swing() {
        return Some((0.0, ON_NEXT_SWING_RANGE));
    }
    let row = row?;
    // The `esi` both arms leave (`0x6e3648`): the unit the bonus reads, if any.
    let (min, max, unit) = if row.is_melee() {
        let unit = targets.target.or(targets.attack_target);
        let reach = unit.map_or(caster.reach, |u| u.reach);
        let sum = f64::from(reach) + f64::from(caster.reach) + f64::from(COMBAT_REACH_ADD);
        let max = if sum > f64::from(MELEE_RANGE_FLOOR) {
            sum as f32
        } else {
            MELEE_RANGE_FLOOR
        };
        (0.0, max, unit)
    } else {
        if row.min == 0.0 && row.max == 0.0 {
            return None;
        }
        let Some(target) = targets.target else {
            return Some((row.min, row.max));
        };
        let pad = f64::from(caster.reach) + f64::from(target.reach);
        let min = if row.min == 0.0 {
            0.0
        } else {
            (pad + f64::from(row.min)) as f32
        };
        (min, (f64::from(row.max) + pad) as f32, Some(target))
    };
    let earns_bonus = |unit: &RangeUnit| {
        caster.motion.moves_at_run_speed()
            && unit.motion.moves_at_run_speed()
            && (row.is_melee() || unit.player)
    };
    let max = if unit.as_ref().is_some_and(earns_bonus) {
        (f64::from(max) + f64::from(MOVING_RANGE_BONUS)) as f32
    } else {
        max
    };
    Some((min, max))
}

/// Whether [`min_max_range`] reads the units it is not handed for this spell and row when it is
/// passed no unit target, as the spell tooltip calls it (`push 0`, `0x52e9c2`): only the melee
/// arm, which sums the auto-attack target's reach, else the caster's own, with the caster's, and
/// asks both units' motion for the moving bonus. The ranged arm pads only for a target passed in,
/// so a tooltip prints its row as it is; the on-next-swing short-circuit, the missing row and the
/// self row read none. With a unit target the ranged arm reads both units too.
pub fn min_max_range_reads_units(
    spell: &crate::spells::SpellDisplay,
    row: Option<&SpellRange>,
) -> bool {
    !spell.on_next_swing() && row.is_some_and(SpellRange::is_melee)
}

/// `SpellRange.dbc`, by row id ([`crate::spells::SpellDisplay::range_index`]).
#[derive(Default)]
pub struct SpellRangeCatalog {
    ranges: HashMap<u32, SpellRange>,
    /// Build 12340's friendly pairs (`MinRangeFriend`, `MaxRangeFriend`, file columns 2 and 4).
    friendly: HashMap<u32, (f32, f32)>,
}

impl SpellRangeCatalog {
    pub fn get(&self, index: u32) -> Option<&SpellRange> {
        self.ranges.get(&index)
    }

    /// A row's friendly `(min, max)`, the pair `GetMinMaxRange 007ff480` reads for a friendly
    /// query (its fifth argument 1); a row without one reads its hostile pair.
    pub fn friendly(&self, index: u32) -> Option<(f32, f32)> {
        let hostile = || self.get(index).map(|r| (r.min, r.max));
        self.friendly.get(&index).copied().or_else(hostile)
    }

    /// [`Self::from_rows`] with friendly pairs for some rows, for tests.
    pub fn with_friendly(mut self, friendly: HashMap<u32, (f32, f32)>) -> Self {
        self.friendly = friendly;
        self
    }

    /// Fixture constructor for tests; the live path is [`load_spell_ranges`].
    pub fn from_rows(ranges: HashMap<u32, SpellRange>) -> Self {
        Self {
            ranges,
            friendly: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

const SPELL_RANGE: &str = "DBFilesClient\\SpellRange.dbc";
/// The 1.12 shape's 22 slots and build 12340's friendly pair at 22-23 (`wrath_layouts`).
const SPELL_RANGE_FIELDS: usize = 24;

/// Load `SpellRange.dbc` off the patch chain.
pub fn load_spell_ranges(chain: &mut Chain) -> Result<SpellRangeCatalog> {
    let bytes = chain
        .read_file(SPELL_RANGE)
        .context("reading SpellRange.dbc")?;
    let mut schema = Schema::new("SpellRange");
    for i in 0..SPELL_RANGE_FIELDS {
        match i {
            1 => schema.add_field(SchemaField::new("MinRange", FieldType::Float32)),
            2 => schema.add_field(SchemaField::new("MaxRange", FieldType::Float32)),
            22 => schema.add_field(SchemaField::new("MinRangeFriend", FieldType::Float32)),
            23 => schema.add_field(SchemaField::new("MaxRangeFriend", FieldType::Float32)),
            _ => schema.add_field(SchemaField::new(format!("F{i}"), FieldType::UInt32)),
        }
    }
    let set = parse(&bytes, schema, "SpellRange.dbc")?;
    let mut ranges = HashMap::new();
    let mut friendly = HashMap::new();
    for r in set.records() {
        let Some(id) = u32_at(r, 0) else { continue };
        if let (Some(min), Some(max)) = (f32_at(r, 22), f32_at(r, 23)) {
            friendly.insert(id, (min, max));
        }
        ranges.insert(
            id,
            SpellRange {
                min: f32_at(r, 1).unwrap_or(0.0),
                max: f32_at(r, 2).unwrap_or(0.0),
                flags: u32_at(r, 3).unwrap_or(0),
            },
        );
    }
    Ok(SpellRangeCatalog { ranges, friendly })
}

#[cfg(test)]
#[path = "ranges_tests.rs"]
mod tests;
