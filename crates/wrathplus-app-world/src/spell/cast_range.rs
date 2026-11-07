//! Build-12340 cast range inputs for `0x7ff480`; the shared evaluator owns the arithmetic.
use wrathplus_formats::{RangeUnit, SpellDisplay, SpellRangeCatalog};
use wrathplus_ui::script::{range_limits, RangeRow, RangeUnitFacts};

use super::{SpellModifiers, OP_RANGE};

pub(super) fn unit(u: RangeUnit) -> RangeUnitFacts {
    RangeUnitFacts {
        reach: u.reach,
        player: u.player,
        // Build-12340 `0x7ff480` tests 0x100d, not the legacy range helper's 0x200d.
        running: u.motion.flags & 0x100d != 0 && u.motion.is_running(),
        ..Default::default()
    }
}

pub(super) fn row(d: &SpellDisplay, ranges: &SpellRangeCatalog) -> Option<RangeRow> {
    let r = ranges.get(d.range_index)?;
    let (min, max) = ranges.friendly(d.range_index).unwrap_or((r.min, r.max));
    Some(RangeRow {
        flags: r.flags,
        min: [r.min, min],
        max: [r.max, max],
    })
}

/// Non-Unit melee calls alone read the auto-attack target (`12340 0x7ff480`);
/// do not give that target's reach or motion to a ground spell's non-melee arm.
pub(super) fn limits(
    d: &SpellDisplay,
    row: Option<RangeRow>,
    inputs: &super::cast_target::RangeInputs,
    mods: &SpellModifiers,
) -> (f32, f32) {
    let melee = row.is_some_and(|r| r.flags & 1 != 0);
    let target = match inputs.target {
        RangeTarget::Unit(u) => Some(u),
        RangeTarget::Corpse if !melee => {
            Some(wrathplus_formats::RangeUnit::still(inputs.caster.reach))
        }
        _ if melee => inputs.attack_target,
        _ => None,
    };
    // A corpse receives the caster's reach twice, but is not a Unit for column selection.
    let friendly = match inputs.target {
        RangeTarget::Unit(_) => inputs.friendly,
        _ => d.is_helpful(),
    };
    range_limits(
        d.attributes,
        row,
        &unit(inputs.caster),
        target.map(unit).as_ref(),
        friendly,
        mods.modifiers(d, OP_RANGE).map(|m| (m.flat, m.pct)),
        inputs.ranged_mod,
    )
}

pub(super) fn refusal(limits: (f32, f32), dist_sq: Option<f32>) -> Option<u8> {
    let (min, max) = limits;
    let d2 = dist_sq?;
    if d2 < min * min {
        return Some(super::validator::ERR_TOO_CLOSE);
    }
    if d2 > max * max {
        return Some(super::validator::ERR_OUT_OF_RANGE);
    }
    None
}

use bevy::prelude::Vec3;

/// Original object kind passed through `0x801650` to `0x7ff480` (build 12340).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum RangeTarget {
    #[default]
    None,
    Unit(wrathplus_formats::RangeUnit),
    Corpse,
}

impl RangeTarget {
    pub(super) fn as_unit(self) -> Option<RangeUnit> {
        match self {
            Self::Unit(u) => Some(u),
            _ => None,
        }
    }
}

/// Inputs for press `0x80cce0 → 0x809610 → 0x803850` and binder `0x80bc80 → 0x803850`.
#[derive(Clone, Copy)]
pub(crate) struct RangeInputs {
    pub(crate) friendly: bool,
    pub(crate) ranged_mod: Option<f32>,
    pub(crate) attack_target: Option<wrathplus_formats::RangeUnit>,
    pub(crate) self_pos: Option<Vec3>,
    pub(crate) target_pos: Option<Vec3>,
    /// The caster's combat reach and motion.
    pub(crate) caster: wrathplus_formats::RangeUnit,
    /// Original bound object kind; only actual Units contribute movement facts.
    pub(crate) target: RangeTarget,
}

impl RangeInputs {
    /// Press/bind `0x803850`: reject below min² first, then above max²; equality and NaN pass.
    /// Missing positions pass locally; the server judges.
    pub(crate) fn refusal(
        &self,
        def: &SpellDisplay,
        ranges: &wrathplus_formats::SpellRangeCatalog,
        mods: &super::SpellModifiers,
    ) -> Option<u8> {
        let dist_sq = self
            .self_pos
            .zip(self.target_pos)
            .map(|(a, b)| a.distance_squared(b));
        refusal(limits(def, row(def, ranges), self, mods), dist_sq)
    }

    /// Hover `0x803a50`, interval at `0x803e84`: inclusive endpoints, unordered rejected.
    pub(crate) fn hover_in_range(
        &self,
        def: &SpellDisplay,
        ranges: &SpellRangeCatalog,
        mods: &SpellModifiers,
    ) -> bool {
        let Some((a, b)) = self.self_pos.zip(self.target_pos) else {
            return true;
        };
        in_interval(
            limits(def, row(def, ranges), self, mods),
            a.distance_squared(b),
        )
    }
}

pub(super) fn in_interval((min, max): (f32, f32), d2: f32) -> bool {
    min * min <= d2 && d2 <= max * max
}

impl Default for RangeInputs {
    fn default() -> Self {
        Self {
            friendly: false,
            ranged_mod: None,
            attack_target: None,
            self_pos: None,
            target_pos: None,
            caster: wrathplus_formats::RangeUnit::still(super::DEFAULT_REACH),
            target: RangeTarget::None,
        }
    }
}

#[cfg(test)]
#[path = "cast_range_native_tests.rs"]
mod native_tests;
