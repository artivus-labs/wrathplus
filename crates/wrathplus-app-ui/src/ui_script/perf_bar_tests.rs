//! The stock latency meter: 3.3.5 hangs it on the Main Menu micro button as an OVERLAY texture
//! (`MainMenuBarMicroButtons.xml:320`), tinted by the button's own 10 s poll, whose hover writes the
//! latency tooltip.

use wrathplus_ui::script::{QuadContent, UiScript};

use super::test_ui::load_ui as load_xml;

/// A 1024-wide screen, so the 1024-wide bar spans x 0..1024 and its offsets are screen coordinates.
fn harness(extra: &[&str]) -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    for f in crate::ui_script::test_ui::production_order(&[
        &[
            "Interface\\FrameXML\\GlobalStrings.lua",
            "Interface\\FrameXML\\Fonts.xml",
            r"Interface\FrameXML\UIParent.xml",
            r"Interface\FrameXML\MoneyFrame.lua",
            r"Interface\FrameXML\MoneyFrame.xml",
            "Interface\\FrameXML\\GameTooltip.xml",
            "Interface\\FrameXML\\TextStatusBar.lua",
            "Interface\\FrameXML\\TextStatusBar.xml",
            "Interface\\FrameXML\\MainMenuBar.xml",
            "Interface\\FrameXML\\Cooldown.xml",
            "Interface\\FrameXML\\ActionButtonTemplate.xml",
            "Interface\\FrameXML\\ActionBarFrame.xml",
            "Interface\\FrameXML\\BonusActionBarFrame.xml",
        ],
        extra,
    ]) {
        load_xml(&mut s, f);
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    s
}

/// The meter texture's quad: its draw-order index, its rect and its tint.
fn bar_quad(s: &mut UiScript) -> (usize, [f32; 4], Option<[f32; 4]>) {
    s.resolve();
    let quads = s.extract();
    let i = quads
        .iter()
        .position(|q| {
            matches!(&q.content, QuadContent::Texture { path: Some(p), .. }
                if p.contains("UI-MainMenuBar-PerformanceBar"))
        })
        .expect("the performance bar's texture quad");
    let rect = quads[i].rect.expect("a resolved rect");
    let color = match &quads[i].content {
        QuadContent::Texture { color, .. } => *color,
        _ => unreachable!(),
    };
    (i, [rect.left, rect.bottom, rect.right, rect.top], color)
}

/// `MainMenuBarMicroButtons.xml:320-327`: a 14x7 OVERLAY texture of `MainMenuMicroButton`, at its
/// TOPLEFT (10, -34); the pressed look moves it to (9, -36) (`MainMenuBarMicroButtons.lua:162-170`).
/// 1.12's own `MainMenuBarPerformanceBarFrame` is gone.
#[test]
fn the_meter_is_an_overlay_on_the_main_menu_button() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness(&[]);
    assert_eq!(
        s.eval::<(String, String, String)>(
            "return MainMenuBarPerformanceBar:GetObjectType(), \
             MainMenuBarPerformanceBar:GetParent():GetName(), \
             (MainMenuBarPerformanceBar:GetDrawLayer())",
        )
        .unwrap(),
        (
            "Texture".into(),
            "MainMenuMicroButton".into(),
            "OVERLAY".into()
        )
    );
    assert!(s
        .eval::<bool>("return MainMenuBarPerformanceBarFrame == nil")
        .unwrap());

    let offset = |s: &mut UiScript| {
        s.resolve();
        let (bl, bt) = s
            .eval::<(f64, f64)>(
                "return MainMenuMicroButton:GetLeft(), MainMenuMicroButton:GetTop()",
            )
            .unwrap();
        let (_, [left, bottom, right, top], _) = bar_quad(s);
        (
            (f64::from(left) - bl, f64::from(top) - bt),
            (right - left, top - bottom),
        )
    };
    assert_eq!(offset(&mut s), ((10.0, -34.0), (14.0, 7.0)));
    s.run("MainMenuMicroButton_SetPushed()").unwrap();
    assert_eq!(
        offset(&mut s).0,
        (9.0, -36.0),
        "pressed, it sinks with the button"
    );
    s.run("MainMenuMicroButton_SetNormal()").unwrap();
    assert_eq!(offset(&mut s).0, (10.0, -34.0));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The button's OnUpdate (`MainMenuBarMicroButtons.xml:342-358`): every 10 s the poll tints the bar
/// green to 300 ms, yellow to 600, red beyond; the tint is the whole readout.
#[test]
fn the_meter_tints_by_latency_on_the_reference_thresholds() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness(&[]);

    // updateInterval starts at 0 (the OnLoad, `MainMenuBarMicroButtons.xml:339`), so the first tick polls: 0 ms, green.
    s.tick(0.016);
    let (_, _, color) = bar_quad(&mut s);
    assert_eq!(color, Some([0.0, 1.0, 0.0, 1.0]), "0 ms ⇒ green");

    for (latency, want, band) in [
        (299, [0.0, 1.0, 0.0, 1.0], "under LOW ⇒ green"),
        (301, [1.0, 1.0, 0.0, 1.0], "past LOW ⇒ yellow"),
        (601, [1.0, 0.0, 0.0, 1.0], "past MEDIUM ⇒ red"),
    ] {
        s.set_latency_ms(Some(latency));
        poll_beat(&mut s);
        let (_, _, color) = bar_quad(&mut s);
        assert_eq!(color, Some(want), "{latency} ms: {band}");
    }

    s.set_latency_ms(Some(0));
    s.tick(1.0);
    let (_, _, color) = bar_quad(&mut s);
    assert_eq!(
        color,
        Some([1.0, 0.0, 0.0, 1.0]),
        "still red until the beat"
    );
    poll_beat(&mut s);
    let (_, _, color) = bar_quad(&mut s);
    assert_eq!(
        color,
        Some([0.0, 1.0, 0.0, 1.0]),
        "the beat repaints it green"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The drawn colour of the tooltip's `NEWBIE_TOOLTIP_LATENCY` line, found by its text.
fn newbie_line_color(s: &mut UiScript) -> Option<[f32; 4]> {
    let want = s
        .eval::<String>("return NEWBIE_TOOLTIP_LATENCY")
        .expect("the string is declared with the meter");
    s.resolve();
    s.extract().iter().find_map(|q| match &q.content {
        QuadContent::Text {
            text: Some(t),
            color,
            ..
        } if *t == want => Some(*color),
        _ => None,
    })?
}

/// One poll beat: OnUpdate polls only on a tick that finds the interval spent, so one tick drains
/// it and the next polls (`MainMenuBarMicroButtons.xml:343`).
fn poll_beat(s: &mut UiScript) {
    s.tick(11.0);
    s.tick(11.0);
}

/// The app pushes the averaged RTT; both bandwidth returns stay 0, as WrathPlus counts no throughput.
#[test]
fn get_net_stats_reports_the_pushed_latency() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness(&[]);
    assert_eq!(
        s.eval::<(f64, f64, f64)>("return GetNetStats()").unwrap(),
        (0.0, 0.0, 0.0),
        "nothing measured yet"
    );
    s.set_latency_ms(Some(42));
    assert_eq!(
        s.eval::<(f64, f64, f64)>("return GetNetStats()").unwrap(),
        (0.0, 0.0, 42.0)
    );
    // A disconnect clears the samples; no sample reads 0, as in the reference.
    s.set_latency_ms(None);
    assert_eq!(
        s.eval::<f64>("local _, _, ms = GetNetStats() return ms")
            .unwrap(),
        0.0
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The tooltip's lines, top to bottom.
fn tooltip_lines(s: &UiScript) -> Vec<String> {
    s.eval::<String>(
        "local t = {} for i = 1, GameTooltip:NumLines() do \
         t[#t + 1] = _G['GameTooltipTextLeft' .. i]:GetText() or '' end \
         return table.concat(t, '\\n')",
    )
    .unwrap()
    .lines()
    .map(str::to_string)
    .collect()
}

/// Hovering the button arms its poll (`OnEnter`, `MainMenuBarMicroButtons.xml:424-427`), whose next
/// beat runs `MainMenuBarPerformanceBarFrame_OnEnter` (`MainMenuBar.lua:489`) while the hover lasts.
#[test]
fn hovering_the_main_menu_button_shows_the_live_latency() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness(&[]);
    s.set_latency_ms(Some(42));
    s.run("SHOW_NEWBIE_TIPS = \"1\"").unwrap();
    s.resolve();
    let (x, y) = s
        .eval::<(f64, f64)>("return MainMenuMicroButton:GetCenter()")
        .unwrap();
    assert_eq!(
        s.hit_test_name(x as f32, y as f32).as_deref(),
        Some("MainMenuMicroButton"),
        "the button takes the mouse"
    );

    s.run("MainMenuMicroButton:GetScript(\"OnEnter\")(MainMenuMicroButton)")
        .unwrap();
    s.tick(0.016);
    let latency = |ms: u32| format!("Latency: {ms} ms");
    assert!(
        tooltip_lines(&s).contains(&latency(42)),
        "MAINMENUBAR_LATENCY_LABEL: {:?}",
        tooltip_lines(&s)
    );
    let newbie = s.eval::<String>("return NEWBIE_TOOLTIP_LATENCY").unwrap();
    assert!(
        tooltip_lines(&s).contains(&newbie),
        "the newbie line follows"
    );
    assert_eq!(
        newbie_line_color(&mut s),
        Some([1.0, 0.82, 0.0, 1.0]),
        "NORMAL_FONT_COLOR"
    );

    // Still hovering: the next beat rewrites the plate.
    s.set_latency_ms(Some(7));
    poll_beat(&mut s);
    assert!(
        tooltip_lines(&s).contains(&latency(7)),
        "{:?}",
        tooltip_lines(&s)
    );
    assert!(
        !tooltip_lines(&s).contains(&latency(42)),
        "rebuilt, not appended"
    );

    s.run("MainMenuMicroButton:GetScript(\"OnLeave\")(MainMenuMicroButton)")
        .unwrap();
    assert!(!s.eval::<bool>("return GameTooltip:IsVisible()").unwrap());
    s.set_latency_ms(Some(999));
    poll_beat(&mut s);
    assert!(
        !s.eval::<bool>("return GameTooltip:IsVisible()").unwrap(),
        "an unhovered button never re-opens the tooltip"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
