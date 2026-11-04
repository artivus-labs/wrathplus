//! `SetMouselookOverrideBinding`'s table in dispatch: probed first while mouselooking.

use super::*;

/// While the TurnOrAction channel is held, a key in the mouselook table runs its override; with
/// the channel released the key runs its normal binding again.
#[test]
fn a_mouselook_override_runs_only_while_mouselooking() {
    let script = with_addon(PROBE_BINDINGS);
    script
        .run(r#"SetBinding("J", "PROBEEDGE") SetMouselookOverrideBinding("J", "PROBEHOLD")"#)
        .expect("bind");
    let mut app = vm_harness(script);

    press_key(&mut app, KeyCode::KeyJ);
    app.update();
    release_key(&mut app, KeyCode::KeyJ);
    app.update();
    assert_eq!(
        lua_count(&app, "PROBE_EDGE"),
        1,
        "not looking: the normal key"
    );
    assert_eq!(lua_count(&app, "PROBE_DOWN"), 0);

    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_turn_or_action_held(true);
    press_key(&mut app, KeyCode::KeyJ);
    app.update();
    release_key(&mut app, KeyCode::KeyJ);
    app.update();
    assert_eq!(lua_count(&app, "PROBE_DOWN"), 1, "looking: the override");
    assert_eq!(lua_count(&app, "PROBE_UP"), 1, "and its release half");
    assert_eq!(lua_count(&app, "PROBE_EDGE"), 1);

    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_turn_or_action_held(false);
    press_key(&mut app, KeyCode::KeyJ);
    app.update();
    assert_eq!(
        lua_count(&app, "PROBE_EDGE"),
        2,
        "released: the normal key again"
    );
}
