//! The social window's raid tab (`RaidFrame.xml` and `Blizzard_RaidUI`): the 8x5 grid's seating
//! and colouring, the drag, the row menu, the ready check and the saved-instance panel.

use wrathplus_ui::script::{
    PartyMemberInfo, PartyRequest, PartyState, RaidMemberInfo, SelectionRequest, UiScript,
    UnitState,
};

#[path = "raid_info_tests.rs"]
mod info;

fn setup() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    super::test_ui::load_social_ui(&mut s);
    // The player object, which the party verbs need (12340 `0x51a6e0` and kin).
    s.set_unit_guids(&wrathplus_ui::script::UnitGuids {
        player: 1,
        ..Default::default()
    });
    s
}

fn visible(s: &UiScript, frame: &str) -> bool {
    s.eval::<bool>(&format!("return {frame}:IsVisible()"))
        .unwrap()
}

fn text_of(s: &UiScript, region: &str) -> String {
    s.eval::<String>(&format!("return {region}:GetText() or \"\""))
        .unwrap()
}

/// One raid row; `rank` is 2 leader, 1 assistant, 0 member. `subgroup` is the 1-based number the
/// pane shows: the record stores it 0-based and the binding adds one (`0x4bb61a`), so a 1-based
/// record would seat every row one group to the right.
fn row(
    name: &str,
    rank: u32,
    subgroup: u32,
    class: &str,
    online: bool,
    dead: bool,
) -> RaidMemberInfo {
    RaidMemberInfo {
        name: name.to_string(),
        // Unique per name: a shared guid would resolve one member's token to another's row.
        guid: name
            .bytes()
            .fold(0xF000, |g: u64, b| g.wrapping_mul(131) ^ u64::from(b)),
        rank,
        subgroup: subgroup - 1,
        level: 60,
        class: Some(class.to_string()),
        class_file: Some(class.to_uppercase()),
        zone: Some("Molten Core".to_string()),
        online,
        ninth: dead,
    }
}

/// Pushes a roster and fires its event, as `ui_party`'s `feed_party` does.
fn push_raid(s: &mut UiScript, raid: Vec<RaidMemberInfo>) {
    // `members` is our own subgroup's slice; non-empty, it makes `IsPartyLeader()` and
    // `IsRaidLeader()` answer 1 for `leader_index` 0.
    let members = raid
        .iter()
        .skip(1)
        .take(4)
        .map(|r| PartyMemberInfo {
            name: r.name.clone(),
            guid: r.guid,
        })
        .collect();
    // The guid table as the app seats it: the player is row 1, `raidN` is row N.
    let me = raid.first().map_or(0, |r| r.guid);
    s.set_unit_guids(&wrathplus_ui::script::UnitGuids {
        player: me,
        raid: raid.iter().map(|r| r.guid).collect(),
        ..Default::default()
    });
    s.set_party(PartyState {
        members,
        leader_index: 0,
        leader_guid: me, // the player leads
        own_guid: me,
        raid: raid.clone(),
        loot_method: "group".into(),
        master_looter: None,
        master_looter_guid: 0,
        loot_threshold: 2,
    });
    // Each row's `raidN` snapshot, as the app seats a member it holds: `UnitPopup_ShowMenu` names
    // the menu by `UnitName(unit)` (UnitPopup.lua).
    for i in 0..40 {
        let unit = raid.get(i).map(|r| UnitState {
            exists: true,
            has_object: true,
            name: Some(r.name.clone()),
            guid: r.guid,
            is_player: true,
            player_controlled: true,
            ..UnitState::default()
        });
        s.set_unit(&format!("raid{}", i + 1), unit);
    }
    s.fire_event("RAID_ROSTER_UPDATE", Vec::new());
}

/// A 12-member raid across three subgroups, led by us, with row 2 an assistant.
fn twelve() -> Vec<RaidMemberInfo> {
    let mut raid = vec![row("Me", 2, 1, "Warrior", true, false)];
    for i in 1..12u32 {
        let subgroup = i / 4 + 1;
        raid.push(row(
            &format!("Member{i}"),
            if i == 1 { 1 } else { 0 },
            subgroup,
            "Priest",
            true,
            false,
        ));
    }
    raid
}

/// Opens the tab.
fn open_raid_tab(s: &mut UiScript) {
    // The fifth tab in 3.3.5: Friends, Who, Guild, Chat, Raid (FriendsFrame.xml:4729-4818).
    s.run("ToggleFriendsFrame(5)").unwrap();
}

// ── The tab ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_fifth_tab_opens_the_raid_pane() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    assert_eq!(
        s.eval::<i64>("return FriendsFrame.numTabs").unwrap(),
        5,
        "PanelTemplates_SetNumTabs(self, 5) (FriendsFrame.lua:194)"
    );
    open_raid_tab(&mut s);
    assert!(visible(&s, "RaidFrame"), "the pane opens");
    assert_eq!(text_of(&s, "FriendsFrameTitleText"), "Raid");
    for other in [
        "FriendsListFrame",
        "IgnoreListFrame",
        "WhoFrame",
        "GuildFrame",
    ] {
        assert!(!visible(&s, other), "{other} goes away");
    }
    s.run("FriendsFrameTab1:Click()").unwrap();
    assert!(!visible(&s, "RaidFrame"));
    s.run("FriendsFrameTab5:Click()").unwrap();
    assert!(visible(&s, "RaidFrame"), "the tab button opens it too");
}

/// `FriendsFrame_OnHide` hides `RaidInfoFrame` too (`FriendsFrame.lua:136`).
#[test]
fn hiding_the_window_closes_the_raid_info_panel() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    s.run("RaidInfoFrame:Show()").unwrap();
    assert!(visible(&s, "RaidInfoFrame"));
    s.run("HideUIPanel(FriendsFrame)").unwrap();
    assert!(
        !visible(&s, "RaidInfoFrame"),
        "the flyout goes with the window"
    );
}

// ── The not-in-a-raid state ──────────────────────────────────────────────────────────────────────

/// Out of a raid the pane is the blurb and Convert To Raid, live only for a party's leader
/// (`RaidFrame.lua:62-69`).
#[test]
fn convert_to_raid_is_live_only_for_a_party_leader() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    assert!(visible(&s, "RaidFrameRaidDescription"), "solo: the blurb");
    assert!(visible(&s, "RaidFrameConvertToRaidButton"));
    assert_eq!(
        s.eval::<i64>("return RaidFrameConvertToRaidButton:IsEnabled()")
            .unwrap(),
        0,
        "solo there is no party to convert"
    );
    assert!(
        !visible(&s, "RaidFrameReadyCheckButton"),
        "and no raid verbs"
    );

    let party = |leader_index, leader_guid| PartyState {
        members: vec![PartyMemberInfo {
            name: "Alice".into(),
            guid: 0xA,
        }],
        leader_index,
        leader_guid,
        ..Default::default()
    };
    s.set_player_req_state(wrathplus_ui::script::PlayerReqState {
        level: 10,
        ..Default::default()
    });
    s.set_party(party(0, 1)); // we lead (`0x52ccd0`) at level 10 (`0x574a59`)
    s.fire_event("PARTY_MEMBERS_CHANGED", Vec::new());
    assert_eq!(
        s.eval::<i64>("return RaidFrameConvertToRaidButton:IsEnabled()")
            .unwrap(),
        1,
        "a party leader can convert"
    );
    s.run("RaidFrameConvertToRaidButton:Click()").unwrap();
    assert!(
        s.take_party_requests()
            .contains(&PartyRequest::ConvertToRaid),
        "and the button really sends it"
    );

    s.set_party(party(1, 0xA));
    s.fire_event("PARTY_LEADER_CHANGED", Vec::new());
    assert_eq!(
        s.eval::<i64>("return RaidFrameConvertToRaidButton:IsEnabled()")
            .unwrap(),
        0,
        "a party member cannot"
    );
}

// ── The grid ─────────────────────────────────────────────────────────────────────────────────────

/// Row order and slot order differ; every drag, kick and menu action addresses a row by row index.
#[test]
fn the_grid_seats_each_row_in_its_own_subgroups_next_free_slot() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());

    // Groups 1-3 hold 4/4/4; the roster is 12 with us at row 1 in group 1.
    assert_eq!(
        s.eval::<String>("return RaidGroupButton1.slot:GetName()")
            .unwrap(),
        "RaidGroup1Slot1",
        "row 1 (us) takes group 1's first seat"
    );
    assert_eq!(
        s.eval::<String>("return RaidGroupButton5.slot:GetName()")
            .unwrap(),
        "RaidGroup2Slot1",
        "row 5 is the first of group 2 — NOT group 1's fifth seat"
    );
    assert_eq!(
        s.eval::<String>("return RaidGroupButton12.slot:GetName()")
            .unwrap(),
        "RaidGroup3Slot4"
    );
    // The slot knows its occupant, which the drop reads to choose move or swap.
    assert_eq!(
        s.eval::<String>("return RaidGroup2Slot1.button").unwrap(),
        "RaidGroupButton5"
    );

    assert_eq!(text_of(&s, "RaidGroupButton1Name"), "Me");
    // The rank is an icon in 3.3.5 (`RaidRoleIconTemplate`, Blizzard_RaidUI.xml:145).
    let rank = |s: &UiScript, n: u32| {
        s.eval::<Option<String>>(&format!(
            "return RaidGroupButton{n}Rank:IsShown() and RaidGroupButton{n}RankTexture:GetTexture() or nil"
        ))
        .unwrap()
    };
    assert_eq!(
        rank(&s, 1).as_deref(),
        Some("Interface\\GroupFrame\\UI-Group-LeaderIcon")
    );
    assert_eq!(
        rank(&s, 2).as_deref(),
        Some("Interface\\GroupFrame\\UI-Group-AssistantIcon")
    );
    assert_eq!(rank(&s, 3), None, "a plain member has none");
    assert_eq!(text_of(&s, "RaidGroupButton1Level"), "60");

    assert!(visible(&s, "RaidGroupButton12"), "the last real row shows");
    assert!(
        !visible(&s, "RaidGroupButton13"),
        "and the first empty one does not"
    );
    assert!(
        visible(&s, "RaidGroup8"),
        "all eight groups show while in a raid"
    );

    push_raid(&mut s, Vec::new());
    assert!(!visible(&s, "RaidGroup1"));
    assert!(!visible(&s, "RaidGroupButton1"));
    assert!(visible(&s, "RaidFrameRaidDescription"));
}

/// Offline grey is tested first, then dead red, then the class colour, on all three columns
/// (`Blizzard_RaidUI.lua:135-152`).
#[test]
fn a_rows_colour_is_offline_then_dead_then_class() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(
        &mut s,
        vec![
            row("Me", 2, 1, "Warrior", true, false),
            row("Corpse", 0, 1, "Mage", true, true),
            row("Gone", 0, 1, "Mage", false, false),
            row("GoneDead", 0, 1, "Mage", false, true),
        ],
    );
    let color = |button: &str| {
        s.eval::<(f32, f32, f32)>(&format!(
            "local r, g, b = {button}Name:GetTextColor() return r, g, b"
        ))
        .unwrap()
    };
    let round = |(r, g, b): (f32, f32, f32)| {
        (
            (r * 100.0).round() / 100.0,
            (g * 100.0).round() / 100.0,
            (b * 100.0).round() / 100.0,
        )
    };
    // WARRIOR is 0.78, 0.61, 0.43 (`RAID_CLASS_COLORS`, `Fonts.xml:52`).
    let warrior: (f32, f32, f32) = s
        .eval("return RAID_CLASS_COLORS.WARRIOR.r, RAID_CLASS_COLORS.WARRIOR.g, RAID_CLASS_COLORS.WARRIOR.b")
        .unwrap();
    assert_eq!(
        round(color("RaidGroupButton1")),
        round(warrior),
        "alive: class colour"
    );
    assert_eq!(
        round(color("RaidGroupButton2")),
        (1.0, 0.1, 0.1),
        "dead: RED_FONT_COLOR"
    );
    assert_eq!(
        round(color("RaidGroupButton3")),
        (0.5, 0.5, 0.5),
        "offline: GRAY_FONT_COLOR"
    );
    assert_eq!(
        round(color("RaidGroupButton4")),
        (0.5, 0.5, 0.5),
        "offline AND dead is grey — offline is tested first"
    );
    // Name and level share the row's colour; the class column is a Button, whose text colour
    // this engine cannot read back.
    assert_eq!(
        round(color("RaidGroupButton2")),
        round(
            s.eval::<(f32, f32, f32)>(
                "local r, g, b = RaidGroupButton2Level:GetTextColor() return r, g, b"
            )
            .unwrap()
        ),
        "name and level share the row's colour"
    );
}

/// `UNIT_LEVEL` repaints one `raidN` row without the roster event; `UNIT_HEALTH` never does,
/// since stock reads `local arg1 = "..."`, a literal, before its `raid([0-9]+)` match
/// (`Blizzard_RaidUI.lua:166-180`), so a death waits for the next roster update.
#[test]
fn a_death_waits_for_the_roster_and_a_level_repaints_its_row() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    let mut raid = twelve();
    push_raid(&mut s, raid.clone());
    assert_eq!(text_of(&s, "RaidGroupButton3Level"), "60");

    // The row dies; only the per-unit event fires.
    raid[2].ninth = true;
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Member1".into(),
            guid: raid[1].guid,
        }],
        leader_index: 0,
        raid: raid.clone(),
        ..Default::default()
    });
    s.fire_event(
        "UNIT_HEALTH",
        vec![wrathplus_ui::script::ScriptValue::Str("raid3".into())],
    );
    let green = |s: &UiScript| {
        s.eval::<f32>("local r, g = RaidGroupButton3Name:GetTextColor() return g")
            .unwrap()
    };
    assert!(
        green(&s) > 0.2,
        "UNIT_HEALTH alone repaints nothing ({})",
        green(&s)
    );
    s.fire_event("RAID_ROSTER_UPDATE", Vec::new());
    assert!(
        green(&s) < 0.2,
        "the roster update paints it red ({})",
        green(&s)
    );

    // A unit event for a token that is not `raidN` is ignored.
    s.fire_event(
        "UNIT_HEALTH",
        vec![wrathplus_ui::script::ScriptValue::Str("target".into())],
    );
    s.fire_event(
        "UNIT_LEVEL",
        vec![wrathplus_ui::script::ScriptValue::Str("player".into())],
    );
    assert_eq!(
        text_of(&s, "RaidGroupButton3Level"),
        "60",
        "and nothing else moved"
    );
}

// ── The management buttons ───────────────────────────────────────────────────────────────────────

/// The ready check is the leader's or an assistant's (`RaidFrameReadyCheckButton_Update`,
/// `Blizzard_RaidUI.lua:1502`); 3.3.5 has no Add Member button.
#[test]
fn ready_check_is_the_leaders() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());
    assert!(
        visible(&s, "RaidFrameReadyCheckButton"),
        "we lead, so we may ask"
    );
    assert!(
        !visible(&s, "RaidFrameConvertToRaidButton"),
        "already a raid"
    );
    assert!(!visible(&s, "RaidFrameRaidDescription"));

    s.run("RaidFrameReadyCheckButton:Click()").unwrap();
    assert!(s
        .take_party_requests()
        .contains(&PartyRequest::ReadyCheckStart));

    let mut raid = twelve(); // another leads, and our rank 0 is no officer (`0x5732b0`)
    raid[0].rank = 0;
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Member1".into(),
            guid: raid[1].guid,
        }],
        leader_index: 1,
        leader_guid: raid[1].guid,
        raid,
        ..Default::default()
    });
    s.fire_event("RAID_ROSTER_UPDATE", Vec::new());
    assert!(!visible(&s, "RaidFrameReadyCheckButton"));
}

// ── The drag ─────────────────────────────────────────────────────────────────────────────────────

/// An empty slot in another group moves, an occupied one swaps, anything else springs the row home
/// (`Blizzard_RaidUI.lua:257-273`); `TARGET_RAID_SLOT` stands in for the hover sweep.
#[test]
fn dragging_a_row_moves_swaps_or_springs_back() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());

    let drag = |s: &UiScript, button: &str, onto: &str| {
        s.run(&format!(
            "MOVING_RAID_MEMBER = {button}; TARGET_RAID_SLOT = {onto}; \
             RaidGroupButton_OnDragStop({button})"
        ))
        .unwrap();
    };

    // Row 12 (group 3, seat 4) onto group 3's fifth seat, its own group: nothing goes out.
    drag(&s, "RaidGroupButton12", "RaidGroup3Slot5");
    assert!(
        s.take_party_requests().is_empty(),
        "a drop inside the row's own group is not a move"
    );

    // Row 12 onto group 5's empty first seat: a move, with the 1-based subgroup.
    drag(&s, "RaidGroupButton12", "RaidGroup5Slot1");
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::SetSubgroup {
            index: 12,
            group: 5
        }]
    );

    // Row 12 onto group 1's second seat, which row 2 holds: a swap, by the two row indices.
    drag(&s, "RaidGroupButton12", "RaidGroup1Slot2");
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::SwapSubgroup {
            index: 12,
            other: 2
        }]
    );
}

/// `OnDragStart` and `OnDragStop` both return for a non-leader (`Blizzard_RaidUI.lua:236`, `:248`).
#[test]
fn only_the_leader_may_drag() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    let raid = twelve();
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Member1".into(),
            guid: raid[1].guid,
        }],
        leader_index: 1,
        raid,
        ..Default::default()
    });
    s.fire_event("RAID_ROSTER_UPDATE", Vec::new());
    s.run(
        "MOVING_RAID_MEMBER = RaidGroupButton12; TARGET_RAID_SLOT = RaidGroup5Slot1; \
         RaidGroupButton_OnDragStop(RaidGroupButton12)",
    )
    .unwrap();
    assert!(
        s.take_party_requests().is_empty(),
        "a member's drag sends nothing"
    );
}

// ── The row's click and menu ─────────────────────────────────────────────────────────────────────

/// The rows are secure unit buttons with no `unit` attribute (`Blizzard_RaidUI.xml:104,244-250`,
/// no `SetAttribute` on them), so a left click targets no one.
#[test]
fn left_clicking_a_row_targets_no_one() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());
    s.run("RaidGroupButton7:Click(\"LeftButton\")").unwrap();
    assert_eq!(s.take_selection_requests(), Vec::<SelectionRequest>::new());
}

/// The RAID menu's rank rules re-read the row's rank by row index (`UnitPopup.lua:382-401`).
#[test]
fn the_row_menu_offers_the_rank_verbs_by_rank() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());

    s.run("RaidGroupButton3:Click(\"RightButton\")").unwrap();
    assert!(
        s.eval::<bool>("return DropDownList1:IsVisible()").unwrap(),
        "the menu opens"
    );
    let shown = |s: &UiScript, key: &str| {
        s.eval::<bool>(&format!(
            "for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
                 local b = getglobal(\"DropDownList1Button\"..i) \
                 if b and b:IsShown() and b.value == \"{key}\" then return true end \
             end return false"
        ))
        .unwrap()
    };
    assert!(
        shown(&s, "RAID_PROMOTE"),
        "a plain member can be made an assistant"
    );
    assert!(!shown(&s, "RAID_DEMOTE"), "and cannot be demoted from one");
    assert!(shown(&s, "RAID_LEADER"), "and can be handed the lead");
    assert!(shown(&s, "RAID_REMOVE"));

    // Row 2 is the assistant: demote, not promote.
    s.run("HideDropDownMenu(1) RaidGroupButton2:Click(\"RightButton\")")
        .unwrap();
    assert!(shown(&s, "RAID_DEMOTE"));
    assert!(!shown(&s, "RAID_PROMOTE"));

    s.run("HideDropDownMenu(1) RaidGroupButton1:Click(\"RightButton\")")
        .unwrap();
    assert!(!shown(&s, "RAID_LEADER"));
    assert!(
        !shown(&s, "RAID_REMOVE"),
        "the leader cannot be kicked, not even by themself"
    );
}

/// The promote goes by name, and so does the kick: 3.3.5 calls `UninviteUnit(fullname)`, which
/// the binding sends as the member's guid (`UnitPopup.lua:1347-1348`).
#[test]
fn the_menu_verbs_queue_the_right_requests() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    let raid = twelve();
    push_raid(&mut s, raid.clone());
    let click = |s: &UiScript, key: &str| {
        s.run(&format!(
            "for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
                 local b = getglobal(\"DropDownList1Button\"..i) \
                 if b and b:IsShown() and b.value == \"{key}\" then b:Click() return end \
             end error(\"{key} not shown\")"
        ))
        .unwrap();
    };
    s.run("RaidGroupButton3:Click(\"RightButton\")").unwrap();
    click(&s, "RAID_PROMOTE");
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::AssistantLeader {
            name: "Member2".into(),
            grant: true
        }]
    );

    s.run("HideDropDownMenu(1) RaidGroupButton3:Click(\"RightButton\")")
        .unwrap();
    click(&s, "RAID_REMOVE");
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::UninviteGuid {
            guid: raid[2].guid,
            reason: String::new()
        }],
        "raid3 is Member2"
    );
}
