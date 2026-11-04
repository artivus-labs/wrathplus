//! A raising or looping addon at world entry: the stock handler files its error, and the load
//! goes on.

use super::*;

/// [`booted_world`] with what a client always has and these error tests need clean: a window,
/// whose display modes seed each VM (stock `UpdateMenuBarTop` divides the current mode's width by
/// its height, `UIParent.lua:1171`), and ChrClasses.dbc (`ChatConfigFrame.xml:134` formats
/// `LOCALIZED_CLASS_NAMES_MALE`), so the only filed error is the test's own.
fn client_world() -> World {
    let mut world = booted_world();
    world.spawn((
        Window {
            resolution: (1024, 768).into(),
            ..Default::default()
        },
        bevy::window::PrimaryWindow,
    ));
    let data = wrathplus_formats::wow_data().expect("installed data");
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let classes = wrathplus_formats::load_chr_classes(&mut chain).expect("ChrClasses.dbc");
    world.insert_resource(crate::chr_classes::ChrClassTable(classes));
    world
}

/// What 3.3.5's `_ERRORMESSAGE` (BasicControls.xml:13-41) filed in `Blizzard_DebugTools`' store.
fn shown_script_error(script: &wrathplus_ui::script::UiScript) -> String {
    // `_ScriptErrorsFrame` is a file local (`Blizzard_DebugTools.lua:369`); its OnLoad stores
    // the frame there, so the filed messages hang off the global frame itself.
    let filed = "return ScriptErrorsFrame and ScriptErrorsFrame.messages \
                 and table.concat(ScriptErrorsFrame.messages, '\\n')";
    script
        .eval::<Option<String>>(filed)
        .expect("eval")
        .unwrap_or_default()
}

/// A file-scope raise drops only its addon, and the stock handler files the error.
#[test]
fn an_addon_error_while_entering_world_reports_on_screen_and_the_sibling_loads() {
    wrathplus_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (tmp, _c, _h) = hermetic_probe("addon-error");
    // A second addon, first alphabetically, that dies at file scope.
    let dir = tmp.join("wrathplus-config/AddOns/AaBroken");
    std::fs::create_dir_all(&dir).expect("broken addon dir");
    std::fs::write(dir.join("AaBroken.toc"), "## Interface: 30300\nboom.lua\n").expect("toc");
    std::fs::write(dir.join("boom.lua"), "error('file-scope boom')\n").expect("lua");
    let mut world = client_world();

    log_in_as(&mut world, "Onehunter", 1);
    assert_eq!(
        probe_saw(&world).as_deref(),
        Some("Onehunter"),
        "the addon AFTER the broken one still loads — a neighbour's error drops only itself"
    );

    // The app's per-frame drain runs the dispatch; the test runs the same call.
    let mut script = world
        .remove_non_send_resource::<wrathplus_ui::script::UiScript>()
        .expect("VM");
    script.dispatch_script_errors_to_handler();
    let shown = shown_script_error(&script);
    assert!(
        shown.contains("file-scope boom"),
        "the handler filed the actual error, got: {shown:?} ",
    );
    drop(script);
    drop(world);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The load's instruction budget fails the looping addon; the test finishing is the claim.
#[test]
fn a_looping_addon_cannot_freeze_world_entry() {
    wrathplus_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (tmp, _c, _h) = hermetic_probe("addon-loop");
    let dir = tmp.join("wrathplus-config/AddOns/AaSpin");
    std::fs::create_dir_all(&dir).expect("spin addon dir");
    std::fs::write(dir.join("AaSpin.toc"), "## Interface: 30300\nspin.lua\n").expect("toc");
    std::fs::write(dir.join("spin.lua"), "while true do end\n").expect("lua");
    let mut world = client_world();

    log_in_as(&mut world, "Onehunter", 1);

    assert_eq!(
        probe_saw(&world).as_deref(),
        Some("Onehunter"),
        "the addon after the spinner still loads — the budget failed one addon, not the entry"
    );
    // The budget raise reaches the handler queue, so the dialog names the loop.
    let mut script = world
        .remove_non_send_resource::<wrathplus_ui::script::UiScript>()
        .expect("VM");
    script.dispatch_script_errors_to_handler();
    let shown = shown_script_error(&script);
    assert!(
        shown.contains("instruction budget exhausted"),
        "the handler filed the runaway loop's budget message, got: {shown:?} ",
    );

    drop(script);
    drop(world);
    let _ = std::fs::remove_dir_all(&tmp);
}
