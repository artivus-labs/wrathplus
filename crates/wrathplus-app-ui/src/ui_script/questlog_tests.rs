//! The quest log window, engine-only: the stock 3.3.5 `QuestLogFrame.xml` in a seated session,
//! fed a synthetic quest log. The list is a `HybridScrollFrame` of `QuestLogTitleButtonTemplate`
//! rows (`QuestLogFrame.lua:227-231`), the detail is `QuestInfo_Display`'s (`QuestInfo.lua:34`),
//! and the tracker is `WatchFrame`.

use wrathplus_ui::script::{
    ExtractedQuad, PartyMemberInfo, PartyState, QuadContent, QuestItemView, QuestLogDetail,
    QuestLogEntryView, QuestLogObjectiveView, QuestLogState, ScriptValue, SoundRequest, UiScript,
};

use super::test_ui::load_ui as load_xml;

/// The session seated through a stock FrameXML entry, logged in, holding `state`.
fn seat(s: &mut UiScript, state: QuestLogState) {
    s.set_screen_size(1024.0, 768.0);
    load_xml(s, "Interface\\FrameXML\\QuestLogFrame.xml");
    // `InterfaceOptionsFrame` writes the uvars (`AUTO_QUEST_WATCH`) at login.
    s.fire_event("PLAYER_ENTERING_WORLD", vec![]);
    s.set_quest_log(state);
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert!(s.errors().is_empty(), "seat errors: {:?}", s.errors());
}

/// [`seat`], then the `L` binding's `ToggleFrame(QuestLogFrame)` (Bindings.xml `TOGGLEQUESTLOG`).
fn open_log(s: &mut UiScript, state: QuestLogState) {
    seat(s, state);
    s.run("ToggleFrame(QuestLogFrame)").unwrap();
    assert!(s.errors().is_empty(), "open errors: {:?}", s.errors());
}

/// List row `i`, `HybridScrollFrame_CreateButtons`' `$parentButton<i>` (HybridScrollFrame.lua:176).
fn row(i: u32) -> String {
    format!("QuestLogScrollFrameButton{i}")
}

fn text(s: &UiScript, global: &str) -> Option<String> {
    s.eval::<Option<String>>(&format!("return {global}:GetText()"))
        .unwrap()
}

fn shown(s: &UiScript, global: &str) -> bool {
    s.eval::<bool>(&format!("return {global}:IsShown() == 1"))
        .unwrap()
}

fn watched(s: &UiScript, index: u32) -> bool {
    s.eval::<bool>(&format!("return IsQuestWatched({index}) == 1"))
        .unwrap()
}

/// The `QuadContent::Frame` rect of the frame named `name`.
fn frame_rect(s: &UiScript, quads: &[ExtractedQuad], name: &str) -> wrathplus_ui::layout::Rect {
    quads
        .iter()
        .find_map(|q| match &q.content {
            QuadContent::Frame if s.quad_owner_name(q.target).as_deref() == Some(name) => q.rect,
            _ => None,
        })
        .unwrap_or_else(|| panic!("no frame quad for {name}"))
}

/// A shift-click by mouse at `button`'s centre: `CHATLINK`'s chord is `SHIFT-BUTTON1`, which reads
/// the pressed button, so a scripted `:Click()` would not match it.
fn shift_click(s: &mut UiScript, button: &str) {
    modified_click(s, button, (true, false, false));
}

/// A left click by mouse at `button`'s centre with `(shift, ctrl, alt)` held.
fn modified_click(s: &mut UiScript, button: &str, held: (bool, bool, bool)) {
    s.resolve();
    let (x, y) = s
        .eval::<(f32, f32)>(&format!(
            "return ({button}:GetLeft() + {button}:GetRight()) / 2, \
             ({button}:GetTop() + {button}:GetBottom()) / 2"
        ))
        .unwrap();
    s.set_modifiers(held.0, held.1, held.2);
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
    s.set_modifiers(false, false, false);
}

thread_local! {
    /// The detail every fixture row carries, so any selection reads it.
    static DETAIL_FIXTURE: QuestLogDetail = QuestLogDetail {
            description: "Speak with Marshal McBride.".into(),
            objectives_text: "Report to Marshal McBride.".into(),
            required_money: 0,
            reward_money: 40,
            choices: vec![],
            rewards: vec![QuestItemView {
                item_id: 2024,
                name: Some("Militia Hammer".into()),
                texture: Some("Interface\\Icons\\INV_Hammer_15".into()),
                count: 1,
                quality: 1,
                usable: true,
                // What `GetQuestLogItemLink` returns for a ctrl or shift click.
                link: Some(HAMMER_LINK.into()),
            }],
            reward_spell: None,
            reward_xp: 0,
            reward_honor: 0,
            reward_talents: 0,
            reward_arena_points: 0,
            reward_title: None,
            completion_text: None,
        };
}

/// `n` quests; only quest 1 has an objective, and each has its own `quest_id`, the watch set's key.
fn entries(n: u32) -> QuestLogState {
    let entries = (1..=n)
        .map(|i| QuestLogEntryView {
            quest_id: i,
            title: format!("Quest {i}"),
            level: 5,
            complete: 0,
            objectives: if i == 1 {
                vec![QuestLogObjectiveView {
                    text: "Kobold Vermin slain: 3/10".into(),
                    kind: "monster".into(),
                    finished: false,
                    cur: 3,
                    req: 10,
                    poi_bit: None,
                }]
            } else {
                vec![]
            },
            detail: Some(DETAIL_FIXTURE.with(Clone::clone)),
            ..Default::default()
        })
        .collect();
    QuestLogState {
        num_quests: n,
        entries,
    }
}

fn eight_entries() -> QuestLogState {
    entries(8)
}

/// The fixture reward's escaped link, a white (quality 1) item, as the fixture serves it.
const HAMMER_LINK: &str = "|cffffffff|Hitem:2024:0:0:0|h[Militia Hammer]|h|r";

#[test]
fn shipped_questlog_frame_loads_clean() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    seat(&mut s, eight_entries());
    assert_eq!(
        s.eval::<(f32, f32)>("return QuestLogFrame:GetWidth(), QuestLogFrame:GetHeight()")
            .unwrap(),
        (682.0, 447.0),
        "the dual-pane window (QuestLogFrame.xml:538-541)"
    );
    assert!(
        s.eval::<i64>("return #QuestLogScrollFrame.buttons")
            .unwrap()
            > 0,
        "the hybrid list built its rows"
    );
}

/// Open, read the list and the detail, abandon through the confirm, close.
#[test]
fn shipped_questlog_frame_drives_end_to_end() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    seat(&mut s, eight_entries());
    let _ = s.take_sounds();
    assert!(!s
        .eval::<bool>("return QuestLogFrame:IsVisible() == 1")
        .unwrap());

    s.run("ToggleFrame(QuestLogFrame)").unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(s
        .eval::<bool>("return QuestLogFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.take_sounds(),
        vec![SoundRequest::KitName("igQuestLogOpen".into())],
        "QuestLog_OnShow plays igQuestLogOpen (QuestLogFrame.lua:289)"
    );

    // A quest row is the two-space-indented title (QuestLogFrame.lua:436); with no selection the
    // first quest selects itself (`QuestLog_SetFirstValidSelection`, :348-355).
    assert_eq!(text(&s, &row(1)).as_deref(), Some("  Quest 1"));
    assert_eq!(
        text(&s, "QuestLogQuestCount").as_deref(),
        Some("Quests: |cffffffff8/25|r"),
        "QUEST_LOG_COUNT_TEMPLATE over MAX_QUESTLOG_QUESTS (QuestLogFrame.lua:4, :525)"
    );
    assert_eq!(s.eval::<i64>("return GetQuestLogSelection()").unwrap(), 1);

    // The detail is `QUEST_TEMPLATE_LOG` (QuestInfo.lua:526-538).
    assert_eq!(text(&s, "QuestInfoTitleHeader").as_deref(), Some("Quest 1"));
    assert_eq!(
        text(&s, "QuestInfoObjective1").as_deref(),
        Some("Kobold Vermin slain: 3/10")
    );
    assert_eq!(
        text(&s, "QuestInfoDescriptionHeader").as_deref(),
        Some("Description")
    );
    assert_eq!(
        text(&s, "QuestInfoItem1Name").as_deref(),
        Some("Militia Hammer")
    );
    assert_eq!(
        text(&s, "QuestInfoRewardsHeader").as_deref(),
        Some("Rewards")
    );
    // No choices: REWARD_ITEMS_ONLY, and the choose text hidden (QuestInfo.lua:316-320, :376).
    assert_eq!(
        text(&s, "QuestInfoItemReceiveText").as_deref(),
        Some("You will receive:")
    );
    assert!(!shown(&s, "QuestInfoItemChooseText"));
    assert!(!shown(&s, "QuestInfoItem2"), "one reward, one item button");

    // Abandon, then No: the ABANDON_QUEST popup closes and queues nothing.
    s.run("QuestLogFrameAbandonButton:Click()").unwrap();
    assert!(s
        .eval::<bool>("return StaticPopup1:IsShown() == 1")
        .unwrap());
    assert_eq!(
        text(&s, "StaticPopup1Text").as_deref(),
        Some("Abandon \"Quest 1\"?"),
        "ABANDON_QUEST_CONFIRM over GetAbandonQuestName (QuestLogFrame.xml:145-155)"
    );
    s.run("StaticPopup_OnClick(StaticPopup1, 2)").unwrap();
    assert!(!s
        .eval::<bool>("return StaticPopup1:IsShown() == 1")
        .unwrap());
    assert!(
        s.take_quest_log_abandons().is_empty(),
        "No queues no abandon"
    );
    let _ = s.take_sounds();

    // Yes: the marked quest drains, with the abandon kit (StaticPopup.lua:1691-1697).
    s.run("QuestLogFrameAbandonButton:Click()").unwrap();
    s.run("StaticPopup_OnClick(StaticPopup1, 1)").unwrap();
    assert_eq!(s.take_quest_log_abandons(), vec![1]);
    assert!(s
        .take_sounds()
        .contains(&SoundRequest::KitName("igQuestLogAbandonQuest".into())));
    assert!(!s
        .eval::<bool>("return StaticPopup1:IsShown() == 1")
        .unwrap());

    s.run("ToggleFrame(QuestLogFrame)").unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(!s
        .eval::<bool>("return QuestLogFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.take_sounds(),
        vec![SoundRequest::KitName("igQuestLogClose".into())],
        "QuestLog_OnHide plays igQuestLogClose (QuestLogFrame.lua:300)"
    );
}

/// The list wheels through `HybridScrollFrame`: a longer log than the rows scrolls them.
#[test]
fn the_wheel_scrolls_a_long_list() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, entries(25));
    s.resolve();
    let quads = s.extract();
    let list = frame_rect(&s, &quads, "QuestLogScrollFrame");
    let (x, y) = (
        (list.left + list.right) * 0.5,
        (list.bottom + list.top) * 0.5,
    );
    let first = s
        .eval::<i64>(&format!("return {}:GetID()", row(1)))
        .unwrap();
    assert_eq!(first, 1);
    s.mouse_wheel(x, y, -1.0);
    assert!(s.errors().is_empty(), "wheel errors: {:?}", s.errors());
    s.resolve();
    let after = s
        .eval::<i64>(&format!("return {}:GetID()", row(1)))
        .unwrap();
    assert!(
        after > first,
        "wheel-down moved the list: row 1 now shows quest {after}"
    );
    assert_eq!(
        s.eval::<i64>("return GetQuestLogSelection()").unwrap(),
        1,
        "the wheel never selects"
    );
}

/// A title shift-click with no chat open toggles the watch (`IsModifiedClick("QUESTWATCHTOGGLE")`,
/// QuestLogFrame.lua:150-153): the row's check and the `WatchFrame` tracker follow.
#[test]
fn shift_click_toggles_the_watch_check_and_the_tracker() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    let check = format!("{}.check", row(1));
    assert!(!shown(&s, &check), "no check before any watch");
    // The fixture feeds no quest POIs, so no quest is on the current map; build 12340's
    // `trackerFilter` "7", read at `VARIABLES_LOADED` (WatchFrame.lua:280), shows remote zones.
    s.fire_event("VARIABLES_LOADED", vec![]);

    shift_click(&mut s, &row(1));
    assert!(
        s.errors().is_empty(),
        "shift-click errors: {:?}",
        s.errors()
    );
    assert!(watched(&s, 1));
    assert_eq!(s.eval::<i64>("return GetNumQuestWatches()").unwrap(), 1);
    assert!(
        shown(&s, &check),
        "a watched row shows its check (QuestLogFrame.lua:463-467)"
    );

    // `WatchFrame_DisplayTrackedQuests` writes the title, then each unfinished objective through
    // `WatchFrame_ReverseQuestObjective` (WatchFrame.lua:823-837).
    s.resolve();
    let lines: Vec<String> = s
        .extract()
        .iter()
        .filter(|q| {
            s.quad_owner_name(q.target)
                .is_some_and(|o| o.starts_with("WatchFrameLine"))
        })
        .filter_map(|q| match &q.content {
            QuadContent::Text { text: Some(t), .. } => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        lines.iter().any(|t| t == "Quest 1"),
        "the tracker titles the watch: {lines:?}"
    );
    assert!(
        lines.iter().any(|t| t.contains("Kobold Vermin")),
        "and lists its objective: {lines:?}"
    );

    shift_click(&mut s, &row(1));
    assert!(!watched(&s, 1));
    assert!(!shown(&s, &check), "unwatching clears the check");
    assert_eq!(s.eval::<i64>("return GetQuestLogSelection()").unwrap(), 1);
}

/// The Track button toggles the selection's watch (`QuestLogFrameTrackButton_OnClick`,
/// QuestLogFrame.lua:784-787); a quest with no objectives is watchable in 3.3.5.
#[test]
fn the_track_button_toggles_the_selected_quests_watch() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    s.run(&format!("{}:Click()", row(2))).unwrap();
    assert_eq!(s.eval::<i64>("return GetQuestLogSelection()").unwrap(), 2);
    s.run("QuestLogFrameTrackButton:Click()").unwrap();
    assert!(s.errors().is_empty(), "track errors: {:?}", s.errors());
    assert!(
        watched(&s, 2),
        "quest 2 has no objectives and still watches"
    );
    s.run("QuestLogFrameTrackButton:Click()").unwrap();
    assert!(!watched(&s, 2));
}

/// `QUEST_ACCEPTED` watches the new quest while `AUTO_QUEST_WATCH` is "1", its uvar default
/// (QuestLogFrame.lua:260-264, InterfaceOptionsFrame.lua:321).
#[test]
fn an_accepted_quest_auto_watches_while_the_uvar_is_on() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    assert_eq!(s.eval::<String>("return AUTO_QUEST_WATCH").unwrap(), "1");
    s.fire_event("QUEST_ACCEPTED", vec![ScriptValue::Int(3)]);
    assert!(s.errors().is_empty(), "accept errors: {:?}", s.errors());
    assert!(watched(&s, 3));

    s.run(r#"AUTO_QUEST_WATCH = "0""#).unwrap();
    s.fire_event("QUEST_ACCEPTED", vec![ScriptValue::Int(4)]);
    assert!(!watched(&s, 4), "the uvar off watches nothing");
}

/// `QUEST_WATCH_UPDATE` on a quest with objectives arms a `MAX_QUEST_WATCH_TIME` watch while
/// `AUTO_QUEST_PROGRESS` is "1" (QuestLogFrame.lua:265-271); the engine expires it (`0x5e64d0`).
#[test]
fn progress_watches_for_five_minutes_while_the_uvar_is_on() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    s.run(r#"AUTO_QUEST_PROGRESS = "1""#).unwrap();
    s.fire_event("QUEST_WATCH_UPDATE", vec![ScriptValue::Int(1)]);
    assert!(s.errors().is_empty(), "progress errors: {:?}", s.errors());
    assert!(watched(&s, 1));
    s.fire_event("QUEST_WATCH_UPDATE", vec![ScriptValue::Int(2)]);
    assert!(!watched(&s, 2), "no objectives, no progress watch");

    s.tick(299.0);
    s.fire_event("QUEST_WATCH_UPDATE", vec![ScriptValue::Int(1)]);
    s.tick(299.0);
    assert!(watched(&s, 1), "fresh progress re-armed it");
    s.tick(2.0);
    assert!(!watched(&s, 1), "expired 300 s after the last progress");

    s.run(r#"AUTO_QUEST_PROGRESS = "0""#).unwrap();
    s.fire_event("QUEST_WATCH_UPDATE", vec![ScriptValue::Int(1)]);
    assert!(!watched(&s, 1), "the uvar off watches nothing");
}

/// A row's tag paints in parentheses on its right-flush `$parentTag`; Failed and Complete replace
/// it (QuestLogFrame.lua:449-462), and a header carries none.
#[test]
fn the_row_tag_is_its_own_right_flush_string_and_the_state_word_wins() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    let mut state = eight_entries();
    state.entries[0].tag = Some("Elite".into());
    state.entries[1].tag = Some("Raid".into());
    state.entries[1].complete = 1;
    state.entries[2].complete = -1;
    state.entries[3].is_header = true;
    state.entries[3].title = "Elwynn Forest".into();
    state.entries[3].tag = None;
    open_log(&mut s, state);

    let tag = |s: &UiScript, i: u32| -> Option<String> {
        let tag = format!("{}Tag", row(i));
        shown(s, &tag).then(|| text(s, &tag)).flatten()
    };
    assert_eq!(tag(&s, 1).as_deref(), Some("(Elite)"));
    assert_eq!(
        tag(&s, 2).as_deref(),
        Some("(Complete)"),
        "COMPLETE overrides the tag"
    );
    assert_eq!(
        tag(&s, 3).as_deref(),
        Some("(Failed)"),
        "FAILED overrides it too"
    );
    assert_eq!(tag(&s, 4), None, "a header hides its tag");
    assert_eq!(
        text(&s, &row(2)).as_deref(),
        Some("  Quest 2"),
        "the state is only the tag's"
    );
    assert_eq!(
        text(&s, &row(4)).as_deref(),
        Some("Elwynn Forest"),
        "a header is unindented"
    );

    // Anchored RIGHT, -2 (QuestLogFrame.xml:16-20).
    let (row_right, tag_right) = s
        .eval::<(f32, f32)>(&format!("return {0}:GetRight(), {0}Tag:GetRight()", row(1)))
        .unwrap();
    assert!(
        (row_right - tag_right - 2.0).abs() < 0.5,
        "tag right-flush at row right -2 (row {row_right}, tag {tag_right})"
    );
}

#[test]
fn an_empty_log_shows_the_empty_pane_and_disables_the_controls() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    join_party(&mut s, 4);
    open_log(&mut s, QuestLogState::default());

    // `QuestLog_Update` (QuestLogFrame.lua:331-336), `QuestLogControlPanel_UpdateState` (:858-861).
    assert!(shown(&s, "EmptyQuestLogFrame"));
    assert_eq!(
        text(&s, "QuestLogNoQuestsText").as_deref(),
        Some("No Active Quests")
    );
    assert!(!shown(&s, &row(1)));
    for button in [
        "QuestLogFrameAbandonButton",
        "QuestLogFrameTrackButton",
        "QuestLogFramePushQuestButton",
    ] {
        assert!(
            !s.eval::<bool>(&format!("return {button}:IsEnabled() == 1"))
                .unwrap(),
            "{button} is dark with no selection, even in a party"
        );
    }
    assert!(!s
        .eval::<bool>("return QuestLogDetailScrollFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        text(&s, "QuestLogQuestCount").as_deref(),
        Some("Quests: |cffffffff0/25|r")
    );
}

/// `QuestInfo_ShowRewards` lays choices two per row, anchors "You will also receive:" under the
/// last choice row's left item, then the reward (QuestInfo.lua:315-467).
#[test]
fn reward_rows_follow_the_stock_two_per_row_layout() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    let mut state = eight_entries();
    let item = |name: &str| QuestItemView {
        item_id: 0,
        name: Some(name.into()),
        texture: None,
        count: 1,
        quality: 1,
        usable: true,
        ..Default::default()
    };
    let detail = QuestLogDetail {
        description: "Speak with Marshal McBride.".into(),
        objectives_text: "Report to Marshal McBride.".into(),
        reward_money: 150,
        choices: vec![item("Worn Sword"), item("Worn Mace")],
        rewards: vec![item("Militia Hammer")],
        ..Default::default()
    };
    for e in &mut state.entries {
        e.detail = Some(detail.clone());
    }
    open_log(&mut s, state);

    // One pool: items 1-2 are the choices, 3 the reward.
    for i in 1..=3 {
        assert!(shown(&s, &format!("QuestInfoItem{i}")), "item {i} shows");
    }
    assert!(!shown(&s, "QuestInfoItem4"));
    assert_eq!(
        text(&s, "QuestInfoItem1Name").as_deref(),
        Some("Worn Sword")
    );
    assert_eq!(text(&s, "QuestInfoItem2Name").as_deref(), Some("Worn Mace"));
    assert_eq!(
        text(&s, "QuestInfoItem3Name").as_deref(),
        Some("Militia Hammer")
    );
    assert_eq!(
        text(&s, "QuestInfoItemChooseText").as_deref(),
        Some("You will be able to choose one of these rewards:"),
        "REWARD_CHOICES: the log never chooses (QUEST_TEMPLATE_LOG.chooseItems is nil)"
    );
    assert_eq!(
        text(&s, "QuestInfoItemReceiveText").as_deref(),
        Some("You will also receive:")
    );
    // `SetPoint` adds to the XML's anchors, so read the TOPLEFT one.
    let point = |s: &UiScript, frame: &str| {
        s.eval::<String>(&format!(
            "for i = 1, {frame}:GetNumPoints() do \
               local p, rel, rp, x, y = {frame}:GetPoint(i) \
               if p == 'TOPLEFT' then \
                 return p .. '|' .. rel:GetName() .. '|' .. rp .. '|' .. x .. '|' .. y \
               end \
             end return 'none'"
        ))
        .unwrap()
    };
    assert_eq!(
        point(&s, "QuestInfoItemReceiveText"),
        "TOPLEFT|QuestInfoItem1|BOTTOMLEFT|3|-5",
        "under the last choice row's left item (QuestInfo.lua:394)"
    );
    assert_eq!(
        point(&s, "QuestInfoItem2"),
        "TOPLEFT|QuestInfoItem1|TOPRIGHT|1|0",
        "the second choice sits right of the first"
    );
}

/// Answer every pending measure with `FixedWidthFont(6.0)`'s metrics: 6 px a glyph, 12 px a line.
fn answer_measures(s: &mut UiScript) {
    let answers: Vec<(u32, f32, f32, u64)> = s
        .fontstrings_needing_measure()
        .into_iter()
        .map(|r| {
            let ink = r.text.chars().count() as f32 * 6.0;
            match r.wrap_width {
                Some(w) if w > 0.0 && ink > w => (r.id, w, (ink / w).ceil() * 12.0, r.key),
                _ => (r.id, ink, 12.0, r.key),
            }
        })
        .collect();
    s.set_measured_text_unwrapped(&answers);
}

fn overflowing_entry() -> QuestLogState {
    let objectives = (1..=10)
        .map(|i| QuestLogObjectiveView {
            text: format!("Objective {i} of 10 slain: 0/5"),
            kind: "monster".into(),
            finished: false,
            cur: 0,
            req: 5,
            poi_bit: None,
        })
        .collect();
    QuestLogState {
        num_quests: 1,
        entries: vec![QuestLogEntryView {
            quest_id: 1,
            title: "A Very Long Quest".into(),
            level: 5,
            complete: 0,
            objectives,
            detail: Some(QuestLogDetail {
                description: "A very long description. ".repeat(20),
                objectives_text: "Report back once every objective below is complete.".into(),
                required_money: 0,
                reward_money: 0,
                choices: vec![],
                rewards: vec![QuestItemView {
                    item_id: 0,
                    name: Some("Militia Hammer".into()),
                    texture: None,
                    count: 1,
                    quality: 1,
                    usable: true,
                    ..Default::default()
                }],
                reward_spell: None,
                ..Default::default()
            }),
            ..Default::default()
        }],
    }
}

/// Quads under the detail scroll child carry `QuestLogDetailScrollFrame`'s rect as their clip;
/// the window's own art outside it carries none.
#[test]
fn overflowing_detail_content_clips_to_the_scrollframe_rect() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, overflowing_entry());
    s.resolve();
    let quads = s.extract();
    let scroll_rect = frame_rect(&s, &quads, "QuestLogDetailScrollFrame");

    let reward_plate_clips: Vec<Option<wrathplus_ui::layout::Rect>> = quads
        .iter()
        .filter_map(|q| match &q.content {
            QuadContent::Texture { path: Some(p), .. } if p.contains("UI-QuestItemNameFrame") => {
                Some(q.clip)
            }
            _ => None,
        })
        .collect();
    assert!(
        !reward_plate_clips.is_empty(),
        "the fixture's reward name plate is drawn"
    );
    for clip in reward_plate_clips {
        assert_eq!(
            clip,
            Some(scroll_rect),
            "a reward row clips to the ScrollFrame's rect"
        );
    }

    let book_icon_clip = quads.iter().find_map(|q| match &q.content {
        QuadContent::Texture { path: Some(p), .. } if p.contains("UI-QuestLog-BookIcon") => {
            Some(q.clip)
        }
        _ => None,
    });
    assert_eq!(
        book_icon_clip,
        Some(None),
        "the window's own chrome is unclipped"
    );
}

/// The detail pane's scroll frame takes the wheel (`UIPanelScrollFrameTemplate`).
#[test]
fn wheel_over_the_detail_pane_changes_vertical_scroll() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_text_measurer(Box::new(super::FixedWidthFont(6.0)));
    open_log(&mut s, overflowing_entry());
    // The wrapped texts measure, their heights reach the scroll child's extent, then the range.
    for _ in 0..3 {
        s.resolve();
        answer_measures(&mut s);
        s.resolve();
    }
    let quads = s.extract();
    let scroll_rect = frame_rect(&s, &quads, "QuestLogDetailScrollFrame");
    let (x, y) = (scroll_rect.left + 10.0, scroll_rect.top - 10.0);
    assert_eq!(
        s.eval::<f32>("return QuestLogDetailScrollFrame:GetVerticalScroll()")
            .unwrap(),
        0.0
    );
    s.mouse_wheel(x, y, -1.0);
    assert!(s.errors().is_empty(), "wheel errors: {:?}", s.errors());
    let after = s
        .eval::<f32>("return QuestLogDetailScrollFrame:GetVerticalScroll()")
        .unwrap();
    assert!(
        after > 0.0,
        "wheel-down scrolled the detail pane, got {after}"
    );
}

/// `QUEST_LOG_UPDATE` refreshes the detail without a scroll reset (`QuestLog_UpdateQuestDetails
/// (false)`, QuestLogFrame.lua:244-247); a selection resets it (`QuestLog_SetSelection`, :667).
#[test]
fn selection_change_resets_detail_scroll_but_a_quest_log_update_refresh_does_not() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_text_measurer(Box::new(super::FixedWidthFont(6.0)));
    open_log(&mut s, overflowing_entry());
    // The wrapped texts measure, their heights reach the scroll child's extent, then the range.
    for _ in 0..3 {
        s.resolve();
        answer_measures(&mut s);
        s.resolve();
    }

    s.run("QuestLogDetailScrollFrameScrollBar:SetValue(10)")
        .unwrap();
    let scroll = |s: &UiScript| {
        s.eval::<f32>("return QuestLogDetailScrollFrame:GetVerticalScroll()")
            .unwrap()
    };
    assert_eq!(scroll(&s), 10.0);
    s.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert!(s.errors().is_empty(), "refresh errors: {:?}", s.errors());
    assert_eq!(scroll(&s), 10.0, "the refresh keeps the scroll");
    s.run(&format!("{}:Click()", row(1))).unwrap();
    assert!(s.errors().is_empty(), "click errors: {:?}", s.errors());
    assert_eq!(
        scroll(&s),
        0.0,
        "a reselect snaps the detail back to the top"
    );
}

/// The reward row's OnEnter calls `SetQuestLogItem` (`QuestInfo.xml:7-26`): a cold store shows
/// the name and asks once, and after the answer a re-hover shows the full item lines.
#[test]
fn reward_row_hover_serves_the_shared_item_tooltip() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    use wrathplus_ui::script::ItemTemplateView;

    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    s.resolve();

    let icon_rect = s
        .extract()
        .iter()
        .find(|q| {
            matches!(&q.content, QuadContent::Texture { path: Some(p), .. } if p.contains("INV_Hammer_15"))
        })
        .and_then(|q| q.rect)
        .expect("the reward row icon");
    // Extracted rects and mouse_move share one y-up space.
    let (cx, cy) = (
        (icon_rect.left + icon_rect.right) * 0.5,
        (icon_rect.bottom + icon_rect.top) * 0.5,
    );
    let has_text = |s: &mut UiScript, t: &str| {
        s.resolve();
        s.extract()
            .iter()
            .any(|q| matches!(&q.content, QuadContent::Text { text: Some(x), .. } if x == t))
    };

    s.mouse_move(cx, cy);
    assert!(s.errors().is_empty(), "hover errors: {:?}", s.errors());
    assert!(
        s.eval::<bool>("return GameTooltip:IsVisible()").unwrap(),
        "tooltip shows on hover"
    );
    assert_eq!(
        s.take_item_stat_asks(),
        vec![2024],
        "the miss recorded the ask"
    );

    s.set_item_template(
        2024,
        ItemTemplateView {
            name: "Militia Hammer".into(),
            quality: 1,
            inventory_type: 21, // Main Hand
            class: 2,
            subclass: 4, // mace
            damages: vec![(4.0, 9.0, 0)],
            delay_ms: 2200,
            ..Default::default()
        },
    );
    s.mouse_move(0.0, 0.0);
    s.mouse_move(cx, cy);
    assert!(s.errors().is_empty(), "re-hover errors: {:?}", s.errors());
    assert!(
        has_text(&mut s, "Main Hand"),
        "slot line from the shared store"
    );
    assert!(
        has_text(&mut s, "4 - 9 Damage"),
        "damage line from the shared store"
    );
}

/// A child takes its parent's strata and level + 1 at creation, so the DIALOG popup's buttons draw
/// above its own backdrop.
#[test]
fn popup_children_inherit_the_dialog_stratum() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    seat(&mut s, QuestLogState::default());
    s.run(
        "StaticPopupDialogs[\"TEST_STRATUM\"] = { text = \"Abandon?\", button1 = \"Yes\", \
         button2 = \"No\", timeout = 0 }\n\
         StaticPopup_Show(\"TEST_STRATUM\")",
    )
    .unwrap();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    s.resolve();
    let quads = s.extract();
    let backdrop_z = quads
        .iter()
        .find_map(|q| match &q.content {
            QuadContent::Backdrop { path, .. } if path.contains("UI-DialogBox-Background") => {
                Some(q.z)
            }
            _ => None,
        })
        .expect("the popup backdrop");
    let button_z = quads
        .iter()
        .find_map(|q| match &q.content {
            QuadContent::Texture { path: Some(p), .. } if p.contains("UI-DialogBox-Button-Up") => {
                Some(q.z)
            }
            _ => None,
        })
        .expect("a popup button face");
    assert!(
        button_z > backdrop_z,
        "the buttons draw ABOVE their dialog's backdrop (button z {button_z:x} vs backdrop z {backdrop_z:x})"
    );
}

/// The reward button's OnClick (QuestInfo.xml:27-45): a modified click goes to
/// `HandleModifiedItemClick` with `GetQuestLogItemLink`; a plain one on a reward does nothing.
#[test]
fn reward_rows_preview_and_post_and_a_plain_click_stays_inert() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    assert!(
        shown(&s, "QuestInfoItem1"),
        "the fixture's reward row is laid out"
    );
    let _ = s.take_dressup_intents();

    s.run("QuestInfoItem1:Click()").unwrap();
    assert!(
        s.take_dressup_intents().is_empty(),
        "a plain click dresses nothing"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());

    // Shift with chat open inserts the link (`ChatEdit_InsertLink`).
    s.run(r#"ChatFrame_OpenChat("")"#).unwrap();
    shift_click(&mut s, "QuestInfoItem1");
    // `ChatEdit_InsertLink` prefixes a space "for proper parsing" (ChatFrame.lua:3492-3493).
    assert_eq!(
        text(&s, "ChatEdit_GetActiveWindow()"),
        Some(format!(" {HAMMER_LINK}")),
        "shift-click posted the reward's link"
    );
    assert!(s.take_dressup_intents().is_empty());

    // Ctrl: `DressUpItemLink` dresses the room, then tries it on, once `IsDressableItem` knows the
    // item has a display (DressUpFrame.lua:2-3).
    s.set_item_template(
        2024,
        wrathplus_ui::script::ItemTemplateView {
            name: "Militia Hammer".into(),
            quality: 1,
            inventory_type: 21, // Main Hand
            class: 2,
            subclass: 4,
            icon: Some("Interface\\Icons\\INV_Hammer_15".into()),
            ..Default::default()
        },
    );
    modified_click(&mut s, "QuestInfoItem1", (false, true, false));
    assert_eq!(
        s.take_dressup_intents(),
        vec![
            wrathplus_ui::script::DressUpIntent::Dress,
            wrathplus_ui::script::DressUpIntent::TryOn(2024)
        ],
        "ctrl-click opened the room wearing the reward"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// With chat open a title shift-click inserts the quest's link and selects
/// (QuestLogFrame.lua:144-149); with chat closed it toggles the watch.
#[test]
fn shift_click_on_a_title_links_the_quest_with_chat_open_and_watches_with_it_closed() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    open_log(&mut s, eight_entries());
    s.run(&format!("{}:Click()", row(2))).unwrap();

    shift_click(&mut s, &row(1));
    assert!(watched(&s, 1), "chat closed: the watch toggles");
    shift_click(&mut s, &row(1));
    assert!(!watched(&s, 1));

    s.run(r#"ChatFrame_OpenChat("")"#).unwrap();
    shift_click(&mut s, &row(1));
    let link = s.eval::<String>("return GetQuestLink(1)").unwrap();
    assert!(
        link.contains("|Hquest:1:") && link.contains("[Quest 1]"),
        "{link}"
    );
    assert_eq!(
        text(&s, "ChatEdit_GetActiveWindow()"),
        Some(format!(" {link}")),
        "the link landed in chat, after ChatEdit_InsertLink's space"
    );
    assert!(!watched(&s, 1), "chat open: the watch does not toggle");
    assert_eq!(s.eval::<i64>("return GetQuestLogSelection()").unwrap(), 1);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

// ── The Share Quest button ──

/// A party of `n` others; `GetNumPartyMembers` is that list's length.
fn party(n: usize) -> PartyState {
    PartyState {
        members: (0..n)
            .map(|i| PartyMemberInfo {
                name: format!("Mate{i}"),
                guid: 0x300 + i as u64,
            })
            .collect(),
        ..PartyState::default()
    }
}

/// [`party`] with each member in range: the app seats `partyN` off the held object, so the stock
/// party frames (`LFDFrame.lua:975` reads `UnitName("partyN")`) see names.
fn join_party(s: &mut UiScript, n: usize) {
    s.set_party(party(n));
    for i in 0..n {
        s.set_unit(
            &format!("party{}", i + 1),
            Some(wrathplus_ui::script::UnitState {
                exists: true,
                has_object: true,
                is_player: true,
                name: Some(format!("Mate{i}")),
                level: 5,
                class: Some("Warrior".into()),
                class_file: Some("WARRIOR".into()),
                ..Default::default()
            }),
        );
    }
}

/// The eight quests with `pushable` set on `ids`; the app reads it from `QUEST_FLAGS_SHARABLE`.
fn entries_sharable(ids: &[u32]) -> QuestLogState {
    let mut state = eight_entries();
    for e in &mut state.entries {
        e.pushable = ids.contains(&e.quest_id);
    }
    state
}

/// Share needs both a sharable selection and a party (`QuestLogControlPanel_UpdateState`,
/// QuestLogFrame.lua:871-875); each half is moved alone.
#[test]
fn share_quest_needs_both_a_sharable_selection_and_a_party() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_party(PartyState::default());
    open_log(&mut s, entries_sharable(&[1]));
    let share_lit = |s: &UiScript| {
        s.eval::<bool>("return QuestLogFramePushQuestButton:IsEnabled() == 1")
            .unwrap()
    };
    assert_eq!(s.eval::<i64>("return GetQuestLogSelection()").unwrap(), 1);
    assert!(s
        .eval::<bool>("return GetQuestLogPushable() ~= nil")
        .unwrap());
    assert!(!share_lit(&s), "solo: sharable is not enough");

    // PARTY_MEMBERS_CHANGED re-tests it (QuestLogFrame.lua:273-277).
    join_party(&mut s, 2);
    s.fire_event("PARTY_MEMBERS_CHANGED", vec![]);
    assert!(share_lit(&s));

    s.run(&format!("{}:Click()", row(2))).unwrap();
    assert!(s
        .eval::<bool>("return GetQuestLogPushable() == nil")
        .unwrap());
    assert!(
        !share_lit(&s),
        "in a party: an unsharable selection is dark"
    );

    s.run(&format!("{}:Click()", row(1))).unwrap();
    assert!(share_lit(&s));
    s.set_party(PartyState::default());
    s.fire_event("PARTY_MEMBERS_CHANGED", vec![]);
    assert!(!share_lit(&s));
}

/// The click queues the selected quest's id and plays igQuestLogOpen (QuestLogFrame.xml:190-193).
#[test]
fn share_quest_click_queues_the_selected_quests_id() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    join_party(&mut s, 1);
    open_log(&mut s, entries_sharable(&[1, 5]));
    s.run(&format!("{}:Click()", row(5))).unwrap();
    let _ = s.take_sounds();
    s.run("QuestLogFramePushQuestButton:Click()").unwrap();
    assert_eq!(s.take_quest_log_pushes(), vec![5]);
    assert!(s.take_quest_log_pushes().is_empty(), "drained");
    assert!(s
        .take_sounds()
        .contains(&SoundRequest::KitName("igQuestLogOpen".into())));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `QuestLogTitleButton_Resize` (QuestLogFrame.lua:183-224) narrows a title that would overrun its
/// tag (the tag's width + 4 from the row's right) or, untagged, the row itself.
#[test]
fn a_long_title_stops_short_of_its_tag_and_its_row() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_text_measurer(Box::new(super::FixedWidthFont(6.0)));
    const LONG: &str = "The Left Piece of Lord Valthalak's Amulet, Recovered at Last";
    let mut state = eight_entries();
    state.entries[0].title = LONG.into();
    state.entries[0].tag = Some("Dungeon".into());
    state.entries[1].title = LONG.into();
    open_log(&mut s, state);
    // Labelled gap: the native `SetText` re-measures a FontString's rect at once, which the
    // resize's `SetWidth(0)` / `SetText` hack relies on (QuestLogFrame.lua:185-192); here region
    // edges are the last resolve's, so the cap holds from the first update after a layout pass.
    s.resolve();
    s.run("QuestLog_Update()").unwrap();
    s.resolve();

    let f = |s: &UiScript, expr: &str| s.eval::<f32>(&format!("return {expr}")).unwrap();
    let (r1, r2) = (row(1), row(2));
    let row_right = f(&s, &format!("{r1}:GetLeft() + {r1}:GetWidth()"));
    let tag_w = f(&s, &format!("{r1}Tag:GetWidth()"));
    assert!(tag_w > 0.0, "the tag measured");
    let title_right = f(
        &s,
        &format!("{r1}NormalText:GetLeft() + {r1}NormalText:GetWidth()"),
    );
    assert!(
        title_right <= row_right - tag_w - 4.0 + 0.5,
        "the tagged title ends at {title_right}, short of the tag at {}",
        row_right - tag_w - 4.0
    );
    assert!(
        f(&s, &format!("{r1}NormalText:GetStringWidth()"))
            > f(&s, &format!("{r1}NormalText:GetWidth()")),
        "the fixture title overflows, or this proves nothing"
    );
    let row2_right = f(&s, &format!("{r2}:GetLeft() + {r2}:GetWidth()"));
    let title2_right = f(
        &s,
        &format!("{r2}NormalText:GetLeft() + {r2}NormalText:GetWidth()"),
    );
    assert!(
        title2_right <= row2_right + 0.5,
        "the untagged title stops at its row's right ({title2_right} vs {row2_right})"
    );
}
