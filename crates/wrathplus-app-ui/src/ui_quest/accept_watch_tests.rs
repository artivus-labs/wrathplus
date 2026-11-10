//! Accept to tracker on the installed stock 3.3.5 `QuestLogFrame.xml`, which carries `WatchFrame`:
//! the slot watcher's line resolves, `QUEST_ACCEPTED` names the quest's log row, stock
//! `QuestLog_OnEvent` watches it (`QuestLogFrame.lua:260-264`), and the tracker draws it expanded
//! at build 12340's `trackerFilter` default. And the stock Accept on every captured detail.

use super::*;
use crate::net::ClientCommand;
use wrathplus_ui::script::{QuestLogEntryView, QuestLogObjectiveView, QuestLogState};

fn header(title: &str) -> QuestLogEntryView {
    QuestLogEntryView {
        title: title.into(),
        is_header: true,
        ..Default::default()
    }
}

fn quest(quest_id: u32, title: &str, objective: &str) -> QuestLogEntryView {
    QuestLogEntryView {
        quest_id,
        title: title.into(),
        level: 5,
        objectives: vec![QuestLogObjectiveView {
            text: objective.into(),
            kind: "monster".into(),
            finished: false,
            cur: 0,
            req: 10,
            poi_bit: None,
        }],
        ..Default::default()
    }
}

/// The text the tracker's lines draw.
fn tracker_lines(s: &mut UiScript) -> Vec<String> {
    s.resolve();
    s.extract()
        .iter()
        .filter(|q| {
            s.quad_owner_name(q.target)
                .is_some_and(|o| o.starts_with("WatchFrameLine"))
        })
        .filter_map(|q| match &q.content {
            wrathplus_ui::script::QuadContent::Text { text: Some(t), .. } => Some(t.clone()),
            _ => None,
        })
        .collect()
}

/// A mouse click at `button`'s centre, as a player's.
fn click(s: &mut UiScript, button: &str) {
    s.resolve();
    let (x, y) = s
        .eval::<(f32, f32)>(&format!(
            "return ({button}:GetLeft() + {button}:GetRight()) / 2, \
             ({button}:GetTop() + {button}:GetBottom()) / 2"
        ))
        .unwrap();
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
}

/// Two accepted quests are watched, the tracker shows them expanded with their objectives, and
/// its button collapses and expands it. Before the fix nothing fired `QUEST_ACCEPTED`, and the
/// shim's `trackerFilter` "0" hid both watches (no POI puts them on the current map): the stock
/// tracker counted "Objectives (2)", collapsed and disabled its button (`WatchFrame.lua:388-404`).
#[test]
fn accepted_quests_are_tracked_expanded_and_the_button_toggles_them() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<QuestLines>()
        .init_resource::<QuestLog>()
        .init_resource::<Items>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::sound::MessageSounds>()
        .add_systems(Update, feed_quest_lines);
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    crate::ui_script::test_ui::load_ui(&mut s, "Interface\\FrameXML\\QuestLogFrame.xml");
    // The login order: the saved CVars (`WatchFrame_OnEvent` reads `trackerFilter`), then the
    // world, and the log `feed_quest_log` pushes ahead of the lines.
    s.fire_event("VARIABLES_LOADED", vec![]);
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    s.set_quest_log(QuestLogState {
        entries: vec![
            header("Elwynn Forest"),
            quest(7, "Kobold Camp Cleanup", "Kobold Vermin slain: 0/10"),
            quest(783, "A Threat Within", "Defias Thug slain: 0/10"),
        ],
        num_quests: 2,
    });
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    app.insert_non_send_resource(s);
    {
        let world = app.world_mut();
        let mut log = world.resource_mut::<QuestLog>();
        log.insert_template(super::tests::template(7, "Kobold Camp Cleanup", 0));
        log.insert_template(super::tests::template(783, "A Threat Within", 0));
        let mut lines = world.resource_mut::<QuestLines>();
        lines.push(QuestLine::Accepted(783));
        lines.push(QuestLine::Accepted(7));
    }
    app.update();

    let mut s = app.world_mut().non_send_resource_mut::<UiScript>();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert_eq!(
        s.eval::<(i64, i64, i64)>(
            "return GetNumQuestWatches(), GetQuestIndexForWatch(1), GetQuestIndexForWatch(2)"
        )
        .unwrap(),
        (2, 3, 2),
        "both watched, by their log rows, in accept order"
    );
    let state = |s: &UiScript| {
        s.eval::<(bool, bool, bool)>(
            "return WatchFrame.collapsed == nil, WatchFrameLines:IsShown() == 1, \
             WatchFrameCollapseExpandButton:IsEnabled() == 1",
        )
        .unwrap()
    };
    assert_eq!(
        state(&s),
        (true, true, true),
        "expanded, lines up, button live"
    );
    assert_eq!(
        s.eval::<String>("return WatchFrameTitle:GetText()")
            .unwrap(),
        "Objectives (2)"
    );
    let lines = tracker_lines(&mut s);
    for want in [
        "A Threat Within",
        "Kobold Camp Cleanup",
        "Defias Thug slain",
        "Kobold Vermin slain",
    ] {
        assert!(
            lines.iter().any(|t| t.contains(want)),
            "the tracker draws {want:?}: {lines:?}"
        );
    }

    click(&mut s, "WatchFrameCollapseExpandButton");
    assert_eq!(state(&s), (false, false, true), "collapsed by the button");
    click(&mut s, "WatchFrameCollapseExpandButton");
    assert_eq!(state(&s), (true, true, true), "and expanded again");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Shannon's log (10-24): four watches, one complete, one an item objective. Every one draws its
/// title and lines at the default filter, and the complete one its own completion text, read by
/// row (`GetQuestLogCompletionText(questIndex)`, `WatchFrame.lua:827`), not the log selection's.
#[test]
fn four_watches_draw_four_quests_with_the_complete_ones_own_text() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    crate::ui_script::test_ui::load_ui(&mut s, "Interface\\FrameXML\\QuestLogFrame.xml");
    s.fire_event("VARIABLES_LOADED", vec![]);
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    let done = |text: &str| wrathplus_ui::script::QuestLogDetail {
        completion_text: Some(text.into()),
        ..Default::default()
    };
    let mut scorpid = quest(789, "Sting of the Scorpid", "Scorpid Worker Tail: 8/8");
    scorpid.complete = 1;
    scorpid.objectives[0].finished = true;
    scorpid.detail = Some(done("Return to Gornek in the Den."));
    let mut medallion = quest(
        784,
        "Burning Blade Medallion",
        "Burning Blade Medallion: 0/1",
    );
    medallion.objectives[0].kind = "item".into();
    let mut peons = quest(5441, "Lazy Peons", "Peons Awoken: 1/5");
    peons.detail = Some(done("Return to Foreman Thazz'ril."));
    s.set_quest_log(QuestLogState {
        entries: vec![
            header("Durotar"),
            scorpid,
            peons,
            medallion,
            quest(790, "Sarkoth", "Sarkoth slain: 0/1"),
        ],
        num_quests: 4,
    });
    s.run("SelectQuestLogEntry(3) for row = 2, 5 do AddQuestWatch(row) end WatchFrame_Update()")
        .unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert_eq!(
        s.eval::<String>("return WatchFrameTitle:GetText()")
            .unwrap(),
        "Objectives (4)"
    );
    let lines = tracker_lines(&mut s);
    for want in [
        "Sting of the Scorpid",
        "Return to Gornek in the Den.",
        "Lazy Peons",
        "1/5 Peons Awoken",
        "Burning Blade Medallion",
        "0/1 Burning Blade Medallion",
        "Sarkoth",
        "0/1 Sarkoth slain",
    ] {
        assert!(
            lines.iter().any(|t| t.contains(want)),
            "the tracker draws {want:?}: {lines:?}"
        );
    }
    assert!(
        !lines.iter().any(|t| t.contains("Foreman Thazz'ril")),
        "the selected quest's text stays on its own row: {lines:?}"
    );
}

/// Every quest detail in Shannon's session 152357 (Valley of Trials to Sen'jin Village): its id
/// and flags. Each detail's byte after the texts was 1, AzerothCore's `activateAccept`.
const CAPTURED_DETAILS: &[(u32, u32)] = &[
    (786, 0x8),      // Thwarting Kolkar Aggression: could not be accepted
    (789, 0x80008),  // Sting of the Scorpid
    (790, 0x80008),  // Sarkoth
    (794, 0x80008),  // Burning Blade Medallion
    (804, 0x80000),  // Sarkoth
    (805, 0x80008),  // Report to Sen'jin Village
    (808, 0x8),      // Minshina's Skull: could not be accepted
    (2161, 0x80000), // A Peon's Burden
    (3090, 0x80000), // Tainted Parchment
    (4402, 0x80008), // Galgar's Cactus Apple Surprise
    (5441, 0x80000), // Lazy Peons
    (6394, 0x80008), // Thazz'ril's Pick
];

/// The stock Accept button on each captured detail. 12340's `QuestGetAutoAccept` (`0x58c430`)
/// reads `QUEST_FLAGS_AUTO_ACCEPT`; the frame hides Decline for such a quest and its Accept only
/// hides the frame (`QuestFrame.lua:328-347`), as AzerothCore already added it. Before the fix
/// the answer came from the always-1 byte, so no quest's Accept sent `CMSG_QUESTGIVER_ACCEPT_QUEST`
/// and the two without the flag (786, 808) could never be taken.
#[test]
fn accept_sends_the_accept_unless_the_server_auto_accepted_the_quest() {
    use super::super::{drain_quest, snapshot, QuestGiver, QuestView};
    use wrathplus_protocol::types::{QuestDetails, WrathQuestInfo};
    let _data = wrathplus_formats::wow_data_or_skip!();
    const NPC: u64 = 0xF130_000C_4400_3B9B;
    for &(quest_id, flags) in CAPTURED_DETAILS {
        let auto = flags & 0x8_0000 != 0;
        let mut giver = QuestGiver::default();
        giver.open(
            NPC,
            QuestView::Detail(QuestDetails {
                wrath: Some(WrathQuestInfo {
                    flags,
                    auto_accept: true,
                    ..Default::default()
                }),
                npc: NPC,
                quest_id,
                title: format!("Quest {quest_id}"),
                details: "The Kolkar raid Sen'jin.".into(),
                objectives: String::new(),
                auto_finish: 0,
                choices: Vec::new(),
                rewards: Vec::new(),
                money: 0,
                reward_spell: 0,
            }),
        );
        let (tx, rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);
        let state = snapshot(
            &giver,
            &Items::default(),
            None,
            &commands,
            &crate::npc_text::MacroContext {
                subject: None,
                states: &crate::app_core::state::WorldStates::default(),
            },
            None,
        )
        .expect("a detail");
        assert_eq!(state.auto_accept, auto, "quest {quest_id}");

        let mut s = UiScript::new().unwrap();
        s.set_screen_size(1024.0, 768.0);
        crate::ui_script::test_ui::load_ui(&mut s, "Interface\\FrameXML\\QuestFrame.xml");
        s.set_quest(Some(state));
        s.fire_event("QUEST_DETAIL", vec![]);
        assert!(
            s.eval::<bool>("return QuestFrame:IsShown() == 1").unwrap(),
            "quest {quest_id}: the detail shows"
        );
        // The text fades in with Accept disabled (`QuestFrame.lua`, `questFadingDisable` off).
        for _ in 0..40 {
            s.tick(0.25);
        }
        assert!(
            s.eval::<bool>("return QuestFrameAcceptButton:IsEnabled() == 1")
                .unwrap(),
            "quest {quest_id}: Accept is live once the text is in"
        );
        s.run("QuestFrameAcceptButton:Click()").unwrap();
        assert!(s.errors().is_empty(), "quest {quest_id}: {:?}", s.errors());

        let mut app = App::new();
        app.insert_resource(commands)
            .init_resource::<crate::net::GuidIndex>()
            .insert_resource(giver)
            .insert_non_send_resource(s)
            .add_systems(Update, drain_quest);
        app.update();
        let sent: Vec<ClientCommand> = rx.try_iter().collect();
        if auto {
            assert!(
                sent.is_empty(),
                "quest {quest_id} was the server's: {sent:?}"
            );
        } else {
            assert!(
                matches!(
                    sent[..],
                    [ClientCommand::QuestgiverAccept { npc: NPC, quest }] if quest == quest_id
                ),
                "quest {quest_id}: {sent:?}"
            );
        }
        // The reference closes on the click (`0x501130(0,0)`): the drain clears the giver, and
        // `feed_quest` fires `QUEST_FINISHED` for it, which hides the frame.
        assert!(
            !app.world().resource::<QuestGiver>().is_open(),
            "quest {quest_id}"
        );
        let mut s = app.world_mut().non_send_resource_mut::<UiScript>();
        s.fire_event("QUEST_FINISHED", vec![]);
        assert!(
            !s.eval::<bool>("return QuestFrame:IsShown() == 1").unwrap(),
            "quest {quest_id}: Accept closes the frame"
        );
        assert!(s.errors().is_empty(), "quest {quest_id}: {:?}", s.errors());
    }
}
