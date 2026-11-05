//! `WMOAreaTable.dbc`, a WMO interior's audio identity per building and per group; the Northshire
//! Abbey chant is its whole-WMO row's `ZoneIntroMusicTable` 221. Rows key on the root's
//! `MOHD.wmoID`, the placement's `MODF.nameSet` and each group's `MOGP.uniqueID` (`+0x38`), with
//! `WMOGroupID` `-1` for the whole-WMO row; the enUS `AreaName` is column 11 of 20.

use std::collections::HashMap;

use anyhow::{Context, Result};
use wrathplus_dbc::{FieldType, Schema, SchemaField};

use crate::dbc::{parse, str_at, u32_at};
use crate::Chain;

/// One `WMOAreaTable` row's audio fields, `0` for none or inherit.
#[derive(Clone)]
pub struct WmoArea {
    /// The row id, the client's indoor dedup key (`[0x86860c]`): while it stays the same, the
    /// zone-text update is skipped.
    pub id: u32,
    /// `SoundProviderPreferences` ids, `[dry, underwater]`.
    pub sound_provider: [u32; 2],
    /// `SoundAmbience.dbc` id.
    pub ambience: u32,
    /// `ZoneMusic.dbc` id.
    pub zone_music: u32,
    /// `ZoneIntroMusicTable.dbc` id, the entry fanfare.
    pub intro_sound: u32,
    /// `Flags`: `4` outdoors and `2` indoors, which the outdoor test `0x77fbf0` reads (`+0x24`)
    /// ahead of the group's own exterior bit.
    pub flags: u32,
    /// The `AreaTable.dbc` area this group counts as (0 = none), read by the area resolver
    /// `0x670250` when the down-ray keeps the WMO over the terrain: Ironforge reports its own area.
    pub area_table_id: u32,
    /// The interior's enUS display name, empty for an unnamed row.
    pub name: String,
}

/// The table keyed for the runtime lookup: exact group rows and whole-WMO defaults.
pub struct WmoAreaCatalog {
    /// `(WMOID, NameSetID, WMOGroupID)` to its group row.
    groups: HashMap<(u32, u32, u32), WmoArea>,
    /// `(WMOID, NameSetID)` to the `WMOGroupID == −1` whole-WMO row.
    defaults: HashMap<(u32, u32), WmoArea>,
}

impl WmoAreaCatalog {
    /// The hit group's own row by exact key, the zone-text chain's second query (`0x69d8f0`, one
    /// bsearch at `0x7ccd30` with no name-set retry, default fallback or overlay): its name, if
    /// any, refills the subzone slot.
    pub fn group_row(&self, wmo_id: u32, name_set: u32, group_id: u32) -> Option<&WmoArea> {
        self.groups.get(&(wmo_id, name_set, group_id))
    }

    /// The whole-WMO row by exact key, the zone-text chain's first query (`0x69d830` to
    /// `0x7ccde0`, `push 0xffffffff`): a name unlike the subzone overrides the zone slot, an
    /// unnamed row falls back to an `AreaTable` name, and a missing row never overrides.
    pub fn default_row(&self, wmo_id: u32, name_set: u32) -> Option<&WmoArea> {
        self.defaults.get(&(wmo_id, name_set))
    }

    /// A WMO interior's audio, the group row's non-zero fields over the whole-WMO row's: the
    /// abbey's chant, music and ambience sit on its whole-WMO row. A name set without rows falls
    /// back to set 0; the reference's handling of that miss is untraced.
    pub fn resolve(&self, wmo_id: u32, name_set: u32, group_id: u32) -> Option<WmoArea> {
        let (group, default) = [name_set, 0]
            .iter()
            .map(|&ns| {
                (
                    self.groups.get(&(wmo_id, ns, group_id)),
                    self.defaults.get(&(wmo_id, ns)),
                )
            })
            .find(|(g, d)| g.is_some() || d.is_some())?;
        let base = default.cloned().unwrap_or_else(|| WmoArea {
            id: 0,
            sound_provider: [0, 0],
            ambience: 0,
            zone_music: 0,
            intro_sound: 0,
            flags: 0,
            area_table_id: 0,
            name: String::new(),
        });
        let Some(g) = group else { return Some(base) };
        let nz = |v: u32, b: u32| if v != 0 { v } else { b };
        Some(WmoArea {
            id: g.id,
            sound_provider: [
                nz(g.sound_provider[0], base.sound_provider[0]),
                nz(g.sound_provider[1], base.sound_provider[1]),
            ],
            ambience: nz(g.ambience, base.ambience),
            zone_music: nz(g.zone_music, base.zone_music),
            intro_sound: nz(g.intro_sound, base.intro_sound),
            flags: g.flags,
            area_table_id: nz(g.area_table_id, base.area_table_id),
            name: if g.name.is_empty() {
                base.name
            } else {
                g.name.clone()
            },
        })
    }

    pub fn len(&self) -> usize {
        self.groups.len() + self.defaults.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn schema() -> Schema {
    let mut s = Schema::new("WMOAreaTable");
    for name in [
        "ID",
        "WMOID",
        "NameSetID",
        "WMOGroupID",
        "SoundProviderPref",
        "SoundProviderPrefUnderwater",
        "AmbienceID",
        "ZoneMusic",
        "IntroSound",
        "Flags",
        "AreaTableID",
    ] {
        s.add_field(SchemaField::new(name, FieldType::UInt32));
    }
    s.add_field(SchemaField::new("AreaName", FieldType::String));
    for i in 12..20 {
        s.add_field(SchemaField::new(format!("_pad{i}"), FieldType::UInt32));
    }
    s
}

/// Read `WMOAreaTable.dbc` off the patch chain.
pub fn load_wmo_area_catalog(chain: &mut Chain) -> Result<WmoAreaCatalog> {
    let bytes = chain
        .read_file("DBFilesClient\\WMOAreaTable.dbc")
        .context("reading WMOAreaTable.dbc")?;
    let rs = parse(&bytes, schema(), "WMOAreaTable")?;
    let mut groups = HashMap::new();
    let mut defaults = HashMap::new();
    for r in rs.records() {
        let wmo_id = u32_at(r, 1).unwrap_or(0);
        let name_set = u32_at(r, 2).unwrap_or(0);
        let group = u32_at(r, 3).unwrap_or(0);
        let area = WmoArea {
            id: u32_at(r, 0).unwrap_or(0),
            sound_provider: [u32_at(r, 4).unwrap_or(0), u32_at(r, 5).unwrap_or(0)],
            ambience: u32_at(r, 6).unwrap_or(0),
            zone_music: u32_at(r, 7).unwrap_or(0),
            intro_sound: u32_at(r, 8).unwrap_or(0),
            flags: u32_at(r, 9).unwrap_or(0),
            area_table_id: u32_at(r, 10).unwrap_or(0),
            name: str_at(&rs, r, 11).unwrap_or_default(),
        };
        if group == u32::MAX {
            defaults.insert((wmo_id, name_set), area);
        } else {
            groups.insert((wmo_id, name_set, group), area);
        }
    }
    Ok(WmoAreaCatalog { groups, defaults })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, flags: u32, intro_sound: u32) -> WmoArea {
        WmoArea {
            id,
            sound_provider: [0, 0],
            ambience: 0,
            zone_music: 0,
            intro_sound,
            flags,
            area_table_id: 0,
            name: String::new(),
        }
    }

    #[test]
    fn a_resolved_group_keeps_its_own_flags_over_the_whole_wmo_row() {
        let catalog = WmoAreaCatalog {
            groups: HashMap::from([((7, 0, 3), row(30, 2, 0)), ((7, 0, 4), row(40, 0, 0))]),
            defaults: HashMap::from([((7, 0), row(1, 4, 221))]),
        };
        assert_eq!(catalog.group_row(7, 0, 3).map(|r| r.flags), Some(2));
        let room = catalog.resolve(7, 0, 3).unwrap();
        assert_eq!((room.flags, room.intro_sound), (2, 221));
        assert_eq!(catalog.resolve(7, 0, 4).unwrap().flags, 0);
        assert_eq!(catalog.resolve(7, 0, 9).unwrap().flags, 4);
    }
}
