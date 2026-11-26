//! Build 12340's map selection by `WorldMapArea` id and dungeon floor, over [`super::worldmap`]'s
//! state. Evidence: one headless Ghidra round (`/tmp/azb-wire-ghidra/worldmap`).
//!
//! The client's cells: continent `[0xaccf04]` (an index, `-1` the world sheet, `-2` an instance
//! map or the cosmic sheet), zone `[0xaccf08]`, the instance map's `WorldMapArea` id `[0xaccf10]`
//! and its `DungeonMap` floor `[0xaccf0c]`. The one setter is `0x546310`
//! ([`super::worldmap::set_map`]).
//! - `GetCurrentMapAreaID` (`0x544e10`): the displayed `WorldMapArea` id plus one, so 0 on the
//!   world and cosmic sheets.
//! - `SetMapByID(id)` (`0x546c50`): a row opening on a floor goes there. Otherwise the continent
//!   drawn with the row's map (`DisplayMapID`, else `MapID`) shows it, as the continent map for
//!   `AreaID` 0 or as one of its zones. Failing that it is an instance map: on terrain for
//!   `DefaultDungeonFloor -1`, else on the map's floor 1 (`DungeonMap.dbc`, `0xad36dc`), else bare.
//! - `ZoomOut` (`0x546a90`): from a floor to its `ParentWorldMapID` zone, or from an instance map
//!   to its row's parent, else to floor 1's. `IsZoomOutAvailable` (`0x545050`) answers whether a
//!   parent exists.
//! - Floors: `GetCurrentMapDungeonLevel` (`0x544fc0`), `SetDungeonMapLevel` (`0x547b80`),
//!   `GetNumDungeonMapLevels` (`0x546190`) and `DungeonUsesTerrainMap` (`0x544c40`). A row with
//!   `DefaultDungeonFloor -1` counts its terrain as level 1, and its floors from 2.
//! - `ClickLandmark(mapLinkID)` (`0x546ef0`): the instance map on that floor's map, on that floor.
//! - `InitWorldMapPing(parent)` (`0x545ff0`) checks its frame.
//! - `GetMapDebugObjectInfo`, `TeleportToDebugObject` and `GetDebugZoneMap` share one stub,
//!   `xor eax,eax; ret` (`0x8e5250`): no returns. `GetNumMapDebugObjects` (`0x608560`) is 0.

use mlua::{Lua, MultiValue, Value};

use super::worldmap::{minus_two, set_map, DungeonFloorView, WorldMapAreaView, WorldMapState};
use super::Model;

/// The displayed `WorldMapArea` id (`0x543770`), `None` on the world and cosmic sheets.
pub(super) fn displayed_wma(wm: &WorldMapState) -> Option<u32> {
    if minus_two(wm) {
        return wm.direct_area;
    }
    let (c, z) = wm.selection;
    let cont = wm.continents.get(c.checked_sub(1)? as usize)?;
    match z {
        0 => Some(cont.wma_id),
        z => cont.zones.get(z as usize - 1).map(|zone| zone.wma_id),
    }
}

/// The instance map's row, `[0xaccf10]` through the table.
fn direct_row(wm: &WorldMapState) -> Option<&WorldMapAreaView> {
    wm.areas.get(&wm.direct_area?)
}

/// The displayed floor's row: `[0xaccf0c]` while an instance map is up.
pub(super) fn floor_row(wm: &WorldMapState) -> Option<&DungeonFloorView> {
    let id = wm.direct_area.and(wm.floor)?;
    wm.floors.iter().find(|f| f.id == id)
}

/// The map's terrain is floor 0 (`DefaultDungeonFloor -1`).
fn uses_terrain(wm: &WorldMapState) -> bool {
    direct_row(wm).is_some_and(|r| r.default_floor == -1)
}

/// The continent index (0-based) and zone index whose row is `wma`, as `ZoomOut`'s tail walks
/// them: continents carrying the row's map, then their zones.
fn zone_of(wm: &WorldMapState, row: &WorldMapAreaView) -> Option<(i64, i64)> {
    wm.continents.iter().enumerate().find_map(|(ci, cont)| {
        (cont.map_id == row.map_id)
            .then(|| cont.zones.iter().position(|z| z.wma_id == row.id))
            .flatten()
            .map(|zi| (ci as i64, zi as i64))
    })
}

fn number(lua: &Lua, args: &MultiValue, i: usize) -> Option<f64> {
    args.get(i)
        .cloned()
        .and_then(|v| lua.coerce_number(v).ok().flatten())
}

/// `SetMapByID`'s walk; `None` for an id no row carries.
fn set_map_by_id(model: &mut Model, id: u32) -> Option<()> {
    let row = model.worldmap.areas.get(&id)?.clone();
    let key = if row.display_map >= 0 {
        row.display_map as u32
    } else {
        row.map_id
    };
    for (ci, cont) in model.worldmap.continents.iter().enumerate() {
        if cont.map_id != key {
            continue;
        }
        if row.default_floor > 0 {
            set_map(model, -2, i64::from(row.id), i64::from(row.default_floor));
            return Some(());
        }
        if row.area_id == 0 {
            set_map(model, ci as i64, -1, -1);
            return Some(());
        }
        if let Some(zi) = cont.zones.iter().position(|z| z.wma_id == row.id) {
            set_map(model, ci as i64, zi as i64, -1);
            return Some(());
        }
    }
    if row.default_floor == -1 {
        set_map(model, -2, i64::from(id), -1);
        return Some(());
    }
    let first = model
        .worldmap
        .floors
        .iter()
        .find(|f| f.map_id == row.map_id && f.floor_index == 1)
        .map(|f| i64::from(f.id));
    set_map(model, -2, i64::from(id), first.unwrap_or(-1));
    Some(())
}

/// `ZoomOut`'s parent row id.
fn zoom_parent(wm: &WorldMapState) -> Option<u32> {
    if let Some(floor) = floor_row(wm) {
        return (floor.parent != 0).then_some(floor.parent);
    }
    if !minus_two(wm) {
        return None;
    }
    let row = direct_row(wm)?;
    let parent = if row.parent != 0 {
        row.parent
    } else {
        wm.floors
            .iter()
            .rfind(|f| f.map_id == row.map_id && f.floor_index == 1)
            .map_or(0, |f| f.parent)
    };
    (parent != 0).then_some(parent)
}

/// `SetDungeonMapLevel`'s floor walk `0x547a70`: the instance map's floors are one run in file
/// order, and a level past them is ignored.
fn set_floor_level(model: &mut Model, level: u32) {
    let Some(row) = direct_row(&model.worldmap).cloned() else {
        return;
    };
    if level == 0 {
        set_map(model, -2, i64::from(row.id), -1);
        return;
    }
    let mut seen = false;
    let mut hit = None;
    for f in &model.worldmap.floors {
        if f.map_id == row.map_id {
            seen = true;
            if f.floor_index == level {
                hit = Some(f.id);
                break;
            }
        } else if seen {
            break;
        }
    }
    if let Some(id) = hit.filter(|&id| id >= 1) {
        set_map(model, -2, i64::from(row.id), i64::from(id));
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetCurrentMapAreaID",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(displayed_wma(&model.worldmap).map_or(0, |id| i64::from(id) + 1))
        })?,
    )?;

    // GetCurrentMapContinent (`0x544d40`): a displayed floor answers the continent carrying its
    // map; otherwise the continent cell plus one, so -1 for an instance map or the cosmic sheet.
    g.set(
        "GetCurrentMapContinent",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            if let Some(floor) = floor_row(wm) {
                if let Some(i) = wm.continents.iter().position(|c| c.map_id == floor.map_id) {
                    return Ok(i as i64 + 1);
                }
            }
            Ok(if minus_two(wm) {
                -1
            } else {
                i64::from(wm.selection.0)
            })
        })?,
    )?;

    // GetCurrentMapZone (`0x544e80`): with no zone cell, an instance map answers its row's index
    // among the zones of a continent carrying the row's map (Dalaran's floors; `0x544eee`-
    // `0x544fa3`); otherwise the zone cell plus one.
    g.set(
        "GetCurrentMapZone",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            let listed = direct_row(wm).and_then(|row| {
                let mut on_map = wm.continents.iter().filter(|c| c.map_id == row.map_id);
                on_map.find_map(|c| c.zones.iter().position(|z| z.wma_id == row.id))
            });
            Ok(listed.map_or(i64::from(wm.selection.1), |i| i as i64 + 1))
        })?,
    )?;

    // SetMapZoom(continent [, zone]) (`0x5469e0`): continent -1 is the cosmic sheet; one past the
    // list raises.
    g.set(
        "SetMapZoom",
        lua.create_function(|lua, args: MultiValue| {
            let Some(c) = number(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: SetMapZoom(continentIndex [,zoneIndex])",
                ));
            };
            let cont = c as i64 - 1;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let count = model.worldmap.continents.len() as i64;
            if !(-2..count.max(1)).contains(&cont) {
                return Err(mlua::Error::runtime(
                    "Usage: SetMapZoom(continentIndex [,zoneIndex]) .. requested continent out of bounds",
                ));
            }
            let zone = number(lua, &args, 1).map_or(-1, |z| z as i64 - 1);
            set_map(&mut model, cont, zone, -1);
            Ok(())
        })?,
    )?;

    g.set(
        "SetMapByID",
        lua.create_function(|lua, args: MultiValue| {
            let Some(id) = number(lua, &args, 0) else {
                return Err(mlua::Error::runtime("Usage: SetMapByID(mapID)"));
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            Ok(
                match u32::try_from(id as i64)
                    .ok()
                    .and_then(|id| set_map_by_id(&mut model, id))
                {
                    Some(()) => MultiValue::from_vec(vec![Value::Boolean(true)]),
                    None => MultiValue::new(),
                },
            )
        })?,
    )?;

    g.set(
        "ZoomOut",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let target = zoom_parent(&model.worldmap)
                .and_then(|id| model.worldmap.areas.get(&id))
                .and_then(|row| zone_of(&model.worldmap, row));
            if let Some((c, z)) = target {
                set_map(&mut model, c, z, -1);
            }
            Ok(())
        })?,
    )?;

    g.set(
        "IsZoomOutAvailable",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            if floor_row(wm).is_some() || !minus_two(wm) {
                return Ok(true);
            }
            Ok(match direct_row(wm) {
                None => false,
                Some(row) if row.default_floor == -1 => true,
                Some(row) => wm.areas.contains_key(&row.parent),
            })
        })?,
    )?;

    g.set(
        "ClickLandmark",
        lua.create_function(|lua, args: MultiValue| {
            let Some(link) = number(lua, &args, 0) else {
                return Err(mlua::Error::runtime("Usage: ClickLandmark(mapLinkID)"));
            };
            // Truncated: fistp under a chopping control word (`0x546f26`-`0x546f40`).
            let link = link as i64;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            let target = u32::try_from(link)
                .ok()
                .and_then(|id| wm.floors.iter().find(|f| f.id == id))
                .and_then(|floor| {
                    wm.direct_areas
                        .iter()
                        .map(|(id, _)| *id)
                        .find(|id| wm.areas.get(id).is_some_and(|r| r.map_id == floor.map_id))
                });
            if let Some(id) = target {
                set_map(&mut model, -2, i64::from(id), link);
            }
            Ok(())
        })?,
    )?;

    g.set(
        "GetCurrentMapDungeonLevel",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            let level = floor_row(wm).map_or(0, |f| i64::from(f.floor_index));
            Ok(level + i64::from(uses_terrain(wm)))
        })?,
    )?;

    g.set(
        "SetDungeonMapLevel",
        lua.create_function(|lua, args: MultiValue| {
            let Some(level) = number(lua, &args, 0) else {
                return Err(mlua::Error::runtime("Usage: SetDungeonMapLevel(level)"));
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let level = level as i64 - i64::from(uses_terrain(&model.worldmap));
            let floor_min = if uses_terrain(&model.worldmap) { 0 } else { 1 };
            if let Ok(level) = u32::try_from(level) {
                if level >= floor_min {
                    set_floor_level(&mut model, level);
                }
            }
            Ok(())
        })?,
    )?;

    g.set(
        "GetNumDungeonMapLevels",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            let Some(row) = direct_row(wm) else {
                return Ok(0i64);
            };
            let mut seen = 0u32;
            for f in wm.floors.iter().filter(|f| f.map_id == row.map_id) {
                seen |= 1u32.checked_shl(f.floor_index).unwrap_or(0);
            }
            Ok(i64::from(seen.count_ones()) + i64::from(uses_terrain(wm)))
        })?,
    )?;

    g.set(
        "DungeonUsesTerrainMap",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(if uses_terrain(&model.worldmap) {
                Value::Integer(1)
            } else {
                Value::Nil
            })
        })?,
    )?;

    // InitWorldMapPing(parent) (`0x545ff0`): `parent` must be a frame; the client records half
    // its `WorldMapPing` child's size for the engine's ping placement (`0x542f80`). Labelled: no
    // engine ping reads that size here, as FrameXML seats and fades the ping model itself.
    g.set(
        "InitWorldMapPing",
        lua.create_function(|lua, args: MultiValue| {
            let Some(Value::Table(parent)) = args.front().cloned() else {
                return Err(mlua::Error::runtime("Usage: InitWorldMapPing(parent)"));
            };
            if super::object::frame_handle_of(lua, &parent).is_err() {
                return Err(mlua::Error::runtime(
                    "InitWorldMapPing(): Couldn't find 'this' in parent object",
                ));
            }
            Ok(())
        })?,
    )?;

    for stub in [
        "GetMapDebugObjectInfo",
        "TeleportToDebugObject",
        "GetDebugZoneMap",
    ] {
        g.set(stub, lua.create_function(|_, _: MultiValue| Ok(()))?)?;
    }
    g.set(
        "GetNumMapDebugObjects",
        lua.create_function(|_, _: MultiValue| Ok(0i64))?,
    )?;

    // GetMapLandmarkInfo(i) (`0x545a30`) → name, description, textureIndex, x, y, mapLinkID,
    // showInBattleMap; the miss is nil, nil, 0, 0, 0, nil, nil.
    g.set(
        "GetMapLandmarkInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(i) = number(lua, &args, 0) else {
                return Err(mlua::Error::runtime("Usage: GetMapLandmarkInfo(index)"));
            };
            let landmark = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                // Truncated as `0x545a68`-`0x545a7d` does.
                usize::try_from(i as i64 - 1)
                    .ok()
                    .and_then(|i| model.map_landmarks().get(i).cloned())
            };
            let Some(l) = landmark else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Nil,
                    Value::Nil,
                ]));
            };
            let text = |s: &str| -> mlua::Result<Value> {
                Ok(if s.is_empty() {
                    Value::Nil
                } else {
                    Value::String(lua.create_string(s)?)
                })
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&l.name)?),
                text(&l.description)?,
                Value::Integer(i64::from(l.texture_index)),
                Value::Number(f64::from(l.uv.0)),
                Value::Number(f64::from(l.uv.1)),
                if l.map_link == 0 {
                    Value::Nil
                } else {
                    Value::Integer(i64::from(l.map_link))
                },
                Value::Boolean(l.battle_map),
            ]))
        })?,
    )?;

    // GetMapInfo() → mapFileName, textureHeight, and the height with its low byte rounded up to
    // a power of two (`0x544ca0`). The height is the displayed floor's, else the continent's or
    // zone's (`0x543cd0`); an instance map with no floor has none.
    g.set(
        "GetMapInfo",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let wm = &model.worldmap;
            let Some(row) = displayed_wma(wm).and_then(|id| wm.areas.get(&id)) else {
                return Ok((Value::Nil, 0i64, 0i64));
            };
            let height = if let Some(floor) = floor_row(wm) {
                let (min_x, max_x, min_y, max_y) = floor.rect;
                aspect_height(max_x - min_x, max_y - min_y)
            } else if minus_two(wm) {
                0
            } else if wm.selection.1 == 0 {
                wm.continents
                    .get(wm.selection.0 as usize - 1)
                    .map_or(0, |c| c.texture_height)
            } else {
                let (left, right, top, bottom) = row.loc_rect;
                aspect_height(left - right, top - bottom)
            };
            let name = Value::String(lua.create_string(&row.map_file)?);
            Ok((name, i64::from(height), i64::from(pow2_low_byte(height))))
        })?,
    )?;

    Ok(())
}

/// `round(h · 1002 / w)`, the x87 round to even; 0 for a zero span.
pub(super) fn aspect_height(w: f32, h: f32) -> u32 {
    if w == 0.0 || h == 0.0 {
        return 0;
    }
    ((h * 1002.0) / w).round_ties_even() as i64 as u32
}

/// `0x544ca0`'s third return: the low byte rounded up to a power of two.
fn pow2_low_byte(n: u32) -> u32 {
    let low = n & 0xff;
    let rounded = if low & low.wrapping_sub(1) != 0 {
        1 << (32 - low.leading_zeros())
    } else {
        low
    };
    n - low + rounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::worldmap::{WorldMapContinentView, WorldMapZoneView};
    use crate::script::UiScript;
    use std::collections::HashMap;

    fn area(
        id: u32,
        map_id: u32,
        area_id: u32,
        default_floor: i32,
        parent: u32,
    ) -> WorldMapAreaView {
        WorldMapAreaView {
            id,
            map_id,
            area_id,
            map_file: format!("Map{id}"),
            loc_rect: (100.0, 0.0, 50.0, 0.0),
            display_map: -1,
            default_floor,
            parent,
        }
    }

    fn floor(id: u32, map_id: u32, floor_index: u32, parent: u32) -> DungeonFloorView {
        DungeonFloorView {
            id,
            map_id,
            floor_index,
            rect: (0.0, 200.0, 0.0, 100.0),
            parent,
        }
    }

    /// Northrend (WMA 485, map 571) with Crystalsong (510) and Dalaran (504, floors 27/28), plus
    /// Naxxramas (535, map 533, floors 56/57, parent Dragonblight 488) and Ulduar (529, terrain).
    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        let zone = |wma_id: u32, name: &str| WorldMapZoneView {
            name: name.into(),
            wma_id,
            ..Default::default()
        };
        s.set_world_map_catalog(vec![WorldMapContinentView {
            name: "Northrend".into(),
            map_id: 571,
            wma_id: 485,
            texture_height: 668,
            zones: vec![
                zone(510, "Crystalsong Forest"),
                zone(504, "Dalaran"),
                zone(488, "Dragonblight"),
            ],
            ..Default::default()
        }]);
        let naxx = WorldMapZoneView {
            wma_id: 535,
            ..Default::default()
        };
        let ulduar = WorldMapZoneView {
            wma_id: 529,
            ..Default::default()
        };
        s.set_world_map_direct_areas(vec![(535, naxx), (529, ulduar)]);
        let areas: HashMap<u32, WorldMapAreaView> = [
            area(485, 571, 0, 0, 0),
            area(510, 571, 2817, 0, 0),
            area(504, 571, 4395, 27, 510),
            area(488, 571, 65, 0, 0),
            area(535, 533, 3456, 0, 0),
            area(529, 603, 4273, -1, 0),
        ]
        .into_iter()
        .map(|a| (a.id, a))
        .collect();
        s.set_world_map_areas(
            areas,
            vec![
                floor(27, 571, 1, 510),
                floor(28, 571, 2, 510),
                floor(56, 533, 1, 488),
                floor(57, 533, 2, 488),
                floor(70, 603, 1, 0),
            ],
        );
        s
    }

    fn state(s: &UiScript) -> (i64, i64, i64, i64) {
        s.eval(
            "return GetCurrentMapContinent(), GetCurrentMapZone(), GetCurrentMapAreaID(), \
             GetCurrentMapDungeonLevel()",
        )
        .unwrap()
    }

    #[test]
    fn set_map_by_id_reaches_zones_floors_and_instance_maps() {
        let s = script();
        assert_eq!(state(&s), (0, 0, 0, 0));
        assert!(s.eval::<bool>("return SetMapByID(510)").unwrap());
        assert_eq!(state(&s), (1, 1, 511, 0));
        // The continent row.
        s.run("SetMapByID(485)").unwrap();
        assert_eq!(state(&s), (1, 0, 486, 0));
        // Dalaran opens on its floor: the floor's continent, its zone index, floor 1.
        s.run("SetMapByID(504)").unwrap();
        assert_eq!(state(&s), (1, 2, 505, 1));
        assert_eq!(s.eval::<i64>("return GetNumDungeonMapLevels()").unwrap(), 2);
        s.run("SetDungeonMapLevel(2)").unwrap();
        assert_eq!(state(&s), (1, 2, 505, 2));
        s.run("SetDungeonMapLevel(9)").unwrap();
        assert_eq!(state(&s).3, 2);
        assert!(s.eval::<bool>("return IsZoomOutAvailable()").unwrap());
        s.run("ZoomOut()").unwrap();
        assert_eq!(state(&s), (1, 1, 511, 0));
        // Naxxramas: no continent carries map 533, so floor 1; zoom out to Dragonblight.
        s.run("SetMapByID(535)").unwrap();
        assert_eq!(state(&s), (-1, 0, 536, 1));
        s.run("ZoomOut()").unwrap();
        assert_eq!(state(&s), (1, 3, 489, 0));
        // Ulduar's terrain is level 1, its floor level 2.
        s.run("SetMapByID(529)").unwrap();
        assert_eq!(state(&s), (-1, 0, 530, 1));
        assert!(s
            .eval::<bool>("return DungeonUsesTerrainMap() == 1")
            .unwrap());
        assert_eq!(s.eval::<i64>("return GetNumDungeonMapLevels()").unwrap(), 2);
        s.run("SetDungeonMapLevel(2)").unwrap();
        assert_eq!(state(&s), (-1, 0, 530, 2));
        s.run("SetDungeonMapLevel(1)").unwrap();
        assert_eq!(state(&s), (-1, 0, 530, 1));
        assert!(s.eval::<bool>("return IsZoomOutAvailable()").unwrap());
        // An unknown id answers nothing.
        assert_eq!(
            s.eval::<i64>("return select('#', SetMapByID(9999))")
                .unwrap(),
            0
        );
        assert!(s.run("SetMapByID()").is_err());
    }

    #[test]
    fn the_cosmic_sheet_and_landmark_clicks() {
        let mut s = script();
        s.run("SetMapZoom(-1)").unwrap();
        assert_eq!(state(&s), (-1, 0, 0, 0));
        assert!(!s.eval::<bool>("return IsZoomOutAvailable()").unwrap());
        assert!(s.eval::<bool>("return GetMapInfo() == nil").unwrap());
        assert!(s.run("SetMapZoom(3)").is_err());
        s.run("SetMapZoom(1, 2)").unwrap();
        // Zone 2 is Dalaran, which opens on its floor.
        assert_eq!(state(&s), (1, 2, 505, 1));
        s.run("ClickLandmark(57)").unwrap();
        assert_eq!(state(&s), (-1, 0, 536, 2));
        s.run("ClickLandmark(12345)").unwrap();
        assert_eq!(state(&s), (-1, 0, 536, 2));
        s.run("ClickLandmark(56.9)").unwrap();
        assert_eq!(state(&s), (-1, 0, 536, 1), "the link truncates");
        let named = |name: &str| super::super::WorldMapLandmarkView {
            name: name.into(),
            ..Default::default()
        };
        s.set_world_map_landmarks(vec![named("A"), named("B")]);
        let first = s
            .eval::<String>("return (GetMapLandmarkInfo(1.9))")
            .unwrap();
        assert_eq!(first, "A", "the index truncates");
        assert!(s.run("GetMapLandmarkInfo()").is_err());
        let (file, h, pow): (String, i64, i64) = s.eval("return GetMapInfo()").unwrap();
        assert_eq!((file.as_str(), h, pow), ("Map535", 501, 512));
        assert_eq!(
            s.eval::<i64>("return select('#', GetMapDebugObjectInfo(1))")
                .unwrap(),
            0
        );
        assert_eq!(s.eval::<i64>("return GetNumMapDebugObjects()").unwrap(), 0);
        s.run("InitWorldMapPing(CreateFrame('Frame'))").unwrap();
        assert!(s.run("InitWorldMapPing()").is_err());
        assert!(s.run("InitWorldMapPing({})").is_err());
    }

    #[test]
    fn heights_round_like_the_client() {
        assert_eq!(aspect_height(200.0, 100.0), 501);
        assert_eq!(pow2_low_byte(501), 512);
        assert_eq!(pow2_low_byte(668), 512 + 256);
        assert_eq!(pow2_low_byte(512), 512);
        assert_eq!(aspect_height(0.0, 5.0), 0);
    }
}
