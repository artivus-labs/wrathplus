//! The bottom-band clearance guards. The main bar, the extra action bars, the stance and pet bars
//! and the reputation bar stack in the bottom band; everything else there (the bags, the cast bar,
//! the chat panes, the default tooltip anchor) clears them through `UIParent_ManageFramePositions`
//! (`UIParent.lua`), which seats the managed frames and writes the managed offsets.

use wrathplus_ui::script::UiScript;

/// The bars a player can raise into the band. The two vertical bars move the tenants sideways,
/// through the `rightLeft` and `rightRight` terms of `CONTAINER_OFFSET_X`.
const RAISABLE_BARS: &[&str] = &[
    "MultiBarBottomLeft",
    "MultiBarBottomRight",
    "ShapeshiftBarFrame",
    "MultiBarRight",
    "MultiBarLeft",
];

/// With any combination of bars raised, nothing in the bottom band overlaps a raised bar. Runs the
/// full UI and reruns the pass after each combination, as a live bar change does.
#[test]
fn no_bottom_band_frame_overlaps_a_raised_bar() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1600.0, 900.0);
    // The in-game UI loads on world entry, so a player always exists by then.
    s.set_unit(
        "player",
        Some(wrathplus_ui::script::UnitState {
            exists: true,
            name: Some("Probefour".into()),
            level: 60,
            ..Default::default()
        }),
    );
    let failures = super::load_default_ui(&mut s);
    assert!(failures.is_empty(), "manifest load errors: {failures:#?}");

    s.run(
        "for _, f in ipairs({WrathPlusBagFrame, WrathPlusBagFrame1, WrathPlusBagFrame2, \
         WrathPlusBagFrame3, WrathPlusBagFrame4}) do if f then f:Show() end end",
    )
    .unwrap();

    // The frames in the band that must clear the bars. Each is checked only while shown, hence
    // the pair floor below.
    const TENANTS: &[&str] = &[
        "WrathPlusBagFrame",
        "WrathPlusBagFrame1",
        "WrathPlusBagFrame2",
        "WrathPlusBagFrame3",
        "WrathPlusBagFrame4",
        "CastingBarFrame",
        "ChatFrame1",
    ];

    let mut pairs = 0usize;
    for mask in 0..(1u32 << RAISABLE_BARS.len()) {
        let mut raised = Vec::new();
        for (i, bar) in RAISABLE_BARS.iter().enumerate() {
            let on = mask & (1 << i) != 0;
            // The stock pass takes its bottom-bar flags from `SHOW_MULTI_ACTIONBAR_1`/`_2`, not
            // from the frames' shown state (`UIParent.lua:1599-1607`).
            let global = match *bar {
                "MultiBarBottomLeft" => Some("SHOW_MULTI_ACTIONBAR_1"),
                "MultiBarBottomRight" => Some("SHOW_MULTI_ACTIONBAR_2"),
                _ => None,
            };
            if let Some(g) = global {
                s.run(&format!("{g} = {}", if on { "1" } else { "nil" }))
                    .unwrap();
            }
            s.run(&format!(
                "if {bar} then {bar}:{}() end",
                if on { "Show" } else { "Hide" }
            ))
            .unwrap();
            if on {
                raised.push(*bar);
            }
        }
        s.run("UIParent_ManageFramePositions()").unwrap();
        s.resolve();

        for bar in &raised {
            if !shown(&s, bar) {
                continue;
            }
            let bar_rect = rect(&s, bar);
            for victim in TENANTS {
                if !shown(&s, victim) {
                    continue;
                }
                let v = rect(&s, victim);
                pairs += 1;
                assert!(
                    !overlaps(bar_rect, v),
                    "with {raised:?} raised, {victim} {v:?} overlaps {bar} {bar_rect:?} — \
                     something in the bottom band is not clearing the bars. Its seat must come \
                     from UIParent_ManageFramePositions, read fresh."
                );
            }
        }
    }

    // A floor on the pairs compared, so the sweep cannot pass by comparing nothing.
    assert!(
        pairs >= 24,
        "only {pairs} bar/tenant pairs were actually compared — the sweep is not exercising the \
         band it claims to guard"
    );
}

fn shown(s: &UiScript, name: &str) -> bool {
    s.eval::<bool>(&format!(
        "return {name} and {name}:IsShown() and true or false"
    ))
    .unwrap_or(false)
}

/// `(left, bottom, right, top)` in screen pixels.
fn rect(s: &UiScript, name: &str) -> (f32, f32, f32, f32) {
    s.eval::<(f32, f32, f32, f32)>(&format!(
        "return {name}:GetLeft(), {name}:GetBottom(), {name}:GetRight(), {name}:GetTop()"
    ))
    .unwrap_or_else(|e| panic!("rect of {name}: {e}"))
}

/// Touching edges do not overlap: the stack seats windows flush against each other.
fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The item-push card overlaps a raised bottom bar, as in the reference: the card is MEDIUM under
/// the bag button, the multibars are `frameStrata="HIGH"` (`MultiActionBars.xml:36`), and
/// `MultiBarBottomRight`'s last buttons sit over the bag bar, so a push draws behind a filled
/// twelfth button. At 1600x900 the card peaks at x 1253.7..1298.0, y 48.9..93.1, and the bar
/// spans x 806..1306, y 57..95.
#[test]
fn the_item_push_card_shares_the_band_with_a_raised_bar_exactly_as_the_reference_does() {
    wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1600.0, 900.0);
    // The in-game UI loads on world entry, so a player always exists by then.
    s.set_unit(
        "player",
        Some(wrathplus_ui::script::UnitState {
            exists: true,
            name: Some("Probefour".into()),
            level: 60,
            ..Default::default()
        }),
    );
    let failures = super::load_default_ui(&mut s);
    assert!(failures.is_empty(), "manifest load errors: {failures:#?}");
    // The stock `<Model>` card sizes from its file's box: `ForcedBackpackItem.m2` has one 1000 ms
    // non-looping sequence and a 0.02707 x 0.07962 box, in model units.
    s.set_model_facts(
        r"Interface\ItemAnimations\ForcedBackpackItem.mdx",
        wrathplus_ui::widget::ModelFileFacts {
            sequences: vec![wrathplus_ui::widget::SequenceFacts {
                anim_id: 0,
                duration_ms: 1000,
                looping: false,
            }],
            bbox: ([0.0, 0.0, 0.0], [0.02707, 0.07962, 0.0]),
            cameras: 0,
        },
    );
    s.run("MultiBarBottomLeft:Show() MultiBarBottomRight:Show() UIParent_ManageFramePositions()")
        .unwrap();
    s.resolve();

    s.fire_event(
        "ITEM_PUSH",
        vec![
            wrathplus_ui::script::ScriptValue::Int(0),
            wrathplus_ui::script::ScriptValue::Str("Interface\\Icons\\INV_Misc_Bag_08".into()),
        ],
    );
    s.tick(0.133); // the card's opaque peak
    s.resolve();
    assert!(
        shown(&s, "MainMenuBarBackpackButtonItemAnim"),
        "the card plays"
    );

    // The pane is the file's box in layout units at 16:9: 42.41 x 124.72, off the button's
    // BOTTOMRIGHT (-10, 0). The reference's card peaks at 48.9..93.1 above the floor, inside it.
    let card = rect(&s, "MainMenuBarBackpackButtonItemAnim");
    let bar = rect(&s, "MultiBarBottomRight");
    assert!(
        (card.2 - card.0 - 42.41).abs() < 0.05 && (card.3 - card.1 - 124.72).abs() < 0.05,
        "the pane's rect is the file's box: {card:?}"
    );
    assert!(
        card.1 <= 48.9 && card.3 >= 93.1,
        "the reference's card peak 48.9..93.1 lies inside the pane's band: {card:?}"
    );
    assert!(
        overlaps(bar, card),
        "the reference's own geometry puts the card inside the bar's band ({bar:?} vs {card:?}) — \
         if this stops being true, WrathPlus has diverged from it and the divergence needs a record"
    );
}
