//! The 3.3.5 plate's cast bar, elite icon and threat flash (`0x98f040`, `0x98e6e0`, `0x98eaef`).

use super::nameplate::{drive, plate, vm};
use crate::script::UiScript;

/// The cast bar (`0x98f040`): hidden without a cast; a cast shows the bar at its value in cast
/// orange with the spell's icon and the border, and an uninterruptible one swaps the border for
/// the shield; the cast's end hides the bar again.
#[test]
fn the_cast_bar_follows_the_targets_cast() {
    let mut s = vm();
    let mut state = plate("Wolf", 30.0, 40.0);
    drive(&mut s, std::slice::from_ref(&state));
    let castbar_shown = |s: &UiScript| {
        s.eval::<bool>(
            "local _, cast = WorldFrame:GetChildren():GetChildren() return cast:IsShown() == 1",
        )
        .unwrap()
    };
    assert!(!castbar_shown(&s), "no cast, no bar");

    state.cast = Some(wrathplus_ui_plate_cast(1.5, 0.5, false));
    drive(&mut s, std::slice::from_ref(&state));
    assert!(castbar_shown(&s));
    assert!(s
        .eval::<bool>(
            r#"
            local p = WorldFrame:GetChildren()
            local _, cast = p:GetChildren()
            local _, _, castborder, shield, icon = p:GetRegions()
            local lo, hi = cast:GetMinMaxValues()
            local r, g, b = cast:GetStatusBarColor()
            return lo == 0 and math.abs(hi - 1.5) < 1e-6 and math.abs(cast:GetValue() - 0.5) < 1e-6
               and math.abs(r - 1) < 1e-6 and math.abs(g - 0.7) < 1e-6 and b == 0
               and castborder:IsShown() == 1 and shield:IsShown() == nil
               and icon:IsShown() == 1 and icon:GetTexture() == "Interface\\Icons\\Spell_Fire_Fireball02"
        "#
        )
        .unwrap());

    state.cast = Some(wrathplus_ui_plate_cast(1.5, 0.75, true));
    drive(&mut s, std::slice::from_ref(&state));
    assert!(s
        .eval::<bool>(
            r#"
            local _, _, castborder, shield = WorldFrame:GetChildren():GetRegions()
            return castborder:IsShown() == nil and shield:IsShown() == 1
        "#
        )
        .unwrap());

    state.cast = None;
    drive(&mut s, std::slice::from_ref(&state));
    assert!(!castbar_shown(&s), "the cast's end hides the bar");
}

fn wrathplus_ui_plate_cast(
    max: f32,
    value: f32,
    not_interruptible: bool,
) -> crate::script::PlateCast {
    crate::script::PlateCast {
        max,
        value,
        icon: Some("Interface\\Icons\\Spell_Fire_Fireball02".into()),
        channel: false,
        not_interruptible,
    }
}

/// The elite dragon shows for an elite unit only (`0x98e6e0`).
#[test]
fn the_elite_icon_marks_an_elite() {
    let mut s = vm();
    let mut state = plate("Ogre", 30.0, 40.0);
    drive(&mut s, std::slice::from_ref(&state));
    let elite = |s: &UiScript| {
        s.eval::<bool>("return select(11, WorldFrame:GetChildren():GetRegions()):IsShown() == 1")
            .unwrap()
    };
    assert!(!elite(&s));
    state.elite = true;
    drive(&mut s, std::slice::from_ref(&state));
    assert!(elite(&s));
}

/// The threat flash (region 1) shows in the warning's colour, and hides without one (`0x98eaef`).
#[test]
fn the_threat_flash_shows_in_the_warnings_colour() {
    let mut s = vm();
    let mut state = plate("Ogre", 30.0, 40.0);
    drive(&mut s, std::slice::from_ref(&state));
    let flash = |s: &UiScript| {
        s.eval::<(bool, f64, f64, f64)>(
            "local f = WorldFrame:GetChildren():GetRegions() \
             local r, g, b = f:GetVertexColor() return f:IsShown() == 1, r, g, b",
        )
        .unwrap()
    };
    assert!(!flash(&s).0, "no warning, no flash");
    state.threat = Some([1.0, 0.6, 0.0]);
    drive(&mut s, std::slice::from_ref(&state));
    let (shown, r, g, b) = flash(&s);
    assert!(shown);
    assert!((r - 1.0).abs() < 1e-6 && (g - 0.6).abs() < 1e-6 && b == 0.0);
    state.threat = None;
    drive(&mut s, std::slice::from_ref(&state));
    assert!(!flash(&s).0);
}
