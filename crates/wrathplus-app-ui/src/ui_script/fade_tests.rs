//! The stock fade and flash kits (`UIParent.lua`), walked from `UIParent`'s own OnUpdate
//! (`UIParent.xml:17`), so hiding `UIParent` stops both.

use wrathplus_ui::script::UiScript;

use super::test_ui::load_ui as load_xml;

#[test]
fn a_started_fade_ramps_off_uiparents_tick_and_completes() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_xml(&mut s, r"Interface\FrameXML\GlobalStrings.lua");
    load_xml(&mut s, r"Interface\FrameXML\BasicControls.xml");
    load_xml(&mut s, r"Interface\FrameXML\UIParent.xml");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.lua");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.xml");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.lua");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.xml");
    load_xml(&mut s, r"Interface\FrameXML\StaticPopup.xml");
    s.resolve();
    // Let the tracking icon's load-time shine finish first: the stock walk skips the fade that
    // slides into a finished one's slot (`UIParent.lua:2456-2491`).
    for _ in 0..3 {
        s.tick(1.0);
    }
    s.run(r#"CreateFrame("Frame", "WrathPlusFadeProbe")"#)
        .unwrap();
    s.run("UIFrameFadeIn(WrathPlusFadeProbe, 1.0)").unwrap();
    assert!(
        s.eval::<bool>("return UIFrameIsFading(WrathPlusFadeProbe) == 1")
            .unwrap(),
        "on the fade list"
    );
    assert_eq!(
        s.eval::<f64>("return WrathPlusFadeProbe:GetAlpha()")
            .unwrap(),
        0.0,
        "the fade armed at its IN startAlpha"
    );

    s.tick(0.5);
    s.resolve();
    let mid = s
        .eval::<f64>("return WrathPlusFadeProbe:GetAlpha()")
        .unwrap();
    assert!(
        (mid - 0.5).abs() < 2.0 / 255.0,
        "half the timeToFade in, half the ramp, to the alpha byte: {mid} {:?}",
        s.errors()
    );

    s.tick(0.6); // past timeToFade
    s.resolve();
    assert_eq!(
        s.eval::<f64>("return WrathPlusFadeProbe:GetAlpha()")
            .unwrap(),
        1.0,
        "the fade completes at its endAlpha"
    );
    assert!(
        !s.eval::<bool>("return UIFrameIsFading(WrathPlusFadeProbe) == 1")
            .unwrap(),
        "…and leaves the list"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn a_flash_alternates_then_stops() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_xml(&mut s, r"Interface\FrameXML\GlobalStrings.lua");
    load_xml(&mut s, r"Interface\FrameXML\BasicControls.xml");
    load_xml(&mut s, r"Interface\FrameXML\UIParent.xml");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.lua");
    load_xml(&mut s, r"Interface\FrameXML\MoneyFrame.xml");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.lua");
    load_xml(&mut s, r"Interface\FrameXML\UIPanelTemplates.xml");
    load_xml(&mut s, r"Interface\FrameXML\StaticPopup.xml");
    s.resolve();

    s.run(
        r#"Probe = CreateFrame("Frame", "WrathPlusFlashProbe", UIParent)
           Probe:SetWidth(10) Probe:SetHeight(10) Probe:SetPoint("TOPLEFT", 0, 0)
           UIFrameFlash(Probe, 0.05, 0.05, 0.3, nil, 0, 0)"#,
    )
    .unwrap();
    assert!(
        s.eval::<bool>("return UIFrameIsFlashing(WrathPlusFlashProbe) == 1")
            .unwrap(),
        "the frame is on the flash list"
    );

    // Re-arming a flashing frame returns early (`UIParent.lua:1235`).
    s.run("UIFrameFlash(WrathPlusFlashProbe, 1, 1, 1, nil, 0, 0)")
        .unwrap();
    assert_eq!(
        s.eval::<i64>("return table.getn(FLASHFRAMES)").unwrap(),
        1,
        "already flashing: the reference returns rather than re-arming"
    );

    // Past flashDuration the frame leaves both lists at full alpha (`UIParent.lua:1272`).
    for _ in 0..40 {
        s.tick(0.016);
        s.resolve();
    }
    assert!(s.errors().is_empty(), "flashing raised: {:?}", s.errors());
    assert_eq!(
        s.eval::<i64>("return table.getn(FLASHFRAMES)").unwrap(),
        0,
        "the flash finished and the frame left the list"
    );
    assert!(
        !s.eval::<bool>("return WrathPlusFlashProbe:IsShown()")
            .unwrap(),
        "showWhenDone was nil, so it ends hidden"
    );
}
