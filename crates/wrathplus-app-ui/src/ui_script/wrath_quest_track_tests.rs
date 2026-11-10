//! The stock objectives tracker against the world map: the auto-progress watch and its expiry
//! redraw the tracker, the two number their shared quests alike, and the map's selection reaches
//! the quest log after the map closes.

use wrathplus_ui::script::{
    QuestLogEntryView, QuestLogObjectiveView, QuestLogState, ScriptValue, UiScript, WrathQuestPoi,
    WrathQuestPoiBlob,
};

/// A stock session with Alpha, Bravo and Charlie (101-103) in the log, each with one unfinished
/// kill objective in slot 0 and a POI blob for it on the displayed map.
fn session() -> Option<UiScript> {
    let data = wrathplus_formats::wow_data_or_skip!(None);
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    let quest = |id: u32, title: &str| QuestLogEntryView {
        quest_id: id,
        title: title.into(),
        level: 5,
        objectives: vec![QuestLogObjectiveView {
            text: "Thing slain: 0/1".into(),
            kind: "monster".into(),
            req: 1,
            poi_bit: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    };
    let header = QuestLogEntryView {
        title: "Elwynn Forest".into(),
        is_header: true,
        ..Default::default()
    };
    s.set_quest_log(QuestLogState {
        entries: vec![
            header,
            quest(101, "Alpha"),
            quest(102, "Bravo"),
            quest(103, "Charlie"),
        ],
        num_quests: 3,
    });
    let poi = |quest_id: u32| WrathQuestPoi {
        quest_id,
        blobs: vec![WrathQuestPoiBlob {
            objective_index: 0,
            area_id: 12,
            points: vec![(-9_460, 60), (-9_440, 80)],
            ..Default::default()
        }],
    };
    s.set_wrath_quest_pois(vec![poi(101), poi(102), poi(103)]);
    s.set_world_loc_projector(Box::new(|_, _, _, _| Some((0.4, 0.6))));
    Some(s)
}

fn title(s: &UiScript) -> String {
    s.eval("return WatchFrameTitle:GetText()").unwrap()
}

/// Progress on an unwatched quest: the packet's `QUEST_WATCH_UPDATE` (row 3, Bravo) comes before
/// the log update, so stock's 300 s auto-watch (`QuestLogFrame.lua:265-269`) is drawn by that
/// update. When it lapses, the engine's sweep (`0x5dfef0`) drops it and fires `QUEST_LOG_UPDATE`,
/// which redraws the tracker without it.
#[test]
fn the_progress_watch_and_its_expiry_each_redraw_the_tracker() {
    let Some(mut s) = session() else { return };
    s.run("AddQuestWatch(2) WatchFrame_Update()").unwrap();
    assert_eq!(title(&s), "Objectives (1)");
    assert_eq!(s.eval::<String>("return AUTO_QUEST_PROGRESS").unwrap(), "1");
    s.fire_event("QUEST_WATCH_UPDATE", vec![ScriptValue::Int(3)]);
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert_eq!(title(&s), "Objectives (2)", "the progress watch is drawn");
    s.tick(299.0);
    assert_eq!(title(&s), "Objectives (2)");
    s.tick(2.0);
    assert_eq!(title(&s), "Objectives (1)", "its expiry redrew the tracker");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

/// Watched Charlie, then Alpha; Bravo unwatched. 12340 lists the watched quests in watch order
/// ahead of the rest (`0x5e3840`), so the map numbers Charlie 1, Alpha 2, Bravo 3, and the
/// tracker, numbering along the watch list, gives Charlie and Alpha the same 1 and 2.
#[test]
fn the_tracker_and_the_map_number_a_quest_alike() {
    let Some(mut s) = session() else { return };
    s.run("AddQuestWatch(4) AddQuestWatch(2) SetMapToCurrentZone()")
        .unwrap();
    s.fire_event("QUEST_POI_UPDATE", vec![]);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    let ids = |s: &UiScript, parent: &str, n: u32| -> String {
        s.eval(&format!(
            "local t = {{}} for i = 1, {n} do local b = _G['poi{parent}1_'..i] \
             t[#t + 1] = tostring(b and b:IsShown() and b.questId) end \
             return table.concat(t, ' ')"
        ))
        .unwrap()
    };
    assert_eq!(ids(&s, "WorldMapPOIFrame", 3), "103 101 102");
    assert_eq!(ids(&s, "WatchFrameLines", 3), "103 101 nil");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

/// On the full-size map a selected quest becomes the quest log's selection
/// (`WorldMapFrame.lua:1674`), which the minimap's quest point reads, and it holds after the map
/// closes. The tracker's POI shows no selection then: stock selects it only while the map is
/// shown (`WatchFrame.lua:783-785`, `:939-941`) and every redraw deselects a reused button
/// (`QuestPOI.lua:78-80`).
#[test]
fn the_map_selection_outlives_the_map_and_the_tracker_shows_none() {
    let Some(mut s) = session() else { return };
    s.run("AddQuestWatch(2) AddQuestWatch(3) SetMapToCurrentZone()")
        .unwrap();
    s.fire_event("QUEST_POI_UPDATE", vec![]);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    let full: bool = s
        .eval("return WORLDMAP_SETTINGS.size ~= WORLDMAP_WINDOWED_SIZE")
        .unwrap();
    assert!(full, "the stock map opens full size");
    s.run(
        "for i = 1, 3 do local f = _G['WorldMapQuestFrame'..i] \
         if f and f.questId == 102 then WorldMapFrame_SelectQuestFrame(f) end end",
    )
    .unwrap();
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.tick(0.05);
    let picked: i64 = s
        .eval("return select(9, GetQuestLogTitle(GetQuestLogSelection()))")
        .unwrap();
    assert_eq!(picked, 102, "the log keeps the map's selection");
    let selected: bool = s
        .eval(
            "for i = 1, 2 do local b = _G['poiWatchFrameLines1_'..i] \
             if b and b.isSelected then return true end end return false",
        )
        .unwrap();
    assert!(
        !selected,
        "stock shows no tracker selection with the map closed"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}
