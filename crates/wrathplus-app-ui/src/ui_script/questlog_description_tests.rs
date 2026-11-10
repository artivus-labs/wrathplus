//! The quest log's description (playtest, quest 806): a template that lands after the log row
//! fills `GetQuestLogQuestText()`'s first return, and the stock detail pane shows it. Stock
//! `QuestInfo_ShowDescriptionText` sets the text, then `SetAlphaGradient(0, 0)`
//! (`QuestInfo.lua:106-107`); 12340 fades nothing at length 0 (`0x6c78f0`), which the text
//! layout's gradient test covers, so here the pane carries the text and that pair. The
//! completion text a zero-objective quest's tracker line reads falls back to the objectives.

use wrathplus_protocol::types::{QuestObjective, QuestTemplate, WrathQuestInfo};
use wrathplus_ui::script::{QuadContent, QuestLogEntryView, QuestLogState, UiScript};

use super::test_ui::load_ui;

const DETAILS: &str = "We cannot allow the Burning Blade a foothold in Durotar, $N! Go, $c.";
const SHOWN: &str =
    "We cannot allow the Burning Blade a foothold in Durotar, Locktest! Go, warlock.";
const OBJECTIVES: &str = "Bring Fizzle's Claw to Orgnil Soulscar in Razor Hill.";

/// Quest 806's text, as `SMSG_QUEST_QUERY_RESPONSE` carries it: macros unexpanded.
fn template() -> QuestTemplate {
    QuestTemplate {
        wrath: Some(WrathQuestInfo::default()),
        quest_id: 806,
        method: 2,
        level: 12,
        zone_or_sort: 14,
        quest_type: 0,
        rep_objective_faction: 0,
        rep_objective_value: 0,
        next_quest_in_chain: 0,
        money: 0,
        money_max_level: 0,
        reward_spell: 0,
        src_item_id: 0,
        flags: 0,
        rewards: [(0, 0); 4],
        choices: [(0, 0); 6],
        point_map_id: 0,
        point_x: 0.0,
        point_y: 0.0,
        point_opt: 0,
        title: "Dark Storms".into(),
        objectives_text: OBJECTIVES.into(),
        details: DETAILS.into(),
        end_text: String::new(),
        objectives: std::array::from_fn(|_| QuestObjective {
            creature_or_go: 0,
            required_count: 0,
            item_id: 0,
            item_count: 0,
            text: String::new(),
        }),
    }
}

/// The log row the template builds, as `ui_quest_log`'s publish does for a cached quest.
fn landed_log() -> QuestLogState {
    let subject = crate::npc_text::Subject {
        name: "Locktest".into(),
        race: 2,
        class: 9,
        gender: 1,
    };
    let states = crate::app_core::state::WorldStates::default();
    let macros = crate::npc_text::MacroContext {
        subject: Some(&subject),
        states: &states,
    };
    let (tx, _rx) = crossbeam_channel::unbounded();
    let commands = crate::net::NetCommands(tx);
    let items = crate::items::Items::default();
    let detail =
        crate::ui_quest_log::build_detail(&template(), &items, None, &commands, &macros, None);
    QuestLogState {
        num_quests: 1,
        entries: vec![QuestLogEntryView {
            quest_id: 806,
            title: "Dark Storms".into(),
            level: 12,
            detail: Some(detail),
            ..Default::default()
        }],
    }
}

/// The description's drawn text quad and its `SetAlphaGradient` pair.
fn drawn_description(s: &mut UiScript) -> Option<(String, Option<(f32, f32)>)> {
    s.resolve();
    s.extract().iter().find_map(|q| match &q.content {
        QuadContent::Text {
            text: Some(t),
            alpha_gradient,
            ..
        } if t == SHOWN => Some((t.clone(), *alpha_gradient)),
        _ => None,
    })
}

#[test]
fn a_template_landing_after_the_row_fills_the_log_and_map_descriptions() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_ui(&mut s, "Interface\\FrameXML\\QuestLogFrame.xml");
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    // The row's template is in flight: the log lists nothing yet (`0x4de510`).
    s.set_quest_log(QuestLogState::default());
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.run("ToggleFrame(QuestLogFrame)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumQuestLogEntries()").unwrap(), 0);

    // The template lands: the publish lists the quest and announces it.
    s.set_quest_log(landed_log());
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.run("QuestLog_SetSelection(1)").unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    let (desc, obj) = s
        .eval::<(String, String)>("return GetQuestLogQuestText()")
        .unwrap();
    assert_eq!((desc.as_str(), obj.as_str()), (SHOWN, OBJECTIVES));
    // No end text: `0x5e06d0` answers the objectives text, the WatchFrame line a zero-objective
    // quest shows (`WatchFrame.lua:834`, by row), not a bare dash.
    for call in [
        "GetQuestLogCompletionText()",
        "GetQuestLogCompletionText(1)",
    ] {
        assert_eq!(
            s.eval::<String>(&format!("return {call}")).unwrap(),
            OBJECTIVES,
            "{call}"
        );
    }
    let log_pane = drawn_description(&mut s);
    assert_eq!(
        log_pane,
        Some((SHOWN.to_string(), Some((0.0, 0.0)))),
        "the log pane"
    );

    // The enlarged map's detail pane (`WorldMapFrame.lua:1674-1675`) reparents the same text.
    s.run("SelectQuestLogEntry(1); QuestInfo_Display(QUEST_TEMPLATE_MAP1, WorldMapQuestDetailScrollChildFrame)")
        .unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert_eq!(
        s.eval::<String>("return QuestInfoDescriptionText:GetParent():GetName()")
            .unwrap(),
        "WorldMapQuestDetailScrollChildFrame"
    );
    assert_eq!(
        s.eval::<String>("return QuestInfoDescriptionText:GetText()")
            .unwrap(),
        SHOWN,
        "the map pane"
    );
}

/// `0x5e06d0`'s choice: the end text when it has one, else the objectives text, and the
/// objectives text whenever the template sets `QUEST_FLAGS_OBJ_TEXT` (0x40000).
#[test]
fn the_completion_text_falls_back_to_the_objectives() {
    let states = crate::app_core::state::WorldStates::default();
    let ctx = crate::npc_text::MacroContext {
        subject: None,
        states: &states,
    };
    let mut t = template();
    assert_eq!(crate::npc_text::completion_text(&t, &ctx), OBJECTIVES);
    t.end_text = "Return to Orgnil Soulscar in Razor Hill.".into();
    assert_eq!(
        crate::npc_text::completion_text(&t, &ctx),
        "Return to Orgnil Soulscar in Razor Hill."
    );
    t.flags |= 0x0004_0000;
    assert_eq!(crate::npc_text::completion_text(&t, &ctx), OBJECTIVES);
}

/// Quest 62, "The Fargodeep Mine", as AzerothCore sends it: no creature or item objective, an
/// area description, exploration flags (`Flags` 12).
fn fargodeep() -> QuestTemplate {
    let mut t = template();
    t.quest_id = 62;
    t.flags = 12;
    t.title = "The Fargodeep Mine".into();
    t.objectives_text =
        "Explore the Fargodeep Mine, then return to Marshal Dughan in Goldshire.".into();
    t.details = String::new();
    t.end_text = "Return to Marshal Dughan at Goldshire in Elwynn Forest.".into();
    t.wrath = Some(WrathQuestInfo {
        flags: 12,
        area_description: "Scout through the Fargodeep Mine".into(),
        ..Default::default()
    });
    t
}

/// Quest 62's objective lines with the log slot's state word `state`.
fn fargodeep_lines(state: u8) -> Vec<wrathplus_ui::script::QuestLogObjectiveView> {
    let slot = wrathplus_protocol::types::QuestLogSlot {
        quest_id: 62,
        counters: [0; 4],
        state,
        timer: 0,
    };
    let store = wrathplus_protocol::ObjectFields::default();
    let mut objs = crate::items::TestObjects::new();
    let (tx, _rx) = crossbeam_channel::unbounded();
    crate::ui_quest_log::build_objectives(
        &fargodeep(),
        &slot,
        &store,
        &objs.get(),
        &crate::items::Items::default(),
        &crate::names::NameCache::default(),
        &crate::net::NetCommands(tx),
        &|_| None,
    )
}

/// An exploration quest's one leaderboard row is its area description, type `event`
/// (`0x5e1460`), unfinished until the server sets the slot's COMPLETE bit on the trigger.
#[test]
fn an_exploration_quest_lists_its_area_description_as_an_event_line() {
    let open = fargodeep_lines(0);
    assert_eq!(open.len(), 1, "{open:#?}");
    assert_eq!(
        (
            open[0].text.as_str(),
            open[0].kind.as_str(),
            open[0].finished
        ),
        ("Scout through the Fargodeep Mine", "event", false)
    );
    let done = fargodeep_lines(wrathplus_protocol::types::quest_slot_state::COMPLETE);
    assert!(done[0].finished, "{done:#?}");
}

/// The stock API over quest 62: one leaderboard row (so WatchFrame does not count it complete,
/// `WatchFrame.lua:808`), `(text, "event", nil)` while open, finished once explored, and the end
/// text as the completion line then.
#[test]
fn the_stock_log_reads_the_event_line_until_the_area_is_explored() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_ui(&mut s, "Interface\\FrameXML\\QuestLogFrame.xml");
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    let log = |state: u8, complete| QuestLogState {
        num_quests: 1,
        entries: vec![QuestLogEntryView {
            quest_id: 62,
            title: "The Fargodeep Mine".into(),
            level: 7,
            complete,
            objectives: fargodeep_lines(state),
            ..Default::default()
        }],
    };
    s.set_quest_log(log(0, 0));
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert_eq!(
        s.eval::<i64>("return GetNumQuestLeaderBoards(1)").unwrap(),
        1
    );
    let (text, kind, finished) = s
        .eval::<(String, String, Option<i64>)>("return GetQuestLogLeaderBoard(1, 1)")
        .unwrap();
    assert_eq!(
        (text.as_str(), kind.as_str(), finished),
        ("Scout through the Fargodeep Mine", "event", None)
    );

    s.set_quest_log(log(
        wrathplus_protocol::types::quest_slot_state::COMPLETE,
        1,
    ));
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    let finished = s
        .eval::<Option<i64>>("return select(3, GetQuestLogLeaderBoard(1, 1))")
        .unwrap();
    assert_eq!(finished, Some(1), "explored");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
