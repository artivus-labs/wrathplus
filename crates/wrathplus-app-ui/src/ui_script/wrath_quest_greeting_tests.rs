//! The questgiver greeting's active-quest icons, against the player's own quest log.

use wrathplus_ui::script::{
    QuestGreetingInfo, QuestLogEntryView, QuestLogObjectiveView, QuestLogState, QuestPanel,
    QuestState,
};

/// An AC greeting (`SMSG_QUESTGIVER_QUEST_LIST`) sends icon 4 for every active quest, complete or
/// not (`PlayerQuest.cpp:75-78`); 12340's `GetActiveTitle` reads completion off the player's log
/// (`0x58bcb0`), so the complete quest wears `ActiveQuestIcon` and the unfinished one
/// `IncompleteQuestIcon`. A quest far below a level-10 player is greyed (`IsActiveQuestTrivial`,
/// `0x58aba0`: level 1 + 5 < 10), the others are not.
#[test]
fn the_greeting_marks_a_complete_quest_yellow_and_an_unfinished_one_grey() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _failures) = super::super::stock_session(&data);
    s.clear_instruction_budget();
    s.set_unit("player", Some(super::super::stock_player(10)));
    let entry = |id: u32, finished: bool, complete: i32| QuestLogEntryView {
        quest_id: id,
        title: format!("Quest {id}"),
        level: 7,
        complete,
        objectives: vec![QuestLogObjectiveView {
            text: "Thing: 0/1".into(),
            kind: "item".into(),
            finished,
            poi_bit: Some(4),
            ..Default::default()
        }],
        ..Default::default()
    };
    s.set_quest_log(QuestLogState {
        entries: vec![
            entry(808, true, 1),
            entry(826, false, 0),
            entry(5, false, 0),
        ],
        num_quests: 3,
    });
    let row = |quest_id: u32, level: i32| QuestGreetingInfo {
        quest_id,
        level,
        ..Default::default()
    };
    s.set_quest(Some(QuestState {
        panel: QuestPanel::Greeting,
        greeting: "Hello".into(),
        active_titles: vec!["Quest 808".into(), "Quest 826".into(), "Quest 5".into()],
        active_info: vec![row(808, 7), row(826, 7), row(5, 1)],
        ..Default::default()
    }));
    s.fire_event("QUEST_GREETING", vec![]);
    s.tick(0.05);
    s.resolve();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let icon = |i: u32| -> String {
        s.eval(&format!(
            "local t = QuestTitleButton{i}QuestIcon local r = t:GetVertexColor() \
             return (t:GetTexture() or 'nil'):match('[^\\\\]+$')..' '..string.format('%.2f', r)"
        ))
        .unwrap()
    };
    assert_eq!(icon(1), "ActiveQuestIcon 1.00", "complete");
    assert_eq!(icon(2), "IncompleteQuestIcon 1.00", "unfinished");
    assert_eq!(
        icon(3),
        "IncompleteQuestIcon 0.75",
        "unfinished and trivial"
    );
}
