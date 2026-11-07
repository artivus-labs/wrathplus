//! Action buttons share `0x7ff480` with `IsSpellInRange` and `IsItemInRange`.
use wrathplus_formats::{RangeUnit, SpellDisplay, SpellRangeCatalog};
use wrathplus_ui::script::{range_limits, RangeRow, RangeUnitFacts};

use crate::spell::{SpellModifiers, OP_RANGE};

fn unit(u: RangeUnit) -> RangeUnitFacts {
    RangeUnitFacts {
        reach: u.reach,
        player: u.player,
        running: u.motion.moves_at_run_speed(),
        ..Default::default()
    }
}

/// A null target chooses the helpful spell's column; a unit chooses its assist relation
/// (`0x801650`). Reach and moving pads are added after op5 in the shared evaluator.
pub(super) fn limits(
    d: &SpellDisplay,
    ranges: &SpellRangeCatalog,
    mods: &SpellModifiers,
    caster: RangeUnit,
    target: Option<RangeUnit>,
    friendly: bool,
    ranged_mod: Option<f32>,
) -> (f32, f32) {
    let row = ranges.get(d.range_index).map(|r| {
        let (min, max) = ranges.friendly(d.range_index).unwrap_or((r.min, r.max));
        RangeRow {
            flags: r.flags,
            min: [r.min, min],
            max: [r.max, max],
        }
    });
    range_limits(
        d.attributes,
        row,
        &unit(caster),
        target.map(unit).as_ref(),
        friendly,
        mods.modifiers(d, OP_RANGE).map(|m| (m.flat, m.pct)),
        ranged_mod,
    )
}
