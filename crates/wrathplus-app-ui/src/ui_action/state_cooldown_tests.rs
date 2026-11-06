//! Action-feed macro and unresolved cooldown regressions.

use super::*;

#[test]
fn the_feed_pushes_a_bare_macro_as_usable() {
    use crate::ui_macro::{BoundSpell, MacroBoundSpells};
    use wrathplus_protocol::types::{ActionButton, ACTION_KIND_MACRO};

    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    let mut actions = PlayerActions::default();
    for (slot, index) in [(0u8, 1u32), (1, 2), (2, 3)] {
        actions.buttons.insert(
            slot,
            ActionButton {
                slot,
                action: index,
                kind: ACTION_KIND_MACRO,
            },
        );
    }
    // Unresolved occupied item and spell slots must retain enable 0 (`0x5a8e40`).
    for (slot, kind) in [(3, ACTION_KIND_ITEM), (4, ACTION_KIND_SPELL)] {
        actions.buttons.insert(
            slot,
            ActionButton {
                slot,
                action: 999999,
                kind,
            },
        );
    }
    // Macro 1 is `.spawn 16032`, 2 an unknown `/cast Pyroblast`, and 3 does not exist.
    let mut bound = MacroBoundSpells::default();
    bound.0.insert(1, BoundSpell::None);
    bound.0.insert(2, BoundSpell::Unresolved);
    app.insert_resource(actions)
        .insert_resource(bound)
        .init_resource::<Cooldowns>()
        .init_resource::<crate::spell::SpellModifiers>()
        .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
        .init_resource::<crate::app_core::ui_flags::UiClock>()
        .init_resource::<AutoRepeatActive>()
        .init_resource::<crate::spell::PendingCast>()
        .init_resource::<crate::spell::QueuedMeleeSpell>()
        .init_resource::<crate::spell::ActiveChannel>()
        .init_resource::<crate::spell::SpellTargeting>()
        .init_resource::<crate::spell::HeldForPick>()
        .init_resource::<Selection>()
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::net::Reputations>()
        .init_resource::<Items>()
        .insert_resource(NetCommands(tx));
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.add_systems(Update, feed_action_state);
    app.update();

    let script = app.world().non_send_resource::<UiScript>();
    let usable = |action: u32| {
        script
            .eval::<bool>(&format!(
                "return (IsUsableAction({action})) and true or false"
            ))
            .unwrap()
    };
    for slot in [4, 5, 6] {
        assert_eq!(
            script
                .eval::<(i64, i64, i64)>(&format!("return GetActionCooldown({slot})"))
                .unwrap(),
            (0, 0, 0)
        );
    }
    assert!(usable(1), "a macro that casts nothing is a usable button");
    assert!(
        !usable(2),
        "a /cast of an unknown spell is grey (the reference's -1)"
    );
    assert!(!usable(3), "a slot whose macro no longer exists is grey");
    assert!(
        !script
            .eval::<bool>("local _, oom = IsUsableAction(2) return oom and true or false")
            .unwrap(),
        "grey, not the out-of-power blue: notEnoughMana stays 0 on the spell-less leg"
    );
}
