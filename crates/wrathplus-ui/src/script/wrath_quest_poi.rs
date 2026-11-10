//! Build-12340 quest POI and quest-log special items (WorldMapFrame, WatchFrame, QuestLogFrame).
//!
//! The POIs are `SMSG_QUEST_POI_QUERY_RESPONSE`'s blobs as the host publishes them; the query
//! itself is engine-side (native `0x5e6940` runs on every quest-log rebuild, not from a Lua
//! binding), so App sends it. A blob counts on the displayed map when its points' centroid projects
//! onto it through the world-map projector (native tests the blob against the zone or dungeon
//! floor, `0x5e0110`/`0x5e0180`). Labelled: `QuestPOIUpdateIcons` (`0x5e2eb0`) and
//! `SortQuestWatches`' sort (`0x5e5c10`) are unread.
//!
//! `GetQuestWorldMapAreaID` (`0x5e56c0` → `0x5e2b60`, `/tmp/azb-app-mig/ghidra/qwmap1.log`) picks
//! among the quest's POI blobs that the icon would mark: one on the displayed map and floor wins at
//! once; else one on the displayed map's continent, the nearest floor, then the nearest to the
//! player; else the first on another continent. A quest with none of those answers its quest-log
//! header's top zone's `WorldMapArea` row, floor 0, and a quest not in the log 0, 0. Labelled: the
//! blob point the distance reads (`+0x10`/`+0x14`) is taken as the points' centroid, the header
//! zone's row as the lowest `WorldMapArea` id carrying it (the native scan is in file order), and
//! the pre-pass `0x547170(0)` is unread.

use mlua::{Lua, MultiValue, Table, Value};

use super::wrath_worldmap::{displayed_wma, floor_row};
use super::Model;

/// One POI blob of a quest (AC `quest_poi`, its points `quest_poi_points`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathQuestPoiBlob {
    /// The objective it marks, -1 the turn-in.
    pub objective_index: i32,
    pub map_id: u32,
    /// WorldMapArea.dbc id.
    pub area_id: u32,
    pub floor: u32,
    /// World x, y.
    pub points: Vec<(i32, i32)>,
}

/// A quest's POIs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathQuestPoi {
    pub quest_id: u32,
    pub blobs: Vec<WrathQuestPoiBlob>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pois: Vec<WrathQuestPoi>,
    pub(crate) blobs: blob::Frames,
    /// Quest id → the bag slot holding its special item.
    special_items: Vec<(u32, (i64, u32))>,
    /// `QuestMapUpdateAllQuests`' visible quests, in quest-log order (`0xc23b28`).
    visible: Vec<u32>,
    /// The player's world `(x, y)`, which `GetQuestWorldMapAreaID`'s nearest-blob leg measures from.
    player: Option<(f32, f32)>,
    /// Each logged quest's quest-log header zone, walked up to its top `AreaTable` zone.
    zones: Vec<(u32, u32)>,
}

impl super::UiScript {
    /// Publish the quest POIs, whole.
    pub fn set_wrath_quest_pois(&mut self, pois: Vec<WrathQuestPoi>) {
        self.model_mut().wrath_quest_poi.pois = pois;
    }

    /// Publish the player's world `(x, y)`.
    pub fn set_wrath_quest_player_position(&mut self, player: Option<(f32, f32)>) {
        self.model_mut().wrath_quest_poi.player = player;
    }

    /// Publish each logged quest's header top zone (quest id, `AreaTable` id), whole.
    pub fn set_wrath_quest_zones(&mut self, zones: Vec<(u32, u32)>) {
        self.model_mut().wrath_quest_poi.zones = zones;
    }

    /// Publish where each quest's special item sits in the bags (quest id, bag, slot).
    pub fn set_wrath_quest_special_items(&mut self, items: Vec<(u32, i64, u32)>) {
        self.model_mut().wrath_quest_poi.special_items =
            items.into_iter().map(|(q, b, s)| (q, (b, s))).collect();
    }
}

/// The quest's incomplete-objective mask (`0x5e2950`): -1 when complete.
fn incomplete_mask(m: &Model, quest_id: u32) -> Option<u32> {
    let e = m
        .quest_log
        .entries
        .iter()
        .find(|e| !e.is_header && e.quest_id == quest_id)?;
    if e.complete == 1 {
        return Some(u32::MAX);
    }
    // Bits are template slots (`0x5e2950`): an empty or source-item slot shifts rows, not bits.
    let open = e.objectives.iter().filter(|o| !o.finished);
    Some(
        open.filter_map(|o| o.poi_bit)
            .fold(0, |mask, b| mask | 1 << b),
    )
}

/// The blob a quest's icon marks on the displayed map, projected: the turn-in when complete,
/// else the first blob of an incomplete objective.
fn icon(m: &Model, quest_id: u32) -> Option<(bool, f32, f32, i32)> {
    let mask = incomplete_mask(m, quest_id)?;
    let complete = mask == u32::MAX;
    let poi = m
        .wrath_quest_poi
        .pois
        .iter()
        .find(|p| p.quest_id == quest_id)?;
    let project = m.world_loc_projector.as_ref()?;
    let selection = m.map_selection();
    poi.blobs.iter().filter(|b| marks(mask, b)).find_map(|b| {
        let (x, y) = centroid(b);
        let (u, v) = project(selection, b.map_id, x, y)?;
        (u * v)
            .abs()
            .gt(&f32::EPSILON)
            .then_some((complete, u, v, b.objective_index))
    })
}

/// Whether the icon would mark this blob: the turn-in when complete, else an incomplete objective.
fn marks(mask: u32, b: &WrathQuestPoiBlob) -> bool {
    if mask == u32::MAX {
        b.objective_index == -1
    } else {
        b.objective_index >= 0 && mask & (1 << (b.objective_index & 31)) != 0
    }
}

fn centroid(b: &WrathQuestPoiBlob) -> (f32, f32) {
    let n = b.points.len().max(1) as f32;
    let (sx, sy) = b
        .points
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.0 as f32, y + p.1 as f32));
    (sx / n, sy / n)
}

/// `0x5e2b60`'s POI leg: the `(WorldMapArea id, floor)` of the blob to show.
fn poi_area(m: &Model, quest_id: u32) -> Option<(u32, u32)> {
    let wm = &m.worldmap;
    let shown = displayed_wma(wm).and_then(|id| wm.areas.get(&id))?;
    let floor = floor_row(wm).map_or(0, |f| f.floor_index);
    let mask = incomplete_mask(m, quest_id)?;
    let poi = m
        .wrath_quest_poi
        .pois
        .iter()
        .find(|p| p.quest_id == quest_id)?;
    let (px, py) = m.wrath_quest_poi.player.unwrap_or_default();
    let mut best = None;
    let mut nearest: Option<(u32, f32)> = None;
    for b in poi.blobs.iter().filter(|b| marks(mask, b)) {
        if b.area_id == shown.id && b.floor == floor {
            return Some((b.area_id, b.floor));
        }
        let Some(row) = wm.areas.get(&b.area_id) else {
            continue;
        };
        if row.map_id == shown.map_id {
            let (cx, cy) = centroid(b);
            let key = (
                floor.abs_diff(b.floor),
                (px - cx).powi(2) + (py - cy).powi(2),
            );
            if nearest.is_none_or(|n| key.0 < n.0 || (key.0 == n.0 && key.1 < n.1)) {
                nearest = Some(key);
                best = Some((b.area_id, b.floor));
            }
        } else if best.is_none() {
            best = Some((b.area_id, b.floor));
        }
    }
    best
}

/// `0x5e2b60`'s fallback: the quest-log header's top zone's `WorldMapArea` row, floor 0.
fn zone_area(m: &Model, quest_id: u32) -> Option<u32> {
    let (_, zone) = m
        .wrath_quest_poi
        .zones
        .iter()
        .find(|(q, _)| *q == quest_id)?;
    m.worldmap
        .areas
        .values()
        .filter(|a| a.area_id == *zone)
        .map(|a| a.id)
        .min()
}

fn log_index(m: &Model, quest_id: u32) -> Option<usize> {
    m.quest_log
        .entries
        .iter()
        .position(|e| !e.is_header && e.quest_id == quest_id)
        .map(|i| i + 1)
}

/// A quest-log row's special item slot, by 1-based log index (`0x5e0da0`).
fn special_slot(m: &Model, index: i64) -> Option<(i64, u32)> {
    let entry = usize::try_from(index - 1)
        .ok()
        .and_then(|i| m.quest_log.entries.get(i))?;
    m.wrath_quest_poi
        .special_items
        .iter()
        .find(|(q, _)| *q == entry.quest_id && !entry.is_header)
        .map(|(_, at)| *at)
        .filter(|(bag, slot)| {
            m.containers
                .get(bag)
                .and_then(|c| c.slots.get(slot))
                .is_some_and(|s| s.item_id != 0)
        })
}

fn number(lua: &Lua, v: Value, usage: &str) -> mlua::Result<i64> {
    lua.coerce_number(v)?
        .map(|n| n.round_ties_even() as i64)
        .ok_or_else(|| mlua::Error::runtime(usage.to_string()))
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

#[path = "wrath_quest_poi_blob.rs"]
pub(crate) mod blob;
#[path = "wrath_quest_poi_order.rs"]
pub(crate) mod order;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // QuestMapUpdateAllQuests() (`0x5e63d0` → `0x5e5a50`) → how many log quests have a POI on the
    // displayed map, which become the visible list, in [`order::visible_quests`]' order.
    set(lua, "QuestMapUpdateAllQuests", |lua, _| {
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let visible = order::visible_quests(&m);
        let n = visible.len() as i64;
        m.wrath_quest_poi.visible = visible;
        Ok(MultiValue::from_vec(vec![Value::Integer(n)]))
    })?;
    // GetQuestWorldMapAreaID(questID) (`0x5e56c0`) → the WorldMapArea id and floor to show it on.
    set(lua, "GetQuestWorldMapAreaID", |lua, args| {
        let quest = number(lua, arg(&args, 0), "Usage: GetQuestWorldMapAreaID(questID)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let (area, floor) = u32::try_from(quest)
            .ok()
            .filter(|&q| q > 0)
            .and_then(|q| poi_area(&m, q).or_else(|| zone_area(&m, q).map(|a| (a, 0))))
            .unwrap_or((0, 0));
        Ok(MultiValue::from_vec(vec![
            Value::Integer(area.into()),
            Value::Integer(floor.into()),
        ]))
    })?;
    // QuestPOIGetQuestIDByVisibleIndex(index) (`0x5e5750`) → questID, quest-log index.
    set(lua, "QuestPOIGetQuestIDByVisibleIndex", |lua, args| {
        let i = number(
            lua,
            arg(&args, 0),
            "Usage: QuestPOIGetQuestIDByVisibleIndex(index)",
        )?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(&quest) = usize::try_from(i - 1)
            .ok()
            .filter(|&i| i < 25)
            .and_then(|i| m.wrath_quest_poi.visible.get(i))
        else {
            return Ok(MultiValue::new());
        };
        let index = log_index(&m, quest).map_or(0, |i| i as i64);
        Ok(MultiValue::from_vec(vec![
            Value::Integer(quest.into()),
            Value::Integer(index),
        ]))
    })?;
    // QuestPOIGetIconInfo(questID) (`0x5e0590`) → completed, x, y, objective on the displayed
    // map; nothing off it.
    set(lua, "QuestPOIGetIconInfo", |lua, args| {
        let q = number(lua, arg(&args, 0), "Usage: QuestPOIGetIconInfo(questID)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some((complete, x, y, objective)) = u32::try_from(q).ok().and_then(|q| icon(&m, q))
        else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(complete),
            Value::Number(f64::from(x)),
            Value::Number(f64::from(y)),
            Value::Integer(objective.into()),
        ]))
    })?;
    // QuestPOIUpdateIcons() (`0x5e5740` → `0x5e2eb0`, labelled: unread): the icons are computed
    // on read here.
    set(lua, "QuestPOIUpdateIcons", |_, _| Ok(MultiValue::new()))?;
    // GetQuestPOILeaderBoard(objective, questIndex) (`0x5e6650`) → text, type, finished (1/nil);
    // three nils for a header or a miss.
    set(lua, "GetQuestPOILeaderBoard", |lua, args| {
        let objective = number(lua, arg(&args, 0), "Usage: GetQuestLogLeaderBoard(index)")?;
        let quest = lua
            .coerce_number(arg(&args, 1))?
            .map_or(0, |n| n.round_ties_even() as i64);
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let line = usize::try_from(quest - 1)
            .ok()
            .and_then(|i| m.quest_log.entries.get(i))
            .filter(|e| !e.is_header)
            .and_then(|e| {
                usize::try_from(objective - 1)
                    .ok()
                    .and_then(|o| e.objectives.get(o))
            });
        Ok(MultiValue::from_vec(match line {
            Some(l) => vec![
                Value::String(lua.create_string(&l.text)?),
                Value::String(lua.create_string(&l.kind)?),
                super::binding_abi::flag(l.finished),
            ],
            None => vec![Value::Nil, Value::Nil, Value::Nil],
        }))
    })?;
    // SortQuestWatches() (`0x5e64b0`) → whether the watch order changed. Labelled: the sort
    // `0x5e5c10` is unread; the order stands.
    set(lua, "SortQuestWatches", |_, _| {
        Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
    })?;

    // ── the quest log's special item: the bag slot the host names ──
    // GetQuestLogSpecialItemInfo(index) (`0x5e52d0`) → link, icon, charges.
    set(lua, "GetQuestLogSpecialItemInfo", |lua, args| {
        let i = number(
            lua,
            arg(&args, 0),
            "Usage: GetQuestLogSpecialItemInfo(index)",
        )?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(slot) =
            special_slot(&m, i).and_then(|(b, s)| m.containers.get(&b)?.slots.get(&s).cloned())
        else {
            return Ok(MultiValue::new());
        };
        let text = |s: &Option<String>| -> mlua::Result<Value> {
            Ok(match s {
                Some(s) => Value::String(lua.create_string(s)?),
                None => Value::Nil,
            })
        };
        Ok(MultiValue::from_vec(vec![
            text(&slot.link)?,
            text(&slot.texture)?,
            Value::Integer(slot.count.into()),
        ]))
    })?;
    // GetQuestLogSpecialItemCooldown(index) (`0x5e53d0`) → start, duration, enable: the slot's
    // cooldown as GetContainerItemCooldown reads it.
    set(lua, "GetQuestLogSpecialItemCooldown", |lua, args| {
        let i = number(
            lua,
            arg(&args, 0),
            "Usage: GetQuestLogSpecialItemCooldown(index)",
        )?;
        let at = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            special_slot(&m, i)
        };
        match at {
            Some((bag, slot)) => lua
                .globals()
                .get::<mlua::Function>("GetContainerItemCooldown")?
                .call::<MultiValue>((bag, slot)),
            None => Ok(MultiValue::new()),
        }
    })?;
    // IsQuestLogSpecialItemInRange(index, unit) (`0x5e54c0`) → 1, 0 or nil. Labelled: the item
    // spell's range check `0x809610` needs host range data; this answers nil, "no range".
    set(lua, "IsQuestLogSpecialItemInRange", |lua, args| {
        number(
            lua,
            arg(&args, 0),
            "Usage: IsQuestLogSpecialItemInRange(index [,target])",
        )?;
        Ok(MultiValue::from_vec(vec![Value::Nil]))
    })?;
    // UseQuestLogSpecialItem(index) (`0x5e5640`): the item used, as from its bag slot.
    set(lua, "UseQuestLogSpecialItem", |lua, args| {
        let i = number(lua, arg(&args, 0), "Usage: UseQuestLogSpecialItem(index)")?;
        let at = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            special_slot(&m, i)
        };
        if let Some((bag, slot)) = at {
            lua.globals()
                .get::<mlua::Function>("UseContainerItem")?
                .call::<()>((bag, slot))?;
        }
        Ok(MultiValue::new())
    })?;
    // GameTooltip:SetQuestLogSpecialItem(index) (`0x630990`): the special item's tooltip.
    let tooltip: Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetQuestLogSpecialItem",
        lua.create_function(|lua, (this, index): (Table, Value)| {
            let name: Option<String> = mlua::ObjectLike::call_method(&this, "GetName", ())?;
            let i = number(
                lua,
                index,
                &format!(
                    "Usage: {}:SetQuestLogSpecialItem(index)",
                    name.unwrap_or_else(|| "<unnamed>".into())
                ),
            )?;
            let at = {
                let m = lua.app_data_ref::<Model>().expect("model app_data");
                special_slot(&m, i)
            };
            if let Some((bag, slot)) = at {
                mlua::ObjectLike::call_method::<()>(&this, "SetBagItem", (bag, slot))?;
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{
        ContainerSlot, ContainerState, QuestLogEntryView, QuestLogObjectiveView, QuestLogState,
        UiScript,
    };

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_quest_log(QuestLogState {
            entries: vec![
                QuestLogEntryView {
                    title: "Zone".into(),
                    is_header: true,
                    ..Default::default()
                },
                QuestLogEntryView {
                    quest_id: 10,
                    title: "Kill".into(),
                    objectives: vec![
                        QuestLogObjectiveView {
                            text: "Wolves: 2/5".into(),
                            kind: "monster".into(),
                            finished: false,
                            poi_bit: Some(0),
                            ..Default::default()
                        },
                        QuestLogObjectiveView {
                            text: "Bears: 5/5".into(),
                            kind: "monster".into(),
                            finished: true,
                            poi_bit: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                QuestLogEntryView {
                    quest_id: 11,
                    title: "Done".into(),
                    complete: 1,
                    ..Default::default()
                },
                QuestLogEntryView {
                    quest_id: 12,
                    title: "Elsewhere".into(),
                    ..Default::default()
                },
            ],
            num_quests: 3,
        });
        // Map 0 projects to the displayed map; anything else is off it.
        s.set_world_loc_projector(Box::new(|_, map, x, y| {
            (map == 0).then_some((x / 100.0, y / 100.0))
        }));
        let blob = |objective_index, map_id, points: Vec<(i32, i32)>| WrathQuestPoiBlob {
            objective_index,
            map_id,
            points,
            ..Default::default()
        };
        s.set_wrath_quest_pois(vec![
            WrathQuestPoi {
                quest_id: 10,
                blobs: vec![
                    blob(1, 0, vec![(10, 10)]),
                    blob(0, 0, vec![(20, 40), (40, 60)]),
                ],
            },
            WrathQuestPoi {
                quest_id: 11,
                blobs: vec![blob(-1, 0, vec![(50, 50)])],
            },
            WrathQuestPoi {
                quest_id: 12,
                blobs: vec![blob(0, 1, vec![(50, 50)])],
            },
        ]);
        s
    }

    #[test]
    fn visible_quests_icons_and_leaderboards() {
        let s = vm();
        for check in [
            "QuestMapUpdateAllQuests() == 2",
            "select(1, QuestPOIGetQuestIDByVisibleIndex(1)) == 11 and select(2, QuestPOIGetQuestIDByVisibleIndex(1)) == 3",
            "select(1, QuestPOIGetQuestIDByVisibleIndex(2)) == 10 and select('#', QuestPOIGetQuestIDByVisibleIndex(3)) == 0",
            "select(1, QuestPOIGetIconInfo(10)) == false and math.abs(select(2, QuestPOIGetIconInfo(10)) - 0.3) < 1e-6 and select(4, QuestPOIGetIconInfo(10)) == 0",
            "select(1, QuestPOIGetIconInfo(11)) == true and select(4, QuestPOIGetIconInfo(11)) == -1",
            "select('#', QuestPOIGetIconInfo(12)) == 0",
            "select(1, GetQuestPOILeaderBoard(1, 2)) == 'Wolves: 2/5' and select(3, GetQuestPOILeaderBoard(2, 2)) == 1",
            "select('#', GetQuestPOILeaderBoard(1, 1)) == 3 and GetQuestPOILeaderBoard(1, 1) == nil",
            "SortQuestWatches() == false and select('#', QuestPOIUpdateIcons()) == 0",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
        assert!(
            s.run("QuestPOIGetIconInfo()").is_err() && s.run("GetQuestPOILeaderBoard()").is_err()
        );
    }

    #[test]
    fn blob_objectives_are_template_slots_not_leaderboard_rows() {
        let s = vm();
        s.model_mut().quest_log.entries[1]
            .objectives
            .iter_mut()
            .zip([1, 0])
            .for_each(|(o, b)| o.poi_bit = Some(b));
        let (x, objective) = s
            .eval::<(f64, i64)>(
                "QuestMapUpdateAllQuests() local _, x, _, o = QuestPOIGetIconInfo(10) return x, o",
            )
            .unwrap();
        assert!((x - 0.1).abs() < 1e-6 && objective == 1, "{x} {objective}");
    }

    #[test]
    fn special_items_read_their_bag_slot() {
        let mut s = vm();
        assert!(s
            .eval::<bool>("return select('#', GetQuestLogSpecialItemInfo(2)) == 0")
            .unwrap());
        let mut bag = ContainerState {
            num_slots: 4,
            ..Default::default()
        };
        bag.slots.insert(
            2,
            ContainerSlot {
                item_id: 33,
                count: 1,
                texture: Some("Interface\\Icons\\INV_Misc_Rune_01".into()),
                link: Some("|Hitem:33|h[Rune]|h".into()),
                ..Default::default()
            },
        );
        s.set_container(0, Some(bag));
        s.set_wrath_quest_special_items(vec![(10, 0, 2)]);
        assert!(s
            .eval::<bool>("local l, t, c = GetQuestLogSpecialItemInfo(2) return l == '|Hitem:33|h[Rune]|h' and t:find('Rune') ~= nil and c == 1 and select('#', GetQuestLogSpecialItemInfo(3)) == 0 and IsQuestLogSpecialItemInRange(2, 'target') == nil")
            .unwrap());
        s.run(
            "GetQuestLogSpecialItemCooldown(2) UseQuestLogSpecialItem(2) UseQuestLogSpecialItem(4)",
        )
        .unwrap();
        assert!(s.run("UseQuestLogSpecialItem()").is_err());
    }

    #[test]
    fn quest_world_map_area_prefers_the_shown_map_then_the_nearest_then_the_header_zone() {
        use crate::script::worldmap::WorldMapAreaView;
        let mut s = vm();
        let area = |id, map_id, area_id| WorldMapAreaView {
            id,
            map_id,
            area_id,
            ..Default::default()
        };
        s.set_world_map_areas(
            [
                area(14, 1, 0),
                area(141, 1, 141),
                area(23, 0, 139),
                area(30, 1, 331),
            ]
            .into_iter()
            .map(|a| (a.id, a))
            .collect(),
            Vec::new(),
        );
        let blob = |objective_index, area_id, x| WrathQuestPoiBlob {
            objective_index,
            area_id,
            points: vec![(x, 0)],
            ..Default::default()
        };
        s.set_wrath_quest_pois(vec![
            // Quest 10's objective 1 is finished, so its blob never counts.
            WrathQuestPoi {
                quest_id: 10,
                blobs: vec![
                    blob(1, 141, 0),
                    blob(0, 23, 0),
                    blob(0, 141, 900),
                    blob(0, 30, 100),
                ],
            },
            WrathQuestPoi {
                quest_id: 11,
                blobs: vec![blob(-1, 23, 0)],
            },
        ]);
        s.set_wrath_quest_player_position(Some((120.0, 0.0)));
        s.set_wrath_quest_zones(vec![(12, 331), (11, 139)]);
        let at = |s: &UiScript, q: u32| {
            s.eval::<(i64, i64)>(&format!("return GetQuestWorldMapAreaID({q})"))
                .unwrap()
        };
        // Nothing displayed: no POI leg, so the header zone answers.
        assert_eq!(at(&s, 10), (0, 0));
        assert_eq!(at(&s, 11), (23, 0));
        // Continent 1 shown: its blob nearest the player beats the other continent's.
        s.model_mut().worldmap.direct_area = Some(14);
        assert_eq!(at(&s, 10), (30, 0));
        // A blob on the shown map and floor wins at once.
        s.model_mut().worldmap.direct_area = Some(141);
        assert_eq!(at(&s, 10), (141, 0));
        assert_eq!(
            at(&s, 11),
            (23, 0),
            "only another continent's blob: the first"
        );
        assert_eq!(at(&s, 12), (30, 0), "no POIs: the header zone's row");
        assert_eq!(at(&s, 99), (0, 0), "not in the log");
        assert!(s.run("GetQuestWorldMapAreaID()").is_err());
    }
}
