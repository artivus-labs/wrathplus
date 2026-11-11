//! `ChrRaces.dbc` column 9: the creature type a player's race reads as, the last stage of the
//! creature-type resolver (`0x605570`: race byte → row `[0xc0dee0][race]`, offset `0x24`, at
//! `0x6055ca`). The runtime uses a frozen copy, `spell::unit_type::race_creature_type`; this
//! loader backs the test that pins it.

use std::collections::HashMap;

use crate::Chain;
use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, RecordSet, Schema, SchemaField};

use crate::dbc::{parse, u32_at};

const CHR_RACES: &str = "DBFilesClient\\ChrRaces.dbc";

/// `ChrRaces.dbc` under its 1.12 schema, which [`crate::wrath_layouts`] maps onto 12340's.
pub(crate) fn read_chr_races(chain: &mut Chain) -> Result<RecordSet> {
    let bytes = chain
        .read_file(CHR_RACES)
        .with_context(|| format!("reading {CHR_RACES}"))?;
    let mut schema = Schema::new("ChrRaces");
    for i in 0..29 {
        let ty = match i {
            15 | 26 | 27 | 28 => FieldType::String,
            _ => FieldType::UInt32,
        };
        schema.add_field(SchemaField::new(format!("f{i}"), ty));
    }
    parse(&bytes, schema, "ChrRaces")
}

/// Each `ChrRaces.dbc` row's creature type by race id; a race without a row is absent.
pub fn load_race_creature_types(chain: &mut Chain) -> Result<HashMap<u8, u32>> {
    let rs = read_chr_races(chain)?;
    Ok(rs
        .records()
        .iter()
        .filter_map(|r| Some((u8::try_from(u32_at(r, 0)?).ok()?, u32_at(r, 9)?)))
        .collect())
}
