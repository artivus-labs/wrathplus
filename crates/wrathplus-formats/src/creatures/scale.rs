//! A creature display's scales: a world unit's, a mount's factor, the glue pet's and a corpse's.

use std::collections::HashMap;

use super::CreatureCatalog;
use crate::dbc::u32_at;
use crate::Chain;

impl CreatureCatalog {
    /// A display's own `creatureModelScale`, a mount's render factor (`0x71c0fb`, from `0x73d660`).
    pub fn display_scale(&self, display_id: u32) -> Option<f32> {
        self.display.get(&display_id).map(|r| r.scale)
    }

    /// A unit display's model scale before its family ramp (`0x722ae0` → `0x71c110`, `0x71c050`).
    pub fn unit_model_scale(&self, display_id: u32) -> Option<f32> {
        let row = self.display.get(&display_id)?;
        let model = self.models.get(&row.model_id)?;
        let race = (row.extended_id != 0)
            .then(|| self.extra.get(&row.extended_id))
            .flatten()
            .and_then(|e| self.race_displays.get(&(e.race, e.sex)))
            .and_then(|d| self.display.get(d))
            .map_or(1.0, |r| r.scale);
        let k = race * row.scale * model.scale;
        Some(if k > 0.0 { k } else { 1.0 })
    }

    /// `modelScale × creatureModelScale`: the glue pet's (`0x472dc6`) and a corpse's (`0x705ac0`).
    pub fn model_scale(&self, display_id: u32) -> Option<f32> {
        let row = self.display.get(&display_id)?;
        let model = self.models.get(&row.model_id)?;
        Some(model.scale * row.scale)
    }
}

/// `ChrRaces` `(race, sex)` → the race's base display (`+0x10` male, `+0x14` female).
pub(super) fn race_displays(chain: &mut Chain) -> HashMap<(u8, u8), u32> {
    crate::race_creature_type::read_chr_races(chain)
        .map(|rs| {
            rs.records()
                .iter()
                .filter_map(|r| {
                    let race = u8::try_from(u32_at(r, 0)?).ok()?;
                    Some([(0, u32_at(r, 4)?), (1, u32_at(r, 5)?)].map(|(sex, d)| ((race, sex), d)))
                })
                .flatten()
                .collect()
        })
        .unwrap_or_default()
}
