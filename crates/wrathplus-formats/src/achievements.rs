//! Build-12340 static achievement metadata. Column layouts follow AzerothCore
//! `DBCStructure.h` (AchievementEntry, AchievementCriteriaEntry, AchievementCategoryEntry),
//! checked against installed headers. These catalogs do not synthesize player progress,
//! hide rows, follow reference achievements, or implement Lua's criteria-index projection.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{i32_at, parse, str_at, u32_at};
use crate::{Chain, DataProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Achievement {
    pub id: u32,
    /// -1 unrestricted, 0 Horde, 1 Alliance; preserve unknown values.
    pub required_faction: i32,
    /// -1 means no specific map.
    pub map_id: i32,
    pub parent_achievement_id: u32,
    pub name: String,
    pub description: String,
    pub category_id: u32,
    pub points: u32,
    pub order: u32,
    pub flags: u32,
    /// SpellIcon.dbc identity, not a texture path.
    pub icon_id: u32,
    pub reward: String,
    pub required_criteria_count: u32,
    /// Separate from the parent chain; consumers decide whether to redirect criteria.
    pub reference_achievement_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementCriterion {
    pub id: u32,
    pub achievement_id: u32,
    pub criteria_type: u32,
    /// Meaning depends on criteria_type (creature, spell, item, map, etc.).
    pub asset_id: u32,
    /// Raw column 4; interpretation depends on criteria_type, not player progress.
    pub required_quantity: u32,
    pub additional_requirements: [(u32, u32); 2],
    pub description: String,
    pub flags: u32,
    pub timed_type: u32,
    pub timer_start_event: u32,
    pub time_limit_seconds: u32,
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementCategory {
    pub id: u32,
    /// -1 denotes a root category.
    pub parent_id: i32,
    pub name: String,
    pub order: u32,
}

macro_rules! catalog {
    ($name:ident, $row:ty) => {
        /// Rows by id, iterated in DBC file order (the client walks the loaded array in place).
        #[derive(Debug, Default)]
        pub struct $name(BTreeMap<u32, $row>, Vec<u32>);
        impl $name {
            pub fn get(&self, id: u32) -> Option<&$row> {
                self.0.get(&id)
            }
            pub fn iter(&self) -> impl Iterator<Item = &$row> {
                self.1.iter().map(|id| &self.0[id])
            }
            pub fn len(&self) -> usize {
                self.0.len()
            }
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
            fn insert(&mut self, row: $row) {
                let id = row.id;
                if self.0.insert(id, row).is_none() {
                    self.1.push(id);
                }
            }
        }
    };
}
catalog!(AchievementCatalog, Achievement);
catalog!(AchievementCategoryCatalog, AchievementCategory);

#[derive(Debug, Default)]
pub struct AchievementCriteriaCatalog {
    rows: BTreeMap<u32, AchievementCriterion>,
    by_achievement: BTreeMap<u32, Vec<u32>>,
}
impl AchievementCriteriaCatalog {
    pub fn get(&self, id: u32) -> Option<&AchievementCriterion> {
        self.rows.get(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &AchievementCriterion> {
        self.rows.values()
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Raw owned rows, in DBC file order. No flag filtering, redirect, or synthesized
    /// special-flag criteria. This is not Lua's filtered one-based criteria index.
    pub fn for_achievement(&self, id: u32) -> impl Iterator<Item = &AchievementCriterion> {
        self.by_achievement
            .get(&id)
            .into_iter()
            .flatten()
            .map(|id| &self.rows[id])
    }
}

fn schema(name: &str, columns: usize, strings: &[usize]) -> Schema {
    let mut s = Schema::new(name);
    for col in 0..columns {
        s.add_field(SchemaField::new(
            format!("c{col}"),
            if strings.contains(&col) {
                FieldType::String
            } else {
                FieldType::UInt32
            },
        ));
    }
    s
}

fn achievements(bytes: &[u8], locale: &str) -> Result<AchievementCatalog> {
    let slot = crate::cfg_categories::locale_slot(locale)?;
    let rs = parse(
        bytes,
        schema("Achievement", 62, &[4 + slot, 21 + slot, 43 + slot]),
        "Achievement",
    )?;
    let mut out = AchievementCatalog::default();
    for r in rs.records() {
        let n = |i| u32_at(r, i).with_context(|| format!("Achievement column {i}"));
        let row = Achievement {
            id: n(0)?,
            required_faction: i32_at(r, 1).context("achievement faction")?,
            map_id: i32_at(r, 2).context("achievement map")?,
            parent_achievement_id: n(3)?,
            name: str_at(&rs, r, 4 + slot).unwrap_or_default(),
            description: str_at(&rs, r, 21 + slot).unwrap_or_default(),
            category_id: n(38)?,
            points: n(39)?,
            order: n(40)?,
            flags: n(41)?,
            icon_id: n(42)?,
            reward: str_at(&rs, r, 43 + slot).unwrap_or_default(),
            required_criteria_count: n(60)?,
            reference_achievement_id: n(61)?,
        };
        out.insert(row);
    }
    Ok(out)
}

fn criteria(bytes: &[u8], locale: &str) -> Result<AchievementCriteriaCatalog> {
    let col = 9 + crate::cfg_categories::locale_slot(locale)?;
    let rs = parse(
        bytes,
        schema("Achievement_Criteria", 31, &[col]),
        "Achievement_Criteria",
    )?;
    let mut out = AchievementCriteriaCatalog::default();
    for r in rs.records() {
        let n = |i| u32_at(r, i).with_context(|| format!("Achievement_Criteria column {i}"));
        let row = AchievementCriterion {
            id: n(0)?,
            achievement_id: n(1)?,
            criteria_type: n(2)?,
            asset_id: n(3)?,
            required_quantity: n(4)?,
            additional_requirements: [(n(5)?, n(6)?), (n(7)?, n(8)?)],
            description: str_at(&rs, r, col).unwrap_or_default(),
            flags: n(26)?,
            timed_type: n(27)?,
            timer_start_event: n(28)?,
            time_limit_seconds: n(29)?,
            order: n(30)?,
        };
        out.by_achievement
            .entry(row.achievement_id)
            .or_default()
            .push(row.id);
        out.rows.insert(row.id, row);
    }
    Ok(out)
}

fn categories(bytes: &[u8], locale: &str) -> Result<AchievementCategoryCatalog> {
    let col = 2 + crate::cfg_categories::locale_slot(locale)?;
    let rs = parse(
        bytes,
        schema("Achievement_Category", 20, &[col]),
        "Achievement_Category",
    )?;
    let mut out = AchievementCategoryCatalog::default();
    for r in rs.records() {
        let row = AchievementCategory {
            id: u32_at(r, 0).context("achievement category id")?,
            parent_id: i32_at(r, 1).context("achievement category parent")?,
            name: str_at(&rs, r, col).unwrap_or_default(),
            order: u32_at(r, 19).context("achievement category order")?,
        };
        out.insert(row);
    }
    Ok(out)
}

fn read(chain: &mut Chain, table: &str) -> Result<(Vec<u8>, String)> {
    let DataProfile::Wrath { locale } = chain.profile();
    let locale = locale.clone();
    let bytes = chain
        .read_file(&format!("DBFilesClient\\{table}.dbc"))
        .with_context(|| format!("reading {table}"))?;
    Ok((bytes, locale))
}

/// Load static rows in the explicitly selected archive locale; Vanilla has no table.
pub fn load_achievements(chain: &mut Chain) -> Result<AchievementCatalog> {
    let (bytes, locale) = read(chain, "Achievement")?;
    achievements(&bytes, &locale)
}
/// Load raw criteria, including hidden rows and unresolved owner IDs.
pub fn load_achievement_criteria(chain: &mut Chain) -> Result<AchievementCriteriaCatalog> {
    let (bytes, locale) = read(chain, "Achievement_Criteria")?;
    criteria(&bytes, &locale)
}
/// `SpellIcon.dbc`, id → texture path, for `Achievement.icon_id`.
pub fn load_achievement_icons(chain: &mut Chain) -> Result<std::collections::HashMap<u32, String>> {
    crate::dbc::load_spell_icon_map(chain)
}
/// Load localized categories, preserving root and unresolved parent IDs.
pub fn load_achievement_categories(chain: &mut Chain) -> Result<AchievementCategoryCatalog> {
    let (bytes, locale) = read(chain, "Achievement_Category")?;
    categories(&bytes, &locale)
}

#[cfg(test)]
mod tests;
