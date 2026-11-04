//! The stock 3.3.5 windows the player reaches first, now that no WrathPlus layer replaces them: the
//! ESC menu (`GameMenuFrame`, through `ToggleGameMenu`, what the ESCAPE binding runs), and from it
//! `VideoOptionsFrame`, `InterfaceOptionsFrame` and `Blizzard_BindingUI`'s `KeyBindingFrame`
//! (loaded on demand by `KeyBindingFrame_LoadUI`). Each opens on the production load and raises
//! nothing.

use wrathplus_ui::script::UiScript;

use crate::local_state::test_env::ENV_LOCK;

fn shown(s: &UiScript, frame: &str) -> bool {
    s.eval::<bool>(&format!("return {frame} ~= nil and {frame}:IsShown() == 1"))
        .unwrap_or(false)
}

/// Run `lua` and a tick; nothing it runs may raise.
fn step(s: &mut UiScript, lua: &str) {
    let before = s.errors().len();
    s.run(lua).unwrap_or_else(|e| panic!("{lua}: {e}"));
    s.tick(0.05);
    let errors = s.errors();
    assert_eq!(errors.len(), before, "{lua}: {:?}", &errors[before..]);
}

#[test]
fn the_escape_menu_and_its_settings_windows_open() {
    wrathplus_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut s, failures) = super::production_load::production_load_with("menus", "", |_| {});
    for event in ["VARIABLES_LOADED", "PLAYER_LOGIN", "PLAYER_ENTERING_WORLD"] {
        s.fire_event(event, vec![]);
    }
    s.tick(0.05);
    // The load itself is clean; the login events' raises are the rig's (a default player
    // without a faction, which `PVPBattlegroundFrame.lua` concatenates), not the menus'.
    assert!(failures.is_empty(), "load failures: {failures:#?}");
    eprintln!("login-event raises (rig): {:?}", s.errors());

    step(&mut s, "ToggleGameMenu()");
    assert!(shown(&s, "GameMenuFrame"), "ESC opens the game menu");

    step(&mut s, "GameMenuButtonOptions:Click()");
    assert!(shown(&s, "VideoOptionsFrame"), "Video opens from the menu");
    step(&mut s, "VideoOptionsFrameCancel:Click()");

    step(&mut s, "ToggleGameMenu()");
    step(&mut s, "GameMenuButtonUIOptions:Click()");
    assert!(
        shown(&s, "InterfaceOptionsFrame"),
        "Interface opens from the menu"
    );
    step(&mut s, "InterfaceOptionsFrameCancel:Click()");

    step(&mut s, "ToggleGameMenu()");
    step(&mut s, "GameMenuButtonKeybindings:Click()");
    assert!(
        s.eval::<bool>("return IsAddOnLoaded('Blizzard_BindingUI') == 1")
            .unwrap(),
        "Key Bindings loads Blizzard_BindingUI on demand"
    );
    assert!(
        shown(&s, "KeyBindingFrame"),
        "Key Bindings opens from the menu"
    );
    step(&mut s, "ToggleGameMenu()");
    assert!(
        !shown(&s, "KeyBindingFrame"),
        "ESC closes the bindings window"
    );
}
