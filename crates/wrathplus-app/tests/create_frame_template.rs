//! `CreateFrame`'s fourth argument against real FrameXML templates, loaded the way the client
//! loads them: the addon idiom `CreateFrame("Button", "MyTab", UIParent, "TabButtonTemplate")`
//! then `getglobal("MyTab".."Text")`.

mod common;

use wrathplus_ui::script::UiScript;

/// The prefix of the production load these templates need. `FauxScrollFrameTemplate` and
/// `TabButtonTemplate` are the stock ones, from `UIPanelTemplates.xml` on the player's chain.
const FILES: &[&str] = &[
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    "Interface\\FrameXML\\BasicControls.xml", // `TEXT`, which StaticPopup.lua reads at file scope
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    "Interface\\FrameXML\\StaticPopup.xml", // the dialog engine
    // A guard, not a dependency: its whole body is `<Script file="FadingFrame.lua"/>`, which loads
    // only if resolved against the document's own directory. It has no `inherits=` and its Lua
    // has no file-scope statements, so it needs nothing before it.
    "Interface\\FrameXML\\FadingFrame.xml",
];

fn load_ui(script: &mut UiScript) {
    for file in FILES {
        common::load_ui(script, file);
    }
}

/// `TabButtonTemplate` (`UIPanelTemplates.xml:303`) exercises every decoration pass at once; an
/// instance publishes its parts under its own name and nothing under the template's.
#[test]
fn a_real_template_reaches_an_addon_through_create_frame() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_ui(&mut s);
    // Clear what the UI load itself reported.
    let _ = s.take_warnings();
    let _ = s.take_errors();

    s.run(
        r#"ProbeTab = CreateFrame("Button", "WrathPlusTemplateProbeTab", UIParent, "TabButtonTemplate")"#,
    )
    .expect("the corpus idiom must not error");

    // The authored `<Size>` is 115x32, but the `<OnLoad>` calls `PanelTemplates_TabResize(0)`
    // (`UIPanelTemplates.xml:371`), fitting an empty tab to its end caps; the height is untouched.
    let (w, h) = s
        .eval::<(f32, f32)>("return ProbeTab:GetWidth(), ProbeTab:GetHeight()")
        .unwrap();
    assert_eq!(h, 32.0, "the template's own <Size> height");
    let caps = s
        .eval::<f32>("return 2 * WrathPlusTemplateProbeTabLeft:GetWidth()")
        .unwrap();
    // `caps` plus the empty label's one-unit floor (`FONTSTRING_MIN_SPAN`).
    assert!(
        w >= caps && w <= caps + 1.5,
        "an unlabelled tab fits down to its two end caps: {w} vs {caps}"
    );
    assert_eq!(
        s.eval::<String>("return ProbeTab:GetParent():GetName()")
            .unwrap(),
        "UIParent",
        "the parent argument, not the template's idea of one"
    );

    // The part globals, named for the instance, as `UIPanelTemplates.lua:42` reads them.
    for suffix in [
        "Text",             // <ButtonText name="$parentText">
        "HighlightTexture", // <HighlightTexture name="$parentHighlightTexture">
        "Left",             // <Layers> slices
        "Middle",
        "Right",
        "LeftDisabled",
    ] {
        assert!(
            s.eval::<bool>(&format!(
                r#"return getglobal("WrathPlusTemplateProbeTab{suffix}") ~= nil"#
            ))
            .unwrap(),
            "WrathPlusTemplateProbeTab{suffix} must exist"
        );
        assert!(
            s.eval::<bool>(&format!(
                r#"return getglobal("TabButtonTemplate{suffix}") == nil"#
            ))
            .unwrap(),
            "nothing may be published under the TEMPLATE's name (TabButtonTemplate{suffix})"
        );
    }

    // The label is a real FontString the addon can drive.
    s.run(r#"getglobal("WrathPlusTemplateProbeTabText"):SetText("Hi")"#)
        .unwrap();
    assert_eq!(
        s.eval::<String>(r#"return getglobal("WrathPlusTemplateProbeTabText"):GetText()"#)
            .unwrap(),
        "Hi"
    );

    // The stock template declares one handler, the `<OnLoad>`.
    assert!(
        s.eval::<bool>(r#"return ProbeTab:GetScript("OnLoad") ~= nil"#)
            .unwrap(),
        "the template's OnLoad is installed"
    );

    assert!(s.take_errors().is_empty());
    assert_eq!(
        s.take_warnings(),
        Vec::<String>::new(),
        "a template that resolves cleanly has nothing to report"
    );
}

/// `FauxScrollFrameTemplate`, the addon corpus's most-instantiated template, runs
/// `ScrollFrame_OnLoad` (`UIPanelTemplates.lua:244`), which indexes
/// `<name>ScrollBarScrollDownButton` first: two levels of `$parent` through a runtime template.
#[test]
fn the_corpus_favourite_template_composes_parent_two_levels_deep() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_ui(&mut s);
    let _ = s.take_warnings();
    let _ = s.take_errors();

    s.run(
        // The stock template is a `<ScrollFrame>` with a `<ScrollChild>`, and `framexml::merge`
        // takes the overriding node's tag, so a "Frame" would drop the scroll-only parts.
        r#"Scroller = CreateFrame("ScrollFrame", "WrathPlusTemplateProbeScroll", UIParent, "FauxScrollFrameTemplate")"#,
    )
    .expect("the corpus's most-used template must not error");

    // `this.offset` is `ScrollFrame_OnLoad`'s last statement.
    assert_eq!(
        s.eval::<f32>("return Scroller.offset").unwrap(),
        0.0,
        "the template's OnLoad ran to its last line"
    );
    for suffix in [
        "ScrollBar",
        "ScrollBarScrollUpButton",
        "ScrollBarScrollDownButton",
    ] {
        assert!(
            s.eval::<bool>(&format!(
                r#"return getglobal("WrathPlusTemplateProbeScroll{suffix}") ~= nil"#
            ))
            .unwrap(),
            "WrathPlusTemplateProbeScroll{suffix} must exist"
        );
    }
    assert_eq!(
        s.eval::<String>("return WrathPlusTemplateProbeScrollScrollBar:GetParent():GetName()")
            .unwrap(),
        "WrathPlusTemplateProbeScroll"
    );
    assert!(
        s.eval::<bool>(r#"return getglobal("FauxScrollFrameTemplateScrollBar") == nil"#)
            .unwrap(),
        "nothing may be published under the template's own name"
    );

    assert!(s.take_errors().is_empty());
    assert_eq!(s.take_warnings(), Vec::<String>::new());
}
