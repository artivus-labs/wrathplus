//! Build-12340 gem lookups for the socketing window: which gem item a socket enchant came from
//! (`SpellItemEnchantment.m_src_itemID`) and a gem's colour mask (`GemProperties.m_color`).
//! Layouts: AzerothCore `DBCStructure.h` (`SpellItemEnchantmentEntry`, `GemPropertiesEntry`),
//! `DBCfmt.h` (`SpellItemEnchantmentfmt`, 38 columns; `GemPropertiesEntryfmt` "nixxi").

use std::collections::HashMap;

use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{parse, u32_at};
use crate::Chain;

const SPELL_ITEM_ENCHANTMENT: &str = "DBFilesClient\\SpellItemEnchantment.dbc";
const GEM_PROPERTIES: &str = "DBFilesClient\\GemProperties.dbc";
const ENCHANT_FIELDS: usize = 38;
/// `m_src_itemID`, the gem the enchant comes from.
const ENCHANT_GEM: usize = 33;
/// `Effect[3]` and `EffectArg[3]`; effect 7 casts the arg's spell on use.
const ENCHANT_EFFECT: usize = 2;
const ENCHANT_ARG: usize = 11;
const USE_SPELL: u32 = 7;
const GEM_PROPERTIES_FIELDS: usize = 5;
const GEM_COLOR: usize = 4;

/// Socket and gem colour bits (AzerothCore `SocketColor`).
pub const SOCKET_COLOR_META: u32 = 1;
pub const SOCKET_COLOR_RED: u32 = 2;
pub const SOCKET_COLOR_YELLOW: u32 = 4;
pub const SOCKET_COLOR_BLUE: u32 = 8;

#[derive(Debug, Default, Clone)]
pub struct GemCatalog {
    gems: HashMap<u32, u32>,
    colors: HashMap<u32, u32>,
    /// The use-spell enchants: id → `(EffectArg, Charges)` of the first `Effect` 7.
    use_spells: HashMap<u32, (u32, i32)>,
}

impl GemCatalog {
    /// The gem item a socket enchant names; `None` for an enchant that is no gem's.
    pub fn gem_of_enchant(&self, enchant_id: u32) -> Option<u32> {
        self.gems.get(&enchant_id).copied()
    }

    /// A gem's colour mask by its template's `GemProperties` id.
    pub fn color(&self, gem_properties: u32) -> Option<u32> {
        self.colors.get(&gem_properties).copied()
    }

    /// Every `(enchant, gem item)` pair.
    pub fn gem_enchants(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.gems.iter().map(|(&e, &g)| (e, g))
    }

    /// A use-spell enchant (`Effect` 7, `ITEM_ENCHANTMENT_TYPE_USE_SPELL`): its spell
    /// (`EffectArg`, `+0x2c`) and the row's `Charges` (`+0x4`), as `0x707c60` and `0x706f40` read
    /// them.
    pub fn use_spell(&self, enchant_id: u32) -> Option<(u32, i32)> {
        self.use_spells.get(&enchant_id).copied()
    }

    /// Build from explicit maps, for tests.
    pub fn from_rows(gems: HashMap<u32, u32>, colors: HashMap<u32, u32>) -> Self {
        GemCatalog {
            gems,
            colors,
            use_spells: HashMap::new(),
        }
    }
}

fn raw_schema(name: &str, fields: usize) -> Schema {
    let mut s = Schema::new(name);
    for i in 0..fields {
        s.add_field(SchemaField::new(format!("F{i}"), FieldType::UInt32));
    }
    s
}

/// Parse the two 3.3.5 tables.
pub fn parse_gem_catalog(enchants: &[u8], gem_properties: &[u8]) -> Result<GemCatalog> {
    let rs = parse(
        enchants,
        raw_schema("SpellItemEnchantment", ENCHANT_FIELDS),
        "SpellItemEnchantment",
    )?;
    let gems = rs
        .records()
        .iter()
        .filter_map(|r| Some((u32_at(r, 0)?, u32_at(r, ENCHANT_GEM)?)))
        .filter(|&(_, gem)| gem != 0)
        .collect();
    let use_spells = rs
        .records()
        .iter()
        .filter_map(|r| {
            // The first effect 7 with a spell (`0x706ec0`: `[+0x2c] != 0 && [+8] == 7`).
            let arg = |i| u32_at(r, ENCHANT_ARG + i).unwrap_or(0);
            let effect = (0..3)
                .find(|&i| u32_at(r, ENCHANT_EFFECT + i) == Some(USE_SPELL) && arg(i) != 0)?;
            let spell = arg(effect);
            Some((u32_at(r, 0)?, (spell, u32_at(r, 1)? as i32)))
        })
        .collect();
    let rs = parse(
        gem_properties,
        raw_schema("GemProperties", GEM_PROPERTIES_FIELDS),
        "GemProperties",
    )?;
    let colors = rs
        .records()
        .iter()
        .filter_map(|r| Some((u32_at(r, 0)?, u32_at(r, GEM_COLOR)?)))
        .collect();
    Ok(GemCatalog {
        gems,
        colors,
        use_spells,
    })
}

pub fn load_gem_catalog(chain: &mut Chain) -> Result<GemCatalog> {
    let enchants = chain
        .read_file(SPELL_ITEM_ENCHANTMENT)
        .context("reading SpellItemEnchantment")?;
    let gem_properties = chain
        .read_file(GEM_PROPERTIES)
        .context("reading GemProperties")?;
    parse_gem_catalog(&enchants, &gem_properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wdbc(rows: &[Vec<u32>]) -> Vec<u8> {
        let fields = rows[0].len() as u32;
        let mut out = b"WDBC".to_vec();
        for word in [rows.len() as u32, fields, fields * 4, 1] {
            out.extend(word.to_le_bytes());
        }
        for row in rows {
            for v in row {
                out.extend(v.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    #[test]
    fn gems_come_from_the_enchant_source_item_and_colours_from_gem_properties() {
        let mut gem_enchant = vec![0; ENCHANT_FIELDS];
        gem_enchant[0] = 3_312;
        gem_enchant[ENCHANT_GEM] = 40_111;
        let mut plain = vec![0; ENCHANT_FIELDS];
        plain[0] = 1_900;
        // A tinker: charges -1, effect 7 in the second slot casting 54_758.
        plain[1] = (-1i32) as u32;
        plain[ENCHANT_EFFECT + 1] = USE_SPELL;
        plain[ENCHANT_ARG + 1] = 54_758;
        let props = vec![
            vec![25, 3_312, 0, 0, SOCKET_COLOR_RED],
            vec![26, 0, 0, 0, 10],
        ];
        let cat = parse_gem_catalog(&wdbc(&[gem_enchant, plain]), &wdbc(&props)).unwrap();
        assert_eq!(cat.gem_of_enchant(3_312), Some(40_111));
        assert_eq!(cat.gem_of_enchant(1_900), None, "an enchant no gem grants");
        assert_eq!(cat.use_spell(1_900), Some((54_758, -1)));
        assert_eq!(cat.use_spell(3_312), None, "a gem enchant casts nothing");
        assert_eq!(cat.color(25), Some(SOCKET_COLOR_RED));
        assert_eq!(cat.color(26), Some(SOCKET_COLOR_RED | SOCKET_COLOR_BLUE));
        assert_eq!(cat.color(27), None);
        assert_eq!(
            cat.gem_enchants().collect::<Vec<_>>(),
            vec![(3_312, 40_111)]
        );
    }

    #[test]
    fn the_1_12_shape_is_refused() {
        let enchant = vec![vec![0; 24]];
        let props = vec![vec![0; GEM_PROPERTIES_FIELDS]];
        assert!(parse_gem_catalog(&wdbc(&enchant), &wdbc(&props)).is_err());
    }
}
