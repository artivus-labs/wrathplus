//! The VM's static `Spell.dbc` catalog, every spell and not only the learned ones, seeded once
//! before FrameXML runs: what GetSpellInfo (`0x540a30`) and the spell predicates read.

use bevy::prelude::*;

use wrathplus_formats::{SpellDisplay, ON_NEXT_SWING_RANGE};
use wrathplus_protocol::types::power_display_scale;
use wrathplus_ui::script::{SpellInfoView, UiScript};

use crate::app_core::state::Spells;

/// Seed static Spell.dbc metadata before FrameXML executes. No per-frame catalog walk.
/// Character-specific cost/cast modifiers remain the live action/tooltip feeds' concern.
pub(crate) fn seed_spell_catalog(world: &World, script: &mut UiScript) {
    if let Some(spells) = world.get_resource::<Spells>() {
        script.set_spell_catalog(
            spells
                .catalog
                .iter()
                .map(|(id, def)| catalog_row(id, def, spells)),
        );
    }
}

fn catalog_row(id: u32, def: &SpellDisplay, spells: &Spells) -> SpellInfoView {
    // GetSpellInfo divides both costs by the power type's display divisor (`0x540be9`, `0x540c00`).
    let scale = power_display_scale(def.power_type);
    let row = spells.ranges.get(def.range_index);
    // Its range pair (`0x540c99`): a melee row is (0, 0) (`0x7ff380`); else `0x801650` with no
    // target, on-next-swing's (0, 100) (`0x7ff4a5`) or the row's friendly pair for a helpful spell
    // (`0x801692`), its hostile pair otherwise.
    let (min, max) = if row.is_some_and(|r| r.is_melee()) {
        (0.0, 0.0)
    } else if def.on_next_swing() {
        (0.0, ON_NEXT_SWING_RANGE)
    } else if def.is_helpful() {
        // A row without its friendly pair reads the hostile one, as the range facts do.
        (spells.ranges.friendly(def.range_index))
            .or(row.map(|r| (r.min, r.max)))
            .unwrap_or((0.0, 0.0))
    } else {
        row.map_or((0.0, 0.0), |r| (r.min, r.max))
    };
    SpellInfoView {
        spell_id: id,
        name: def.name.clone(),
        rank: def.rank.clone(),
        texture: def.icon.clone(),
        cost: def.mana_cost / scale,
        cost_per_second: def.mana_per_second / scale,
        power_type: def.power_type as i32,
        // A passive answers 0 (`0x540c4b`).
        cast_time_ms: match def.passive {
            true => 0,
            false => spells
                .cast_times
                .get(def.casting_time_index)
                .map_or(0, |row| row.base_ms.max(0) as u32),
        },
        min_range: f64::from(min),
        max_range: f64::from(max),
        passive: def.passive,
        weapon_icon: def.is_melee_auto_attack() || def.ranged_icon_substitution(),
        trade_skill_link: None,
        ..crate::ui_action::spell_use::catalog_facts(def, spells)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wrathplus_formats::{SpellCatalog, SpellRange, SpellRangeCatalog};

    #[test]
    fn wrath_catalog_seed_includes_unlearned_spells_and_replacement_vms() {
        let def = SpellDisplay {
            name: "Unlearned".into(),
            rank: Some("Rank 2".into()),
            attributes: 0x40,
            passive: true,
            icon: Some("Interface\\Icons\\Test".into()),
            mana_cost: 27,
            power_type: u32::MAX - 1,
            attributes_ex: 0x4,
            mana_per_second: 3,
            casting_time_index: 2,
            range_index: 3,
            ..Default::default()
        };
        // A rage strike on the melee row, and a helpful spell on a row with a friendly pair.
        let strike = SpellDisplay {
            name: "Strike".into(),
            power_type: 1,
            mana_cost: 150,
            range_index: 2,
            casting_time_index: 2,
            ..Default::default()
        };
        let heal = SpellDisplay {
            name: "Heal".into(),
            targets: 0x100,
            range_index: 3,
            ..Default::default()
        };
        let range = |min, max, flags| SpellRange { min, max, flags };
        let spells = Spells {
            catalog: SpellCatalog::from_displays(HashMap::from([
                (42, def),
                (78, strike),
                (2050, heal),
            ])),
            cast_times: wrathplus_formats::SpellCastTimeCatalog::from_rows([(
                2,
                wrathplus_formats::SpellCastTime {
                    base_ms: 1500,
                    per_level_ms: 0,
                    minimum_ms: 0,
                },
            )]),
            ranges: SpellRangeCatalog::from_rows(HashMap::from([
                (2, range(0.0, 5.0, 1)),
                (3, range(5.0, 30.0, 0)),
            ]))
            .with_friendly(HashMap::from([(3, (0.0, 40.0))])),
            ..Spells::empty_for_tests()
        };
        let mut world = World::new();
        world.insert_resource(spells);
        for _ in 0..2 {
            let mut script = UiScript::new().unwrap();
            seed_spell_catalog(&world, &mut script);
            script.set_unit_guids(&wrathplus_ui::script::UnitGuids {
                player: 1,
                ..Default::default()
            });
            script
                .run(
                    r#"
                -- A passive casts in 0; the per-second cost replaces the cost and says so.
                local n,r,i,c,f,p,t,min,max = GetSpellInfo(42)
                assert(n == 'Unlearned' and r == 'Rank 2' and i == 'Interface\\Icons\\Test')
                assert(c == 3 and f == true and p == -2 and t == 0 and min == 5 and max == 30)
                assert(select('#', IsPassiveSpell(42)) == 0 and GetSpellInfo('Unlearned') == nil)
                -- Rage divides by ten; a melee row answers (0, 0).
                _,_,i,c,f,p,t,min,max = GetSpellInfo(78)
                assert(i == '' and c == 15 and f == false and p == 1 and t == 1500)
                assert(min == 0 and max == 0)
                -- A helpful spell reads the friendly pair.
                assert(select(9, GetSpellInfo(2050)) == 40 and select(8, GetSpellInfo(2050)) == 0)
            "#,
                )
                .unwrap();
        }
    }
}
