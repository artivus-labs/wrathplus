//! Event POIs through the real descriptor -> log feed and raw POI reply -> cache -> VM.
use bevy::prelude::*;
use wrathplus_protocol::types::{QuestObjective, QuestTemplate, WrathQuestInfo};
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::script::UiScript;

use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::net::{Guid, NetCommands};
use crate::ui_native::wrath_quest_pois::{QuestPois, QuestPoisView};
use crate::ui_quest_log::{feed_quest_log, QuestLog};

// AC base quest_template: 273, flags 12, ZoneOrSort 38, area description, no item/kill slots.
fn template() -> QuestTemplate {
    QuestTemplate {
        quest_id: 273,
        method: 2,
        level: 15,
        zone_or_sort: 38,
        quest_type: 0,
        flags: 12,
        title: "Resupplying the Excavation".into(),
        objectives_text: "Speak with Huldar.".into(),
        end_text: "Speak to Huldar at The Loch in Loch Modan.".into(),
        wrath: Some(WrathQuestInfo {
            flags: 12,
            area_description: "Find Huldar, Miran, and Saean".into(),
            ..Default::default()
        }),
        rep_objective_faction: 0,
        rep_objective_value: 0,
        next_quest_in_chain: 0,
        money: 0,
        money_max_level: 0,
        reward_spell: 0,
        src_item_id: 0,
        rewards: [(0, 0); 4],
        choices: [(0, 0); 6],
        point_map_id: 0,
        point_x: 0.0,
        point_y: 0.0,
        point_opt: 0,
        details: String::new(),
        objectives: std::array::from_fn(|_| QuestObjective {
            creature_or_go: 0,
            required_count: 0,
            item_id: 0,
            item_count: 0,
            text: String::new(),
        }),
    }
}

fn app(script: UiScript) -> App {
    let mut app = App::new();
    let (tx, _) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx))
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::names::NameCache>()
        .init_resource::<crate::items::Items>()
        .init_resource::<crate::app_core::state::WorldStates>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::sound::MessageSounds>()
        .init_resource::<crate::ui_quest::QuestLines>()
        .init_resource::<crate::app_core::state::QuestGiver>()
        .init_resource::<QuestLog>();
    crate::net::wrath_quest_pois::register(&mut app);
    app.insert_non_send_resource(script);
    // Cache the wire template, not a fabricated QuestLogEntryView.
    app.world_mut()
        .resource_mut::<QuestLog>()
        .insert_template(template());
    app.world_mut().spawn((
        ObjectStore(ObjectFields::from_pairs(&[(158, 273), (159, 0)])),
        Guid(42),
        SelfPlayer,
    ));
    app.add_systems(Update, feed_quest_log);
    // AC quest_poi: objective 16 and turn-in -1, map 0 / WMA 35. Points from local base data.
    let mut body = 1u32.to_le_bytes().to_vec();
    for n in [273u32, 2] {
        body.extend(n.to_le_bytes());
    }
    for (id, objective, points) in [
        (
            0u32,
            16i32,
            &[
                (-5764i32, -3469i32),
                (-5735, -3440),
                (-5764, -3411),
                (-5793, -3440),
            ][..],
        ),
        (1, -1, &[(-5763, -3433)][..]),
    ] {
        body.extend(id.to_le_bytes());
        body.extend(objective.to_le_bytes());
        for n in [0u32, 35, 0, 0, 1, points.len() as u32] {
            body.extend(n.to_le_bytes());
        }
        for (x, y) in points {
            body.extend(x.to_le_bytes());
            body.extend(y.to_le_bytes());
        }
    }
    let mut adapter = wrathplus_protocol::wrath_events::WrathEventAdapter::new();
    crate::ui_native::wrath_quest_pois::tests::dispatch(&mut app, &mut adapter, 0x1e4, &body);
    let pois = app.world().resource::<QuestPois>().view(&[273]);
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_wrath_quest_pois(pois);
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_world_loc_projector(Box::new(|_, map, _, _| (map == 0).then_some((0.4, 0.6))));
    app.update();
    app
}

#[test]
fn event_poi_production_chain_keeps_open_and_completed_quests_on_the_map() {
    let mut app = app(UiScript::new().unwrap());
    let ask = |app: &App, code: &str| {
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<String>(code)
            .unwrap()
    };
    assert_eq!(ask(&app, "QuestMapUpdateAllQuests() local id = QuestPOIGetQuestIDByVisibleIndex(1) local done, _, _, slot = QuestPOIGetIconInfo(273) return tostring(id)..' '..tostring(done)..' '..tostring(slot)"), "273 false 16");
    assert_eq!(ask(&app, "local text, kind, done = GetQuestLogLeaderBoard(1, 2) return text..' '..kind..' '..tostring(done)"), "Find Huldar, Miran, and Saean event nil");
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<SelfPlayer>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert(ObjectStore(ObjectFields::from_pairs(&[
            (158, 273),
            (159, 1),
        ])));
    app.update();
    assert_eq!(ask(&app, "QuestMapUpdateAllQuests() local done, _, _, slot = QuestPOIGetIconInfo(273) return tostring(QuestPOIGetQuestIDByVisibleIndex(1))..' '..tostring(done)..' '..tostring(slot)"), "273 true -1");
    assert_eq!(
        ask(
            &app,
            "return tostring(select(3, GetQuestLogLeaderBoard(1, 2)))"
        ),
        "1"
    );
}

#[test]
fn event_poi_stock_tracker_decoration_and_map_list_follow_the_production_log() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (script, failures) = super::super::super::stock_session(&data);
    assert!(failures.is_empty(), "{failures:?}");
    script.clear_instruction_budget();
    let mut app = app(script);
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    s.run("AddQuestWatch(2) SetMapToCurrentZone()").unwrap();
    s.fire_event("QUEST_POI_UPDATE", vec![]);
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    assert_eq!(
        s.eval::<u32>("return WorldMapQuestFrame1.questId").unwrap(),
        273
    );
    assert_eq!(
        s.eval::<u32>("return poiWatchFrameLines1_1.questId")
            .unwrap(),
        273
    );
    assert!(s
        .eval::<bool>("return poiWatchFrameLines1_1:IsShown() and poiWorldMapPOIFrame1_1:IsShown() and poiWorldMapPOIFrame1_1.questId == poiWatchFrameLines1_1.questId")
        .unwrap());
    assert_eq!(
        s.eval::<u32>("return select(4, QuestPOIGetIconInfo(273))")
            .unwrap(),
        16
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn event_poi_missing_or_off_map_does_not_invent_a_marker_or_hide_the_watch() {
    let mut app = app(UiScript::new().unwrap());
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    s.run("AddQuestWatch(2)").unwrap();
    for missing in [false, true] {
        if missing {
            s.set_wrath_quest_pois(Vec::new());
        } else {
            s.set_world_loc_projector(Box::new(|_, _, _, _| None));
        }
        assert_eq!(
            s.eval::<u32>("return QuestMapUpdateAllQuests()").unwrap(),
            0
        );
        assert_eq!(s.eval::<u32>("return GetNumQuestWatches()").unwrap(), 1);
        assert_eq!(
            s.eval::<u32>("return select('#', QuestPOIGetIconInfo(273))")
                .unwrap(),
            0
        );
    }
}

// The reported collection quests use required-item slot 0 (POI bit 4), not event bit 16.
#[test]
fn collection_pois_from_the_log_feed_reach_stock_tracker_and_map_selection() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (script, failures) = super::super::super::stock_session(&data);
    assert!(failures.is_empty(), "{failures:?}");
    script.clear_instruction_budget();
    let mut app = app(script);
    let cases = [
        (963, "For Love Eternal", 5382, (5760, 190)),
        (3524, "Washed Ashore", 12242, (6110, 560)),
        (2518, "Tears of the Moon", 8344, (10979, 1366)),
    ];
    let mut fields = Vec::new();
    let mut body = (cases.len() as u32).to_le_bytes().to_vec();
    for (i, (id, title, item, (x, y))) in cases.iter().enumerate() {
        let mut t = template();
        t.quest_id = *id;
        t.title = (*title).into();
        t.zone_or_sort = 148;
        t.wrath.as_mut().unwrap().area_description.clear();
        t.objectives[0].item_id = *item;
        t.objectives[0].item_count = 1;
        app.world_mut()
            .resource_mut::<QuestLog>()
            .insert_template(t);
        fields.extend([(158 + i as u16 * 5, *id), (159 + i as u16 * 5, 0)]);
        // AC quest_poi / quest_poi_points: item objective 4, Kalimdor, WMA 42/41.
        for n in [
            *id,
            1,
            0,
            4,
            1,
            if *id == 2518 { 41 } else { 42 },
            0,
            0,
            1,
            1,
            *x,
            *y,
        ] {
            body.extend(n.to_le_bytes());
        }
    }
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<SelfPlayer>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert(ObjectStore(ObjectFields::from_pairs(&fields)));
    let mut adapter = wrathplus_protocol::wrath_events::WrathEventAdapter::new();
    crate::ui_native::wrath_quest_pois::tests::dispatch(&mut app, &mut adapter, 0x1e4, &body);
    app.world_mut().resource_mut::<QuestPois>().fresh = false;
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    let mut chain = wrathplus_formats::open_chain(&data).unwrap();
    let areas = wrathplus_formats::load_area_table_catalog(&mut chain).unwrap();
    let maps =
        wrathplus_assets::MapCatalogRes(wrathplus_formats::load_map_catalog(&mut chain).unwrap());
    let (catalog, map_data) =
        crate::ui_world_map::build_catalog(&mut chain, &areas, &maps).unwrap();
    s.set_world_map_catalog(catalog);
    // Actual installed geographic projector, not an unconditional point on every map.
    s.set_world_loc_projector(Box::new(move |selection, map, x, y| {
        crate::ui_world_map::project_on_displayed(&map_data, selection, map, x, y)
    }));
    app.update();
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    s.run("for _,id in ipairs({963,3524,2518}) do for row=1,GetNumQuestLogEntries() do if select(9,GetQuestLogTitle(row)) == id then AddQuestWatch(row) end end end SetMapZoom(1)")
        .unwrap();
    // Rebuild before the cached POIs have been published, as at VM replacement.
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    app.insert_resource(bevy::prelude::State::new(
        crate::app_core::state::ClientState::InWorld,
    ));
    crate::ui_native::wrath_quest_pois::register(&mut app);
    app.update();
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    // No manual POI event or map toggle: the real feed must refresh stock's tracker.
    assert!(s
        .eval::<bool>("return CURRENT_MAP_QUESTS[963] ~= nil")
        .unwrap());
    s.run("ToggleFrame(WorldMapFrame)").unwrap();
    s.tick(0.05);
    s.resolve();
    assert_eq!(
        s.eval::<u32>("return QuestMapUpdateAllQuests()").unwrap(),
        3
    );

    for (i, (id, _, _, _)) in cases.iter().enumerate() {
        assert_eq!(
            s.eval::<i32>(&format!("return select(4, QuestPOIGetIconInfo({id}))"))
                .unwrap(),
            4
        );
        assert!(s.eval::<bool>(&format!("local b = _G['poiWatchFrameLines1_{}'] return b and b:IsShown() and b.questId == {id}", i+1)).unwrap());
    }
    // The numeric decoration is the stock map-opening hit target, not merely watch membership.
    s.run("WatchFrameQuestPOI_OnClick(poiWatchFrameLines1_1)")
        .unwrap();
    assert_eq!(
        s.eval::<u32>("return WORLDMAP_SETTINGS.selectedQuestId")
            .unwrap(),
        963
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}
