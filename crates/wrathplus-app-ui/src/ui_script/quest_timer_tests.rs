//! The timed quests on 3.3.5's objective tracker (`WatchFrame_DisplayQuestTimers`,
//! `WatchFrame.lua:477-512`): a `QUEST_TIMERS` header line, then one ` - SecondsToTime` line per
//! timed quest, none at zero timers, each line mapped back to its quest. 3.3.5 has no
//! `QuestTimerFrame`.

use wrathplus_ui::script::{
    QuestLogEntryView, QuestLogObjectiveView, QuestLogState, ScriptValue, UiScript,
};

use super::test_ui::load_ui as load_xml;

/// The window's own dependencies only, so a missing one shows.
fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    // The title's `text="QUEST_TIMERS"` resolves at load; an unknown name draws as the key.
    load_xml(&mut s, "Interface\\FrameXML\\GlobalStrings.lua");
    load_xml(&mut s, "Interface\\FrameXML\\Fonts.xml");
    load_xml(&mut s, r"Interface\FrameXML\BasicControls.xml");
    // `SecondsToTime` for every row, `UIParent_ManageFramePositions` for OnShow and OnHide.
    load_xml(&mut s, r"Interface\FrameXML\UIParent.xml");
    // QuestLogFrame.xml brings `MAX_QUESTS` (`QuestLogFrame.lua:2`), the repaint's loop bound.
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.lua");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.xml");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.lua");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.xml");
    load_xml(&mut s, r"Interface\FrameXML\StaticPopup.xml");
    load_xml(&mut s, r"Interface\FrameXML\MainMenuBarMicroButtons.xml");
    load_xml(&mut s, "Interface\\FrameXML\\ItemButtonTemplate.xml");
    load_xml(&mut s, "Interface\\FrameXML\\QuestFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\QuestLogFrame.xml");
    load_xml(&mut s, "Interface\\FrameXML\\WatchFrame.xml");
    s
}

/// A quest-log row; `timer` is the absolute deadline, 0 for untimed.
fn quest(id: u32, title: &str, timer: u32) -> QuestLogEntryView {
    QuestLogEntryView {
        quest_id: id,
        title: title.into(),
        level: 10,
        timer,
        objectives: vec![QuestLogObjectiveView {
            text: "Bloodscalp Scout slain: 0/8".into(),
            kind: "monster".into(),
            finished: false,
            cur: 0,
            req: 8,
            poi_bit: None,
        }],
        ..Default::default()
    }
}

fn log(entries: Vec<QuestLogEntryView>) -> QuestLogState {
    QuestLogState {
        num_quests: entries.len() as u32,
        entries,
    }
}

/// The shown timer lines' texts, header first.
fn timer_lines(s: &UiScript) -> Vec<String> {
    s.eval(
        "local t = {} for _, l in ipairs(WATCHFRAME_TIMERLINES) do \
         if l:IsShown() then t[#t + 1] = l.text:GetText() end end return t",
    )
    .unwrap()
}

const HEADER: &str = "|cffffd200Quest Timers";

#[test]
fn the_window_follows_the_timed_quests() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();

    s.set_quest_log(log(vec![quest(783, "A Threat Within", 0)]));
    s.set_server_unix_time(1_000_000.0);
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.tick(0.1);
    assert!(timer_lines(&s).is_empty(), "no timed quest, no timer lines");

    // Two timed quests around an untimed one, 860 s and 45 s out.
    s.set_quest_log(log(vec![
        quest(1, "Deliver the Message", 1_000_860),
        quest(2, "A Threat Within", 0),
        quest(3, "Escort the Caravan", 1_000_045),
    ]));
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.tick(0.1);
    // The reference's timers read 1 s short, formatted by `SecondsToTime`.
    assert_eq!(timer_lines(&s), [HEADER, " - 14 Min 19 Sec", " - 44 Sec"]);
    // Line 2 maps to the third log entry, past the untimed one.
    assert_eq!(s.eval::<i64>("return GetQuestIndexForTimer(2)").unwrap(), 3);

    // The clock alone moves; the tracker's update function repaints with no event.
    s.set_server_unix_time(1_000_030.0);
    s.tick(1.0);
    assert_eq!(timer_lines(&s)[2], " - 14 Sec");

    s.set_quest_log(log(vec![quest(2, "A Threat Within", 0)]));
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.tick(0.1);
    assert!(timer_lines(&s).is_empty());
}

#[test]
fn the_countdown_reaches_the_draw_list() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    s.set_quest_log(log(vec![quest(1, "Deliver the Message", 1_000_860)]));
    s.set_server_unix_time(1_000_000.0);
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    s.tick(0.1);

    let quads = s.extract();
    let text: Vec<String> = quads
        .iter()
        .filter_map(|q| match &q.content {
            wrathplus_ui::script::QuadContent::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        text.iter().any(|t| t == " - 14 Min 19 Sec"),
        "the countdown never reached the draw list: {text:?}"
    );
    assert!(
        text.iter().any(|t| t == HEADER),
        "the QUEST_TIMERS header never reached the draw list: {text:?}"
    );
}

/// The tracker also lays out on `PLAYER_ENTERING_WORLD`.
#[test]
fn a_world_entry_paints_a_quest_already_running() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    s.set_quest_log(log(vec![quest(1, "Deliver the Message", 1_000_090)]));
    s.set_server_unix_time(1_000_000.0);
    s.fire_event("PLAYER_ENTERING_WORLD", vec![ScriptValue::Bool(true)]);
    s.tick(0.1);
    // One minute takes the singular, not its plural; 90 s reads 89.
    assert_eq!(timer_lines(&s), [HEADER, " - 1 Min 29 Sec"]);
}
