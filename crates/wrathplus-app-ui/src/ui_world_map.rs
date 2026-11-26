//! The world-map data feed behind the stock `WorldMapFrame.xml` and wrathplus-ui's map verbs:
//!
//! - [`seed_world_map_catalog`], at world entry: the static catalog from `WorldMapArea`,
//!   `AreaTable`, `WorldMapContinent`, `Map` and the `.zmp` bitmaps, with the instance maps no
//!   continent owns beside it, plus build 12340's dungeon-map tables
//!   ([`wrathplus_formats::world_map_wrath`]): a zone belongs to the continent its `DisplayMapID`
//!   names (Eversong on the Eastern Kingdoms), else its own map's.
//! - [`feed_world_map`], every frame: the player's selection, the blips projected onto the
//!   displayed map, and the landmark list, rebuilt when its inputs change.
//! - [`dev_map_jump`]: Alt+click the map to go there.
//!
//! Build 12340's projection (`0x544140`, `/tmp/azb-wire-ghidra/worldmap`) first moves a position
//! through `WorldMapTransforms` (`0x543920`). The world sheet, a continent and its own art then
//! read the moved position; a zone reads the original, on the original or moved map. An instance
//! map projects through its displayed floor's `DungeonMap` rect when the position is on that floor,
//! hides it when it is on another, and uses the area rect when no floor is shown.
//!
//! The floor a position is on is `0x5439e0`'s scan of `DungeonMapChunk.dbc` ([`floor_at`]).
//!
//! `SetMapToCurrentZone`'s resolver is 12340's `0x547170` ([`resolve_player_selection`]). Labelled,
//! not ported: on a continent, a zone row with an all-zero rect takes the player's floor or its
//! `ParentWorldMapID` zone (`0x5477de`), which the catalog does not carry; off one, an instance
//! with no area row maps to the continent cell under its `Map.dbc` entrance (`0x5477ae`, "Inside
//! Instance"), and failing that the cosmic sheet (`0x5477bd`, "True Orphan"); both give the
//! default selection here.

use bevy::ecs::system::NonSendMut;
use bevy::prelude::*;

use std::collections::HashMap;
use wrathplus_assets::coords::bevy_to_wow;

use wrathplus_formats::world_map_wrath::{
    AreaPoiLinks, DungeonMapChunk, WorldMapTransform, WrathMapTables,
};
use wrathplus_formats::{
    load_world_map_area_catalog, load_world_map_continent_catalog, load_world_map_overlay_catalog,
    load_zone_map, WorldMapArea,
};
use wrathplus_ui::script::{
    DungeonFloorView, MapPos, MapSelection, UiScript, WorldMapAreaView, WorldMapContinentView,
    WorldMapLandmarkView, WorldMapOverlayView, WorldMapZoneView,
};

use crate::app_core::player::Player;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use wrathplus_assets::MapCatalogRes;
use wrathplus_assets::{LockRecover, WorldAssets};
use wrathplus_world::map_proj::{self, WorldProj, ZoneRect};
use wrathplus_world::world_map::CurrentMap;

/// The app's copy of the pushed catalog's projection data (rects, world-sheet constants), in the
/// engine's order: the indices must agree.
#[derive(Resource, Clone)]
pub(crate) struct WorldMapUiData {
    continents: Vec<ContinentEntry>,
    /// The instance maps no continent owns, `0x4a5d00`'s third array in `WorldMapArea.dbc` file
    /// order, unsorted: what the `-2` direct-area selection resolves against.
    direct: Vec<DirectAreaEntry>,
    /// Every `WorldMapArea` row by id, for a selection by id (`SetMapByID`, a floor's map).
    rows: HashMap<u32, RowEntry>,
    /// `DungeonMap.dbc`, file order.
    floors: Vec<FloorEntry>,
    chunks: Vec<DungeonMapChunk>,
    transforms: Vec<WorldMapTransform>,
    /// `AreaPOI.dbc`'s icons and map links by row id.
    poi_links: HashMap<u32, AreaPoiLinks>,
    /// What the engine is handed for its own walks.
    area_views: HashMap<u32, WorldMapAreaView>,
    floor_views: Vec<DungeonFloorView>,
}

#[derive(Clone)]
struct RowEntry {
    map_id: u32,
    area_id: u32,
    rect: ZoneRect,
}

#[derive(Clone)]
struct FloorEntry {
    id: u32,
    /// The floor's `DungeonMap` rect as a loc rect: `MaxX`/`MinX` are its left and right edges,
    /// `MaxY`/`MinY` its top and bottom (`0x544140`).
    rect: ZoneRect,
}

#[derive(Clone)]
pub(crate) struct ContinentEntry {
    map_id: u32,
    wma_id: u32,
    proj: Option<WorldProj>,
    rect: ZoneRect,
    zones: Vec<ZoneEntry>,
}

#[derive(Clone)]
struct ZoneEntry {
    wma_id: u32,
    area_id: u32,
    rect: ZoneRect,
    /// `DefaultDungeonFloor`: a zone opening on a floor (Dalaran) is that floor's instance map.
    default_floor: i32,
}

/// One instance map, selected directly rather than through a continent and zone; 1.12 ships
/// three, the battlegrounds.
#[derive(Clone)]
struct DirectAreaEntry {
    /// The `WorldMapArea` row id, which the third selection cell `[0x845074]` holds (`0x4a6717`);
    /// never a list index.
    id: u32,
    /// Matched against the player's map by the resolver (`0x4a66f6`); the projection admits only a
    /// body on this map (`0x4a7437`).
    map_id: u32,
    /// The row's loc rect, which `0x4a7360`'s direct-area leg projects through as a zone's. Only
    /// the projection tests read it: the leg goes through the selected row's rect.
    #[cfg(test)]
    rect: ZoneRect,
    /// What the engine is handed for this row (`GetMapInfo`'s art folder, the overlays).
    view: WorldMapZoneView,
}

fn zone_rect(a: &WorldMapArea) -> ZoneRect {
    ZoneRect {
        left: a.loc_left,
        right: a.loc_right,
        top: a.loc_top,
        bottom: a.loc_bottom,
    }
}

/// One `WorldMapArea` row as the engine sees it. Zones, cities and instance maps are alike: one
/// `0x4a6cf0` lookup answers `GetMapInfo` for all three.
fn map_row_view(
    wma_id: u32,
    a: &WorldMapArea,
    name: String,
    areas: &wrathplus_formats::AreaTableCatalog,
    wmo: &wrathplus_formats::WorldMapOverlayCatalog,
) -> WorldMapZoneView {
    WorldMapZoneView {
        name,
        wma_id,
        area_id: a.area_id,
        map_file: a.name.clone(),
        loc_rect: (a.loc_left, a.loc_right, a.loc_top, a.loc_bottom),
        // WorldMapOverlay rows by WMA id, each revealed by its covered areas' `exploreFlag` bits.
        overlays: wmo
            .for_area(wma_id)
            .iter()
            .map(|o| WorldMapOverlayView {
                texture: format!("Interface\\WorldMap\\{}\\{}", a.name, o.texture_name),
                width: o.texture_width,
                height: o.texture_height,
                offset_x: o.offset_x,
                offset_y: o.offset_y,
                explore_bits: o
                    .area_id
                    .iter()
                    .filter(|&&aid| aid != 0)
                    .filter_map(|&aid| areas.get(aid).map(|r| r.explore_flag))
                    .collect(),
                // An area row with a negative `ExplorationLevel` reveals without exploring
                // (`0x546310`'s overlay walk).
                always: o
                    .area_id
                    .iter()
                    .filter(|&&aid| aid != 0)
                    .any(|&aid| areas.get(aid).is_some_and(|r| r.exploration_level < 0)),
                hit_rect: (
                    o.hit_rect_top,
                    o.hit_rect_left,
                    o.hit_rect_bottom,
                    o.hit_rect_right,
                ),
                // The hover label is the first area slot's name (`0x4a7fa0` reads `+0x8` only);
                // with no row the overlay is hidden from the hover, not from the draw.
                area_name: areas.name(o.area_id[0]).map(str::to_string),
            })
            .collect(),
    }
}

/// The catalog off the patch chain, the pure half of [`seed_world_map_catalog`]; `None` when a
/// DBC fails to load, which disables the map window.
pub(crate) fn build_catalog(
    chain: &mut wrathplus_formats::Chain,
    areas: &wrathplus_formats::AreaTableCatalog,
    maps: &MapCatalogRes,
) -> Option<(Vec<WorldMapContinentView>, WorldMapUiData)> {
    let loaded = load_world_map_area_catalog(&mut *chain).and_then(|wma| {
        let wmc = load_world_map_continent_catalog(&mut *chain)?;
        let wmo = load_world_map_overlay_catalog(&mut *chain)?;
        Ok((wma, wmc, wmo))
    });
    let (wma, wmc, wmo) = match loaded {
        Ok(t) => t,
        Err(e) => {
            error!("world map: DBC load failed, map window disabled: {e:#}");
            return None;
        }
    };
    let wrath = if matches!(
        chain.profile(),
        wrathplus_formats::DataProfile::Wrath { .. }
    ) {
        wrathplus_formats::world_map_wrath::load_wrath_map_tables(&mut *chain).unwrap_or_else(|e| {
            warn!("world map: dungeon-map tables failed to load, no floors: {e:#}");
            WrathMapTables::default()
        })
    } else {
        WrathMapTables::default()
    };
    let link = |id: u32| wrath.area_links.get(&id).copied();
    // The continent a row is drawn with: `DisplayMapID`, else its own map (`0x546c50`).
    let drawn_on = |id: u32, a: &WorldMapArea| match link(id).map(|l| l.display_map_id) {
        Some(m) if m >= 0 => m as u32,
        _ => a.map_id,
    };
    let default_floor = |id: u32| link(id).map_or(0, |l| l.default_dungeon_floor);

    // Continents: the areaId 0 rows in WorldMapArea file order, the Lua continent index
    // (`0x4a5d00`'s walk).
    let cont_rows: Vec<(u32, &WorldMapArea)> = wma.iter().filter(|(_, a)| a.area_id == 0).collect();

    let mut entries = Vec::with_capacity(cont_rows.len());
    let mut views = Vec::with_capacity(cont_rows.len());
    for (cont_id, cont) in cont_rows {
        // Zones: the other rows drawn with this continent, sorted case-insensitively by AreaTable
        // name, the Lua zone index (`0x4a6390`'s `SStrCmpI`).
        let mut zones: Vec<(u32, &WorldMapArea, String)> = wma
            .iter()
            .filter(|&(id, a)| a.area_id != 0 && drawn_on(id, a) == cont.map_id)
            .map(|(id, a)| {
                let name = areas.name(a.area_id).unwrap_or(a.name.as_str()).to_string();
                (id, a, name)
            })
            .collect();
        zones.sort_by_key(|(_, _, name)| name.to_lowercase());

        let proj = wmc.get(cont.map_id).map(|c| WorldProj {
            offset_u: c.offset_x,
            offset_v: c.offset_y,
            scale: c.scale,
        });
        // The world-sheet rect, `0x4a5d00` over the WorldMapContinent tile bounds: the world-level
        // click box, disjoint per continent unlike the art rects.
        let world_rect = wmc
            .get(cont.map_id)
            .zip(proj)
            .map(|(c, p)| {
                map_proj::continent_sheet_rect(
                    (
                        c.left_boundary,
                        c.right_boundary,
                        c.top_boundary,
                        c.bottom_boundary,
                    ),
                    p,
                )
            })
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        let rect = zone_rect(cont);

        // The area bitmap as 1-based zone indices: the load-time remap `0x4a6070`, a one-hop
        // parent rollup, then the (mapId, areaId) match.
        let zone_grid: Vec<u16> = match load_zone_map(&mut *chain, &cont.name) {
            Ok(grid) => grid
                .iter()
                .map(|&raw| {
                    let mut area_id = raw;
                    if let Some(row) = areas.get(area_id) {
                        if row.zone_id != 0 {
                            area_id = row.zone_id; // one hop, not a full walk
                        }
                    }
                    zones
                        .iter()
                        .position(|(_, a, _)| a.area_id == area_id)
                        .map(|i| i as u16 + 1)
                        .unwrap_or(0)
                })
                .collect(),
            Err(e) => {
                // The client tolerates a missing bitmap: hover and click are inert.
                warn!("world map: no zone bitmap for {}: {e:#}", cont.name);
                Vec::new()
            }
        };

        // `GetMapInfo`'s continent-level height, off the tile bounds (`0x543cd0`).
        let texture_height = wmc.get(cont.map_id).map_or(0, |c| {
            let w = c.right_boundary as f32 - c.left_boundary as f32 + 1.0;
            let h = c.bottom_boundary as f32 - c.top_boundary as f32 + 1.0;
            if w == 0.0 || h == 0.0 {
                0
            } else {
                ((h * 1002.0) / w).round_ties_even() as i64 as u32
            }
        });
        views.push(WorldMapContinentView {
            map_id: cont.map_id,
            wma_id: cont_id,
            texture_height,
            // Map.dbc's localized name, never the art folder (`0x4a65a0`).
            name: maps
                .0
                .name(cont.map_id)
                .unwrap_or(cont.name.as_str())
                .to_string(),
            map_file: cont.name.clone(),
            world_rect,
            loc_rect: (cont.loc_left, cont.loc_right, cont.loc_top, cont.loc_bottom),
            zone_grid,
            zones: zones
                .iter()
                .map(|(wma_id, a, name)| map_row_view(*wma_id, a, name.clone(), areas, &wmo))
                .collect(),
        });
        entries.push(ContinentEntry {
            map_id: cont.map_id,
            wma_id: cont_id,
            proj,
            rect,
            zones: zones
                .iter()
                .map(|(id, a, _)| ZoneEntry {
                    wma_id: *id,
                    area_id: a.area_id,
                    rect: zone_rect(a),
                    default_floor: default_floor(*id),
                })
                .collect(),
        });
    }

    // The instance maps: rows with `areaID != 0` on no continent's map, in file order, unsorted
    // (`0x4a5d00`'s passes `0x4a6130` and `0x4a61c9`); the `areaID` test is the reference's,
    // though inert on the shipped data (`0x4a6170`).
    let continent_maps: Vec<u32> = entries.iter().map(|c| c.map_id).collect();
    let direct: Vec<DirectAreaEntry> = wma
        .iter()
        .filter(|&(id, a)| a.area_id != 0 && !continent_maps.contains(&drawn_on(id, a)))
        .map(|(id, a)| DirectAreaEntry {
            id,
            map_id: a.map_id,
            #[cfg(test)]
            rect: zone_rect(a),
            view: map_row_view(
                id,
                a,
                areas.name(a.area_id).unwrap_or(a.name.as_str()).to_string(),
                areas,
                &wmo,
            ),
        })
        .collect();

    let rows = wma
        .iter()
        .map(|(id, a)| {
            (
                id,
                RowEntry {
                    map_id: a.map_id,
                    area_id: a.area_id,
                    rect: zone_rect(a),
                },
            )
        })
        .collect();
    let area_views = wma
        .iter()
        .map(|(id, a)| {
            let l = link(id);
            (
                id,
                WorldMapAreaView {
                    id,
                    map_id: a.map_id,
                    area_id: a.area_id,
                    map_file: a.name.clone(),
                    loc_rect: (a.loc_left, a.loc_right, a.loc_top, a.loc_bottom),
                    display_map: l.map_or(-1, |l| l.display_map_id),
                    default_floor: l.map_or(0, |l| l.default_dungeon_floor),
                    parent: l.map_or(0, |l| l.parent_world_map_id),
                },
            )
        })
        .collect();
    let floors = wrath
        .floors
        .iter()
        .map(|f| FloorEntry {
            id: f.id,
            rect: ZoneRect {
                left: f.max_x,
                right: f.min_x,
                top: f.max_y,
                bottom: f.min_y,
            },
        })
        .collect();
    let floor_views = wrath
        .floors
        .iter()
        .map(|f| DungeonFloorView {
            id: f.id,
            map_id: f.map_id,
            floor_index: f.floor_index,
            rect: (f.min_x, f.max_x, f.min_y, f.max_y),
            parent: f.parent_world_map_id,
        })
        .collect();

    Some((
        views,
        WorldMapUiData {
            continents: entries,
            direct,
            rows,
            floors,
            chunks: wrath.chunks,
            transforms: wrath.transforms,
            poi_links: wrath.poi_links,
            area_views,
            floor_views,
        },
    ))
}

/// The built continent catalog, kept for the process: the walk cannot change, so a later login
/// re-seeds its VM from here. The instance maps ride [`WorldMapUiData`].
#[derive(Resource)]
pub(crate) struct WorldMapCatalog(pub(crate) Vec<WorldMapContinentView>);

/// Push the map catalog into the VM before any interface file runs, from
/// [`crate::ui_script::load_ingame_ui_on_world_entry`]: addons read `GetMapContinents` and
/// `GetMapZones` at file scope (Astrolabe builds its zone table there), and the reference holds
/// this DBC data from load.
pub(crate) fn seed_world_map_catalog(world: &mut World, script: &mut UiScript) {
    if !world.contains_resource::<WorldMapCatalog>() {
        let Some((views, data)) = build_catalog_from_world(world) else {
            return;
        };
        info!(
            "world map: catalog — {} continents, {} zones, {} instance maps",
            views.len(),
            views.iter().map(|c| c.zones.len()).sum::<usize>(),
            data.direct.len()
        );
        world.insert_resource(data);
        world.insert_resource(WorldMapCatalog(views));
    }
    let Some(catalog) = world.get_resource::<WorldMapCatalog>() else {
        return;
    };
    script.set_world_map_catalog(catalog.0.clone());
    // The instance maps, at the same edge: the reference fills both containers in one walk.
    if let Some(data) = world.get_resource::<WorldMapUiData>() {
        script.set_world_map_direct_areas(
            data.direct
                .iter()
                .map(|d| (d.id, d.view.clone()))
                .collect::<Vec<_>>(),
        );
        script.set_world_map_areas(data.area_views.clone(), data.floor_views.clone());
    }
}

/// [`build_catalog`] over the app's `Startup` resources; only a bare test world gets `None`.
fn build_catalog_from_world(world: &World) -> Option<(Vec<WorldMapContinentView>, WorldMapUiData)> {
    let assets = world.get_resource::<WorldAssets>()?;
    let maps = world.get_resource::<MapCatalogRes>()?;
    let areas = world.get_resource::<crate::area::AreaTableRes>()?;
    let mut chain = assets.chain.lock_recover();
    build_catalog(&mut chain, &areas.0, maps)
}

/// The displayed level the landmark gates branch on, from the selection cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MapLevel {
    /// Both continents on one sheet (continent `-1`).
    World,
    /// One continent, no zone selected (`continent >= 0, zone == -1`).
    Continent,
    /// One zone (`continent >= 0, zone >= 0`).
    Zone,
    /// An instance map or the cosmic sheet (continent `-2`).
    MinusTwo,
}

impl MapLevel {
    fn of(sel: MapSelection) -> Self {
        if sel.direct.is_some() || sel.cosmic {
            Self::MinusTwo
        } else if sel.continent == 0 {
            Self::World
        } else if sel.zone == 0 {
            Self::Continent
        } else {
            Self::Zone
        }
    }
}

/// The inputs the landmark list is a pure function of: a selection change, a world-state push, a
/// `PLAYER_EXPLORED_ZONES` change, the gossip marker's set and clear, and the player's WMO group,
/// which picks a position's floor.
#[derive(PartialEq, Eq)]
struct LandmarkKey {
    selection: MapSelection,
    map: u32,
    states: u64,
    explored: Vec<u32>,
    /// The guard-directions marker `(continent, pos, icon)`, the position as raw `f32` bits: it is
    /// copied from the packet, so a bit compare is exact.
    marker: Option<(u32, [u32; 3], u32)>,
    group: Option<u32>,
}

/// The builder's near-zero skip (epsilon `2.384e-7`, `_DAT_009ea27c`): a UV of 0 on both axes is
/// off the displayed map; a POI on a rect's edge has one non-zero axis and stays.
fn is_degenerate(uv: (f32, f32)) -> bool {
    const EPS: f32 = 2.384e-7;
    uv.0.abs() < EPS && uv.1.abs() < EPS
}

/// The landmark walk's selection gates (`0x5445f0`), before the projection:
/// - an instance map, the cosmic sheet or a zone wants `Flags & 0x04`;
/// - the world sheet wants `0x10`;
/// - the world and continent sheets and the cosmic sheet want `0x08`;
/// - a row with `0x400` shows only on the map whose `WorldMapArea` row has its area and map.
fn landmark_level_gates(
    poi: &wrathplus_formats::AreaPoi,
    sel: MapSelection,
    displayed: Option<&RowEntry>,
) -> bool {
    let level = MapLevel::of(sel);
    let a = matches!(level, MapLevel::World | MapLevel::Continent) || poi.flags & 0x04 != 0;
    let b = level != MapLevel::World || poi.flags & 0x10 != 0;
    let c = level == MapLevel::Zone || sel.direct.is_some() || poi.flags & 0x08 != 0;
    let d = poi.flags & 0x400 == 0
        || displayed.is_some_and(|r| r.area_id == poi.area_id && r.map_id == poi.continent_id);
    a && b && c && d
}

/// The walk's state gates after the projection (`0x546310`): a signed `AreaID > 0` needs its
/// `AreaTable` row and, unless its `ExplorationLevel` is negative, its explore bit; a row with a
/// `WorldStateID` shows only while that state is non-zero.
fn landmark_state_gates(
    poi: &wrathplus_formats::AreaPoi,
    areas: &wrathplus_formats::AreaTableCatalog,
    explored: &[u32],
    world_states: &crate::app_core::state::WorldStates,
) -> bool {
    if poi.area_id as i32 > 0 {
        let Some(area) = areas.get(poi.area_id) else {
            return false;
        };
        if area.exploration_level >= 0 && !explored_bit(explored, area.explore_flag) {
            return false;
        }
    }
    poi.world_state_id == 0 || world_states.get(poi.world_state_id) != 0
}

/// `PLAYER_EXPLORED_ZONES` bit `bit`; the little-endian dword slots hold the bytes' bits.
fn explored_bit(explored: &[u32], bit: u32) -> bool {
    let (slot, within) = (bit as usize / 32, bit % 32);
    explored.get(slot).is_some_and(|w| w & (1 << within) != 0)
}

/// `GetMapLandmarkInfo`'s `textureIndex` (`0x545a30`): `Icon[state - 1]` for a world-state row
/// (state 1-9, else the first), but 0 on a zone or an instance map for a row without
/// `Flags & 0x80`.
fn landmark_texture_index(
    poi: &wrathplus_formats::AreaPoi,
    links: Option<&AreaPoiLinks>,
    level: MapLevel,
    world_states: &crate::app_core::state::WorldStates,
) -> u32 {
    let shown = poi.flags & 0x80 != 0 || matches!(level, MapLevel::World | MapLevel::Continent);
    if !shown {
        return 0;
    }
    let slot = if poi.world_state_id != 0 {
        usize::try_from(world_states.get(poi.world_state_id) - 1)
            .ok()
            .filter(|&i| i < 9)
            .unwrap_or(0)
    } else {
        0
    };
    links.map_or(poi.icon, |l| l.icons[slot])
}

/// Which map the player's position selects, in the Lua-visible encoding; at most one leg is set.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct PlayerSelection {
    /// The 1-based `(continent, zone)`, zone 0 for the continent map.
    zone: Option<(u32, u32)>,
    /// The `WorldMapArea` row id of the instance map the player is on.
    direct: Option<u32>,
    /// The `DungeonMap` floor of [`Self::direct`].
    floor: Option<u32>,
}

/// `0x543920`: the first `WorldMapTransforms` region holding the position moves it onto its new
/// map, offset, with that region's floor; else it stays, with no floor.
fn transform(data: &WorldMapUiData, map: u32, x: f32, y: f32) -> (u32, f32, f32, Option<u32>) {
    data.transforms
        .iter()
        .find(|t| t.holds(map, x, y))
        .map_or((map, x, y, None), |t| {
            (
                t.new_map_id,
                x + t.offset_x,
                y + t.offset_y,
                (t.new_dungeon_map_id != 0).then_some(t.new_dungeon_map_id),
            )
        })
}

/// The floor a position is on, as 12340's `0x5439e0`: the map's run of `DungeonMapChunk` rows in
/// table order, in it the WMO group's run, and the last row there whose `MinZ` is at or under the
/// height. None for a map or group id that is not positive (map 0 has no floors) or a position
/// under every row.
fn floor_at(data: &WorldMapUiData, map: u32, group: Option<u32>, z: f32) -> Option<u32> {
    let group = group.filter(|&g| g as i32 > 0)?;
    if map as i32 <= 0 {
        return None;
    }
    let mut floor = None;
    let mut in_group = false;
    let run = data.chunks.iter().skip_while(|c| c.map_id != map);
    for c in run.take_while(|c| c.map_id == map) {
        if c.wmo_group_id == group {
            in_group = true;
            if c.min_z <= z {
                floor = Some(c.dungeon_map_id);
            }
        } else if in_group {
            break;
        }
    }
    floor
}

/// `SetMapToCurrentZone`'s resolver, 12340's `0x547170`, as its setter calls `0x546310` give it:
/// - the continent drawn with the player's moved map (`0x543920`) and its zone, by area, at
///   `0x547a56`; the setter then opens a zone with `DefaultDungeonFloor >= 1` (Dalaran) as that
///   row's instance map on that default floor;
/// - else the area row on the player's map (`0x5475a5`), on the player's floor when it is above 0,
///   else on the transform's override floor, else on none: the area's terrain ("NO MAP FOUND").
fn resolve_player_selection(
    data: &WorldMapUiData,
    map_id: u32,
    pos: (f32, f32),
    top_zone: Option<u32>,
    floor: Option<u32>,
) -> PlayerSelection {
    let (drawn, _, _, override_floor) = transform(data, map_id, pos.0, pos.1);
    if let Some(ci) = data.continents.iter().position(|c| c.map_id == drawn) {
        let zone = top_zone.and_then(|top| {
            data.continents[ci]
                .zones
                .iter()
                .position(|z| z.area_id == top)
        });
        if let Some(zi) = zone {
            let z = &data.continents[ci].zones[zi];
            if z.default_floor >= 1 {
                return PlayerSelection {
                    zone: None,
                    direct: Some(z.wma_id),
                    floor: Some(z.default_floor as u32),
                };
            }
        }
        return PlayerSelection {
            zone: Some((ci as u32 + 1, zone.map_or(0, |zi| zi as u32 + 1))),
            direct: None,
            floor: None,
        };
    }
    let Some(d) = data.direct.iter().find(|d| d.map_id == map_id) else {
        return PlayerSelection::default();
    };
    PlayerSelection {
        zone: None,
        direct: Some(d.id),
        floor: floor.filter(|&f| f >= 1).or(override_floor),
    }
}

/// The engine's world-enter sync, which fires once per session: it waits for a known zone, then
/// applies whatever the resolver answers, the world view and an instance map included.
fn world_enter_selection(
    synced: bool,
    top_zone: Option<u32>,
    player: PlayerSelection,
) -> Option<PlayerSelection> {
    (!synced && top_zone.is_some()).then_some(player)
}

/// The displayed `WorldMapArea` row id, `None` on the world and cosmic sheets.
fn displayed_row(data: &WorldMapUiData, sel: MapSelection) -> Option<u32> {
    if sel.cosmic {
        return None;
    }
    if let Some(id) = sel.direct {
        return Some(id);
    }
    let cont = data
        .continents
        .get(sel.continent.checked_sub(1)? as usize)?;
    match sel.zone {
        0 => Some(cont.wma_id),
        z => cont.zones.get(z as usize - 1).map(|z| z.wma_id),
    }
}

/// A position's UV on the displayed map (`0x544140`), `None` off it, which Lua sees as the
/// `(0, 0)` hide; the position's floor is unknown, as for a body with no WMO group.
pub(crate) fn project_on_displayed(
    data: &WorldMapUiData,
    selection: MapSelection,
    pos_map: u32,
    px: f32,
    py: f32,
) -> Option<(f32, f32)> {
    project_on_floor(data, selection, pos_map, px, py, None)
}

/// [`project_on_displayed`] for a position on a known `DungeonMap` floor.
fn project_on_floor(
    data: &WorldMapUiData,
    sel: MapSelection,
    pos_map: u32,
    px: f32,
    py: f32,
    pos_floor: Option<u32>,
) -> Option<(f32, f32)> {
    /// `0x212`: Outland has no place on the Azeroth sheet.
    const OUTLAND: u32 = 530;
    let (drawn, tx, ty, t_floor) = transform(data, pos_map, px, py);
    if !sel.cosmic && sel.direct.is_none() && sel.continent == 0 {
        if drawn == OUTLAND {
            return None;
        }
        return data
            .continents
            .iter()
            .find(|cont| cont.map_id == drawn)
            .and_then(|cont| cont.proj)
            .map(|p| map_proj::world_uv(p, tx, ty));
    }
    if sel.direct.is_none()
        && data
            .continents
            .get(sel.continent.checked_sub(1)? as usize)
            .is_none_or(|c| c.map_id != drawn)
    {
        return None;
    }
    let row = data.rows.get(&displayed_row(data, sel)?)?;
    if row.map_id != pos_map && row.map_id != drawn {
        return None;
    }
    // A zone reads the original position; a continent and an instance map the moved one.
    let (x, y) = if sel.direct.is_none() && sel.zone != 0 {
        (px, py)
    } else {
        (tx, ty)
    };
    let shown = sel.floor;
    let floor_rect = |id: Option<u32>| {
        id.and_then(|id| data.floors.iter().find(|f| f.id == id))
            .map(|f| f.rect)
    };
    let rect = if pos_floor == shown || (t_floor.is_some() && t_floor == shown) {
        floor_rect(shown).unwrap_or(row.rect)
    } else if shown.is_some() {
        return None;
    } else {
        row.rect
    };
    Some(map_proj::zone_uv(rect, x, y))
}

/// Hand the engine its call-time projections over one shared copy of the catalog:
/// `GetWorldLocMapPosition`'s, and the floor-aware one the position getters take (`0x544140`).
fn install_world_loc_projector(script: &mut UiScript, data: &WorldMapUiData) {
    let copy = std::sync::Arc::new(data.clone());
    let shared = copy.clone();
    script.set_world_loc_projector(Box::new(move |selection, map, x, y| {
        project_on_displayed(&copy, selection, map, x, y)
    }));
    script.set_world_map_projection(move |sel, p| {
        project_on_floor(&shared, sel, p.map, p.x, p.y, p.floor)
    });
}

/// [`feed_world_map`]'s memos under one [`crate::ui_script::VmMemo`], so a new VM resets them all.
#[derive(Default)]
struct FeedMemos {
    explored: Option<Vec<u32>>,
    landmarks: Option<LandmarkKey>,
    /// The world-enter sync has run: the reference's `old == 0` gate.
    map_synced: bool,
    /// `GetWorldLocMapPosition`'s projection is installed over the current catalog.
    projector: bool,
}

fn feed_world_map(
    script: Option<NonSendMut<UiScript>>,
    data: Option<Res<WorldMapUiData>>,
    player: Res<Player>,
    map: Option<Res<CurrentMap>>,
    world: wrathplus_world::world_point::WorldPoint,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    areas: Option<Res<crate::area::AreaTableRes>>,
    death_net: Res<crate::app_core::death::DeathNet>,
    poi_marker: Res<crate::poi_marker::PoiMarker>,
    group: Res<crate::app_core::group::GroupState>,
    guids: Res<crate::net::GuidIndex>,
    unit_pos: Query<&GlobalTransform, With<crate::net::NetEntity>>,
    pois: Option<Res<crate::area_poi::AreaPoiRes>>,
    world_states: Res<crate::app_core::state::WorldStates>,
    mut memo: Local<crate::app_core::vm_memo::VmMemo<FeedMemos>>,
) {
    let (Some(mut script), Some(data), Some(map), Some(areas)) = (script, data, map, areas) else {
        return;
    };
    // PLAYER_EXPLORED_ZONES, pushed on change; the setter queues WORLD_MAP_UPDATE for newly
    // explored art.
    let explored: Vec<u32> = self_q
        .iter()
        .next()
        .map(|store| {
            (0..wrathplus_protocol::types::PLAYER_EXPLORED_ZONES_SLOTS)
                .map(|i| store.0.player_explored_zone_slot(i))
                .collect()
        })
        .unwrap_or_default();
    {
        let memo = memo.get(&script);
        if !explored.is_empty() && memo.explored.as_ref() != Some(&explored) {
            memo.explored = Some(explored.clone());
            script.set_world_map_explored(explored.clone());
        }
    }
    let wow = bevy_to_wow(player.pos);
    let (wx, wy, wz) = (wow[0], wow[1], wow[2]);
    // The player's WMO group, which picks every position's floor (`DAT_00bd0814`).
    let wmo_group = world.interior().map(|k| k.group_area_id);
    let player_floor = floor_at(&data, map.0, wmo_group, wz);

    // CurrentArea is the MCNK leaf; its top zone is the zone-level area the reference matches.
    let top_zone = world.area().and_then(|aid| areas.0.top_zone(aid));
    let player_sel = resolve_player_selection(&data, map.0, (wx, wy), top_zone, player_floor);

    // First world enter: the zone updater `0x494780` (`crate::area`), finding the cached zone id
    // 0, selects the player's zone itself through `0x4a6650` and the setter `0x4a67a0` before any
    // Lua asks. Once per VM, each login being a first enter.
    {
        let memo = memo.get(&script);
        if let Some(sel) = world_enter_selection(memo.map_synced, top_zone, player_sel) {
            memo.map_synced = true;
            let (c, z) = sel.zone.unwrap_or((0, 0));
            script.sync_world_map_to_player_zone(c, z, sel.direct, sel.floor);
        }
    }

    // `GetWorldLocMapPosition` projects synchronously with the engine's own selection, so the
    // engine holds the projection over its own copy of the catalog.
    {
        let memo = memo.get(&script);
        if !memo.projector || data.is_changed() {
            memo.projector = true;
            install_world_loc_projector(&mut script, &data);
        }
    }

    let selection = script.world_map_selection();
    let project =
        |pos_map: u32, px: f32, py: f32| project_on_displayed(&data, selection, pos_map, px, py);
    // A position's floor is read with the player's WMO group and its own height (`0x544140`).
    let project_z = |pos_map: u32, px: f32, py: f32, pz: f32| {
        project_on_floor(
            &data,
            selection,
            pos_map,
            px,
            py,
            floor_at(&data, pos_map, wmo_group, pz),
        )
    };
    let player_pos = MapPos {
        map: map.0,
        x: wx,
        y: wy,
        floor: player_floor,
    };
    // The corpse at the query answer's display position, a dungeon corpse at its entrance.
    let corpse_pos = death_net.corpse.map(|cp| {
        let map = u32::try_from(cp.display_map).unwrap_or(u32::MAX);
        let [x, y, z] = cp.position;
        let floor = floor_at(&data, map, wmo_group, z);
        MapPos { map, x, y, floor }
    });

    // The landmarks (`GetNumMapLandmarks`), rebuilt only when a `LandmarkKey` input moves.
    let marker = poi_marker
        .on_map(map.0)
        .map(|m| (m.continent_id, m.pos.map(f32::to_bits), m.icon));
    let states_gen = world_states.generation();
    let unchanged = memo.get(&script).landmarks.as_ref().is_some_and(|k| {
        *k == LandmarkKey {
            selection,
            map: map.0,
            states: states_gen,
            explored: explored.clone(),
            marker,
            group: wmo_group,
        }
    });
    if !unchanged {
        let level = MapLevel::of(selection);
        let displayed = displayed_row(&data, selection).and_then(|id| data.rows.get(&id));
        let mut landmarks = Vec::new();
        // The DBC rows first, in file order, the Lua landmark index (`0x546310`'s walk).
        if let Some(pois) = pois.as_ref() {
            for (id, poi) in pois.0.rows() {
                if !landmark_level_gates(poi, selection, displayed) {
                    continue;
                }
                let Some(uv) = project_z(poi.continent_id, poi.pos[0], poi.pos[1], poi.pos[2])
                else {
                    continue;
                };
                if is_degenerate(uv)
                    || !landmark_state_gates(poi, &areas.0, &explored, &world_states)
                {
                    continue;
                }
                let links = data.poi_links.get(&id);
                landmarks.push(WorldMapLandmarkView {
                    name: poi.name.clone(),
                    description: poi.description.clone(),
                    texture_index: landmark_texture_index(poi, links, level, &world_states),
                    uv,
                    map_link: links.map_or(0, |l| l.world_map_link),
                    battle_map: poi.flags & 0x200 != 0,
                });
            }
        }
        // Then the guard-directions marker, last (`0x4a69c7`): no gate and no icon substitution,
        // only the degenerate-projection test.
        if let Some(poi) = poi_marker.on_map(map.0) {
            if let Some(uv) = project(poi.continent_id, poi.pos[0], poi.pos[1]) {
                if !is_degenerate(uv) {
                    landmarks.push(WorldMapLandmarkView {
                        name: poi.name.clone(),
                        description: poi.description.clone(),
                        texture_index: poi.icon,
                        uv,
                        map_link: 0,
                        battle_map: false,
                    });
                }
            }
        }
        memo.get(&script).landmarks = Some(LandmarkKey {
            selection,
            map: map.0,
            states: states_gen,
            explored: explored.clone(),
            marker,
            group: wmo_group,
        });
        script.set_world_map_landmarks(landmarks);
    }

    // Each member's position, projected at the call: a streamed member's transform, else the
    // `i16` position from `SMSG_PARTY_MEMBER_STATS`, whose map is the continent owning its zone,
    // else ours.
    let members: Vec<(u64, MapPos)> = group
        .members
        .iter()
        .filter_map(|m| {
            let (x, y) = crate::minimap::party_member_pos(m, &group, &guids, &unit_pos)?;
            let zone = group.stats.get(&m.guid).and_then(|st| st.zone);
            let member_map = zone
                .and_then(|zone| {
                    data.continents
                        .iter()
                        .find(|cont| cont.zones.iter().any(|z| z.area_id == u32::from(zone)))
                        .map(|cont| cont.map_id)
                })
                .unwrap_or(map.0);
            let pos = MapPos {
                map: member_map,
                x,
                y,
                floor: None,
            };
            Some((m.guid, pos))
        })
        .collect();

    script.set_world_map_feed(
        player_sel.zone,
        Some(player_pos),
        player.facing(),
        corpse_pos,
        members,
    );
    // The instance-map leg for `SetMapToCurrentZone`, which the battlefield minimap calls on
    // `PLAYER_ENTERING_WORLD` (`Blizzard_BattlefieldMinimap.lua:58-62`).
    script.set_world_map_player_direct_area(player_sel.direct, player_sel.floor);
}

/// Alt+click the world map to go there, a dev jump: the click's UV back through the displayed
/// rect to a world `(x, y)`, sent as vmangos's `.go xy x y <mapid>` so the server finds the ground.
/// It declines on the world sheet, where the client's click law `0x4a7100` does not invert its
/// projection (`map_proj::world_click_world`). The click is not consumed: the stock
/// `WorldMapButton_OnClick` still zooms into the clicked zone.
fn dev_map_jump(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui_scale: Res<crate::app_core::ui_flags::UiScaleCvar>,
    script: Option<NonSendMut<UiScript>>,
    data: Option<Res<WorldMapUiData>>,
    net: Res<crate::net::NetCommands>,
) {
    // A dev affordance compiled into every build, so gated at run time.
    if !crate::run_mode::dev_affordances() {
        return;
    }
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if !alt || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let (Some(script), Some(data), Ok(window)) = (script, data, windows.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    // Window px (y-down) to the VM's y-up 768-unit space, as the pointer feed converts them.
    let s = crate::app_core::ui_state::seam_scale(window.height(), ui_scale.0);
    let Some((u, v)) = script.world_map_uv_at(cursor.x / s, (window.height() - cursor.y) / s)
    else {
        return;
    };
    let sel = script.world_map_selection();
    let (c, z) = (sel.continent, sel.zone);
    // An instance map inverts like a zone, through its floor's rect when one is shown.
    let (rect, map_id) = match sel.direct {
        Some(id) => match data.rows.get(&id) {
            Some(row) => (
                sel.floor
                    .and_then(|f| data.floors.iter().find(|x| x.id == f))
                    .map_or(row.rect, |f| f.rect),
                row.map_id,
            ),
            None => return,
        },
        None => {
            let Some(cont) = c
                .checked_sub(1)
                .and_then(|i| data.continents.get(i as usize))
            else {
                info!(
                    "map-jump: no exact click→world law at the world level — zoom into a \
                     continent first"
                );
                return;
            };
            let rect = match z.checked_sub(1) {
                None => cont.rect,
                Some(i) => match cont.zones.get(i as usize) {
                    Some(zone) => zone.rect,
                    None => return,
                },
            };
            (rect, cont.map_id)
        }
    };
    // `zone_world` lerps both axes by one `t`, the reference's shape, so it runs once per axis.
    let (_, wy) = map_proj::zone_world(rect, u);
    let (wx, _) = map_proj::zone_world(rect, v);
    let text = format!(".go xy {wx:.2} {wy:.2} {map_id}");
    info!("map-jump: {text}");
    let _ = net.0.send(crate::net::ClientCommand::Chat {
        kind: crate::net::ChatKind::Say,
        target: None,
        text,
        language: None,
    });
}

/// The world-map data feed.
pub struct WorldMapUiPlugin;

impl Plugin for WorldMapUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // After the script tick: a selection changed this tick projects on the next.
                feed_world_map.after(UiInput),
                // After the script tick, so the jump hit-tests this tick's frame rects.
                dev_map_jump.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::state::WorldStates;
    use wrathplus_formats::AreaPoi;

    /// A bare row; each test sets only the columns its gate reads.
    fn poi(flags: u32, area_id: u32, world_state_id: u32) -> AreaPoi {
        AreaPoi {
            importance: 0,
            icon: 4,
            faction_id: 0,
            pos: [0.0, 0.0, 0.0],
            continent_id: 0,
            flags,
            area_id,
            name: String::new(),
            description: String::new(),
            world_state_id,
        }
    }

    fn empty_areas() -> wrathplus_formats::AreaTableCatalog {
        wrathplus_formats::AreaTableCatalog::from_rows(Vec::new())
    }

    fn sel(continent: u32, zone: u32, direct: Option<u32>) -> MapSelection {
        MapSelection {
            continent,
            zone,
            direct,
            ..Default::default()
        }
    }

    #[test]
    fn the_level_flags_gate_each_sheet() {
        let pass = |flags, s| landmark_level_gates(&poi(flags, 0, 0), s, None);
        let (world, cont, zone, inst) = (
            sel(0, 0, None),
            sel(2, 0, None),
            sel(2, 5, None),
            sel(0, 0, Some(535)),
        );

        // A town (0x0d = 0x01|0x04|0x08): zone, continent and instance map, not the world sheet.
        assert!(pass(0x0d, zone) && pass(0x0d, cont) && pass(0x0d, inst));
        assert!(!pass(0x0d, world));
        // A capital (0x1d = 0x0d|0x10): every sheet.
        assert!(pass(0x1d, world) && pass(0x1d, cont) && pass(0x1d, zone));
        // An Eastern Plaguelands tower (0x87): zone only.
        assert!(pass(0x87, zone) && !pass(0x87, cont) && !pass(0x87, world));
        // `0x10` alone: the world sheet still wants `0x08`.
        assert!(!pass(0x10, world) && !pass(0x10, cont) && !pass(0x10, zone));
        // The cosmic sheet wants both `0x04` and `0x08`.
        let cosmic = MapSelection {
            cosmic: true,
            ..Default::default()
        };
        assert!(pass(0x0c, cosmic) && !pass(0x08, cosmic) && !pass(0x04, cosmic));
    }

    #[test]
    fn a_map_bound_row_shows_only_on_its_own_area_map() {
        let mut p = poi(0x404, 4395, 0);
        p.continent_id = 571;
        let dalaran = RowEntry {
            map_id: 571,
            area_id: 4395,
            rect: ZoneRect {
                left: 1.0,
                right: 0.0,
                top: 1.0,
                bottom: 0.0,
            },
        };
        let other = RowEntry {
            area_id: 65,
            ..dalaran.clone()
        };
        let s = sel(0, 0, Some(504));
        assert!(landmark_level_gates(&p, s, Some(&dalaran)));
        assert!(!landmark_level_gates(&p, s, Some(&other)));
        assert!(!landmark_level_gates(&p, s, None));
    }

    #[test]
    fn a_world_state_row_appears_only_while_its_key_is_set() {
        let (areas, explored) = (empty_areas(), Vec::new());
        let mut states = WorldStates::default();
        let alliance = poi(0x87, 0, 2372);
        let horde = poi(0x87, 0, 2373);
        let plain = poi(0x87, 0, 0);
        let pass = |p: &AreaPoi, st: &WorldStates| landmark_state_gates(p, &areas, &explored, st);

        assert!(
            pass(&plain, &states),
            "an ungated row is always a candidate"
        );
        assert!(!pass(&alliance, &states) && !pass(&horde, &states));
        states.write(&[(2372, 1)]);
        assert!(pass(&alliance, &states) && !pass(&horde, &states));
        states.write(&[(2372, 0), (2373, 1)]);
        assert!(!pass(&alliance, &states) && pass(&horde, &states));
    }

    #[test]
    fn the_exploration_gate_reads_the_right_bit_and_needs_the_row() {
        use wrathplus_formats::AreaTableRow;
        let row = |explore_flag, exploration_level| AreaTableRow {
            map_id: 0,
            zone_id: 0,
            explore_flag,
            flags: 0,
            faction_group_mask: 0,
            exploration_level,
            name: String::new(),
        };
        let areas = wrathplus_formats::AreaTableCatalog::from_rows(vec![
            (139, row(40, 0)),
            (200, row(41, -1)),
        ]);
        let states = WorldStates::default();
        let unexplored = vec![0u32; 4];
        let mut explored = vec![0u32; 4];
        explored[1] = 1 << 8;
        let pass = |p: &AreaPoi, ex: &[u32]| landmark_state_gates(p, &areas, ex, &states);

        assert!(!pass(&poi(0x04, 139, 0), &unexplored), "not discovered yet");
        assert!(pass(&poi(0x04, 139, 0), &explored));
        assert!(
            pass(&poi(0x04, 200, 0), &unexplored),
            "ExplorationLevel -1 exempts the row"
        );
        assert!(
            pass(&poi(0x04, u32::MAX, 0), &unexplored),
            "AreaID -1 is not > 0"
        );
        assert!(pass(&poi(0x04, 0, 0), &unexplored), "AreaID 0 likewise");
        assert!(
            !pass(&poi(0x04, 999, 0), &unexplored),
            "build 12340 drops a row whose area the table lacks"
        );
    }

    #[test]
    fn the_texture_index_reads_the_world_state_icon_and_blanks_zone_rows() {
        let links = AreaPoiLinks {
            icons: [10, 11, 12, 13, 14, 15, 16, 17, 18],
            ..Default::default()
        };
        let mut states = WorldStates::default();
        let town = poi(0x0d, 0, 0);
        let tower = poi(0x87, 0, 2372);
        assert_eq!(
            landmark_texture_index(&town, Some(&links), MapLevel::Zone, &states),
            0
        );
        assert_eq!(
            landmark_texture_index(&town, Some(&links), MapLevel::MinusTwo, &states),
            0
        );
        assert_eq!(
            landmark_texture_index(&town, Some(&links), MapLevel::Continent, &states),
            10
        );
        assert_eq!(
            landmark_texture_index(&town, None, MapLevel::World, &states),
            4
        );
        // State 3 picks the third icon; an unset or out-of-range state the first.
        assert_eq!(
            landmark_texture_index(&tower, Some(&links), MapLevel::Zone, &states),
            10
        );
        states.write(&[(2372, 3)]);
        assert_eq!(
            landmark_texture_index(&tower, Some(&links), MapLevel::Zone, &states),
            12
        );
        states.write(&[(2372, 12)]);
        assert_eq!(
            landmark_texture_index(&tower, Some(&links), MapLevel::Zone, &states),
            10
        );
    }

    #[test]
    fn only_a_both_axes_zero_projection_is_dropped() {
        assert!(is_degenerate((0.0, 0.0)));
        assert!(is_degenerate((1e-8, -1e-8)));
        assert!(!is_degenerate((0.0, 0.5)));
        assert!(!is_degenerate((0.5, 0.0)));
        assert!(!is_degenerate((1e-6, 0.0)));
    }

    #[test]
    fn the_selection_maps_onto_the_reference_levels() {
        assert_eq!(MapLevel::of(sel(0, 0, None)), MapLevel::World);
        assert_eq!(
            MapLevel::of(sel(0, 3, None)),
            MapLevel::World,
            "continent 0 wins"
        );
        assert_eq!(MapLevel::of(sel(2, 0, None)), MapLevel::Continent);
        assert_eq!(MapLevel::of(sel(2, 7, None)), MapLevel::Zone);
        assert_eq!(MapLevel::of(sel(0, 0, Some(443))), MapLevel::MinusTwo);
    }
}

#[cfg(test)]
#[path = "ui_world_map_player_zone_tests.rs"]
mod player_zone_tests;

#[cfg(test)]
mod direct_area_tests {
    use super::*;

    /// The real catalog off the player's own chain, or a skip.
    fn real_catalog() -> Option<(Vec<WorldMapContinentView>, WorldMapUiData)> {
        let data = wrathplus_formats::wow_data_or_skip!(None);
        let mut chain = wrathplus_formats::open_configured_chain(&data).expect("chain");
        let areas = wrathplus_formats::load_area_table_catalog(&mut chain).expect("AreaTable");
        let maps = wrathplus_assets::MapCatalogRes(
            wrathplus_formats::load_map_catalog(&mut chain).expect("Map"),
        );
        Some(build_catalog(&mut chain, &areas, &maps).expect("the real catalog"))
    }

    /// Build 12340's four continents, with the Burning Crusade starting zones drawn on the old
    /// continents, and the instance maps (battlegrounds and Northrend dungeons) beside them.
    #[test]
    fn the_real_catalog_draws_zones_on_their_display_continent() {
        let Some((views, data)) = real_catalog() else {
            return;
        };
        assert_eq!(
            views
                .iter()
                .map(|c| (c.map_id, c.wma_id))
                .collect::<Vec<_>>(),
            vec![(1, 13), (0, 14), (530, 466), (571, 485)]
        );
        let zone_on =
            |cont: usize, file: &str| views[cont].zones.iter().any(|z| z.map_file == file);
        assert!(zone_on(1, "EversongWoods") && zone_on(1, "SilvermoonCity"));
        assert!(zone_on(0, "AzuremystIsle") && zone_on(0, "TheExodar"));
        assert!(!zone_on(2, "EversongWoods"));
        assert!(zone_on(3, "Dalaran"));
        let direct: Vec<u32> = data.direct.iter().map(|d| d.id).collect();
        for id in [443, 535, 529] {
            assert!(direct.contains(&id), "{id} is an instance map");
        }
        assert!(!direct.contains(&504), "Dalaran is a Northrend zone");
        assert!(!data.floors.is_empty());
        assert!(!data.transforms.is_empty());
    }

    /// `GetMapInfo()` must answer `"WarsongGulch"` for the battlefield minimap to draw; the
    /// continent reads back -1 and the zone 0.
    #[test]
    fn a_body_in_warsong_gulch_lands_on_the_warsong_gulch_map() {
        let Some((views, data)) = real_catalog() else {
            return;
        };
        let player = resolve_player_selection(&data, 489, (0.0, 0.0), Some(3277), None);
        assert_eq!(player.direct, Some(443));

        let mut script = UiScript::new().expect("VM");
        script.set_world_map_catalog(views);
        script.set_world_map_direct_areas(
            data.direct
                .iter()
                .map(|d| (d.id, d.view.clone()))
                .collect::<Vec<_>>(),
        );
        script.set_world_map_areas(data.area_views.clone(), data.floor_views.clone());
        let (c, z) = player.zone.unwrap_or((0, 0));
        script.sync_world_map_to_player_zone(c, z, player.direct, player.floor);
        assert_eq!(
            script
                .eval::<(String, i64, i64, i64)>(
                    "return GetMapInfo(), GetCurrentMapContinent(), GetCurrentMapZone(), \
                     GetCurrentMapAreaID()"
                )
                .expect("the getters answer"),
            ("WarsongGulch".into(), -1, 0, 444)
        );
        let wsg = data
            .direct
            .iter()
            .find(|d| d.id == 443)
            .expect("Warsong Gulch");
        let (wx, wy) = (
            (wsg.rect.top + wsg.rect.bottom) / 2.0,
            (wsg.rect.left + wsg.rect.right) / 2.0,
        );
        let selection = script.world_map_selection();
        let uv = project_on_displayed(&data, selection, 489, wx, wy).expect("inside the rect");
        assert!(
            (uv.0 - 0.5).abs() < 1e-5 && (uv.1 - 0.5).abs() < 1e-5,
            "{uv:?}"
        );
        assert_eq!(project_on_displayed(&data, selection, 0, wx, wy), None);

        // SetMapByID reaches Dalaran's floor and Naxxramas' first floor.
        script.run("SetMapByID(504)").expect("SetMapByID");
        assert_eq!(
            script
                .eval::<(i64, i64)>("return GetCurrentMapAreaID(), GetCurrentMapDungeonLevel()")
                .unwrap(),
            (505, 1)
        );
        script.run("SetMapByID(535)").expect("SetMapByID");
        assert!(
            script
                .eval::<i64>("return GetNumDungeonMapLevels()")
                .unwrap()
                >= 4
        );
        script.run("SetMapZoom(1)").expect("SetMapZoom");
        assert_eq!(
            script.world_map_selection(),
            MapSelection {
                continent: 1,
                ..Default::default()
            }
        );
    }
}
