//! `CreatureFamily.dbc` and `ItemPetFood.dbc`: a pet's family word (`UnitCreatureFamily`,
//! `0x51a310`) and the diet its food mask names (`GetPetFoodTypes`, `0x4bea10`).
//!
//! `CreatureFamily.dbc` is 18 fields in 0x48-byte records, 23 rows with ids 1..28 (10, 13, 14, 18
//! and 22 absent). `ItemPetFood.dbc` is 10 fields, 8 rows. Mask bit `b` names food row `b + 1`,
//! low bit first, which is the client's record order (vmangos `Objects/Pet.cpp:1503-1504`).
//!
//! `GetPetFoodTypes`' gate (the pet is ours and we are a Hunter, `0x6116e0`) is live state, applied
//! at the feed (`wrathplus_app::ui_pet_stats`): a charmed boar under a priest has no diet.

use std::collections::HashMap;

use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::chain::Chain;
use crate::dbc::{f32_at, i32_at, parse, str_at, u32_at};

const CREATURE_FAMILY: &str = "DBFilesClient\\CreatureFamily.dbc";
const ITEM_PET_FOOD: &str = "DBFilesClient\\ItemPetFood.dbc";

/// `CreatureFamily.dbc`'s field count, which `wrathplus-dbc` checks against the header.
const FAMILY_FIELDS: usize = 19;
const FOOD_FIELDS: usize = 10;

const FAMILY_MIN_SCALE_FIELD: usize = 1;
const FAMILY_MIN_SCALE_LEVEL_FIELD: usize = 2;
const FAMILY_MAX_SCALE_FIELD: usize = 3;
const FAMILY_MAX_SCALE_LEVEL_FIELD: usize = 4;
const FAMILY_FOOD_MASK_FIELD: usize = 0x1c / 4;
/// The enUS name, the localized block's first slot.
const FAMILY_NAME_FIELD: usize = 0x20 / 4;
/// `iconFile`, after the name block's locale flags.
const FAMILY_ICON_FIELD: usize = 0x44 / 4;
/// Build 12340's pet talent type (`+0x20`, `GetPetTalentTree` `0x5d3d80`), appended past the 1.12
/// width and mapped by [`crate::wrath_layouts`]; -1 for a family without pet talents.
const FAMILY_PET_TALENT_TYPE_FIELD: usize = 18;
/// `ItemPetFood.dbc`'s enUS name.
const FOOD_NAME_FIELD: usize = 1;

/// The reference tests `1 << (row - 1)` for each of the 8 food rows, so higher bits name nothing.
const MAX_FOOD_BITS: u32 = 8;

/// One `CreatureFamily.dbc` row: the family word, icon, diet mask and level-to-size ramp.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatureFamily {
    /// The localized family word ("Wolf", "Imp"), what `UnitCreatureFamily` returns.
    pub name: String,
    /// `Interface\\Icons\\Ability_Hunter_Pet_<Family>`; empty when the row ships none.
    pub icon: String,
    /// The diet bitfield; 0 on the five warlock minions and row 28, so an empty diet is real.
    pub pet_food_mask: u32,
    /// The size ramp: the model scale at `min_scale_level` and at `max_scale_level`.
    pub min_scale: f32,
    pub min_scale_level: u32,
    pub max_scale: f32,
    pub max_scale_level: u32,
    /// The pet talent type, the `TalentTab` pet-mask bit; -1 for none.
    pub pet_talent_type: i32,
}

impl CreatureFamily {
    /// A pet's render scale at `level`, the reference's size law for the character-select pet:
    ///
    /// ```text
    /// S = minScale + (maxScale − minScale) · clamp(level − minScaleLevel, 0, range) / range
    /// range = maxScaleLevel − minScaleLevel
    /// ```
    ///
    /// The reference overwrites the `modelScale × scale` product with it (`0x472e87`), so the
    /// product survives only a family lookup miss; a world unit's ramp is the same line
    /// (`0x71c1af`). Deviation: a zero range (row 28 only) is `None`, the lookup-miss fallback:
    /// the glue path divides by zero into NaN, the world path draws at row 28's `minScale` of 0.
    pub fn scale_at(&self, level: u32) -> Option<f32> {
        let range = self.max_scale_level.checked_sub(self.min_scale_level)?;
        if range == 0 {
            return None;
        }
        let over = level.saturating_sub(self.min_scale_level).min(range);
        let t = over as f32 / range as f32;
        Some(self.min_scale + (self.max_scale - self.min_scale) * t)
    }
}

/// `CreatureFamily.dbc`, by id.
#[derive(Debug, Default, Clone)]
pub struct CreatureFamilies(HashMap<u32, CreatureFamily>);

impl CreatureFamilies {
    /// The row for a family id; the wire's 0 (no family) is `None`, `UnitCreatureFamily`'s nil.
    pub fn get(&self, id: u32) -> Option<&CreatureFamily> {
        self.0.get(&id)
    }

    /// The icon for the stable's slots and `GetPetIcon`; `None` for an unknown family or an empty
    /// path, which would draw white.
    pub fn icon(&self, id: u32) -> Option<&str> {
        self.0
            .get(&id)
            .map(|f| f.icon.as_str())
            .filter(|i| !i.is_empty())
    }

    pub fn name(&self, id: u32) -> Option<&str> {
        self.get(id).map(|f| f.name.as_str())
    }

    /// [`CreatureFamily::scale_at`] by family id; on `None` the caller takes the display's scale
    /// product, as the reference does.
    pub fn pet_scale(&self, family: u32, level: u32) -> Option<f32> {
        self.get(family)?.scale_at(level)
    }

    /// A world unit's `[unit+0xb3c]` (`0x71c110`): the ramp if larger, or for a pet (`0x71c1ba`).
    pub fn unit_model_scale(&self, base: f32, family: u32, level: u32, is_pet: bool) -> f32 {
        match self.pet_scale(family, level) {
            Some(ramp) if is_pet || ramp > base => ramp,
            _ => base,
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `ItemPetFood.dbc`: food-type id → its localized name.
#[derive(Debug, Default, Clone)]
pub struct PetFoodNames(HashMap<u32, String>);

impl PetFoodNames {
    /// The name for a food-type id (1-based, as the file numbers them).
    pub fn name(&self, id: u32) -> Option<&str> {
        self.0.get(&id).map(String::as_str)
    }

    /// The food names a [`CreatureFamily::pet_food_mask`] selects, bit `b` naming row `b + 1` in
    /// the reference's record order (`0x4bea10`). The `0x6116e0` gate is the caller's.
    pub fn for_mask(&self, mask: u32) -> Vec<&str> {
        (0..MAX_FOOD_BITS)
            .filter(|b| mask & (1 << b) != 0)
            .filter_map(|b| self.name(b + 1))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn family_schema() -> Schema {
    let mut s = Schema::new("CreatureFamily");
    for i in 0..FAMILY_FIELDS {
        let ty = match i {
            // minScale and maxScale, the record's only floats.
            1 | 3 => FieldType::Float32,
            FAMILY_NAME_FIELD | FAMILY_ICON_FIELD => FieldType::String,
            // The other locale slots, all 0 in the shipped file, read as dwords.
            _ => FieldType::UInt32,
        };
        s.add_field(SchemaField::new(format!("F{i}"), ty));
    }
    s
}

fn food_schema() -> Schema {
    let mut s = Schema::new("ItemPetFood");
    for i in 0..FOOD_FIELDS {
        let ty = if i == FOOD_NAME_FIELD {
            FieldType::String
        } else {
            FieldType::UInt32
        };
        s.add_field(SchemaField::new(format!("F{i}"), ty));
    }
    s
}

/// Load `CreatureFamily.dbc` from the patch chain.
pub fn load_creature_families(chain: &mut Chain) -> Result<CreatureFamilies> {
    let bytes = chain
        .read_file(CREATURE_FAMILY)
        .with_context(|| format!("reading {CREATURE_FAMILY}"))?;
    let rs = parse(&bytes, family_schema(), "CreatureFamily.dbc")?;
    let mut by_id = HashMap::with_capacity(rs.records().len());
    for r in rs.records() {
        let Some(id) = u32_at(r, 0) else { continue };
        // A nameless row is dropped: `UnitCreatureFamily` answers a word or nil, never "".
        let Some(name) = str_at(&rs, r, FAMILY_NAME_FIELD).filter(|n| !n.is_empty()) else {
            continue;
        };
        by_id.insert(
            id,
            CreatureFamily {
                name,
                icon: str_at(&rs, r, FAMILY_ICON_FIELD).unwrap_or_default(),
                pet_food_mask: u32_at(r, FAMILY_FOOD_MASK_FIELD).unwrap_or(0),
                min_scale: f32_at(r, FAMILY_MIN_SCALE_FIELD).unwrap_or(1.0),
                min_scale_level: u32_at(r, FAMILY_MIN_SCALE_LEVEL_FIELD).unwrap_or(0),
                max_scale: f32_at(r, FAMILY_MAX_SCALE_FIELD).unwrap_or(1.0),
                max_scale_level: u32_at(r, FAMILY_MAX_SCALE_LEVEL_FIELD).unwrap_or(0),
                pet_talent_type: i32_at(r, FAMILY_PET_TALENT_TYPE_FIELD).unwrap_or(-1),
            },
        );
    }
    Ok(CreatureFamilies(by_id))
}

/// Load `ItemPetFood.dbc` from the patch chain.
pub fn load_pet_food_names(chain: &mut Chain) -> Result<PetFoodNames> {
    let bytes = chain
        .read_file(ITEM_PET_FOOD)
        .with_context(|| format!("reading {ITEM_PET_FOOD}"))?;
    let rs = parse(&bytes, food_schema(), "ItemPetFood.dbc")?;
    let mut by_id = HashMap::with_capacity(rs.records().len());
    for r in rs.records() {
        let Some(id) = u32_at(r, 0) else { continue };
        if let Some(name) = str_at(&rs, r, FOOD_NAME_FIELD) {
            by_id.insert(id, name);
        }
    }
    Ok(PetFoodNames(by_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Option<Chain> {
        let data = crate::wow_data_or_skip!(None);
        Some(crate::open_chain(&data).expect("open chain"))
    }

    /// Build 12340's pet talent type: Ferocity 0, Tenacity 1, Cunning 2; -1 off the hunter pets.
    #[test]
    fn installed_families_carry_their_pet_talent_type() {
        let data = crate::wow_data_or_skip!();
        let profile = crate::DataProfile::from_env().expect("profile");
        if !matches!(profile, crate::DataProfile::Wrath { .. }) {
            return;
        }
        let mut chain = Chain::open_profile(&data, &profile).expect("open chain");
        let fams = load_creature_families(&mut chain).expect("families");
        let ty = |id| fams.get(id).map(|f| f.pet_talent_type);
        assert_eq!(ty(1), Some(0), "Wolf is Ferocity");
        assert_eq!(ty(21), Some(1), "Turtle is Tenacity");
        assert_eq!(ty(20), Some(1), "Scorpid is Tenacity in build 12340");
        assert_eq!(ty(23), Some(-1), "the Imp has no pet talents");
        assert!(
            (1..=60).any(|id| ty(id) == Some(2)),
            "some family is Cunning"
        );
    }

    #[test]
    fn the_family_size_ramp_reads_off_the_shipped_file() {
        let Some(mut chain) = chain() else { return };
        let fams = load_creature_families(&mut chain).expect("families");

        for (id, f) in (1..=27).filter_map(|id| fams.get(id).map(|f| (id, f))) {
            assert_eq!(
                (f.min_scale_level, f.max_scale_level),
                (1, 60),
                "family {id} ({}) does not ramp 1 → 60",
                f.name
            );
        }

        // Wolf: 0.7 at level 1 to 1.0 at 60, linear between.
        let wolf = fams.get(1).expect("Wolf");
        assert_eq!(wolf.name, "Wolf");
        assert!((wolf.scale_at(1).unwrap() - 0.7).abs() < 1e-6);
        assert!((wolf.scale_at(60).unwrap() - 1.0).abs() < 1e-6);
        let mid = wolf.scale_at(30).unwrap();
        assert!(
            (mid - (0.7 + 0.3 * 29.0 / 59.0)).abs() < 1e-6,
            "midpoint {mid} is not on the line"
        );
        // The ramp clamps at both ends.
        assert_eq!(wolf.scale_at(0), wolf.scale_at(1));
        assert_eq!(wolf.scale_at(255), wolf.scale_at(60));

        // The warlock minions are flat: `min == max`.
        for (id, want) in [(23, 0.5_f32), (16, 0.8), (15, 0.7), (17, 1.0), (19, 1.0)] {
            let f = fams.get(id).expect("warlock family");
            for level in [1, 30, 60] {
                let s = f.scale_at(level).expect("a real family ramps");
                assert!(
                    (s - want).abs() < 1e-6,
                    "{} at level {level} is {s}, expected a flat {want}",
                    f.name
                );
            }
        }

        // Row 28's zero range: `None`, not NaN.
        let remote = fams.get(28).expect("Remote Control");
        assert_eq!(remote.scale_at(60), None);

        // The wire's 0 and an id gap are unknown families.
        assert_eq!(fams.pet_scale(0, 60), None);
        assert_eq!(fams.pet_scale(10, 60), None);
        assert!((fams.pet_scale(1, 60).unwrap() - 1.0).abs() < 1e-6);

        // A world unit: the Mangy Duskbat (Bat, 0.2, level 3).
        let bat = 0.4 + 0.23 * 2.0 / 59.0;
        assert!((fams.unit_model_scale(0.2, 24, 3, false) - bat).abs() < 1e-6);
        assert_eq!(fams.unit_model_scale(0.9, 24, 3, false), 0.9);
        assert!((fams.unit_model_scale(0.9, 24, 3, true) - bat).abs() < 1e-6);
        assert_eq!(fams.unit_model_scale(0.2, 0, 3, false), 0.2);
    }

    #[test]
    fn the_real_pet_food_names_are_the_eight_diets() {
        let Some(mut chain) = chain() else { return };
        let n = load_pet_food_names(&mut chain).expect("load ItemPetFood.dbc");
        assert_eq!(n.len(), 8);
        for (id, want) in [
            (1, "Meat"),
            (2, "Fish"),
            (3, "Cheese"),
            (4, "Bread"),
            (5, "Fungus"),
            (6, "Fruit"),
            (7, "Raw Meat"),
            (8, "Raw Fish"),
        ] {
            assert_eq!(n.name(id), Some(want), "food id {id}");
        }
        assert_eq!(n.name(0), None, "the file is 1-based; there is no row 0");
        assert_eq!(n.name(9), None);
    }

    #[test]
    fn mask_bits_past_the_table_are_ignored() {
        let mut names = HashMap::new();
        names.insert(1, "Meat".to_string());
        names.insert(3, "Cheese".to_string());
        let n = PetFoodNames(names);

        assert_eq!(n.for_mask(0), Vec::<&str>::new());
        assert_eq!(n.for_mask(0b101), ["Meat", "Cheese"]);
        // Bit 1 names row 2, absent here: skipped.
        assert_eq!(n.for_mask(0b111), ["Meat", "Cheese"]);
        // Bit 31 is past `MAX_FOOD_BITS` entirely.
        assert_eq!(n.for_mask(0x8000_0001), ["Meat"]);
    }
}
