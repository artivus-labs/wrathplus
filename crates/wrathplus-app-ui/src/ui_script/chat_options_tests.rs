//! The chat tab's options menu, driven from the mouse through the stock frames and colour picker.

use wrathplus_ui::script::UiScript;

use super::test_ui::load_ui as load_xml;

/// The menu's labels at their stock `GlobalStrings.lua` values.
fn bake_strings(s: &UiScript) {
    s.run(
        r#"
        BACKGROUND = "Background"
        DISPLAY = "Display"
        FONT_SIZE = "Font Size"
        FONT_SIZE_TEMPLATE = "%d pt"
        CHAT_OPTIONS_LABEL = "Chat Options"
        NEWBIE_TOOLTIP_CHATOPTIONS = "Right-click to get a list of customizable options for this window. Left-click and drag to move the window."
    "#,
    )
    .unwrap();
}

/// The default window's Background swatch row.
const BACKGROUND_ROW: &str = "DropDownList1Button8";

/// A window's alpha byte until a setter moves it, `0x28` (`0x501813`), as `GetChatWindowInfo` reads it.
const STOCK_ALPHA: f64 = (40.0f32 * (1.0f32 / 255.0f32)) as f64;

fn chat_with_menu() -> UiScript {
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
        "Interface\\FrameXML\\ChatFrame.xml",
        "Interface\\FrameXML\\FloatingChatFrame.xml",
        "Interface\\FrameXML\\ColorPickerFrame.xml",
    ] {
        load_xml(&mut s, file);
    }
    bake_strings(&s);
    super::fire_chat_login(&mut s);
    s.set_screen_size(1600.0, 900.0);
    s.resolve();
    s
}

/// The tabs ship hidden until a stationary hover reveals them (`FCF_OnUpdate`), so a tab click
/// needs the reveal first.
fn reveal_then(s: &mut UiScript) {
    reveal_dock(s);
}

fn right_click(s: &mut UiScript, frame: &str) {
    let (x, y) = s
        .eval::<(f64, f64)>(&format!("return {frame}:GetCenter()"))
        .unwrap();
    s.mouse_button(x as f32, y as f32, "RightButton", true);
    s.mouse_button(x as f32, y as f32, "RightButton", false);
    s.resolve();
}

fn hover(s: &mut UiScript, frame: &str) {
    let (x, y) = s
        .eval::<(f64, f64)>(&format!("return {frame}:GetCenter()"))
        .unwrap();
    s.mouse_move(x as f32, y as f32);
    s.resolve();
}

/// Parks the cursor mid-window past the dock's 0.2 s show delay and 0.15 s fade.
fn reveal_dock(s: &mut UiScript) {
    let (x, y): (f32, f32) = s
        .eval(
            "return (ChatFrame1:GetLeft() + ChatFrame1:GetRight()) / 2, \
             (ChatFrame1:GetBottom() + ChatFrame1:GetTop()) / 2",
        )
        .unwrap();
    s.mouse_move(x, y);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
}

fn left_click(s: &mut UiScript, frame: &str) {
    let (x, y) = s
        .eval::<(f64, f64)>(&format!("return {frame}:GetCenter()"))
        .unwrap();
    s.mouse_button(x as f32, y as f32, "LeftButton", true);
    s.mouse_button(x as f32, y as f32, "LeftButton", false);
    s.resolve();
}

#[test]
fn right_clicking_a_chat_tab_opens_its_options_menu() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    assert!(
        s.eval::<bool>("return not DropDownList1:IsVisible()")
            .unwrap(),
        "no menu before any click"
    );
    reveal_then(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    assert!(
        s.eval::<bool>("return DropDownList1:IsVisible()").unwrap(),
        "the tab's right-click opens the options menu"
    );
    // The default window's level-1 rows (`FCFOptionsDropDown_Initialize`,
    // FloatingChatFrame.lua:178-340); only the default window has New and Reset.
    for (n, key) in [
        (1, "UNLOCK_WINDOW"),
        (2, "MAKE_UNINTERACTABLE"),
        (3, "RENAME_CHAT_WINDOW"),
        (4, "NEW_CHAT_WINDOW"),
        (5, "RESET_ALL_WINDOWS"),
        (6, "DISPLAY"),
        (7, "FONT_SIZE"),
        (8, "BACKGROUND"),
    ] {
        assert!(
            s.eval::<bool>(&format!("return DropDownList1Button{n}:GetText() == {key}"))
                .unwrap(),
            "row {n} is {key}"
        );
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn a_left_click_still_selects_the_tab_and_opens_no_menu() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    reveal_then(&mut s);
    left_click(&mut s, "ChatFrame2Tab");
    assert_eq!(
        s.eval::<i64>("return SELECTED_DOCK_FRAME:GetID()").unwrap(),
        2,
        "left-click selects"
    );
    assert!(
        s.eval::<bool>("return not DropDownList1:IsVisible()")
            .unwrap(),
        "a left click opens no menu"
    );
    right_click(&mut s, "ChatFrame1Tab");
    assert!(s.eval::<bool>("return DropDownList1:IsVisible()").unwrap());
    // A second release on the same tab inside the double-click window is its `OnDoubleClick`, not
    // a click, so the player's left click comes a beat later.
    s.tick(0.5);
    left_click(&mut s, "ChatFrame1Tab");
    assert!(
        s.eval::<bool>("return not DropDownList1:IsVisible()")
            .unwrap(),
        "the left click closed the open menu"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The opacity slider is reversed (0 is fully opaque), so it opens at `1 - a` and writes
/// `1 - value`; the store is a byte, so 0.8 reads back as 204/255.
#[test]
fn the_background_row_opens_the_picker_and_its_opacity_slider_drives_the_window() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    reveal_then(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    assert_eq!(
        s.eval::<f64>("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
            .unwrap(),
        STOCK_ALPHA
    );
    assert_eq!(
        s.eval::<f64>(&format!("return {BACKGROUND_ROW}.opacity"))
            .unwrap(),
        1.0 - STOCK_ALPHA,
        "info.opacity is 1 - a (FloatingChatFrame.lua:321-325: 'the slider is reversed')"
    );
    left_click(&mut s, &format!("{BACKGROUND_ROW}ColorSwatch"));
    assert!(
        s.eval::<bool>("return ColorPickerFrame:IsVisible()")
            .unwrap(),
        "the Background swatch opens the colour picker"
    );
    // The cursor leaves the chat frame for the picker, so the hover fade-out runs first.
    s.mouse_move(1500.0, 850.0);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(
        s.eval::<bool>("return OpacitySliderFrame:IsVisible()")
            .unwrap(),
        "and the picker wears its opacity slider — the chat window's 'background slider'"
    );
    assert_eq!(
        s.eval::<f64>("return OpacitySliderFrame:GetValue()")
            .unwrap(),
        f64::from(0.84f32),
        "the slider opens seeded from the window, `1 - a` on its 0.01 step"
    );
    s.run("OpacitySliderFrame:SetValue(1 - 0.8)").unwrap();
    let stored: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    // `valueStep="0.01"` rebuilds `1 - 0.8` as `n * step`, a hair below the literal, so
    // `1 - value` is a hair above 0.8 and the store's floor quantizer keeps 204, not 203.
    // The getter scales the byte by an f32 reciprocal (`wrath_chat_window.rs`, `unit`).
    let byte = f64::from(204.0f32 * (1.0f32 / 255.0f32));
    assert!(
        (stored - byte).abs() < 1e-12,
        "the drag reached the engine store, quantized to its byte: {stored}"
    );
    s.mouse_move(1500.0, 850.0);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    let painted: f64 = s.eval("return ChatFrame1Background:GetAlpha()").unwrap();
    assert!(
        (painted - 0.8).abs() < 1e-6,
        "a window the player made solid stays solid off-hover: {painted}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn the_shipped_window_fades_from_its_default_alpha_to_a_quarter() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    s.mouse_move(1500.0, 850.0);
    for _ in 0..4 {
        s.tick(0.016);
        s.resolve();
    }
    assert_eq!(
        s.eval::<f64>("return ChatFrame1Background:GetAlpha()")
            .unwrap(),
        STOCK_ALPHA,
        "at rest: the default record's alpha"
    );
    reveal_dock(&mut s);
    let alpha: f64 = s.eval("return ChatFrame1Background:GetAlpha()").unwrap();
    assert!(
        (alpha - 0.25).abs() < 1e-6,
        "hovered: DEFAULT_CHATFRAME_ALPHA — got {alpha}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn the_font_size_submenu_resizes_the_window_and_stores_the_pick() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    reveal_then(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    hover(&mut s, "DropDownList1Button7");
    assert_eq!(
        s.eval::<i64>("return DropDownList2.numButtons").unwrap(),
        4,
        "CHAT_FONT_HEIGHTS is 12, 14, 16, 18"
    );
    // `for index, value in CHAT_FONT_HEIGHTS` walks a `[n] =` keyed table in hash order, so a
    // row is found by its value.
    let row_with = |s: &UiScript, pt: i64| -> String {
        s.eval::<String>(&format!(
            "for i = 1, DropDownList2.numButtons do \
                 local b = getglobal('DropDownList2Button'..i) \
                 if b.value == {pt} then return b:GetName() end \
             end return ''"
        ))
        .unwrap()
    };
    assert_eq!(
        s.eval::<String>(&format!("return {}:GetText()", row_with(&s, 12)))
            .unwrap(),
        "12 pt",
        "FONT_SIZE_TEMPLATE over the value"
    );
    let ticked = row_with(&s, 14);
    assert!(
        s.eval::<bool>(&format!("return {ticked}Check:IsVisible()"))
            .unwrap(),
        "the tick follows the font the frame is actually wearing (ChatFontNormal, 14)"
    );
    let sixteen = row_with(&s, 16);
    left_click(&mut s, &sixteen);
    let (_, height): (String, f64) = s
        .eval("local f, h = ChatFrame1:GetFont() return f, h")
        .unwrap();
    assert_eq!(height, 16.0, "the live font moved");
    assert_eq!(
        s.eval::<i64>("local _, size = GetChatWindowInfo(1) return size")
            .unwrap(),
        16,
        "and the pick is stored as the cache's SIZE"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `FCF_GetCurrentChatFrameID` reads the id of the open dropdown's parent, the clicked tab.
#[test]
fn each_tabs_menu_writes_its_own_window() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    reveal_then(&mut s);
    right_click(&mut s, "ChatFrame2Tab");
    assert_eq!(
        s.eval::<i64>("return FCF_GetCurrentChatFrameID()").unwrap(),
        2
    );
    // The docked combat log has no New, Reset or Close rows, so Background is row 6.
    assert!(s
        .eval::<bool>("return DropDownList1Button6:GetText() == BACKGROUND")
        .unwrap());
    left_click(&mut s, "DropDownList1Button6ColorSwatch");
    s.run("OpacitySliderFrame:SetValue(0)").unwrap(); // reversed: 0 = fully opaque
    assert_eq!(
        s.eval::<f64>("local _,_,_,_,_,a = GetChatWindowInfo(2) return a")
            .unwrap(),
        1.0,
        "window 2 took the write"
    );
    assert_eq!(
        s.eval::<f64>("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
            .unwrap(),
        STOCK_ALPHA,
        "window 1 did not"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn cancelling_the_picker_puts_the_window_back() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    s.run("FCF_SetWindowAlpha(ChatFrame1, 0.4) FCF_SetWindowColor(ChatFrame1, 0.2, 0.4, 0.6)")
        .unwrap();
    let before: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    reveal_then(&mut s);
    right_click(&mut s, "ChatFrame1Tab");
    left_click(&mut s, &format!("{BACKGROUND_ROW}ColorSwatch"));
    s.run("OpacitySliderFrame:SetValue(0)").unwrap();
    assert_eq!(
        s.eval::<f64>("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
            .unwrap(),
        1.0,
        "the drag previewed live"
    );
    left_click(&mut s, "ColorPickerCancelButton");
    let after: f64 = s
        .eval("local _,_,_,_,_,a = GetChatWindowInfo(1) return a")
        .unwrap();
    assert!(
        (after - before).abs() < 1e-9,
        "Cancel restored the alpha: {after} vs {before}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The host restores saved windows into the engine table, then fires `UPDATE_CHAT_WINDOWS`.
#[test]
fn the_restore_event_repaints_the_window_from_the_store() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = chat_with_menu();
    s.set_chat_window_looks([(
        0,
        wrathplus_ui::script::ChatWindowLook {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
            font_size: 18,
            locked: true,
            docked: Some(1),
            ..Default::default()
        },
    )]);
    // Colour, alpha and lock paint only on a frame's first `UPDATE_CHAT_WINDOWS`
    // (`FloatingChatFrame.lua:52`); the helper already fired one, so the latch is reset.
    s.run("ChatFrame1.isInitialized = nil").unwrap();
    s.fire_event("UPDATE_CHAT_WINDOWS", vec![]);
    s.mouse_move(1500.0, 850.0);
    for _ in 0..45 {
        s.tick(0.016);
        s.resolve();
    }
    assert_eq!(
        s.eval::<f64>("return ChatFrame1Background:GetAlpha()")
            .unwrap(),
        1.0,
        "the restored alpha is on screen"
    );
    let (r, g, b): (f64, f64, f64) = s
        .eval("return ChatFrame1Background:GetVertexColor()")
        .unwrap();
    assert_eq!((r, g, b), (1.0, 0.0, 0.0), "and the restored tint");
    // The restore paints without saving, but the dock pass after it (`FCF_DockFrame`,
    // `FCF_SaveDock`, `SetChatWindowDocked`) writes windows 1 and 2 itself.
    assert_eq!(s.take_chat_window_changes(), vec![0, 1]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
