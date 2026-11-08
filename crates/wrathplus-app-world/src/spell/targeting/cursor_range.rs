use crate::spell::SpellModifiers;
use bevy::prelude::*;
use wrathplus_formats::{RangeUnit, SpellDisplay};

/// Ground `0x4f66c0 → 0x803ee0 → 0x802c30 → 0x801650 → 0x7ff480` passes null;
/// GameObject hover `0x803a50` also has no Unit. Polarity selects the range columns.
#[derive(Clone, Copy)]
pub(super) struct RangeCall<'a> {
    pub(super) spell: &'a SpellDisplay,
    pub(super) row: Option<wrathplus_ui::script::RangeRow>,
    pub(super) mods: &'a SpellModifiers,
    pub(super) caster: RangeUnit,
    pub(super) attack_target: Option<RangeUnit>,
    pub(super) friendly: bool,
    pub(super) ranged_mod: Option<f32>,
}

/// No catalog or unknown spell is permissive; a known spell with a missing range row still
/// enters the native evaluator (including the on-next-swing early return).
pub(super) fn range_call<'a>(
    checks: &'a super::BindChecks,
    spell_id: u32,
) -> Option<RangeCall<'a>> {
    let spells = checks.spells.as_deref()?;
    let spell = spells.catalog.get(spell_id)?;
    Some(RangeCall {
        spell,
        row: super::super::cast_range::row(spell, &spells.ranges),
        friendly: spell.is_helpful(),
        ranged_mod: checks.range_units.ranged_mod(),
        mods: &checks.spell_mods,
        caster: checks.range_units.caster(),
        attack_target: checks.range_units.caster_attack_target(),
    })
}

/// Build-12340 `0x803ee0` accepts min² ≤ distance² ≤ max², rejecting NaN. This is a
/// hover-only verdict, not a spell-failure reason; terrain clicks do not ask it.
pub(super) fn ground_point_in_range(call: Option<RangeCall>, self_pos: Vec3, point: Vec3) -> bool {
    let Some(call) = call else {
        return true;
    };
    let inputs = super::super::cast_target::RangeInputs {
        caster: call.caster,
        attack_target: call.attack_target,
        friendly: call.friendly,
        ranged_mod: call.ranged_mod,
        ..Default::default()
    };
    let bounds = super::super::cast_range::limits(call.spell, call.row, &inputs, call.mods);
    let dist_sq = self_pos.distance_squared(point);
    super::super::cast_range::in_interval(bounds, dist_sq)
}
