//! Build 12340's `QuestPOIFrame`, the world map's quest blobs (`WorldMapBlobFrame`): its method
//! table at `0xacf180`.. (one Wow.exe round, `target/azb-live/ghidra/ui/questblob.log`).
//!
//! - `DrawQuestBlob(questId, draw)` (`0x5905f0`): `draw` takes one of four slots (`+0x2d0`,
//!   `0x590560`) for a quest in the visible POI list (`0x5e3ac0`), unless one holds it already;
//!   `draw` false frees the quest's slot (`0x58fb40`).
//! - `SetFillTexture`/`SetBorderTexture` (`0x58fe50`/`0x58fea0`) take the art; `SetFillAlpha` and
//!   `SetBorderAlpha` (`0x58e660`/`0x58e6f0`) clamp to `[0, 255]` (`[0x9e30c0]`) and keep the
//!   rounded byte; `SetBorderScalar` (`0x58e780`) clamps to `[0, 10]` (`[0x9e30cc]`);
//!   `EnableSmoothing`/`EnableMerging` (`0x58e800`/`0x58e850`) keep a flag.
//! - `UpdateMouseOverTooltip(x, y)` (`0x58e9c0` → `0x58e0d0`): frame fractions, y down; a point
//!   outside `[0, 1]` clears. The first slot with a blob under the point answers the quest's log
//!   row and how many objectives the blob covers, kept for `GetNumTooltips` (`0x58eac0`) and
//!   `GetTooltipIndex(i)` (`0x58ea50`, 0 outside `1..=4`). A blob counts when its objective is
//!   open in the quest's mask (`0x5e2950`; a complete quest's -1 never hits), it lies on the
//!   displayed zone and floor (`0x5e0110`) and has three points or more (`0x9830d0` tests it).
//!
//! Labelled: the slot geometry builder (`0x58fef0`/`0x58f1a0`) is unread. The frame draws the
//! blobs the hit test reads, as polygons in frame fractions; the art's scale and the border's
//! width are the renderer's. Merging is unread too, so a blob covers its own objective alone,
//! and `GetTooltipIndex` answers that objective's 1-based leaderboard row.

use std::collections::HashMap;

use mlua::{Lua, Table, Value};

use super::super::object::frame_handle_of;
use super::super::Model;
use crate::widget::FrameHandle;

/// The kind's method table, which `kind_method_registries` links ahead of the frame methods.
pub(crate) const REG_QUEST_POI_METHODS: &str = "__wrathplus_quest_poi_methods";

/// One `QuestPOIFrame`'s state.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    fill: Option<String>,
    border: Option<String>,
    fill_alpha: u8,
    border_alpha: u8,
    border_scalar: f32,
    smoothing: bool,
    merging: bool,
    /// The four quest slots (`+0x2d0`, stride `0x18`).
    slots: [Option<u32>; 4],
    /// The last hit: the quest and the objectives its blob covers (`+0x2b8`, `+0x2c0`..).
    hit: Option<(u32, Vec<u32>)>,
}

impl Default for Frame {
    // Labelled: the constructor's defaults are unread; stock sets every one at OnLoad.
    fn default() -> Self {
        Self {
            fill: None,
            border: None,
            fill_alpha: 255,
            border_alpha: 255,
            border_scalar: 1.0,
            smoothing: false,
            merging: false,
            slots: [None; 4],
            hit: None,
        }
    }
}

/// Every `QuestPOIFrame`'s state, by frame.
pub(crate) type Frames = HashMap<FrameHandle, Frame>;

/// What the frame draws: each slot's open-objective blobs on the displayed map, in frame
/// fractions (y down), with the art and alphas set on it.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestBlobDraw {
    pub polygons: Vec<Vec<[f32; 2]>>,
    pub fill: Option<String>,
    pub border: Option<String>,
    pub fill_alpha: u8,
    pub border_alpha: u8,
    pub border_scalar: f32,
    pub smoothing: bool,
    pub merging: bool,
}

/// The quest's blobs the frame reads (an open objective's, on the displayed zone and floor, three
/// points or more), each with its objective, projected into map fractions.
fn blobs(m: &Model, quest_id: u32) -> Vec<(i32, Vec<[f32; 2]>)> {
    let Some(mask) = super::incomplete_mask(m, quest_id) else {
        return Vec::new();
    };
    let wm = &m.worldmap;
    let shown = super::displayed_wma(wm);
    let floor = super::floor_row(wm).map_or(0, |f| f.floor_index);
    let (Some(poi), Some(project)) = (
        m.wrath_quest_poi
            .pois
            .iter()
            .find(|p| p.quest_id == quest_id),
        m.world_loc_projector.as_ref(),
    ) else {
        return Vec::new();
    };
    let selection = m.map_selection();
    let open = |o: i32| mask != u32::MAX && o >= 0 && mask & (1 << (o & 31)) != 0;
    poi.blobs
        .iter()
        .filter(|b| open(b.objective_index) && b.points.len() > 2)
        .filter(|b| Some(b.area_id) == shown && b.floor == floor)
        .filter_map(|b| {
            let points = b.points.iter().map(|&(x, y)| {
                project(selection, b.map_id, x as f32, y as f32).map(|(u, v)| [u, v])
            });
            Some((b.objective_index, points.collect::<Option<Vec<_>>>()?))
        })
        .collect()
}

/// Even-odd point-in-polygon, the closed ring through every point.
fn inside(p: [f32; 2], ring: &[[f32; 2]]) -> bool {
    let mut hit = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            hit = !hit;
        }
        j = i;
    }
    hit
}

/// The extract's content for a `QuestPOIFrame`: `None` for any other frame.
pub(crate) fn content(m: &Model, fh: FrameHandle) -> Option<super::super::QuadContent> {
    let f = m.wrath_quest_poi.blobs.get(&fh)?;
    let polygons = f
        .slots
        .iter()
        .flatten()
        .flat_map(|&q| blobs(m, q).into_iter().map(|(_, ring)| ring))
        .collect();
    Some(super::super::QuadContent::QuestBlob(QuestBlobDraw {
        polygons,
        fill: f.fill.clone(),
        border: f.border.clone(),
        fill_alpha: f.fill_alpha,
        border_alpha: f.border_alpha,
        border_scalar: f.border_scalar,
        smoothing: f.smoothing,
        merging: f.merging,
    }))
}

/// A `0x84e0b0` flag: anything but nil, false or 0.
fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Boolean(false))
        && !matches!(v, Value::Integer(0))
        && !matches!(v, Value::Number(n) if *n == 0.0)
}

fn number(lua: &Lua, v: Value) -> f64 {
    lua.coerce_number(v).ok().flatten().unwrap_or(0.0)
}

/// Runs `edit` on the frame's state, made on first use.
fn with<R>(lua: &Lua, this: &Table, edit: impl FnOnce(&mut Frame) -> R) -> mlua::Result<R> {
    let h = frame_handle_of(lua, this)?;
    let mut m = lua.app_data_mut::<Model>().expect("model app_data");
    Ok(edit(m.wrath_quest_poi.blobs.entry(h).or_default()))
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let m = lua.create_table()?;
    m.set(
        "DrawQuestBlob",
        lua.create_function(|lua, (this, quest, draw): (Table, Value, Value)| {
            let quest = number(lua, quest).round() as i64 as u32;
            let visible = {
                let m = lua.app_data_ref::<Model>().expect("model app_data");
                super::order::visible_quests(&m).contains(&quest)
            };
            with(lua, &this, |f| {
                if !truthy(&draw) {
                    f.slots
                        .iter_mut()
                        .filter(|s| **s == Some(quest))
                        .for_each(|s| *s = None);
                } else if visible && !f.slots.contains(&Some(quest)) {
                    if let Some(free) = f.slots.iter_mut().find(|s| s.is_none()) {
                        *free = Some(quest);
                    }
                }
            })
        })?,
    )?;
    for (name, border) in [("SetFillTexture", false), ("SetBorderTexture", true)] {
        m.set(
            name,
            lua.create_function(move |lua, (this, path): (Table, Option<String>)| {
                with(lua, &this, |f| {
                    *(if border { &mut f.border } else { &mut f.fill }) = path
                })
            })?,
        )?;
    }
    for (name, border) in [("SetFillAlpha", false), ("SetBorderAlpha", true)] {
        m.set(
            name,
            lua.create_function(move |lua, (this, alpha): (Table, Value)| {
                let byte = number(lua, alpha).clamp(0.0, 255.0).round() as u8;
                with(lua, &this, |f| {
                    *(if border {
                        &mut f.border_alpha
                    } else {
                        &mut f.fill_alpha
                    }) = byte;
                })
            })?,
        )?;
    }
    m.set(
        "SetBorderScalar",
        lua.create_function(|lua, (this, s): (Table, Value)| {
            let s = number(lua, s).clamp(0.0, 10.0) as f32;
            with(lua, &this, |f| f.border_scalar = s)
        })?,
    )?;
    for (name, merging) in [("EnableSmoothing", false), ("EnableMerging", true)] {
        m.set(
            name,
            lua.create_function(move |lua, (this, on): (Table, Value)| {
                let on = truthy(&on);
                with(lua, &this, |f| {
                    *(if merging {
                        &mut f.merging
                    } else {
                        &mut f.smoothing
                    }) = on
                })
            })?,
        )?;
    }
    m.set(
        "UpdateMouseOverTooltip",
        lua.create_function(|lua, (this, x, y): (Table, Value, Value)| {
            let p = [number(lua, x) as f32, number(lua, y) as f32];
            let h = frame_handle_of(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let slots = model.wrath_quest_poi.blobs.entry(h).or_default().slots;
            let in_frame = p.iter().all(|c| (0.0..=1.0).contains(c));
            let hit = slots.iter().flatten().filter(|_| in_frame).find_map(|&q| {
                let hits: Vec<u32> = blobs(&model, q)
                    .into_iter()
                    .filter(|(_, ring)| inside(p, ring))
                    .map(|(o, _)| leaderboard_row(&model, q, o).unwrap_or(o as u32 + 1))
                    .take(1)
                    .collect();
                (!hits.is_empty()).then_some((q, hits))
            });
            let answer = hit
                .as_ref()
                .and_then(|(q, hits)| Some((super::log_index(&model, *q)?, hits.len())));
            model.wrath_quest_poi.blobs.entry(h).or_default().hit = hit;
            Ok(match answer {
                Some((row, n)) => mlua::MultiValue::from_vec(vec![
                    Value::Integer(row as i64),
                    Value::Integer(n as i64),
                ]),
                None => mlua::MultiValue::new(),
            })
        })?,
    )?;
    m.set(
        "GetNumTooltips",
        lua.create_function(|lua, this: Table| {
            with(lua, &this, |f| {
                f.hit.as_ref().map_or(0, |(_, h)| h.len() as i64)
            })
        })?,
    )?;
    m.set(
        "GetTooltipIndex",
        lua.create_function(|lua, (this, i): (Table, Value)| {
            let i = number(lua, i).round() as i64;
            with(lua, &this, |f| {
                let hits = f
                    .hit
                    .as_ref()
                    .map(|(_, h)| h.as_slice())
                    .unwrap_or_default();
                usize::try_from(i - 1)
                    .ok()
                    .filter(|&i| i < 4)
                    .and_then(|i| hits.get(i))
                    .map_or(0, |&r| i64::from(r))
            })
        })?,
    )?;
    lua.set_named_registry_value(REG_QUEST_POI_METHODS, m)
}

/// The 1-based leaderboard row of the quest's objective in template slot `objective`.
fn leaderboard_row(m: &Model, quest_id: u32, objective: i32) -> Option<u32> {
    let e = m
        .quest_log
        .entries
        .iter()
        .find(|e| !e.is_header && e.quest_id == quest_id)?;
    let slot = u8::try_from(objective).ok()?;
    let row = e.objectives.iter().position(|o| o.poi_bit == Some(slot))?;
    Some(row as u32 + 1)
}

#[cfg(test)]
mod tests {
    use crate::script::worldmap::WorldMapAreaView;
    use crate::script::{
        QuadContent, QuestLogEntryView, QuestLogObjectiveView, QuestLogState, UiScript,
        WrathQuestPoi, WrathQuestPoiBlob,
    };

    /// Quest 10 with its objective in slot 1 open and a square blob for it (0.2..0.4 of the map)
    /// on the displayed zone 141; a stock-shaped `QuestPOIFrame` over the whole screen.
    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_quest_log(QuestLogState {
            entries: vec![QuestLogEntryView {
                quest_id: 10,
                title: "Kill".into(),
                objectives: vec![QuestLogObjectiveView {
                    text: "Wolves: 2/5".into(),
                    kind: "monster".into(),
                    poi_bit: Some(1),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            num_quests: 1,
        });
        s.set_world_loc_projector(Box::new(|_, _, x, y| Some((x / 100.0, y / 100.0))));
        let area = WorldMapAreaView {
            id: 141,
            area_id: 141,
            ..Default::default()
        };
        s.set_world_map_areas([(141, area)].into_iter().collect(), Vec::new());
        s.model_mut().worldmap.direct_area = Some(141);
        s.set_wrath_quest_pois(vec![WrathQuestPoi {
            quest_id: 10,
            blobs: vec![WrathQuestPoiBlob {
                objective_index: 1,
                area_id: 141,
                points: vec![(20, 20), (40, 20), (40, 40), (20, 40)],
                ..Default::default()
            }],
        }]);
        s.run(
            "Blob = CreateFrame('QuestPOIFrame', 'Blob', UIParent) Blob:SetAllPoints(UIParent) \
             Blob:SetFillTexture('Interface\\\\WorldMap\\\\UI-QuestBlob-Inside') \
             Blob:SetFillAlpha(300) Blob:SetBorderAlpha(-4) Blob:SetBorderScalar(12) \
             QuestMapUpdateAllQuests()",
        )
        .unwrap();
        s.resolve();
        s
    }

    fn polygons(s: &UiScript) -> Vec<Vec<[f32; 2]>> {
        s.extract()
            .into_iter()
            .find_map(|q| match q.content {
                QuadContent::QuestBlob(d) => Some(d),
                _ => None,
            })
            .map(|d| {
                assert_eq!(
                    (d.fill_alpha, d.border_alpha, d.border_scalar),
                    (255, 0, 10.0)
                );
                assert!(d.fill.unwrap().ends_with("UI-QuestBlob-Inside"));
                d.polygons
            })
            .expect("the blob frame's quad")
    }

    #[test]
    fn a_drawn_quest_blob_is_drawn_and_hit_tested() {
        let s = vm();
        assert_eq!(
            s.eval::<String>("return Blob:GetObjectType()").unwrap(),
            "QuestPOIFrame"
        );
        assert!(
            polygons(&s).is_empty(),
            "nothing drawn before DrawQuestBlob"
        );
        s.run("Blob:DrawQuestBlob(10, true) Blob:DrawQuestBlob(99, true)")
            .unwrap();
        let drawn = polygons(&s);
        assert_eq!(drawn.len(), 1, "a quest off the visible list takes no slot");
        assert_eq!(drawn[0][2], [0.4, 0.4]);
        let hit = "local row, n = Blob:UpdateMouseOverTooltip(%s) \
                   return tostring(row)..' '..tostring(n)..' '..Blob:GetNumTooltips()..' '\
                   ..Blob:GetTooltipIndex(1)..' '..Blob:GetTooltipIndex(5)";
        let at = |s: &UiScript, xy: &str| s.eval::<String>(&hit.replace("%s", xy)).unwrap();
        assert_eq!(at(&s, "0.3, 0.3"), "1 1 1 1 0", "the open objective's row");
        assert_eq!(at(&s, "0.5, 0.5"), "nil nil 0 0 0");
        assert_eq!(
            at(&s, "1.3, 0.3"),
            "nil nil 0 0 0",
            "outside the frame clears"
        );
        s.model_mut().quest_log.entries[0].complete = 1;
        assert_eq!(
            at(&s, "0.3, 0.3"),
            "nil nil 0 0 0",
            "a complete quest never hits"
        );
        s.run("Blob:DrawQuestBlob(10, false)").unwrap();
        assert!(polygons(&s).is_empty(), "false frees the slot");
        assert!(s
            .run("CreateFrame('Frame'):DrawQuestBlob(10, true)")
            .is_err());
    }
}
