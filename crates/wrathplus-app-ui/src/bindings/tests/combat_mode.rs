//! Build 12340's combat binding mode in dispatch: a key bound in mode 2 runs while the player is
//! auto-attacking or in combat, its mode-1 command otherwise.

use super::*;
use crate::app_core::session::{ObjectStore, SelfPlayer};
use wrathplus_protocol::types::Field;
use wrathplus_protocol::ObjectFields;

/// The player's in-combat flag or an attack target switches mode 2 on; the switch fires
/// `UPDATE_BINDINGS` once per flip and dispatch follows the same frame.
#[test]
fn mode_two_keys_run_while_the_player_fights() {
    let script = with_addon(PROBE_BINDINGS);
    script
        .run(
            r#"SetBinding("J", "PROBEEDGE") SetBinding("J", "PROBEHOLD", 2)
               UPDATES = 0 local f = CreateFrame("Frame") f:RegisterEvent("UPDATE_BINDINGS")
               f:SetScript("OnEvent", function() UPDATES = UPDATES + 1 end)"#,
        )
        .expect("bind");
    let mut app = vm_harness(script);
    app.add_systems(
        Update,
        crate::bindings::combat_mode::sync_combat_binding_mode.before(sync_dispatch),
    );
    let me = app
        .world_mut()
        .spawn((SelfPlayer, ObjectStore(ObjectFields::from_fields(&[]))))
        .id();
    let tap = |app: &mut App| {
        press_key(app, KeyCode::KeyJ);
        app.update();
        release_key(app, KeyCode::KeyJ);
        app.update();
    };
    let updates = |app: &App| lua_count(app, "UPDATES");

    tap(&mut app);
    assert_eq!(lua_count(&app, "PROBE_EDGE"), 1, "out of combat: mode 1");
    let before = updates(&app);

    app.world_mut()
        .entity_mut(me)
        .insert(crate::creature_anim::Engaged(7));
    tap(&mut app);
    assert_eq!(lua_count(&app, "PROBE_DOWN"), 1, "attacking: mode 2");
    assert_eq!(updates(&app), before + 1, "one flip, one UPDATE_BINDINGS");

    app.world_mut()
        .entity_mut(me)
        .remove::<crate::creature_anim::Engaged>()
        .insert(ObjectStore(ObjectFields::from_fields(&[(
            Field::UnitFlags,
            0x0008_0000,
        )])));
    tap(&mut app);
    assert_eq!(lua_count(&app, "PROBE_DOWN"), 2, "in combat: still mode 2");
    assert_eq!(updates(&app), before + 1, "no flip");

    app.world_mut()
        .entity_mut(me)
        .insert(ObjectStore(ObjectFields::from_fields(&[])));
    tap(&mut app);
    assert_eq!(lua_count(&app, "PROBE_EDGE"), 2, "out again: mode 1");
    assert_eq!(updates(&app), before + 2);
}
