//! The stock 3.3.5 cast bar (`CastingBarFrame.lua`): the fill tracking the clock, the green
//! completion flash and fade, red Failed with its 1 s hold, the green channel bar draining. The
//! bar reads `UnitCastingInfo`/`UnitChannelInfo` on each `UNIT_SPELLCAST_*` edge, so a cast here is
//! seated as App seats it: the snapshot first, then the event (`wrath_unit_casts`).

use std::collections::HashMap;

use wrathplus_ui::script::{
    ExtractedQuad, QuadContent, ScriptValue, UiScript, UnitGuids, UnitState, WrathUnitCast,
};

use super::test_ui::load_ui as load_xml;

const PLAYER: u64 = 0x10;
const CAST_ID: u8 = 7;

fn harness() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    load_xml(&mut s, "Interface\\FrameXML\\CastingBarFrame.xml");
    s.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            name: Some("Probefour".into()),
            guid: PLAYER,
            ..Default::default()
        }),
    );
    // `UnitCastingInfo("player")` resolves the token through the resolver's guids.
    s.set_unit_guids(&UnitGuids {
        player: PLAYER,
        ..Default::default()
    });
    s
}

/// The player's cast or channel, `ms` long from now, published before its start event.
fn seat(s: &mut UiScript, name: &str, ms: u32, channel: bool) {
    let now = (s.eval::<f64>("return GetTime()").unwrap() * 1000.0) as u32;
    let cast = WrathUnitCast {
        spell_id: 116,
        name: name.into(),
        icon: "Interface\\Icons\\Spell_Frost_FrostBolt02".into(),
        cast_count: if channel { 0 } else { CAST_ID },
        start_ms: now,
        end_ms: now + ms,
        ..Default::default()
    };
    let one = HashMap::from([(PLAYER, cast)]);
    if channel {
        s.set_wrath_unit_casts(HashMap::new(), one);
    } else {
        s.set_wrath_unit_casts(one, HashMap::new());
    }
}

fn cast(s: &mut UiScript, name: &str, ms: u32) {
    seat(s, name, ms, false);
    s.fire_event("UNIT_SPELLCAST_START", vec![player()]);
}

fn channel(s: &mut UiScript, name: &str, ms: u32) {
    seat(s, name, ms, true);
    s.fire_event("UNIT_SPELLCAST_CHANNEL_START", vec![player()]);
}

fn player() -> ScriptValue {
    ScriptValue::Str("player".into())
}

/// `UNIT_SPELLCAST_STOP` and friends carry `unit, spell, rank, castID` (the arms compare arg 4).
fn spell_event(s: &mut UiScript, event: &str) {
    s.set_wrath_unit_casts(HashMap::new(), HashMap::new());
    s.fire_event(
        event,
        vec![
            player(),
            ScriptValue::Str("Frostbolt".into()),
            ScriptValue::Str(String::new()),
            ScriptValue::Int(i64::from(CAST_ID)),
        ],
    );
}

fn bar_value(s: &UiScript) -> f64 {
    s.eval::<f64>("return CastingBarFrame:GetValue()").unwrap()
}

fn bar_color(s: &UiScript) -> (f64, f64, f64) {
    s.eval::<(f64, f64, f64)>("local r, g, b = CastingBarFrame:GetStatusBarColor(); return r, g, b")
        .unwrap()
}

fn shown(s: &UiScript) -> bool {
    s.eval::<bool>("return CastingBarFrame:IsShown() == 1")
        .unwrap()
}

fn close3(c: (f64, f64, f64), want: (f64, f64, f64)) -> bool {
    (c.0 - want.0).abs() < 1e-6 && (c.1 - want.1).abs() < 1e-6 && (c.2 - want.2).abs() < 1e-6
}

/// One frame in the app's order (`tick_script`, then `paint_script`): OnUpdate, resolve, draw list.
fn frame(s: &mut UiScript, dt: f32) -> Vec<ExtractedQuad> {
    s.tick(dt);
    s.resolve();
    s.extract()
}

/// The cast bar's own texture quad whose art path ends in `leaf`, if any: the session's other
/// bars share the art.
fn tex_quad<'a>(s: &UiScript, quads: &'a [ExtractedQuad], leaf: &str) -> Option<&'a ExtractedQuad> {
    quads.iter().find(|q| match &q.content {
        QuadContent::Texture { path: Some(p), .. } => {
            p.ends_with(leaf) && s.quad_owner_name(q.target).as_deref() == Some("CastingBarFrame")
        }
        _ => false,
    })
}

#[test]
fn cast_fills_then_completes_green_and_fades() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    assert!(!shown(&s), "hidden at load");

    cast(&mut s, "Frostbolt", 3000);
    assert!(shown(&s));
    assert_eq!(
        s.eval::<String>("return CastingBarFrameText:GetText()")
            .unwrap(),
        "Frostbolt"
    );
    assert!(
        close3(bar_color(&s), (1.0, 0.7, 0.0)),
        "casting is orange (CastingBarFrame.lua:82): {:?}",
        bar_color(&s)
    );

    let start = bar_value(&s);
    for _ in 0..15 {
        s.tick(0.1);
    }
    let mid = bar_value(&s);
    assert!(
        (mid - start - 1.5).abs() < 0.05,
        "1.5 s of ticks advance the fill by 1.5 (got {})",
        mid - start
    );

    spell_event(&mut s, "UNIT_SPELLCAST_STOP");
    assert_eq!(bar_color(&s), (0.0, 1.0, 0.0), "completed is green");
    for _ in 0..30 {
        s.tick(0.1);
    }
    assert!(!shown(&s), "the flash and fade end hidden");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Pushback re-reads `UnitCastingInfo` on `UNIT_SPELLCAST_DELAYED` (CastingBarFrame.lua:163-185):
/// the window moves out, and the bar neither hides nor fails.
#[test]
fn a_hit_pushes_the_bar_back_it_does_not_cancel() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Fireball", 3000);
    for _ in 0..15 {
        s.tick(0.1);
    }
    let minmax = "local a, b = CastingBarFrame:GetMinMaxValues(); return a, b";
    let (_, max_before) = s.eval::<(f64, f64)>(minmax).unwrap();
    let remaining_before = max_before - bar_value(&s);
    assert!(
        (remaining_before - 1.5).abs() < 0.05,
        "half-way: ~1.5 s left (got {remaining_before})"
    );

    // The server's delay moves the end out by 500 ms; App republishes, then fires.
    let model_start = s
        .eval::<f64>("return select(5, UnitCastingInfo('player'))")
        .unwrap() as u32;
    let end = s
        .eval::<f64>("return select(6, UnitCastingInfo('player'))")
        .unwrap() as u32;
    let delayed = WrathUnitCast {
        name: "Fireball".into(),
        cast_count: CAST_ID,
        start_ms: model_start,
        end_ms: end + 500,
        ..Default::default()
    };
    s.set_wrath_unit_casts(HashMap::from([(PLAYER, delayed)]), HashMap::new());
    s.fire_event("UNIT_SPELLCAST_DELAYED", vec![player()]);
    assert!(shown(&s), "a hit never hides the bar");
    assert!(
        close3(bar_color(&s), (1.0, 0.7, 0.0)),
        "still orange, not failed red: {:?}",
        bar_color(&s)
    );
    let (_, max_after) = s.eval::<(f64, f64)>(minmax).unwrap();
    assert!(
        (max_after - max_before - 0.5).abs() < 1e-6,
        "the window end moved out by the 0.5 s pushback"
    );
    let remaining_after = max_after - bar_value(&s);
    assert!(
        (remaining_after - remaining_before - 0.5).abs() < 0.05,
        "the cast has ~0.5 s more to run: {remaining_before} -> {remaining_after}"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `UNIT_SPELLCAST_FAILED` with the cast's own castID turns the bar red, writes `FAILED` and holds
/// it `CASTING_BAR_HOLD_TIME` before the fade (CastingBarFrame.lua:143-162).
#[test]
fn failed_cast_turns_red_holds_then_fades() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Frostbolt", 3000);
    spell_event(&mut s, "UNIT_SPELLCAST_FAILED");
    assert_eq!(bar_color(&s), (1.0, 0.0, 0.0), "failed is red");
    assert_eq!(
        s.eval::<String>("return CastingBarFrameText:GetText()")
            .unwrap(),
        "Failed"
    );
    for _ in 0..5 {
        s.tick(0.1);
    }
    assert_eq!(
        s.eval::<f64>("return CastingBarFrame:GetAlpha()").unwrap(),
        1.0,
        "holds at full alpha inside CASTING_BAR_HOLD_TIME"
    );
    for _ in 0..30 {
        s.tick(0.1);
    }
    assert!(!shown(&s), "after the hold the fade ends hidden");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// A channel opens green and drains (CastingBarFrame.lua:186-210); its update re-anchors the
/// window, and its stop flashes and fades.
#[test]
fn channel_counts_down_not_up() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    channel(&mut s, "Arcane Missiles", 6000);
    assert!(shown(&s));
    assert_eq!(
        s.eval::<String>("return CastingBarFrameText:GetText()")
            .unwrap(),
        "Channeling",
        "the player's own channel reads CHANNELING without attributesEx 0x20000000 (0x612151)"
    );
    assert_eq!(
        bar_color(&s),
        (0.0, 1.0, 0.0),
        "a 3.3.5 channel opens green"
    );

    let full = bar_value(&s);
    for _ in 0..10 {
        s.tick(0.1);
    }
    let after_1s = bar_value(&s);
    assert!(
        (full - after_1s - 1.0).abs() < 0.05,
        "a channel drains: 1 s of ticks take the value down by 1 (got {})",
        full - after_1s
    );

    // The server's mid-channel update: 2 s left, republished, then fired.
    seat(&mut s, "Arcane Missiles", 2000, true);
    s.fire_event("UNIT_SPELLCAST_CHANNEL_UPDATE", vec![player()]);
    s.tick(0.1);
    let corrected = bar_value(&s);
    assert!(
        corrected < after_1s,
        "an update to 2 s left pulls the fill down ({corrected} < {after_1s})"
    );

    spell_event(&mut s, "UNIT_SPELLCAST_CHANNEL_STOP");
    for _ in 0..30 {
        s.tick(0.1);
    }
    assert!(!shown(&s), "ends hidden");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// A channel that runs out closes on the bar's own clock, and a late stop lands on a hidden frame,
/// which the stop arm ignores.
#[test]
fn a_finished_channel_fades_on_its_own_clock_and_the_late_stop_is_inert() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    channel(&mut s, "Arcane Missiles", 1000);
    assert!(shown(&s));
    for _ in 0..9 {
        s.tick(0.1);
    }
    assert!(shown(&s), "still channelling at 0.9 s of a 1 s channel");
    for _ in 0..40 {
        s.tick(0.1);
    }
    assert!(!shown(&s), "the bar closed on its own clock");
    spell_event(&mut s, "UNIT_SPELLCAST_CHANNEL_STOP");
    assert!(
        !shown(&s),
        "the late stop lands on a hidden frame and does nothing"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

// ── The draw list ────────────────────────────────────────────────────────────────────────────────

const FLASH: &str = "UI-CastingBar-Flash";
const SPARK: &str = "UI-CastingBar-Spark";
const BORDER: &str = "UI-CastingBar-Border";
const FILL: &str = "UI-StatusBar";

/// The completion flash, an additive texture over the whole frame, stays hidden through the cast:
/// every casting `OnUpdate` hides it.
#[test]
fn the_flash_stays_hidden_for_the_whole_cast() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Frostbolt", 3000);
    for i in 0..25 {
        let quads = frame(&mut s, 0.1);
        assert!(
            tex_quad(&s, &quads, FLASH).is_none(),
            "tick {i}: the flash must not draw during a cast"
        );
        assert!(
            tex_quad(&s, &quads, SPARK).is_some(),
            "tick {i}: the spark rides the fill's leading edge"
        );
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// On completion the flash shows at alpha 0 and each `OnUpdate` adds `CASTING_BAR_FLASH_STEP`
/// (0.2), once per frame whatever its length (CastingBarFrame.lua:290-300). The spark hides, and
/// the frame stays opaque until the ramp ends.
#[test]
fn the_flash_blooms_from_zero_only_on_completion() {
    wrathplus_formats::wow_data_or_skip!();
    const REF_TICK: f32 = 1.0 / 30.0; // any frame time: the ramp steps once per OnUpdate
    let mut s = harness();
    cast(&mut s, "Frostbolt", 3000);
    frame(&mut s, 0.1);
    spell_event(&mut s, "UNIT_SPELLCAST_STOP");

    // The frame is opaque through the ramp, so the quad's alpha is the flash's own.
    for (i, expected) in [0.2f32, 0.4, 0.6, 0.8, 1.0].into_iter().enumerate() {
        let quads = frame(&mut s, REF_TICK);
        let flash = tex_quad(&s, &quads, FLASH).expect("the flash draws after completion");
        assert!(
            (flash.alpha - expected).abs() < 1e-5,
            "ramp step {i}: flash alpha {} != {expected}",
            flash.alpha
        );
        assert!(
            tex_quad(&s, &quads, SPARK).is_none(),
            "ramp step {i}: the spark is hidden once the cast lands"
        );
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Only the stop arms set the flash ramp (CastingBarFrame.lua:118-142); a failure never does.
#[test]
fn a_failed_cast_never_flashes() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Frostbolt", 3000);
    frame(&mut s, 0.1);
    spell_event(&mut s, "UNIT_SPELLCAST_FAILED");
    for i in 0..25 {
        let quads = frame(&mut s, 0.1);
        assert!(
            tex_quad(&s, &quads, FLASH).is_none(),
            "tick {i}: a failed cast must not bloom"
        );
    }
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// `drawLayer="BORDER"` on the `<StatusBar>` (`CastingBarFrame.xml:4`) draws the fill under the
/// ARTWORK border art.
#[test]
fn the_fill_draws_beneath_the_border_art() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Frostbolt", 3000);
    let quads = frame(&mut s, 0.1);
    let fill = tex_quad(&s, &quads, FILL).expect("fill");
    let border = tex_quad(&s, &quads, BORDER).expect("border");
    assert!(
        fill.z < border.z,
        "the fill (z {}) draws before the border art (z {})",
        fill.z,
        border.z
    );
}

/// A StatusBar fill crops its texture rather than squeezing it (`0x770410`): at fraction f the
/// quad is f of the width and samples u in [0, f].
#[test]
fn the_fill_crops_its_texture_rather_than_stretching_it() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    cast(&mut s, "Frostbolt", 4000);
    for _ in 0..10 {
        frame(&mut s, 0.1);
    }
    let quads = frame(&mut s, 0.0);
    let fill = tex_quad(&s, &quads, FILL).expect("fill");
    let QuadContent::Texture { tex_coords, .. } = &fill.content else {
        panic!("fill is a texture")
    };
    let [l, r, t, b] = tex_coords
        .expect("a bar fill always carries its crop")
        .edges();
    let rect = fill.rect.expect("fill rect");
    let frac = rect.width() / 195.0;
    assert!((frac - 0.25).abs() < 0.02, "a quarter filled (got {frac})");
    assert!(
        (l - 0.0).abs() < 1e-6 && (t - 0.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6,
        "a horizontal bar crops only the u axis (got {l} {r} {t} {b})"
    );
    assert!(
        (r - frac).abs() < 1e-5,
        "u1 tracks the fill fraction: {r} != {frac}"
    );
}

/// The resolved bottom edge of a named frame, in UI units (y-up from the screen bottom).
fn bottom(s: &UiScript, name: &str) -> f64 {
    s.eval::<f64>(&format!("return {name}:GetBottom()"))
        .unwrap()
}

/// The stock manage pass seats the cast bar from `UIPARENT_MANAGED_FRAME_POSITIONS`
/// (UIParent.lua:1186): `menuBarTop` plus its `yOffset`, plus `bottomEither` once a bottom
/// multibar shows (`UIParent_ManageFramePosition`, :1225-1270).
#[test]
fn managed_positions_track_the_bottom_bar_stack() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = harness();
    let row = |s: &UiScript, key: &str| -> f64 {
        s.eval::<f64>(&format!(
            "return UIPARENT_MANAGED_FRAME_POSITIONS.CastingBarFrame.{key}"
        ))
        .unwrap()
    };
    s.run("UIParent_ManageFramePositions()").unwrap();
    s.resolve();
    // `menuBarTop` is a file local: 55 on this 4:3 screen (`UpdateMenuBarTop`, :1167-1175).
    let base = 55.0 + row(&s, "yOffset");
    assert_eq!(bottom(&s, "CastingBarFrame"), base, "menuBarTop + yOffset");

    s.run("SHOW_MULTI_ACTIONBAR_1 = 1 MultiActionBar_Update() UIParent_ManageFramePositions()")
        .unwrap();
    s.resolve();
    assert_eq!(
        bottom(&s, "CastingBarFrame"),
        base + row(&s, "bottomEither"),
        "a shown bottom bar raises it by bottomEither"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}
