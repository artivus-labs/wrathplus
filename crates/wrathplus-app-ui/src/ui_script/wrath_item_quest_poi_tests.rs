//! An item-collection quest's POI through the real descriptor and bag -> log feed and raw POI
//! reply -> VM: AzerothCore 9305 "Spare Parts" (four Emitter Spare Parts, `quest_poi` objective 4
//! at the lake and -1 at the turn-in, map 530 / WMA 464). 12340 judges it done off the bags
//! (`0x5e0ea0`), so the slot's COMPLETE bit is never set here.
use bevy::prelude::*;
use wrathplus_protocol::types::{Field, QuestObjective, QuestTemplate, WrathQuestInfo};
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::script::UiScript;

use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::net::{Guid, GuidIndex, NetCommands};
use crate::ui_native::wrath_quest_pois::{QuestPois, QuestPoisView};
use crate::ui_quest_log::{feed_quest_log, QuestLog};

const QUEST: u32 = 9_305;
const PARTS: u32 = 22_978;
const PARTS_GUID: u64 = 0xA1;

fn template() -> QuestTemplate {
    let mut required_items = [(0, 0); 6];
    required_items[0] = (PARTS, 4);
    QuestTemplate {
        quest_id: QUEST,
        method: 2,
        level: 4,
        zone_or_sort: 3_526,
        quest_type: 0,
        flags: 0x80088,
        title: "Spare Parts".into(),
        objectives_text: "Collect 4 Emitter Spare Parts.".into(),
        end_text: String::new(),
        wrath: Some(WrathQuestInfo {
            flags: 0x80088,
            required_items,
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
        point_map_id: 530,
        point_x: -4456.14,
        point_y: -13923.0,
        point_opt: 0,
        details: String::new(),
        objectives: std::array::from_fn(|i| QuestObjective {
            creature_or_go: 0,
            required_count: 0,
            item_id: if i == 0 { PARTS } else { 0 },
            item_count: if i == 0 { 4 } else { 0 },
            text: String::new(),
        }),
    }
}

/// The parts' stack in backpack slot 1.
fn hold(app: &mut App, parts: u32) {
    let fields = ObjectFields::from_pairs(&[
        (Field::ObjectEntry.index(), PARTS),
        (Field::ItemStackCount.index(), parts),
    ]);
    let held = app
        .world()
        .resource::<GuidIndex>()
        .0
        .get(&PARTS_GUID)
        .copied();
    match held {
        Some(e) => {
            app.world_mut().entity_mut(e).insert(ObjectStore(fields));
        }
        None => {
            crate::items::test_spawn_item(app.world_mut(), PARTS_GUID, fields, false);
        }
    }
    app.update();
}

fn app(script: UiScript) -> App {
    let mut app = App::new();
    let (tx, _) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx))
        .init_resource::<GuidIndex>()
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
    app.world_mut()
        .resource_mut::<QuestLog>()
        .insert_template(template());
    // Quest log slot 0 holds the quest, state 0: the server never sets COMPLETE here.
    let pack = Field::PlayerPackSlot1.index();
    app.world_mut().spawn((
        ObjectStore(ObjectFields::from_pairs(&[
            (158, QUEST),
            (159, 0),
            (pack, PARTS_GUID as u32),
            (pack + 1, 0),
        ])),
        Guid(42),
        SelfPlayer,
    ));
    app.add_systems(Update, feed_quest_log);
    // AzerothCore quest_poi / quest_poi_points for 9305.
    let lake: &[(i32, i32)] = &[
        (-4482, -14122),
        (-4461, -14114),
        (-4358, -13833),
        (-4362, -13826),
        (-4424, -13799),
        (-4437, -13797),
        (-4669, -13970),
    ];
    let mut body = 1u32.to_le_bytes().to_vec();
    for n in [QUEST, 2] {
        body.extend(n.to_le_bytes());
    }
    for (id, objective, points) in [(0u32, 4i32, lake), (1, -1, &[(-4185, -13733)][..])] {
        body.extend(id.to_le_bytes());
        body.extend(objective.to_le_bytes());
        for n in [530u32, 464, 0, 0, 1, points.len() as u32] {
            body.extend(n.to_le_bytes());
        }
        for (x, y) in points {
            body.extend(x.to_le_bytes());
            body.extend(y.to_le_bytes());
        }
    }
    let mut adapter = wrathplus_protocol::wrath_events::WrathEventAdapter::new();
    crate::ui_native::wrath_quest_pois::tests::dispatch(&mut app, &mut adapter, 0x1e4, &body);
    let pois = app.world().resource::<QuestPois>().view(&[QUEST]);
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_wrath_quest_pois(pois);
    hold(&mut app, 3);
    app
}

fn ask(app: &App, code: &str) -> String {
    app.world()
        .non_send_resource::<UiScript>()
        .eval::<String>(code)
        .unwrap()
}

const ICON: &str = "local done, _, _, objective = QuestPOIGetIconInfo(9305) \
     return tostring(done)..' '..tostring(objective)..' '..tostring(select(7, GetQuestLogTitle(2)))";

#[test]
fn the_fourth_part_moves_the_icon_to_the_turn_in_without_the_servers_complete_bit() {
    let mut app = app(UiScript::new().unwrap());
    // A projector that keeps the blobs apart: the lake's centroid and the NPC land elsewhere.
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_world_loc_projector(Box::new(|_, map, x, y| {
            (map == 530).then_some((-x / 10_000.0, -y / 20_000.0))
        }));
    assert_eq!(ask(&app, ICON), "false 4 nil");
    hold(&mut app, 4);
    assert_eq!(ask(&app, ICON), "true -1 1");
    let at_npc = "local _, x, y = QuestPOIGetIconInfo(9305) \
         return tostring(math.abs(x - 0.4185) < 1e-4 and math.abs(y - 0.68665) < 1e-4)";
    assert_eq!(ask(&app, at_npc), "true");
    // A part dropped reopens the objective, as the bags rule both ways.
    hold(&mut app, 3);
    assert_eq!(ask(&app, ICON), "false 4 nil");
}

#[test]
fn stock_world_map_shows_the_turn_in_swap_once_the_bags_hold_the_parts() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (script, failures) = super::super::super::stock_session(&data);
    assert!(failures.is_empty(), "{failures:?}");
    script.clear_instruction_budget();
    let mut app = app(script);
    let mut chain = wrathplus_formats::open_chain(&data).unwrap();
    let areas = wrathplus_formats::load_area_table_catalog(&mut chain).unwrap();
    let maps =
        wrathplus_assets::MapCatalogRes(wrathplus_formats::load_map_catalog(&mut chain).unwrap());
    let (catalog, map_data) =
        crate::ui_world_map::build_catalog(&mut chain, &areas, &maps).unwrap();
    let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
    s.set_world_map_catalog(catalog);
    s.set_world_loc_projector(Box::new(move |selection, map, x, y| {
        crate::ui_world_map::project_on_displayed(&map_data, selection, map, x, y)
    }));
    s.run("ShowUIPanel(WorldMapFrame) SetMapByID(464)").unwrap();
    let swap = |app: &mut App| {
        let s = &mut app.world_mut().non_send_resource_mut::<UiScript>();
        s.run("WorldMapFrame_UpdateQuests()").unwrap();
        s.tick(0.05);
        s.resolve();
        s.eval::<String>(
            "local f = WorldMapQuestFrame1 \
             return f.questId..' '..tostring(f.completed)..' '..f.poiIcon.type..' '..\
             select(4, QuestPOIGetIconInfo(9305))",
        )
        .unwrap()
    };
    let numeric = ask(&app, "return QUEST_POI_NUMERIC");
    assert_eq!(swap(&mut app), format!("9305 nil {numeric} 4"));
    hold(&mut app, 4);
    let complete = ask(&app, "return QUEST_POI_COMPLETE_SWAP");
    // `completed` is GetQuestLogTitle's isComplete, which 12340 returns as the number 1.
    assert_eq!(swap(&mut app), format!("9305 1 {complete} -1"));
    let s = app.world().non_send_resource::<UiScript>();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}
