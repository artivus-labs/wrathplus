//! The stock Reputation tab and its watch bar, both `ReputationFrame.xml`, engine-only: what the
//! faction tuples become on screen.

use wrathplus_ui::script::{FactionEntry, QuadContent, ReputationState, UiScript, UnitState};

/// A visible bar row: `standing_id` 5 (Friendly), 1000 into a 6000-wide rank window.
fn entry(faction_id: u32, rep_list_id: u32, parent_id: u32, name: &str) -> FactionEntry {
    FactionEntry {
        faction_id,
        rep_list_id,
        parent_id,
        name: name.into(),
        description: format!("About the {name}."),
        standing: 4000,
        standing_id: 5,
        bar_min: 3000,
        bar_max: 9000,
        visible: true,
        is_header: false,
        at_war: false,
        can_toggle_at_war: true,
        inactive: false,
        has_rep: false,
    }
}

/// A header as the wire delivers one: flag `0x08`, not visible, which never gates its group.
fn header(faction_id: u32, rep_list_id: u32, name: &str) -> FactionEntry {
    FactionEntry {
        visible: false,
        is_header: true,
        ..entry(faction_id, rep_list_id, 0, name)
    }
}

/// `wrathplus-ui`'s own reputation fixture, pushed out of order so the sort shows. Visible slots:
/// 1 Alliance (header), 2 Ironforge, 3 Stormwind, 4 Steamwheedle Cartel (header), 5 Booty Bay,
/// 6 Other (header), 7 Argent Dawn, 8 Bloodsail Buccaneers.
fn state() -> ReputationState {
    ReputationState {
        entries: vec![
            entry(72, 19, 469, "Stormwind"),
            entry(529, 13, 0, "Argent Dawn"),
            header(469, 11, "Alliance"),
            entry(21, 1, 169, "Booty Bay"),
            entry(47, 20, 469, "Ironforge"),
            header(169, 10, "Steamwheedle Cartel"),
            entry(87, 0, 0, "Bloodsail Buccaneers"),
        ],
        watched: None,
    }
}

/// The character window, [`super::test_ui::CHARACTER_UI`], which carries `ReputationFrame.xml`:
/// `CharacterFrame_ShowSubFrame` hides every page by name, unguarded (CharacterFrame.lua:25-33).
fn load_page(s: &mut UiScript) {
    for file in super::test_ui::CHARACTER_UI {
        super::test_ui::load_ui_strict(s, file);
    }
}

/// A player with a guid and race and class pairs, unguarded in PaperDollFrame.lua:101.
fn player(level: u32) -> UnitState {
    UnitState {
        exists: true,
        guid: 0x0100_0000_0000_0001,
        level,
        race: Some("Human".into()),
        race_file: Some("Human".into()),
        class: Some("Warrior".into()),
        class_file: Some("WARRIOR".into()),
        ..UnitState::default()
    }
}

/// The Reputation page, open on its tab, with [`state`] pushed and a level-40 player behind it.
fn shown_reputation_page() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_page(&mut s);
    s.set_unit("player", Some(player(40)));
    s.set_reputation(state());
    s.run(r#"ToggleCharacter("ReputationFrame")"#).unwrap();
    s.resolve();
    s
}

/// Click a named frame's centre through the pointer pipeline, so a frame eating the clicks fails.
fn click_center(s: &mut UiScript, name: &str) {
    s.resolve();
    let (x, y) = s
        .eval::<(f32, f32)>(&format!(
            "return ({name}:GetLeft() + {name}:GetRight()) / 2, \
                    ({name}:GetTop() + {name}:GetBottom()) / 2"
        ))
        .unwrap();
    s.mouse_move(x, y);
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
}

fn text_of(s: &mut UiScript, expr: &str) -> String {
    s.eval::<String>(&format!("return {expr}:GetText() or \"\""))
        .unwrap()
}

/// `IsVisible`, not `IsShown`, for the scroll bar: when the list fits, `FauxScrollFrame_Update`
/// hides the scroll frame, not the bar (UIPanelTemplates.lua:169-170).
fn visible(s: &mut UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!("return {name}:IsVisible() and true or false"))
        .unwrap_or(false)
}

fn shown(s: &mut UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!("return {name}:IsShown() and true or false"))
        .unwrap()
}

/// `ToggleCharacter` selects the tab from the page's own `id=` (CharacterFrame.lua:10), and the
/// tabs open Reputation at 3 and Skills at 4 (CharacterFrame.lua:40-43).
#[test]
fn the_reputation_page_opens_on_the_windows_third_tab() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = shown_reputation_page();
    assert!(
        s.eval::<bool>("return ReputationFrame:IsVisible()")
            .unwrap(),
        "the Reputation page is up"
    );
    assert_eq!(
        s.eval::<i64>("return PanelTemplates_GetSelectedTab(CharacterFrame)")
            .unwrap(),
        3,
        "and the tab row selects 3 — the reference's own slot for Reputation"
    );
    assert_eq!(
        s.eval::<i64>("return ReputationFrame:GetID()").unwrap(),
        3,
        "the id IS the tab slot the reference's tab 3 opens"
    );
    assert_eq!(
        s.eval::<i64>("return SkillFrame:GetID()").unwrap(),
        4,
        "and Skills is the reference's 4 beside it"
    );
    // `CHARACTERFRAME_SUBFRAMES` is the hide-all list, not the tab map (CharacterFrame.lua:1): a
    // page missing from it is never hidden.
    assert!(
        s.eval::<bool>(
            "for _, v in ipairs(CHARACTERFRAME_SUBFRAMES) do if v == \"ReputationFrame\" then return true end end return false"
        )
        .unwrap(),
        "the hide-all list names this page"
    );

    // Eight rows in fifteen slots: slot 9 is hidden, and a fitting list has no scroll bar.
    assert_eq!(s.eval::<i64>("return GetNumFactions()").unwrap(), 8);
    assert!(
        !shown(&mut s, "ReputationBar9"),
        "slot 9 has nothing to hold"
    );
    assert!(!visible(&mut s, "ReputationListScrollFrameScrollBar"));

    s.run(r#"ToggleCharacter("PaperDollFrame")"#).unwrap();
    assert!(!shown(&mut s, "ReputationFrame"), "one page at a time");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// 3.3.5 draws headers and factions in the same `ReputationBarN` rows; a header shows its fold
/// button, whose art follows `isCollapsed` (ReputationFrame.lua:158-163) and whose click is the fold
/// binding (ReputationFrame.xml:10-30), which raises `UPDATE_FACTION`, the pane's repaint.
#[test]
fn a_header_row_paints_its_fold_icon_and_folds_on_click() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = shown_reputation_page();

    assert_eq!(text_of(&mut s, "ReputationBar1FactionName"), "Alliance");
    assert!(
        shown(&mut s, "ReputationBar1ExpandOrCollapseButton"),
        "slot 1 is a header"
    );
    assert!(
        !shown(&mut s, "ReputationBar1ReputationBar"),
        "so its bar is down"
    );
    let icon = |s: &mut UiScript| {
        s.eval::<String>(
            "return ReputationBar1ExpandOrCollapseButton:GetNormalTexture():GetTexture() or \"\"",
        )
        .unwrap()
    };
    assert!(
        icon(&mut s).contains("UI-MinusButton-Up"),
        "an expanded header wears the MINUS: {}",
        icon(&mut s)
    );
    assert_eq!(
        text_of(&mut s, "ReputationBar2FactionName"),
        "Ironforge",
        "its two children follow it"
    );

    click_center(&mut s, "ReputationBar1ExpandOrCollapseButton");

    assert!(
        icon(&mut s).contains("UI-PlusButton-Up"),
        "a collapsed header wears the PLUS: {}",
        icon(&mut s)
    );
    assert_eq!(
        text_of(&mut s, "ReputationBar2FactionName"),
        "Steamwheedle Cartel",
        "and slot 2 has re-bound to the next group — the repaint the click owes"
    );
    assert!(
        shown(&mut s, "ReputationBar2ExpandOrCollapseButton"),
        "slot 2 is a header now: Ironforge is not on screen at all any more"
    );
    assert_eq!(
        s.eval::<i64>("return GetNumFactions()").unwrap(),
        6,
        "the two folded children leave the visible list"
    );

    click_center(&mut s, "ReputationBar1ExpandOrCollapseButton");
    assert_eq!(text_of(&mut s, "ReputationBar2FactionName"), "Ironforge");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The bar's range is the rank window normalized, `barMax - barMin` and `barValue - barMin`
/// (ReputationFrame.lua:171-174), which is why the binding reports all three absolute.
#[test]
fn a_bar_row_paints_name_standing_and_the_faction_bar_colour() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = shown_reputation_page();

    assert_eq!(text_of(&mut s, "ReputationBar2FactionName"), "Ironforge");
    assert_eq!(
        text_of(&mut s, "ReputationBar2ReputationBarFactionStanding"),
        "Friendly",
        "standingID 5 is FACTION_STANDING_LABEL5"
    );
    assert_eq!(
        s.eval::<(f64, f64)>("return ReputationBar2ReputationBar:GetMinMaxValues()")
            .unwrap(),
        (0.0, 6000.0),
        "the 3000..9000 rank window, normalized to 0..6000"
    );
    assert_eq!(
        s.eval::<f64>("return ReputationBar2ReputationBar:GetValue()")
            .unwrap(),
        1000.0,
        "and 4000 standing is 1000 into it"
    );
    let (r, g, b) = s
        .eval::<(f32, f32, f32)>("return ReputationBar2ReputationBar:GetStatusBarColor()")
        .unwrap();
    assert_eq!(
        (r, g, b),
        (0.0, 0.6, 0.1),
        "FACTION_BAR_COLORS[5] — the green every friendly-and-better rank shares"
    );

    // Hover shows progress and glow; leaving restores the word (ReputationFrame.xml:209-222).
    assert!(
        !shown(&mut s, "ReputationBar2ReputationBarHighlight1"),
        "glow starts down"
    );
    let (x, y) = s
        .eval::<(f32, f32)>(
            "return (ReputationBar2:GetLeft() + ReputationBar2:GetRight()) / 2, \
                    (ReputationBar2:GetTop() + ReputationBar2:GetBottom()) / 2",
        )
        .unwrap();
    s.mouse_move(x, y);
    assert_eq!(
        text_of(&mut s, "ReputationBar2ReputationBarFactionStanding"),
        "|cffffffff 1000 / 6000|r",
        "hovering shows the numbers, in the reference's own colour-coded form"
    );
    assert!(shown(&mut s, "ReputationBar2ReputationBarHighlight1"));
    assert!(shown(&mut s, "ReputationBar2ReputationBarHighlight2"));
    s.mouse_move(0.0, 0.0);
    assert_eq!(
        text_of(&mut s, "ReputationBar2ReputationBarFactionStanding"),
        "Friendly",
        "and leaving restores the word"
    );
    assert!(
        !shown(&mut s, "ReputationBar2ReputationBarHighlight1"),
        "the glow goes with it — this row is not the selected one"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The faux scroll kit re-binds the 15 fixed slots to a moving offset; nothing scrolls or clips.
#[test]
fn the_scroll_offset_rebinds_the_fixed_row_slots() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_page(&mut s);
    s.set_unit("player", Some(player(40)));

    // One header over twenty children: 21 visible rows against 15 slots.
    let mut entries = vec![header(469, 11, "Alliance")];
    for i in 1..=20u32 {
        entries.push(entry(1000 + i, i, 469, &format!("Faction {i:02}")));
    }
    s.set_reputation(ReputationState {
        entries,
        watched: None,
    });
    s.run(r#"ToggleCharacter("ReputationFrame")"#).unwrap();
    s.resolve();

    assert_eq!(s.eval::<i64>("return GetNumFactions()").unwrap(), 21);
    assert_eq!(text_of(&mut s, "ReputationBar1FactionName"), "Alliance");
    assert_eq!(text_of(&mut s, "ReputationBar15FactionName"), "Faction 14");
    assert!(
        visible(&mut s, "ReputationListScrollFrameScrollBar"),
        "21 rows in 15 slots raises the scroll bar"
    );
    // Scroll three rows down: every slot re-binds, and slot 1 stops being a header.
    s.run("FauxScrollFrame_SetOffset(ReputationListScrollFrame, 3) ReputationFrame_Update()")
        .unwrap();
    assert!(
        !shown(&mut s, "ReputationBar1ExpandOrCollapseButton"),
        "slot 1 now holds a faction, so its fold button is down"
    );
    assert_eq!(text_of(&mut s, "ReputationBar1FactionName"), "Faction 03");
    assert_eq!(text_of(&mut s, "ReputationBar15FactionName"), "Faction 17");

    // Past the end the offset is stored unclamped (UIPanelTemplates.lua:239-241), and
    // `ReputationFrame_Update` hides the slots past the list (ReputationFrame.lua:50,141-144).
    s.run("FauxScrollFrame_SetOffset(ReputationListScrollFrame, 9) ReputationFrame_Update()")
        .unwrap();
    assert_eq!(
        s.eval::<i64>("return FauxScrollFrame_GetOffset(ReputationListScrollFrame)")
            .unwrap(),
        9,
        "the offset is stored as asked; the reference guards the binding, not the offset"
    );
    assert_eq!(text_of(&mut s, "ReputationBar1FactionName"), "Faction 09");
    assert!(
        !shown(&mut s, "ReputationBar15"),
        "slot 15 would want faction 24 of 21, so it stays hidden rather than binding past the end"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// A peace-forced faction's At War box is disabled and greyed (ReputationFrame.lua:238-246), and
/// clicking the selected row again closes the popup (`ReputationBar_OnClick`, :311-321).
#[test]
fn clicking_a_bar_opens_the_detail_popup_on_that_faction() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_page(&mut s);
    s.set_unit("player", Some(player(40)));
    // Ironforge at war and peace-forced.
    let mut st = state();
    for e in &mut st.entries {
        if e.name == "Ironforge" {
            e.at_war = true;
            e.can_toggle_at_war = false;
        }
    }
    s.set_reputation(st);
    s.run(r#"ToggleCharacter("ReputationFrame")"#).unwrap();
    s.resolve();

    assert!(!shown(&mut s, "ReputationDetailFrame"), "closed to start");
    assert!(
        shown(&mut s, "ReputationBar2ReputationBarAtWarHighlight1"),
        "the at-war glow lights on the row itself (ReputationFrame.lua:224-230)"
    );

    click_center(&mut s, "ReputationBar2");

    assert!(shown(&mut s, "ReputationDetailFrame"), "the popup opened");
    assert_eq!(
        s.eval::<i64>("return GetSelectedFaction()").unwrap(),
        2,
        "on Ironforge's visible row"
    );
    assert_eq!(text_of(&mut s, "ReputationDetailFactionName"), "Ironforge");
    assert_eq!(
        text_of(&mut s, "ReputationDetailFactionDescription"),
        "About the Ironforge."
    );
    assert!(
        shown(&mut s, "ReputationBar2ReputationBarHighlight1"),
        "and the selected row keeps its glow with the mouse away"
    );

    let checked = |s: &mut UiScript, box_name: &str| {
        s.eval::<bool>(&format!("return {box_name}:GetChecked() and true or false"))
            .unwrap()
    };
    assert!(
        checked(&mut s, "ReputationDetailAtWarCheckBox"),
        "at war, so the box is ticked"
    );
    assert!(
        !s.eval::<bool>("return ReputationDetailAtWarCheckBox:IsEnabled() ~= 0")
            .unwrap(),
        "peace-forced, so the box is dead"
    );
    let (r, g, b) = s
        .eval::<(f32, f32, f32)>("return ReputationDetailAtWarCheckBoxText:GetTextColor()")
        .unwrap();
    assert_eq!(
        (r, g, b),
        (0.5, 0.5, 0.5),
        "and its label greys with it (GRAY_FONT_COLOR)"
    );
    assert!(!checked(&mut s, "ReputationDetailInactiveCheckBox"));
    assert!(!checked(&mut s, "ReputationDetailMainScreenCheckBox"));

    click_center(&mut s, "ReputationBar2");
    assert!(!shown(&mut s, "ReputationDetailFrame"), "clicked shut");

    // The whole row is the click target: 3.3.5's row button carries the name and the bar.
    let name_x = s
        .eval::<f32>(
            "return (ReputationBar2FactionName:GetLeft() + ReputationBar2FactionName:GetRight()) / 2",
        )
        .unwrap();
    let bar_left = s
        .eval::<f32>("return ReputationBar2ReputationBar:GetLeft()")
        .unwrap();
    assert!(
        name_x < bar_left,
        "the name really does sit left of the bar ({name_x} < {bar_left})"
    );
    let y = s
        .eval::<f32>("return (ReputationBar2:GetTop() + ReputationBar2:GetBottom()) / 2")
        .unwrap();
    assert_eq!(
        s.hit_test_name(name_x, y).as_deref(),
        Some("ReputationBar2"),
        "the row's hit rect covers the name"
    );
    s.mouse_move(name_x, y);
    s.mouse_button(name_x, y, "LeftButton", true);
    s.mouse_button(name_x, y, "LeftButton", false);
    assert!(
        shown(&mut s, "ReputationDetailFrame"),
        "and a click there opens the popup, the same as a click on the bar"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The At War box has a `<NormalTexture>` and no `<DisabledTexture>`, and is disabled for a
/// peace-forced faction (ReputationFrame.lua:115-120). `SetState` (`0x779790`) hides the old
/// texture only when the new state has one, so the box stays drawn, under a grey tick at war.
#[test]
fn the_at_war_box_keeps_its_art_while_it_is_disabled() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_page(&mut s);
    s.set_unit("player", Some(player(40)));
    let mut st = state();
    for e in &mut st.entries {
        match e.name.as_str() {
            "Ironforge" => e.can_toggle_at_war = false,
            "Stormwind" => {
                e.can_toggle_at_war = false;
                e.at_war = true;
            }
            _ => {}
        }
    }
    s.set_reputation(st);
    s.run(r#"ToggleCharacter("ReputationFrame")"#).unwrap();
    s.resolve();

    /// Every texture the At War box itself draws, in painter order.
    fn box_art(s: &UiScript) -> Vec<String> {
        s.extract()
            .iter()
            .filter(|q| {
                s.quad_owner_name(q.target).as_deref() == Some("ReputationDetailAtWarCheckBox")
            })
            .filter_map(|q| match &q.content {
                QuadContent::Texture { path: Some(p), .. } => Some(p.clone()),
                _ => None,
            })
            .collect()
    }
    let up = "Interface\\Buttons\\UI-CheckBox-Up".to_string();

    // Ironforge (row 2): peace-forced, at peace.
    click_center(&mut s, "ReputationBar2");
    assert_eq!(
        s.eval::<i64>("return ReputationDetailAtWarCheckBox:IsEnabled()")
            .unwrap(),
        0,
        "the box is disabled for a faction whose war flag is locked"
    );
    assert_eq!(
        box_art(&s),
        vec![up.clone()],
        "and it still draws its box — the sticky shown texture"
    );

    // Stormwind (row 3): peace-forced and at war.
    click_center(&mut s, "ReputationBar3");
    assert_eq!(
        box_art(&s),
        vec![
            up.clone(),
            "Interface\\Buttons\\UI-CheckBox-Check-Disabled".to_string()
        ],
        "a peace-forced faction at war keeps the box under its grey tick"
    );

    // Booty Bay (row 5), the control: toggleable.
    click_center(&mut s, "ReputationBar5");
    assert_eq!(
        s.eval::<i64>("return ReputationDetailAtWarCheckBox:IsEnabled()")
            .unwrap(),
        1
    );
    assert_eq!(box_art(&s), vec![up], "a live box is the same box");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Below the 3.3.5 cap (80) the watch bar stacks on the XP bar, 8 px in its own end art; at 80 it
/// replaces it, 13 px in the dwarf art, and the rested tick hides (ReputationFrame.lua:323-418).
#[test]
fn the_watch_bar_shows_the_watched_factions_progress_and_swaps_at_max_level() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_page(&mut s);
    crate::test_guids::push_player(&mut s, player(40));
    // The XP bar needs its range before the watch bar's update, or `TextStatusBar_UpdateTextString`
    // hides it (TextStatusBar.lua:76-82); the live client has XP from login.
    s.set_player_xp(0, 50_000);
    s.fire_event(
        "PLAYER_XP_UPDATE",
        vec![wrathplus_ui::script::ScriptValue::Str("player".into())],
    );
    assert!(
        !shown(&mut s, "ReputationWatchBar"),
        "nothing watched: the bar stays down"
    );

    // Ironforge is rep slot 20; watching it is a server field, so it rides the push.
    let mut st = state();
    st.watched = Some(20);
    s.set_reputation(st);
    s.fire_event("UPDATE_FACTION", vec![]);
    s.resolve();

    assert!(shown(&mut s, "ReputationWatchBar"), "the bar came up");
    assert_eq!(
        text_of(&mut s, "ReputationWatchStatusBarText"),
        "Ironforge 1000 / 6000",
        "name plus the NORMALIZED progress, the reference's own string"
    );
    let (r, g, b) = s
        .eval::<(f32, f32, f32)>("return ReputationWatchStatusBar:GetStatusBarColor()")
        .unwrap();
    assert_eq!((r, g, b), (0.0, 0.6, 0.1), "FACTION_BAR_COLORS[5] again");
    assert_eq!(
        s.eval::<(f64, f64)>("return ReputationWatchStatusBar:GetMinMaxValues()")
            .unwrap(),
        (0.0, 6000.0)
    );
    assert!(
        shown(&mut s, "MainMenuExpBar"),
        "below 80 the strip STACKS on the XP bar rather than replacing it"
    );
    assert!(
        shown(&mut s, "ReputationWatchBarTexture0"),
        "rep end art up"
    );
    assert!(!shown(&mut s, "ReputationXPBarTexture0"), "dwarf art down");
    assert_eq!(
        s.eval::<f64>("return ReputationWatchStatusBar:GetHeight()")
            .unwrap(),
        8.0
    );

    // A stacked bar makes the manage pass add each row's `reputation` offset (UIParent.lua:1257-1262,
    // :1787-1789), scaled by 9 at load (:1211-1214): `BATTLEFIELD_TAB_OFFSET_Y` is baseY 210 plus 9.
    assert_eq!(
        s.eval::<f64>("return BATTLEFIELD_TAB_OFFSET_Y").unwrap(),
        219.0,
        "the row lifts by its reputation offset while the stacked watch bar is up"
    );
    s.run("ReputationWatchBar:Hide() UIParent_ManageFramePositions()")
        .unwrap();
    assert_eq!(
        s.eval::<f64>("return BATTLEFIELD_TAB_OFFSET_Y").unwrap(),
        210.0,
        "and drops back to the base when it goes"
    );
    s.run("ReputationWatchBar:Show() UIParent_ManageFramePositions()")
        .unwrap();

    // Ding to 80: the reputation strip takes the XP bar's place, art and all.
    s.set_unit("player", Some(player(80)));
    s.fire_event(
        "PLAYER_LEVEL_UP",
        vec![wrathplus_ui::script::ScriptValue::Int(80)],
    );
    s.resolve();

    assert!(shown(&mut s, "ReputationWatchBar"), "still watching");
    assert!(
        !shown(&mut s, "MainMenuExpBar"),
        "at 80 the strip REPLACES the XP bar"
    );
    assert!(
        !shown(&mut s, "MainMenuBarMaxLevelBar"),
        "and the brass rail stays down — the watched bar is what fills that space"
    );
    assert!(
        !shown(&mut s, "ExhaustionTick"),
        "no XP strip, no rested tick"
    );
    assert!(shown(&mut s, "ReputationXPBarTexture0"), "dwarf art up");
    assert!(!shown(&mut s, "ReputationWatchBarTexture0"), "rep art down");
    assert_eq!(
        s.eval::<f64>("return ReputationWatchStatusBar:GetHeight()")
            .unwrap(),
        13.0
    );

    // Stop watching: at 80 with nothing watched the brass rail is what takes the XP bar's place.
    let mut st = state();
    st.watched = None;
    s.set_reputation(st);
    s.fire_event("UPDATE_FACTION", vec![]);
    s.resolve();
    assert!(!shown(&mut s, "ReputationWatchBar"));
    assert!(shown(&mut s, "MainMenuBarMaxLevelBar"), "the rail is back");
    assert!(!shown(&mut s, "MainMenuExpBar"));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The "Show as Experience Bar" box calls `SetWatchedFactionIndex`, then
/// `ReputationWatchBar_Update` (ReputationFrame.xml), but watching is not optimistic: the
/// index is the server field `PLAYER_FIELD_WATCHED_FACTION_INDEX`, with no client mirror
/// (`0x4d6b60`), so the bar comes up only when the descriptor update lands.
#[test]
fn the_show_as_experience_bar_box_sends_the_watch_and_the_server_brings_the_bar_up() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = shown_reputation_page();
    let _ = s.take_reputation_sends();

    // Ironforge: visible row 2, reputation slot 20.
    click_center(&mut s, "ReputationBar2");
    assert!(shown(&mut s, "ReputationDetailFrame"), "the popup opened");
    assert!(
        !s.eval::<bool>("return ReputationDetailMainScreenCheckBox:GetChecked() and true or false")
            .unwrap(),
        "nothing watched, so the box starts clear"
    );
    assert!(
        !shown(&mut s, "ReputationWatchBar"),
        "and the strip is down"
    );

    // The box ticks before its handler runs, as in the reference, so the handler takes the
    // `GetChecked()` branch that watches.
    click_center(&mut s, "ReputationDetailMainScreenCheckBox");
    assert_eq!(
        s.take_reputation_sends(),
        [wrathplus_ui::script::ReputationSend::Watch(Some(20))],
        "one send, carrying Ironforge's reputation SLOT — not its visible row"
    );
    assert!(
        !shown(&mut s, "ReputationWatchBar"),
        "and nothing on screen moved: watching is not optimistic"
    );

    // The server's answer: the descriptor update arrives as a fresh push, and the event with it.
    let mut watched = state();
    watched.watched = Some(20);
    s.set_reputation(watched);
    s.fire_event("UPDATE_FACTION", vec![]);
    s.resolve();

    assert!(
        s.eval::<bool>("return ReputationDetailMainScreenCheckBox:GetChecked() and true or false")
            .unwrap(),
        "NOW the box reads ticked — off the server's field, not off the click"
    );
    assert!(shown(&mut s, "ReputationWatchBar"), "and the strip is up");
    assert_eq!(
        text_of(&mut s, "ReputationWatchStatusBarText"),
        "Ironforge 1000 / 6000"
    );
    // 3.3.5 marks the watch only in the detail box (ReputationFrame.lua:256-260); 1.12's row
    // checkmark is gone.

    // Untick: stock passes 0 for none, and the send is `None`, not slot 0, which is a real faction.
    click_center(&mut s, "ReputationDetailMainScreenCheckBox");
    assert_eq!(
        s.take_reputation_sends(),
        [wrathplus_ui::script::ReputationSend::Watch(None)]
    );
    let mut cleared = state();
    cleared.watched = None;
    s.set_reputation(cleared);
    s.fire_event("UPDATE_FACTION", vec![]);
    s.resolve();
    assert!(!shown(&mut s, "ReputationWatchBar"), "the strip goes down");
    assert!(!s
        .eval::<bool>("return ReputationDetailMainScreenCheckBox:GetChecked() and true or false")
        .unwrap());
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
