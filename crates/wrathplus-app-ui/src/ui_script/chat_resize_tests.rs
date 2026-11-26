//! The chat window's move, resize and lock, driven from the mouse through the stock 3.3.5 frames:
//! one `$parentResizeButton` grip (FloatingChatFrame.xml:539-570) and a tab drag that
//! `FCFTab_OnUpdate` ends once the button is up (FloatingChatFrame.lua:1164-1185).

use wrathplus_ui::script::UiScript;

use super::test_ui::load_ui as load_xml;

fn chat_ui() -> UiScript {
    let mut s = UiScript::new().unwrap();
    for file in [
        "Interface\\FrameXML\\GlobalStrings.lua",
        "Interface\\FrameXML\\Fonts.xml",
        "Interface\\FrameXML\\BasicControls.xml",
        r"Interface\FrameXML\UIParent.xml",
        r"Interface\FrameXML\MoneyFrame.lua",
        r"Interface\FrameXML\MoneyFrame.xml",
        "Interface\\FrameXML\\GameTooltip.xml",
        "Interface\\FrameXML\\UIMenu.xml", // the kit ChatMenu/EmoteMenu/VoiceMacroMenu build from
        "Interface\\FrameXML\\UIDropDownMenu.xml",
        r"Interface\FrameXML\UIPanelTemplates.lua",
        r"Interface\FrameXML\UIPanelTemplates.xml",
        "Interface\\FrameXML\\StaticPopup.xml",
        "Interface\\FrameXML\\TextStatusBar.lua",
        "Interface\\FrameXML\\TextStatusBar.xml",
        // A tab-drag stop's `FCF_ValidateChatFramePosition` reads `MainMenuBar:GetHeight()`.
        "Interface\\FrameXML\\MainMenuBar.xml",
        "Interface\\FrameXML\\Cooldown.xml",
        "Interface\\FrameXML\\ActionButtonTemplate.xml",
        "Interface\\FrameXML\\ChatFrame.xml",
        "Interface\\FrameXML\\FloatingChatFrame.xml",
        "Interface\\FrameXML\\ColorPickerFrame.xml",
    ] {
        load_xml(&mut s, file);
    }
    super::fire_chat_login(&mut s);
    s.set_screen_size(1600.0, 900.0);
    s.resolve();
    s
}

fn centre(s: &mut UiScript, frame: &str) -> (f32, f32) {
    s.resolve();
    let (x, y): (f64, f64) = s
        .eval(&format!("return {frame}:GetCenter()"))
        .unwrap_or_else(|e| panic!("{frame}:GetCenter(): {e}"));
    (x as f32, y as f32)
}

fn width(s: &mut UiScript) -> f32 {
    s.resolve();
    s.eval::<f64>("return ChatFrame1:GetWidth()").unwrap() as f32
}

fn height(s: &mut UiScript) -> f32 {
    s.resolve();
    s.eval::<f64>("return ChatFrame1:GetHeight()").unwrap() as f32
}

fn left(s: &mut UiScript) -> f32 {
    s.resolve();
    s.eval::<f64>("return ChatFrame1:GetLeft()").unwrap() as f32
}

fn right_click(s: &mut UiScript, frame: &str) {
    let (x, y) = centre(s, frame);
    s.mouse_button(x, y, "RightButton", true);
    s.mouse_button(x, y, "RightButton", false);
    s.resolve();
}

fn left_click(s: &mut UiScript, frame: &str) {
    let (x, y) = centre(s, frame);
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
    s.resolve();
}

/// A stationary hover fades the window in (`FCF_OnUpdate`, FloatingChatFrame.lua:1074-1083).
fn reveal(s: &mut UiScript) {
    let (cx, cy) = centre(s, "ChatFrame1");
    s.mouse_move(cx, cy);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
}

/// Press the tab, cross the drag threshold, move, release, then tick once: the tab's `OnUpdate`
/// sees the button up and stops the move (FloatingChatFrame.lua:1181-1185).
fn drag_tab(s: &mut UiScript, tab: &str, dx: f32, dy: f32) {
    reveal(s);
    let (tx, ty) = centre(s, tab);
    s.mouse_button(tx, ty, "LeftButton", true);
    s.mouse_move(tx + 5.0, ty);
    s.mouse_move(tx + dx, ty + dy);
    s.mouse_button(tx + dx, ty + dy, "LeftButton", false);
    s.tick(0.016);
    s.resolve();
}

const GRIP: &str = "ChatFrame1ResizeButton";

/// A window's alpha byte until a setter moves it, `0x28` (`0x501813`), as `GetChatWindowInfo` reads it.
const STOCK_ALPHA: f64 = (40.0f32 * (1.0f32 / 255.0f32)) as f64;

fn grab_grip(s: &mut UiScript, grip: &str) -> (f32, f32) {
    let (x, y) = centre(s, grip);
    s.mouse_button(x, y, "LeftButton", true);
    s.resolve();
    (x, y)
}

fn shown(s: &UiScript, frame: &str) -> bool {
    s.eval::<bool>(&format!("return {frame}:IsShown() == 1"))
        .unwrap()
}

/// Both dock windows ship locked, and `FCF_SetLocked` hides a locked window's grip
/// (FloatingChatFrame.lua:950-959).
#[test]
fn both_dock_windows_ship_locked_without_a_grip() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let s = chat_ui();
    for i in 1..=2 {
        assert!(
            s.eval::<bool>(&format!("return ChatFrame{i}.isLocked ~= nil"))
                .unwrap(),
            "window {i} is seated locked"
        );
        assert!(
            !shown(&s, &format!("ChatFrame{i}ResizeButton")),
            "a locked window hides its grip"
        );
    }
    assert_eq!(
        s.eval::<i64>("local _,_,_,_,_,_,_,locked = GetChatWindowInfo(1) return locked")
            .unwrap(),
        1,
        "GetChatWindowInfo carries the lock"
    );
}

/// The lock verb is the tab menu's first row (`FCF_Tab_OnClick` → the dropdown,
/// FloatingChatFrame.lua:209-219): Unlock while locked, Lock while not.
#[test]
fn the_tab_menus_lock_row_toggles_the_window_and_flips_its_label() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    reveal(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    assert!(s
        .eval::<bool>("return DropDownList1Button1:GetText() == UNLOCK_WINDOW")
        .unwrap());
    left_click(&mut s, "DropDownList1Button1");
    assert!(
        s.eval::<Option<i64>>("return ChatFrame1.isLocked")
            .unwrap()
            .is_none(),
        "the row unlocked the window"
    );
    assert!(shown(&s, GRIP), "and showed its grip");
    assert!(
        s.eval::<Option<i64>>("local _,_,_,_,_,_,_,l = GetChatWindowInfo(1) return l")
            .unwrap()
            .is_none(),
        "written through SetChatWindowLocked, so it persists"
    );
    reveal(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    assert!(s
        .eval::<bool>("return DropDownList1Button1:GetText() == LOCK_WINDOW")
        .unwrap());
    left_click(&mut s, "DropDownList1Button1");
    assert_eq!(
        s.eval::<i64>("return ChatFrame1.isLocked").unwrap(),
        1,
        "and back"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Authored 430x120 with `<ResizeBounds>` 296x120 to 608x400 (FloatingChatFrame.xml:517-524); the
/// grip's `OnMouseDown` sizes from `BOTTOMRIGHT` (:547-556).
#[test]
fn an_unlocked_grip_resizes_and_clamps_at_the_bounds() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    assert_eq!(width(&mut s), 430.0, "the authored width");
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();
    let (x, y) = centre(&mut s, GRIP);
    assert_eq!(
        s.hit_test_name(x, y).as_deref(),
        Some(GRIP),
        "the grip is the topmost hit"
    );

    let top_before: f64 = s.eval("return ChatFrame1:GetTop()").unwrap();
    grab_grip(&mut s, GRIP);
    s.mouse_move(x + 50.0, y - 30.0);
    assert_eq!(width(&mut s), 480.0, "the right edge followed the cursor");
    assert_eq!(height(&mut s), 150.0, "and the bottom edge with it");
    let top_after: f64 = s.eval("return ChatFrame1:GetTop()").unwrap();
    assert!(
        (top_after - top_before).abs() < 1e-3,
        "the TOP edge stayed put"
    );

    s.mouse_move(x + 900.0, y - 900.0);
    assert_eq!((width(&mut s), height(&mut s)), (608.0, 400.0), "maxResize");
    s.mouse_move(x - 900.0, y + 900.0);
    assert_eq!((width(&mut s), height(&mut s)), (296.0, 120.0), "minResize");
    s.mouse_button(x - 900.0, y + 900.0, "LeftButton", false);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `OnMouseUp` goes to the button that took the press, wherever the cursor is, and its
/// `StopMovingOrSizing` ends the resize (FloatingChatFrame.xml:558-566).
#[test]
fn the_release_ends_the_resize_from_anywhere_on_screen() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();
    let (x, y) = grab_grip(&mut s, GRIP);
    s.mouse_move(x + 900.0, y);
    assert_eq!(width(&mut s), 608.0, "held against the maximum");
    s.mouse_button(x + 900.0, y, "LeftButton", false);
    s.resolve();
    s.mouse_move(x - 200.0, y);
    assert_eq!(width(&mut s), 608.0, "the resize is over");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The default window's tab `OnDragStart` returns while it is locked, else starts the move
/// (FloatingChatFrame.xml:445-452); the tab's `OnUpdate` stops it once the button is up.
#[test]
fn the_tab_drag_moves_an_unlocked_window_and_a_locked_one_stays_put() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    let start = left(&mut s);
    drag_tab(&mut s, "ChatFrame1Tab", 80.0, 0.0);
    assert_eq!(left(&mut s), start, "a locked window does not drag");
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();
    reveal(&mut s);
    let (tx, ty) = centre(&mut s, "ChatFrame1Tab");
    s.mouse_button(tx, ty, "LeftButton", true);
    s.mouse_move(tx + 5.0, ty);
    s.mouse_move(tx + 80.0, ty);
    assert_eq!(
        left(&mut s),
        start + 75.0,
        "the window followed the cursor from where the drag started"
    );
    s.mouse_button(tx + 80.0, ty, "LeftButton", false);
    s.tick(0.016);
    s.mouse_move(tx + 400.0, ty);
    assert_eq!(
        left(&mut s),
        start + 75.0,
        "the tab's OnUpdate stopped the move once the button was up"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `UIParent_ManageFramePositions` skips a user-placed frame, the chat frames only outside simple
/// chat (`UIParent.lua:1690-1692`).
#[test]
fn a_moved_window_is_user_placed_and_the_managed_pass_skips_it() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.run("UIParent_ManageFramePositions()").unwrap();
    let seated = left(&mut s);
    assert!(
        !s.eval::<bool>("return ChatFrame1:IsUserPlaced()").unwrap(),
        "nothing has placed it yet"
    );
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();
    drag_tab(&mut s, "ChatFrame1Tab", 120.0, 0.0);
    let moved = left(&mut s);
    assert_eq!(
        moved,
        seated + 115.0,
        "120 less the 5px spent on the threshold"
    );
    assert!(
        s.eval::<bool>("return ChatFrame1:IsUserPlaced()").unwrap(),
        "the drag set the userPlaced bit"
    );
    s.run("UIParent_ManageFramePositions()").unwrap();
    assert_eq!(
        left(&mut s),
        moved,
        "the managed pass does not re-seat a frame the player placed"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn the_geometry_round_trips_through_the_save_file() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();

    let (gx, gy) = grab_grip(&mut s, GRIP);
    s.mouse_move(gx + 70.0, gy - 40.0);
    s.mouse_button(gx + 70.0, gy - 40.0, "LeftButton", false);
    drag_tab(&mut s, "ChatFrame1Tab", 90.0, 60.0);

    let want = (
        width(&mut s),
        height(&mut s),
        left(&mut s),
        s.eval::<f64>("return ChatFrame1:GetBottom()").unwrap() as f32,
    );
    assert_ne!(want.0, 430.0, "the resize happened");
    assert_ne!(want.2, 32.0, "the move happened");

    let saved = s.user_placed_layouts();
    assert_eq!(saved.len(), 1, "one window was placed: {saved:?}");
    assert_eq!(saved[0].name, "ChatFrame1");
    let text = crate::ui_layout::render(&saved);
    let read_back = crate::ui_layout::parse(&text);
    assert_eq!(
        read_back, saved,
        "the file expresses what the seam produced"
    );

    // A fresh VM is the relog: every window starts on its authored anchors.
    let mut fresh = chat_ui();
    assert_eq!(width(&mut fresh), 430.0);
    fresh.restore_user_placed_layouts(read_back);
    fresh.resolve();
    assert_eq!(
        (
            width(&mut fresh),
            height(&mut fresh),
            left(&mut fresh),
            fresh.eval::<f64>("return ChatFrame1:GetBottom()").unwrap() as f32
        ),
        want,
        "the window came back exactly where it was left"
    );
    assert!(
        fresh
            .eval::<bool>("return ChatFrame1:IsUserPlaced()")
            .unwrap(),
        "a restored window keeps the bit, so the managed pass keeps its hands off"
    );
    assert!(
        fresh.errors().is_empty(),
        "script errors: {:?}",
        fresh.errors()
    );
}

/// `CHAT_FRAME_TEXTURES` (FloatingChatFrame.lua:30-46) fade in together on a hover to
/// `DEFAULT_CHATFRAME_ALPHA`, and `FCF_SetWindowColor` tints them as one.
#[test]
fn the_chrome_follows_the_windows_reveal_and_tint() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.mouse_move(1500.0, 850.0);
    for _ in 0..4 {
        s.tick(0.016);
        s.resolve();
    }
    let pieces = ["Background", "BottomRightTexture", "TopTexture"];
    for piece in pieces {
        assert_eq!(
            s.eval::<f64>(&format!("return ChatFrame1{piece}:GetAlpha()"))
                .unwrap(),
            STOCK_ALPHA,
            "{piece} rests at the window's alpha"
        );
    }
    reveal(&mut s);
    for piece in pieces {
        let alpha: f64 = s
            .eval(&format!("return ChatFrame1{piece}:GetAlpha()"))
            .unwrap();
        assert!(
            (alpha - 0.25).abs() < 1e-6,
            "{piece} reveals to DEFAULT_CHATFRAME_ALPHA: {alpha}"
        );
    }
    s.run("FCF_SetWindowColor(ChatFrame1, 1, 0, 0)").unwrap();
    let (r, g, b): (f64, f64, f64) = s
        .eval("return ChatFrame1TopTexture:GetVertexColor()")
        .unwrap();
    assert_eq!((r, g, b), (1.0, 0.0, 0.0));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// A pushed grip holds the window faded in with the cursor off it (`FCF_OnUpdate`'s
/// `resizeButton:GetButtonState() == "PUSHED"`, FloatingChatFrame.lua:1066-1072).
#[test]
fn a_resize_in_flight_holds_the_chrome_visible() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.run("FCF_SetLocked(ChatFrame1, nil)").unwrap();
    reveal(&mut s);
    grab_grip(&mut s, GRIP);
    assert_eq!(
        s.eval::<String>(&format!("return {GRIP}:GetButtonState()"))
            .unwrap(),
        "PUSHED"
    );
    s.mouse_move(1500.0, 820.0);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    let alpha: f64 = s.eval("return ChatFrame1Background:GetAlpha()").unwrap();
    assert!(
        (alpha - 0.25).abs() < 1e-6,
        "the pushed grip holds the reveal open: {alpha}"
    );
    s.mouse_button(1500.0, 820.0, "LeftButton", false);
    // `CHAT_TAB_HIDE_DELAY` 1 s, then `CHAT_FRAME_FADE_OUT_TIME` 2 s (FloatingChatFrame.lua:9-11).
    for _ in 0..200 {
        s.tick(0.016);
        s.resolve();
    }
    let alpha: f64 = s.eval("return ChatFrame1Background:GetAlpha()").unwrap();
    assert!(
        (alpha - STOCK_ALPHA).abs() < 1e-6,
        "and the chrome fades once it is let go: {alpha}"
    );
}

/// Dragging a docked window's tab tears it off the dock, unlocked, as its own window
/// (FloatingChatFrame.xml:453-478).
#[test]
fn dragging_the_docked_combat_logs_tab_tears_it_off_the_dock() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_ui();
    s.run("FCF_SelectDockFrame(ChatFrame2)").unwrap();
    assert!(s.eval::<bool>("return ChatFrame2.isDocked ~= nil").unwrap());
    drag_tab(&mut s, "ChatFrame2Tab", 60.0, 120.0);
    assert!(
        s.eval::<bool>("return ChatFrame2.isDocked == nil").unwrap(),
        "the drag undocked it"
    );
    assert!(
        s.eval::<bool>("return ChatFrame2.isLocked == nil").unwrap(),
        "and unlocked it"
    );
    let (one, two): (f64, f64) = s
        .eval("return ChatFrame1:GetLeft(), ChatFrame2:GetLeft()")
        .unwrap();
    assert_ne!(one, two, "it no longer shares the dock's rect");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
