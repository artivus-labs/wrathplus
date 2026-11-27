//! The raid tab's saved-instance panel, its ready-check popup and the drag's edge cases.

use wrathplus_ui::script::{PartyRequest, UiScript};

use super::{open_raid_tab, push_raid, row, setup, text_of, twelve, visible};

// ── Raid Info ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_player_with_no_lockouts_loses_the_raid_info_button_on_the_second_answer() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);

    // Answer one only arms the latch; `RaidFrame_OnLoad` leaves the button as it loaded.
    s.fire_event("UPDATE_INSTANCE_INFO", Vec::new());
    assert_eq!(
        s.eval::<i64>("return RaidFrame.hasRaidInfo or 0").unwrap(),
        1,
        "the first answer only arms the latch"
    );

    s.fire_event("UPDATE_INSTANCE_INFO", Vec::new());
    assert_eq!(
        s.eval::<i64>("return RaidFrameRaidInfoButton:IsEnabled()")
            .unwrap(),
        0,
        "an empty lockout list is a dead button"
    );

    // The button is dead to a real click through the pointer, not merely drawn grey.
    let (bx, by) = {
        let l: f32 = s.eval("return RaidFrameRaidInfoButton:GetLeft()").unwrap();
        let r: f32 = s.eval("return RaidFrameRaidInfoButton:GetRight()").unwrap();
        let t: f32 = s.eval("return RaidFrameRaidInfoButton:GetTop()").unwrap();
        let b: f32 = s
            .eval("return RaidFrameRaidInfoButton:GetBottom()")
            .unwrap();
        ((l + r) / 2.0, (t + b) / 2.0)
    };
    s.mouse_button(bx, by, "LeftButton", true);
    s.mouse_button(bx, by, "LeftButton", false);
    s.tick(0.016);
    assert!(
        !visible(&s, "RaidInfoFrame"),
        "a disabled button does not open the panel"
    );
}

#[test]
fn the_ready_check_popup_opens_and_answers() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());
    assert!(!visible(&s, "ReadyCheckFrame"), "nothing to answer yet");

    // `READY_CHECK(initiator, timeLeft)` (ReadyCheck.lua:11-22,29-32), named, over a live check
    // (`ConfirmReadyCheck` answers only inside one, `0x5740c0`).
    let member1 = twelve()[1].guid;
    let ready_check = |s: &mut UiScript| {
        s.wrath_ready_check_start(member1);
        let name = wrathplus_ui::script::ScriptValue::Str("Member1".into());
        s.fire_event(
            "READY_CHECK",
            vec![name, wrathplus_ui::script::ScriptValue::Int(30)],
        );
    };
    ready_check(&mut s);
    assert!(visible(&s, "ReadyCheckFrame"), "the check opens it");
    assert!(
        text_of(&s, "ReadyCheckFrameText").starts_with("Member1 has initiated a ready check."),
        "named for the initiator: {:?}",
        text_of(&s, "ReadyCheckFrameText")
    );

    s.run("ReadyCheckFrameYesButton:Click()").unwrap();
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::ReadyCheckAnswer(true)]
    );
    assert!(!visible(&s, "ReadyCheckFrame"));

    ready_check(&mut s);
    s.run("ReadyCheckFrameNoButton:Click()").unwrap();
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::ReadyCheckAnswer(false)],
        "No passes no argument at all, and absent must mean not-ready"
    );

    // The check's end closes it (READY_CHECK_FINISHED, ReadyCheck.lua:33-39).
    ready_check(&mut s);
    assert!(visible(&s, "ReadyCheckFrame"));
    s.fire_event("READY_CHECK_FINISHED", Vec::new());
    assert!(!visible(&s, "ReadyCheckFrame"));
}

/// No release reaches a gesture once the OS pointer leaves the window, so
/// `UiScript::pointer_left_window` ends the drag: `OnDragStop` runs, the row goes home, and the
/// next row still drags.
#[test]
fn a_drag_carried_off_the_window_edge_ends_instead_of_gluing_the_row_to_the_cursor() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    push_raid(&mut s, twelve());

    let left_of =
        |s: &UiScript, f: &str| -> f32 { s.eval(&format!("return {f}:GetLeft()")).unwrap() };
    let centre = |s: &UiScript, f: &str| -> (f32, f32) {
        let l: f32 = s.eval(&format!("return {f}:GetLeft()")).unwrap();
        let r: f32 = s.eval(&format!("return {f}:GetRight()")).unwrap();
        let t: f32 = s.eval(&format!("return {f}:GetTop()")).unwrap();
        let b: f32 = s.eval(&format!("return {f}:GetBottom()")).unwrap();
        ((l + r) / 2.0, (t + b) / 2.0)
    };

    let home = left_of(&s, "RaidGroupButton7");
    let (fx, fy) = centre(&s, "RaidGroupButton7");
    s.mouse_button(fx, fy, "LeftButton", true);
    s.mouse_move(fx + 40.0, fy + 40.0); // past the threshold ⇒ OnDragStart ⇒ StartMoving
    s.tick(0.016);
    assert_eq!(
        s.eval::<String>("return MOVING_RAID_MEMBER:GetName()")
            .unwrap(),
        "RaidGroupButton7",
        "the drag is in flight"
    );

    s.pointer_left_window();
    s.tick(0.016);
    assert!(
        s.eval::<bool>("return MOVING_RAID_MEMBER == nil").unwrap(),
        "the pane's own drag state is cleared by the OnDragStop the abandon fires"
    );
    assert_eq!(
        left_of(&s, "RaidGroupButton7"),
        home,
        "the row springs back to its slot — a drop on nothing is not a move"
    );

    s.mouse_move(400.0, 300.0);
    s.tick(0.016);
    s.mouse_move(500.0, 200.0);
    s.tick(0.016);
    assert_eq!(
        left_of(&s, "RaidGroupButton7"),
        home,
        "…and it stays there however far the cursor travels"
    );

    let (gx, gy) = centre(&s, "RaidGroupButton8");
    let (tx, ty) = centre(&s, "RaidGroup5Slot1");
    s.mouse_button(gx, gy, "LeftButton", true);
    s.mouse_move(gx + 10.0, gy + 10.0);
    s.tick(0.016);
    s.mouse_move(tx, ty);
    s.tick(0.016);
    s.mouse_button(tx, ty, "LeftButton", false);
    assert_eq!(
        s.take_party_requests(),
        vec![PartyRequest::SetSubgroup { index: 8, group: 5 }],
        "the row after the abandoned one still moves"
    );
    assert!(
        s.errors().is_empty(),
        "and nothing raised: {:?}",
        s.errors()
    );
}

/// Three whole gestures through the real pointer path, with the roster echo `/partytest raid`
/// supplies in between.
#[test]
fn one_drag_does_not_cost_the_next_one() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    open_raid_tab(&mut s);
    let mut raid = vec![row("Me", 2, 1, "Warrior", true, false)];
    for i in 1..25u32 {
        raid.push(row(
            &format!("Member{i}"),
            0,
            i / 5 + 1,
            "Priest",
            true,
            false,
        ));
    }
    push_raid(&mut s, raid.clone());

    let centre = |s: &UiScript, f: &str| -> (f32, f32) {
        let l: f32 = s.eval(&format!("return {f}:GetLeft()")).unwrap();
        let r: f32 = s.eval(&format!("return {f}:GetRight()")).unwrap();
        let t: f32 = s.eval(&format!("return {f}:GetTop()")).unwrap();
        let b: f32 = s.eval(&format!("return {f}:GetBottom()")).unwrap();
        ((l + r) / 2.0, (t + b) / 2.0)
    };

    for (row_index, group) in [(7u32, 6u32), (8, 6), (9, 7)] {
        let from = format!("RaidGroupButton{row_index}");
        let seat = if group == 6 && row_index == 8 { 2 } else { 1 };
        let to = format!("RaidGroup{group}Slot{seat}");
        let (fx, fy) = centre(&s, &from);
        let (tx, ty) = centre(&s, &to);
        s.mouse_button(fx, fy, "LeftButton", true);
        s.mouse_move(fx + 10.0, fy + 10.0);
        s.tick(0.016);
        s.mouse_move(tx, ty);
        s.tick(0.016);
        s.mouse_button(tx, ty, "LeftButton", false);
        s.tick(0.016);
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::SetSubgroup {
                index: row_index,
                group
            }],
            "drag {row_index} onto group {group}"
        );
        assert!(s.errors().is_empty(), "{from}: {:?}", s.errors());
        // The echo, so the next drag starts from a repainted grid.
        raid[row_index as usize - 1].subgroup = group - 1;
        push_raid(&mut s, raid.clone());
    }
}
