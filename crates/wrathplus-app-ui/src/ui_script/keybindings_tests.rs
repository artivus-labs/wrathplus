//! The stock binding surface over the engine's binding table: `GetBindingText`, the install's
//! `Bindings.xml` command bodies, and the production load's `ToggleUI`.

use wrathplus_ui::script::UiScript;

/// A stock kit over the stock commands.
pub(crate) fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    crate::ui_script::load_stock_bindings(&mut s);
    s.set_screen_size(1024.0, 768.0);
    for file in [
        "Interface\\FrameXML\\GlobalStrings.lua",
        "Interface\\FrameXML\\Fonts.xml",
        "Interface\\FrameXML\\BasicControls.xml",
        r"Interface\FrameXML\UIParent.xml",
        r"Interface\FrameXML\MoneyFrame.lua",
        r"Interface\FrameXML\MoneyFrame.xml",
        "Interface\\FrameXML\\GameTooltip.xml",
        "Interface\\FrameXML\\UIDropDownMenu.xml",
        r"Interface\FrameXML\UIPanelTemplates.lua",
        r"Interface\FrameXML\UIPanelTemplates.xml",
        r"Interface\FrameXML\GameMenuFrame.xml",
        "Interface\\FrameXML\\StaticPopup.xml",
    ] {
        super::test_ui::load_ui(&mut s, file);
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    s
}

#[test]
fn the_action_bar_abbreviation_is_the_refs_own_getbindingtext() {
    wrathplus_formats::wow_data_or_skip!();
    let s = harness();
    // Stock `GetBindingText` (`UIParent.lua:1819`): one modifier abbreviates…
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("SHIFT-2", "KEY_", 1)"#)
            .unwrap(),
        "s-2",
        "SHIFT-2 reads s-2, not a SHIF… truncation"
    );
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("ALT-Z", "KEY_", 1)"#)
            .unwrap(),
        "a-Z"
    );
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("W", "KEY_", 1)"#)
            .unwrap(),
        "W"
    );
    // …two or more collapse to a dot, `CTRL--` included: the dash-counting loop counts its second
    // dash as a modifier.
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("CTRL-SHIFT-2", "KEY_", 1)"#)
            .unwrap(),
        "·"
    );
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("CTRL--", "KEY_", 1)"#)
            .unwrap(),
        "·"
    );
    // The full form keeps the prefixes and reads the KEY_* global when it exists.
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("SHIFT-2", "KEY_")"#)
            .unwrap(),
        "SHIFT-2"
    );
    s.run(r#"KEY_SPACE = "Spacebar""#).unwrap();
    assert_eq!(
        s.eval::<String>(r#"return GetBindingText("SPACE", "KEY_")"#)
            .unwrap(),
        "Spacebar"
    );
    assert!(s
        .eval::<bool>(r#"return GetBindingText(nil) == """#)
        .unwrap());
}

/// `loadstring` on each stock command body: the reference compiles each at load (`0x704c70`),
/// and running one would need FrameXML this test does not load.
#[test]
fn every_command_body_compiles() {
    wrathplus_formats::wow_data_or_skip!();
    let stock = wrathplus_ui::bindings_xml::parse(
        &crate::ui_script::stock_bindings_file().expect("the install's Bindings.xml"),
    )
    .unwrap();
    let s = UiScript::new().unwrap();
    for b in &stock {
        s.run(&format!(
            "local f, err = loadstring({body:?}); \
             if not f then WrathPlusBodyError = {name:?} .. \": \" .. err end",
            body = b.body,
            name = b.name
        ))
        .unwrap_or_else(|e| panic!("{}: {e}", b.name));
    }
    let bad: Vec<String> = s.errors();
    assert!(bad.is_empty(), "script errors: {bad:?}");
    s.run("if WrathPlusBodyError then error(WrathPlusBodyError) end")
        .unwrap_or_else(|e| panic!("a command body is not valid Lua: {e}"));
}

/// Every function and object a stock command body names exists once the in-game UI has loaded:
/// the engine's behaviour sits behind the Lua globals the bodies call. Read off each body: a
/// `Name(` call is a function, an `Object:` or `Object.` receiver a frame or table.
#[test]
fn every_stock_body_calls_what_the_load_defines() {
    wrathplus_formats::wow_data_or_skip!();
    let _l = crate::local_state::test_env::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (s, failures) = super::production_load::production_load_with("bindcalls", "", |_| {});
    assert!(failures.is_empty(), "load failures: {failures:#?}");
    let stock = wrathplus_ui::bindings_xml::parse(
        &crate::ui_script::stock_bindings_file().expect("the install's Bindings.xml"),
    )
    .unwrap();
    const KEYWORDS: [&str; 14] = [
        "if", "then", "else", "elseif", "end", "not", "and", "or", "local", "function", "return",
        "do", "while", "for",
    ];
    let mut missing = Vec::new();
    for b in stock.iter().filter(|b| b.platform.is_none()) {
        let body = b.body.as_bytes();
        let mut i = 0;
        while i < body.len() {
            let c = body[i];
            if !(c.is_ascii_alphabetic() || c == b'_') {
                i += 1;
                continue;
            }
            let start = i;
            while i < body.len() && (body[i].is_ascii_alphanumeric() || body[i] == b'_') {
                i += 1;
            }
            // A field or method name is its receiver's, not a global.
            if start > 0 && matches!(body[start - 1], b'.' | b':') {
                continue;
            }
            let name = &b.body[start..i];
            let next = b.body[i..].trim_start().chars().next();
            let want = match next {
                Some('(') if !KEYWORDS.contains(&name) => "function",
                Some(':') | Some('.') => "table",
                _ => continue,
            };
            let kind: String = s.eval(&format!("return type({name})")).unwrap();
            if kind != want {
                missing.push(format!("{}: {name} is {kind}", b.name));
            }
        }
    }
    assert!(missing.is_empty(), "unresolved: {missing:#?}");
}

/// ALT-Z runs the stock TOGGLEUI body (`Bindings.xml:655-661`): `CloseAllWindows()` and
/// `UIParent:Hide()`, then `UIParent:Show()`, so the interface under `UIParent` stops drawing and
/// an open window is closed, as in the reference.
#[test]
fn toggleui_hides_uiparent_and_closes_the_windows() {
    wrathplus_formats::wow_data_or_skip!();
    let _l = crate::local_state::test_env::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut s, failures) = super::production_load::production_load_with("toggleui", "", |_| {});
    assert!(failures.is_empty(), "load failures: {failures:#?}");
    s.run("ShowUIPanel(GameMenuFrame)").unwrap();
    assert!(s
        .eval::<bool>("return GameMenuFrame:IsVisible() == 1")
        .unwrap());
    s.resolve();
    let lit = s.extract().len();
    assert!(s.execute_binding("TOGGLEUI", true).unwrap());
    s.resolve();
    assert!(s
        .eval::<bool>("return UIParent:IsVisible() == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("return GameMenuFrame:IsShown() == nil")
        .unwrap());
    assert!(
        s.extract().len() < lit,
        "the frames under UIParent stop drawing"
    );
    assert!(s.execute_binding("TOGGLEUI", true).unwrap());
    assert!(s.eval::<bool>("return UIParent:IsVisible() == 1").unwrap());
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}
