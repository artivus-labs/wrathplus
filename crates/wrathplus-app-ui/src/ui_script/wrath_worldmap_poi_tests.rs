//! The stock world map's quest POI buttons draw above its detail tiles and overlays on first show.

use wrathplus_ui::script::{
    QuestLogEntryView, QuestLogObjectiveView, QuestLogState, UiScript, WrathQuestPoi,
    WrathQuestPoiBlob,
};

fn quests(s: &mut UiScript) {
    let entry = |id: u32, title: &str| QuestLogEntryView {
        quest_id: id,
        title: title.into(),
        level: 5,
        objectives: vec![QuestLogObjectiveView {
            text: "Thing: 0/1".into(),
            kind: "monster".into(),
            finished: false,
            poi_bit: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    };
    s.set_quest_log(QuestLogState {
        entries: vec![
            QuestLogEntryView {
                title: "Elwynn Forest".into(),
                is_header: true,
                ..Default::default()
            },
            entry(1, "Vile Familiars"),
            QuestLogEntryView {
                complete: 1,
                objectives: vec![QuestLogObjectiveView {
                    text: "Thing: 1/1".into(),
                    kind: "monster".into(),
                    finished: true,
                    ..Default::default()
                }],
                ..entry(2, "Tainted Parchment")
            },
        ],
        ..Default::default()
    });
    let poi = |id: u32| WrathQuestPoi {
        quest_id: id,
        blobs: vec![WrathQuestPoiBlob {
            objective_index: 0,
            map_id: 0,
            area_id: 12,
            floor: 0,
            points: vec![(-9_460, 60), (-9_440, 80)],
        }],
    };
    let mut done = poi(2);
    done.blobs[0].objective_index = -1;
    s.set_wrath_quest_pois(vec![poi(1), done]);
    s.set_world_loc_projector(Box::new(|_, _, _, _| Some((0.4, 0.6))));
}

/// Durotar's first map open with an unfinished quest and a completed one listed after it (Vile
/// Familiars, then Tainted Parchment): 12340 lists the completed quest first (`0x5e3840`), so its
/// "?" is `poiWorldMapPOIFrame4_1` and survives `QuestPOI_HideButtons(…, numPOICompleteSwap + 1)`.
/// As quest frame 1 it is selected (`WorldMapFrame.lua:1453`), so `_Swap` stands in for it. The
/// POI buttons show above the detail frame and draw after every detail tile and overlay.
#[test]
fn every_quest_poi_shows_above_the_detail_art_on_the_first_open() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    quests(&mut s);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let state: String = s
        .eval(
            "local function f(b) return b and (b:IsVisible() and 'v' or 'h')..b:GetFrameLevel() \
                 or 'nil' end \
             return f(poiWorldMapPOIFrame4_1)..' '..f(poiWorldMapPOIFrame1_1)..' '\
                 ..f(poiWorldMapPOIFrame_Swap)..' '..WorldMapDetailFrame:GetFrameLevel()",
        )
        .unwrap();
    assert_eq!(
        state, "h101 v101 v103 88",
        "selected ? swapped in, numeric POI shown, above the detail"
    );

    let quads = s.extract();
    let owner = |i: usize| s.quad_owner_name(quads[i].target).unwrap_or_default();
    let detail_last = (0..quads.len())
        .filter(|&i| matches!(owner(i).as_str(), "WorldMapDetailFrame" | "WorldMapButton"))
        .max()
        .expect("the detail tiles draw");
    let pois: Vec<usize> = (0..quads.len())
        .filter(|&i| owner(i).starts_with("poiWorldMapPOIFrame"))
        .collect();
    assert!(
        !pois.is_empty() && pois.iter().all(|&i| i > detail_last),
        "{pois:?} vs {detail_last}"
    );
}

/// Shannon's Durotar tracker (10-24): Minshina's Skull and A Solvent Spirit had POI blobs for
/// item objectives (template slots 4 and 5, `0x5e2950`) but drew no number, as the mask used
/// leaderboard rows. Both now count as on the map and number 1 and 2; the complete one is "?".
#[test]
fn watched_item_objective_quests_number_their_poi_icons() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    let item = |bit: u8, finished: bool| QuestLogObjectiveView {
        text: "Thing: 0/1".into(),
        kind: "item".into(),
        finished,
        poi_bit: Some(bit),
        ..Default::default()
    };
    let entry = |id: u32, title: &str, objectives| QuestLogEntryView {
        quest_id: id,
        title: title.into(),
        level: 5,
        objectives,
        ..Default::default()
    };
    let header = QuestLogEntryView {
        title: "Durotar".into(),
        is_header: true,
        ..Default::default()
    };
    let done = QuestLogEntryView {
        complete: 1,
        ..entry(789, "Sting of the Scorpid", vec![item(4, true)])
    };
    let skull = entry(808, "Minshina's Skull", vec![item(4, false)]);
    let spirit = entry(818, "A Solvent Spirit", vec![item(4, true), item(5, false)]);
    s.set_quest_log(QuestLogState {
        entries: vec![header, skull, spirit, done],
        num_quests: 3,
        ..Default::default()
    });
    let poi = |quest_id: u32, objectives: &[i32]| WrathQuestPoi {
        quest_id,
        blobs: objectives
            .iter()
            .map(|&objective_index| WrathQuestPoiBlob {
                objective_index,
                area_id: 12,
                points: vec![(-9_460, 60)],
                ..Default::default()
            })
            .collect(),
    };
    s.set_wrath_quest_pois(vec![poi(808, &[4]), poi(818, &[4, 5]), poi(789, &[-1])]);
    s.set_world_loc_projector(Box::new(|_, _, _, _| Some((0.4, 0.6))));
    s.run("for row = 2, 4 do AddQuestWatch(row) end SetMapToCurrentZone()")
        .unwrap();
    s.fire_event("QUEST_POI_UPDATE", vec![]);
    s.tick(0.05);
    s.resolve();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let shown: String = s
        .eval(
            "local t = {} for _, n in ipairs({'1_1', '1_2', '1_3', '2_1'}) do \
                 local b = _G['poiWatchFrameLines'..n] \
                 t[#t + 1] = n..'='..(b and b:IsVisible() and (b.questId or '?') or '-') end \
             return table.concat(t, ' ')",
        )
        .unwrap();
    assert_eq!(shown, "1_1=808 1_2=818 1_3=- 2_1=789");
}

/// Selecting the unfinished quest on the map draws its blob: `WorldMapFrame_SelectQuestFrame`
/// calls `WorldMapBlobFrame:DrawQuestBlob(questId, true)` (`WorldMapFrame.lua:1691`), and the blob
/// frame's quad carries the quest's polygon, in the stock art and alphas set at its OnLoad.
#[test]
fn the_selected_quests_blob_is_drawn_on_the_map() {
    use wrathplus_ui::script::QuadContent;
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    quests(&mut s);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    // A zone the map shows (terrain as floor 0, as `SetMapByID` opens a map no continent
    // carries), and the blobs on it so they count as on it.
    let area = 900;
    let row = wrathplus_ui::script::WorldMapAreaView {
        id: area,
        map_id: 999,
        area_id: 1,
        display_map: -1,
        default_floor: -1,
        ..Default::default()
    };
    s.set_world_map_areas([(area, row)].into_iter().collect(), Vec::new());
    assert!(s.eval::<bool>("return SetMapByID(900)").unwrap());
    let poi = |quest_id: u32, objective_index: i32| WrathQuestPoi {
        quest_id,
        blobs: vec![WrathQuestPoiBlob {
            objective_index,
            area_id: area,
            points: vec![(-9_460, 60), (-9_440, 80), (-9_450, 90)],
            ..Default::default()
        }],
    };
    s.set_wrath_quest_pois(vec![poi(1, 0), poi(2, -1)]);
    s.run("WorldMapFrame_UpdateQuests() WorldMapFrame_SelectQuestById(1)")
        .unwrap();
    s.tick(0.05);
    s.resolve();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let quads = s.extract();
    let draw = quads
        .iter()
        .find_map(|q| match &q.content {
            QuadContent::QuestBlob(d)
                if s.quad_owner_name(q.target).as_deref() == Some("WorldMapBlobFrame") =>
            {
                Some(d.clone())
            }
            _ => None,
        })
        .expect("WorldMapBlobFrame's quad");
    assert_eq!(draw.polygons.len(), 1, "quest 1's one blob");
    assert_eq!((draw.fill_alpha, draw.border_alpha), (128, 192));
    assert!(draw.border.unwrap().ends_with("UI-QuestBlob-Outside"));
}

/// The map's open sequence over its first frames: whatever draws in an early frame still draws
/// once the map settles, so nothing (a POI, a blob, a detail tile) flashes and vanishes, and the
/// selected quest's blob draws from the first frame on.
#[test]
fn the_map_draws_nothing_on_its_first_frames_that_it_then_drops() {
    use std::collections::BTreeSet;
    use wrathplus_ui::script::QuadContent;
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    quests(&mut s);
    let row = wrathplus_ui::script::WorldMapAreaView {
        id: 900,
        map_id: 999,
        area_id: 1,
        display_map: -1,
        default_floor: -1,
        ..Default::default()
    };
    s.set_world_map_areas([(900, row)].into_iter().collect(), Vec::new());
    s.run("SetMapByID(900)").unwrap();
    let blob = |id: u32, objective_index: i32| WrathQuestPoi {
        quest_id: id,
        blobs: vec![WrathQuestPoiBlob {
            objective_index,
            area_id: 900,
            points: vec![(-9_460, 60), (-9_440, 80), (-9_450, 90)],
            ..Default::default()
        }],
    };
    s.set_wrath_quest_pois(vec![blob(1, 0), blob(2, -1)]);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    let mut frames: Vec<(BTreeSet<String>, usize)> = Vec::new();
    for _ in 0..8 {
        s.tick(0.03);
        s.resolve();
        let quads = s.extract();
        let owners: BTreeSet<String> = quads
            .iter()
            .filter_map(|q| s.quad_owner_name(q.target))
            .filter(|n| n.starts_with("WorldMap") || n.starts_with("poiWorldMap"))
            .collect();
        let blobs = quads
            .iter()
            .filter_map(|q| match &q.content {
                QuadContent::QuestBlob(d) => Some(d.polygons.len()),
                _ => None,
            })
            .sum();
        frames.push((owners, blobs));
    }
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let (settled, settled_blobs) = frames.last().cloned().unwrap();
    for (i, (owners, blobs)) in frames.iter().enumerate() {
        let gone: Vec<&String> = owners.difference(&settled).collect();
        assert!(
            gone.is_empty(),
            "frame {i} drew {gone:?}, which the settled map drops"
        );
        assert_eq!(*blobs, settled_blobs, "frame {i}'s blob count");
    }
}

#[path = "wrath_event_poi_tests.rs"]
mod event_poi;
