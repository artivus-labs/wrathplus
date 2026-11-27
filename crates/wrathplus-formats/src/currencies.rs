//! Native build-12340 token currencies. Layouts: AzerothCore `CurrencyTypesEntry` and
//! `CurrencyCategoryEntry` (`DBCStructure.h`), checked against installed table headers.
//! Names/icons belong to the referenced item, not CurrencyTypes. Honor/arena points use
//! player descriptors and have no rows here. Unknown category ids remain intact.

use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{parse, str_at, u32_at};
use crate::{Chain, DataProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyType {
    pub id: u32,
    pub item_id: u32,
    pub category_id: u32,
    /// One-based index in the two-word PLAYER_FIELD_KNOWN_CURRENCIES descriptor.
    pub bit_index: u32,
}

impl CurrencyType {
    /// AzerothCore `Player::AddKnownCurrency`: 1 << (BitIndex - 1).
    /// Malformed zero/out-of-range indices never wrap or panic.
    pub fn known_mask(&self) -> Option<u64> {
        1u64.checked_shl(self.bit_index.checked_sub(1)?)
    }
}

/// The rows in file order, which is not id order: 12340 walks the table as loaded (`0x5b0340`).
#[derive(Debug, Default)]
pub struct CurrencyCatalog {
    rows: Vec<CurrencyType>,
    ids: BTreeMap<u32, usize>,
    item_ids: BTreeMap<u32, usize>,
}

impl CurrencyCatalog {
    pub fn get(&self, id: u32) -> Option<&CurrencyType> {
        self.rows.get(*self.ids.get(&id)?)
    }
    /// The first row naming the item, as `0x5affd0` finds it.
    pub fn get_by_item(&self, item_id: u32) -> Option<&CurrencyType> {
        self.rows.get(*self.item_ids.get(&item_id)?)
    }
    pub fn iter(&self) -> impl Iterator<Item = &CurrencyType> {
        self.rows.iter()
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyCategory {
    pub id: u32,
    pub flags: u32,
    /// Selected archive locale slot, without substituting a different language.
    pub name: String,
}

#[derive(Debug, Default)]
pub struct CurrencyCategoryCatalog(BTreeMap<u32, CurrencyCategory>);

impl CurrencyCategoryCatalog {
    pub fn get(&self, id: u32) -> Option<&CurrencyCategory> {
        self.0.get(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &CurrencyCategory> {
        self.0.values()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn schema(name: &str, fields: usize, string_col: Option<usize>) -> Schema {
    let mut schema = Schema::new(name);
    for i in 0..fields {
        schema.add_field(SchemaField::new(
            format!("c{i}"),
            if string_col == Some(i) {
                FieldType::String
            } else {
                FieldType::UInt32
            },
        ));
    }
    schema
}

fn currencies(bytes: &[u8]) -> Result<CurrencyCatalog> {
    let rs = parse(bytes, schema("CurrencyTypes", 4, None), "CurrencyTypes")?;
    let mut catalog = CurrencyCatalog::default();
    for r in rs.records() {
        let row = CurrencyType {
            id: u32_at(r, 0).context("currency id")?,
            item_id: u32_at(r, 1).context("currency item")?,
            category_id: u32_at(r, 2).context("currency category")?,
            bit_index: u32_at(r, 3).context("currency bit index")?,
        };
        let at = catalog.rows.len();
        catalog.item_ids.entry(row.item_id).or_insert(at);
        catalog.ids.insert(row.id, at);
        catalog.rows.push(row);
    }
    Ok(catalog)
}

fn categories(bytes: &[u8], locale: &str) -> Result<CurrencyCategoryCatalog> {
    let col = 2 + crate::cfg_categories::locale_slot(locale)?;
    let rs = parse(
        bytes,
        schema("CurrencyCategory", 19, Some(col)),
        "CurrencyCategory",
    )?;
    let mut rows = BTreeMap::new();
    for r in rs.records() {
        let id = u32_at(r, 0).context("currency category id")?;
        rows.insert(
            id,
            CurrencyCategory {
                id,
                flags: u32_at(r, 1).context("currency category flags")?,
                name: str_at(&rs, r, col).unwrap_or_default(),
            },
        );
    }
    Ok(CurrencyCategoryCatalog(rows))
}

/// Load Wrath token metadata; Vanilla has no corresponding table.
pub fn load_currency_types(chain: &mut Chain) -> Result<CurrencyCatalog> {
    ensure!(
        matches!(chain.profile(), DataProfile::Wrath { .. }),
        "CurrencyTypes requires Wrath data"
    );
    currencies(
        &chain
            .read_file("DBFilesClient\\CurrencyTypes.dbc")
            .context("reading CurrencyTypes")?,
    )
}

/// Load category labels using the chain's explicitly selected archive locale.
pub fn load_currency_categories(chain: &mut Chain) -> Result<CurrencyCategoryCatalog> {
    let DataProfile::Wrath { locale } = chain.profile();
    let locale = locale.clone();
    categories(
        &chain
            .read_file("DBFilesClient\\CurrencyCategory.dbc")
            .context("reading CurrencyCategory")?,
        &locale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbc(rows: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
        let fields = rows[0].len() as u32;
        let mut bytes = b"WDBC".to_vec();
        for n in [rows.len() as u32, fields, fields * 4, strings.len() as u32] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        for row in rows {
            for n in row {
                bytes.extend_from_slice(&n.to_le_bytes());
            }
        }
        bytes.extend_from_slice(strings);
        bytes
    }

    #[test]
    fn item_id_dbc_id_category_and_bit_are_distinct() {
        let c = currencies(&dbc(
            &[vec![341, 49426, 22, 29], vec![141, 43949, 2089878896, 20]],
            b"\0",
        ))
        .unwrap();
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
        assert_eq!(c.get_by_item(49426), c.get(341));
        assert!(c.get(49426).is_none());
        assert!(c.get_by_item(341).is_none());
        let frost = c.get(341).unwrap();
        assert_eq!(frost.category_id, 22);
        assert_eq!(frost.known_mask(), Some(1 << 28));
        assert_eq!(c.get(141).unwrap().category_id, 2089878896);
        let order: Vec<u32> = c.iter().map(|row| row.id).collect();
        assert_eq!(order, [341, 141], "file order, not id order");
    }

    #[test]
    fn known_currency_bit_boundaries_do_not_wrap() {
        let mut row = CurrencyType {
            id: 1,
            item_id: 2,
            category_id: 3,
            bit_index: 0,
        };
        for (bit, expected) in [
            (0, None),
            (1, Some(1)),
            (32, Some(1 << 31)),
            (33, Some(1 << 32)),
            (64, Some(1 << 63)),
            (65, None),
            (u32::MAX, None),
        ] {
            row.bit_index = bit;
            assert_eq!(row.known_mask(), expected);
        }
    }

    #[test]
    fn category_uses_selected_locale_and_preserves_flags() {
        let mut row = vec![0; 19];
        row[0] = 22;
        row[1] = 3;
        row[2] = 1;
        row[4] = 9;
        let bytes = dbc(&[row], b"\0English\0Francais\0");
        let en = categories(&bytes, "enUS").unwrap();
        assert_eq!(en.get(22).unwrap().name, "English");
        assert_eq!(en.get(22).unwrap().flags, 3);
        assert_eq!(
            categories(&bytes, "frFR").unwrap().get(22).unwrap().name,
            "Francais"
        );
        assert_eq!(
            categories(&bytes, "deDE").unwrap().get(22).unwrap().name,
            ""
        );
        assert_eq!(en.len(), 1);
        assert!(!en.is_empty());
        assert_eq!(en.iter().count(), 1);
        assert!(en.get(24).is_none());
        assert!(categories(&bytes, "invalid").is_err());
    }

    #[test]
    fn changed_layouts_fail_instead_of_guessing() {
        assert!(currencies(&dbc(&[vec![1, 2, 3]], b"\0")).is_err());
        assert!(categories(&dbc(&[vec![0; 18]], b"\0"), "enUS").is_err());
    }
}
