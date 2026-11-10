//! The zones as the client's own data describes them, for the agent's loose zone progression
//! (`wp.goals`): where each zone is and what levels it suits, nothing the server alone knows.
//!
//! Lua: `AgentZones()` → `{ { id, name, map, faction, city, min, max, x, y }, ... }`, empty until
//! the client's tables are read.
//! - A zone is a top-level `AreaTable.dbc` row (no parent) that has a `WorldMapArea.dbc` map;
//!   `x, y` is the middle of the world rect that map covers, a point to walk toward.
//! - Its level band is read off its subareas' `ExplorationLevel` (the level exploring one is
//!   reckoned at; 12340's zone rows themselves hold 0): `min` and `max` are the 10th and 90th
//!   percentiles, so one stray row (Duskwood's level 10 corner) does not stretch the band. A zone
//!   with no levelled subarea is left out.
//! - `faction` is "Alliance" or "Horde" by the owner mask (2, 4), else "contested"; `city` marks
//!   the capital flag (`0x100`).

use bevy::prelude::*;
use mlua::Lua;
use std::sync::{Arc, Mutex};
use wrathplus_app_core::area::AreaTableRes;
use wrathplus_assets::{LockRecover, WorldAssets};
use wrathplus_formats::AreaTableCatalog;

/// One zone, as `AgentZones` hands it out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Zone {
    pub id: u32,
    pub name: String,
    pub map: u32,
    pub faction: &'static str,
    pub city: bool,
    pub min: i32,
    pub max: i32,
    pub x: f32,
    pub y: f32,
    /// The world rect its map covers, world x from `bottom` to `top` and y from `right` to `left`.
    pub rect: (f32, f32, f32, f32),
}

/// The zone a point lies in, by the client's world map: the smallest zone map on `map` whose
/// rect holds it. Zone maps overlap their neighbours at the edges, so near a border this is a
/// good guess rather than the terrain's own answer.
pub(crate) fn zone_at(zones: &[Zone], map: u32, x: f32, y: f32) -> Option<&Zone> {
    zones
        .iter()
        .filter(|z| {
            let (left, right, top, bottom) = z.rect;
            z.map == map
                && (bottom.min(top)..=bottom.max(top)).contains(&x)
                && (right.min(left)..=right.max(left)).contains(&y)
        })
        .min_by(|a, b| area(a).total_cmp(&area(b)))
}

fn area(z: &Zone) -> f32 {
    let (left, right, top, bottom) = z.rect;
    ((left - right) * (top - bottom)).abs()
}

type Known = Arc<Mutex<Vec<Zone>>>;

/// The Lua side: the zones once read.
#[derive(Clone, Default)]
pub(crate) struct Handles {
    known: Known,
}

#[derive(Resource)]
pub(crate) struct ZoneLink {
    known: Known,
    loaded: bool,
}

impl Handles {
    /// The name of the zone a point lies in ([`zone_at`]), once the zones are read.
    pub(crate) fn name_at(&self, map: u32, x: f32, y: f32) -> Option<String> {
        let known = self.known.lock().ok()?;
        zone_at(&known, map, x, y).map(|z| z.name.clone())
    }
}

pub(crate) fn link() -> (Handles, ZoneLink) {
    let known = Known::default();
    (
        Handles {
            known: known.clone(),
        },
        ZoneLink {
            known,
            loaded: false,
        },
    )
}

/// The capital flag (AzerothCore `AREA_FLAG_CAPITAL`).
const CAPITAL: u32 = 0x100;

/// The zones from the area catalog and the world map's rows, given as `(area id, map id, loc
/// rect (left, right, top, bottom))`: left and right are world y, top and bottom world x.
pub(crate) fn build(
    areas: &AreaTableCatalog,
    maps: impl Iterator<Item = (u32, u32, (f32, f32, f32, f32))>,
) -> Vec<Zone> {
    let mut levels: std::collections::HashMap<u32, Vec<i32>> = Default::default();
    for row in areas.rows() {
        if row.zone_id == 0 || !(1..=80).contains(&row.exploration_level) {
            continue;
        }
        if let Some(zone) = areas.top_zone(row.zone_id) {
            levels.entry(zone).or_default().push(row.exploration_level);
        }
    }
    let mut out = Vec::new();
    for (area, map, (left, right, top, bottom)) in maps {
        let Some(row) = areas.get(area).filter(|r| area != 0 && r.zone_id == 0) else {
            continue;
        };
        let Some(mut band) = levels.get(&area).cloned() else {
            continue;
        };
        band.sort_unstable();
        let at = |p: usize| band[(band.len() - 1) * p / 100];
        out.push(Zone {
            id: area,
            name: row.name.clone(),
            map,
            faction: match row.faction_group_mask {
                2 => "Alliance",
                4 => "Horde",
                _ => "contested",
            },
            city: row.flags & CAPITAL != 0,
            min: at(10),
            max: at(90),
            x: (top + bottom) / 2.0,
            y: (left + right) / 2.0,
            rect: (left, right, top, bottom),
        });
    }
    out.sort_by_key(|z| z.id);
    out
}

/// Read the zones once the area catalog is in, off the same patch chain the world map reads.
pub(crate) fn load(
    mut link: ResMut<ZoneLink>,
    areas: Option<Res<AreaTableRes>>,
    assets: Option<Res<WorldAssets>>,
) {
    if link.loaded {
        return;
    }
    let (Some(areas), Some(assets)) = (areas, assets) else {
        return;
    };
    link.loaded = true;
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_world_map_area_catalog(&mut chain)
    };
    let maps = match loaded {
        Ok(maps) => maps,
        Err(e) => {
            warn!("agent port: no zones for agents, WorldMapArea failed to load: {e:#}");
            return;
        }
    };
    let rows = maps.iter().map(|(_, a)| {
        let rect = (a.loc_left, a.loc_right, a.loc_top, a.loc_bottom);
        (a.area_id, a.map_id, rect)
    });
    let zones = build(&areas.0, rows);
    info!("agent port: {} zones for agents", zones.len());
    if let Ok(mut known) = link.known.lock() {
        *known = zones;
    }
}

pub(crate) fn install(lua: &Lua, handles: &Handles) -> mlua::Result<()> {
    let known = handles.known.clone();
    lua.globals().set(
        "AgentZones",
        lua.create_function(move |lua, ()| {
            let out = lua.create_table()?;
            let zones = known.lock().map(|k| k.clone()).unwrap_or_default();
            for z in zones {
                let t = lua.create_table()?;
                t.set("id", z.id)?;
                t.set("name", z.name)?;
                t.set("map", z.map)?;
                t.set("faction", z.faction)?;
                t.set("city", z.city)?;
                t.set("min", z.min)?;
                t.set("max", z.max)?;
                t.set("x", z.x)?;
                t.set("y", z.y)?;
                out.push(t)?;
            }
            Ok(out)
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::AreaTableRow;

    fn row(zone_id: u32, level: i32, mask: u32, flags: u32, name: &str) -> AreaTableRow {
        AreaTableRow {
            map_id: 0,
            zone_id,
            explore_flag: 0,
            flags,
            faction_group_mask: mask,
            exploration_level: level,
            name: name.into(),
        }
    }

    #[test]
    fn a_zones_band_comes_from_its_subareas_without_the_stray_rows() {
        let mut rows = vec![
            (10, row(0, 0, 0, 0x40, "Duskwood")),
            (1519, row(0, 10, 2, 0x200138, "Stormwind City")),
            (21, row(0, 0, 0, 0, "Kul Tiras")),
        ];
        // Duskwood: one level 10 corner, the rest 19 to 30; a sub-subarea counts for the zone.
        for (i, level) in [10, 19, 19, 24, 24, 25, 25, 25, 26, 26, 28, 28, 28, 30]
            .into_iter()
            .enumerate()
        {
            rows.push((100 + i as u32, row(10, level, 0, 0, "x")));
        }
        rows.push((200, row(100, 27, 0, 0, "Darkshire Inn")));
        rows.push((300, row(1519, 10, 2, 0, "Trade District")));
        rows.push((301, row(1519, -1, 2, 0, "Unlevelled")));
        let areas = AreaTableCatalog::from_rows(rows);
        let maps = [
            (10, 0, (833.0, -1866.0, -9716.0, -11516.0)),
            (1519, 0, (1722.0, -14.0, -7996.0, -9153.0)),
            (21, 0, (0.0, 0.0, 0.0, 0.0)),
            (0, 0, (0.0, 0.0, 0.0, 0.0)),
        ];
        let zones = build(&areas, maps.into_iter());
        assert_eq!(
            zones.len(),
            2,
            "Kul Tiras has no levels, the continent row no zone"
        );
        let dusk = &zones[0];
        assert_eq!(
            (dusk.name.as_str(), dusk.min, dusk.max),
            ("Duskwood", 19, 28)
        );
        assert_eq!((dusk.faction, dusk.city), ("contested", false));
        assert_eq!((dusk.x, dusk.y), (-10616.0, -516.5));
        let city = &zones[1];
        assert_eq!((city.faction, city.city, city.min), ("Alliance", true, 10));
        // A point is in the smallest zone map that holds it: Stormwind's inside Elwynn-sized
        // rects, Duskwood's south of it; nothing off the maps or on another map.
        let name = |map, x, y| zone_at(&zones, map, x, y).map(|z| z.name.as_str());
        assert_eq!(name(0, -8900.0, 600.0), Some("Stormwind City"));
        assert_eq!(name(0, -10500.0, -1000.0), Some("Duskwood"));
        assert_eq!(name(0, -5000.0, 0.0), None);
        assert_eq!(name(1, -10500.0, -1000.0), None);
        let mut wide = zones.clone();
        wide[0].rect = (2000.0, -2000.0, -7000.0, -12000.0);
        assert_eq!(
            zone_at(&wide, 0, -8900.0, 600.0).map(|z| z.name.as_str()),
            Some("Stormwind City")
        );
    }
}
