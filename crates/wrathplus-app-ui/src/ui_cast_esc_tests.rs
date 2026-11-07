//! `SpellStopCasting()` from the VM against the cast lifecycle: the Esc ladder's casting half,
//! which needs the VM, the casting flag and `script_calls`, so it lives on the UI side of
//! `spell::inflight`'s local cancel.

use std::collections::HashMap;
use std::time::Instant;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use wrathplus_formats::SpellDisplay;
use wrathplus_ui::script::{ScriptCall, UiScript};

use super::push_casting_flag;
use crate::spell::cast_test_support::{harness, mark_casting, ARCANE_MISSILES, HEROIC_STRIKE};
use crate::spell::{ActiveChannel, AutoRepeatActive, PendingCast, QueuedMeleeSpell, SelfCancel};

const FIREBALL: u32 = 133;

/// The queued repeat must be stoppable before movement/facing permits its first send.
#[test]
fn esc_spends_one_press_on_queued_auto_repeat_without_cancelling_the_pending_cast() {
    let (mut app, rx) = harness(HashMap::new());
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.world_mut().resource_mut::<AutoRepeatActive>().1.queued = Some(75);
    app.world_mut()
        .resource_mut::<PendingCast>()
        .arm(FIREBALL, Instant::now(), true);
    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1)
    );
    apply_stops(&mut app);
    assert!(!app.world().resource::<AutoRepeatActive>().any());
    assert!(app
        .world()
        .resource::<PendingCast>()
        .in_flight(Instant::now()));
    assert!(rx.is_empty(), "no running repeat or cast was cancelled");
}

/// Apply the `SpellStopCasting()` calls the VM queued, as `crate::script_calls` does.
fn apply_stops(app: &mut App) {
    app.world_mut()
        .run_system_once(|mut script: NonSendMut<UiScript>, mut cancel: SelfCancel| {
            crate::script_calls::in_call_order(&mut script, |_, call| {
                if call == ScriptCall::SpellStopCasting {
                    cancel.stop_casting();
                }
            });
        })
        .unwrap();
}

/// Mid-channel `SpellStopCasting()` answers nil and nothing is sent.
#[test]
fn esc_cannot_stop_a_channel() {
    let (mut app, rx) = harness(HashMap::new());
    app.insert_non_send_resource(UiScript::new().unwrap());
    let now = Instant::now();
    app.world_mut()
        .resource_mut::<ActiveChannel>()
        .start(ARCANE_MISSILES, 5_000, now);

    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        None,
        "mid-channel the binding answers nil — the /stopcasting quirk"
    );
    apply_stops(&mut app);
    assert!(
        rx.try_recv().is_err(),
        "no wire cancel — channels break only on the movement path"
    );
    assert_eq!(
        app.world().resource::<ActiveChannel>().current(now),
        Some(ARCANE_MISSILES)
    );
}

/// `SpellStopCasting` (`0x6e6e80`) stops the auto-repeat first and spends the press.
#[test]
fn esc_cancels_the_auto_repeat_first_and_the_cast_on_the_next_press() {
    const AUTO_SHOT: u32 = 75;
    let (mut app, rx) = harness(HashMap::new());
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.world_mut().resource_mut::<AutoRepeatActive>().0 = Some(AUTO_SHOT);
    app.world_mut()
        .resource_mut::<PendingCast>()
        .arm(FIREBALL, Instant::now(), true);
    mark_casting(&mut app, FIREBALL);

    // Press 1 stops the auto-repeat only.
    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1)
    );
    apply_stops(&mut app);
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::net::ClientCommand::CancelAutoRepeat)
        ),
        "the auto-repeat cancel ships first — the reference's priority branch"
    );
    assert!(rx.try_recv().is_err(), "the cast survives the first press");
    assert!(app.world().resource::<AutoRepeatActive>().0.is_none());
    assert!(
        app.world()
            .resource::<PendingCast>()
            .in_flight(Instant::now()),
        "the send guard still holds — one press stopped one thing"
    );

    // Press 2 cancels the cast.
    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1)
    );
    apply_stops(&mut app);
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::net::ClientCommand::CancelCast { spell_id: FIREBALL })
        ),
        "the second press reaches the cast half"
    );
}

/// Esc with a strike queued cancels it and returns 1 (`IsCasting 0x6e3d30` is true), so
/// the Esc ladder stops before `ClearTarget()`.
#[test]
fn esc_unqueues_a_strike_and_eats_the_press() {
    let (mut app, rx) = harness(HashMap::from([(
        HEROIC_STRIKE,
        SpellDisplay {
            attributes: 0x5_0014, // the shipped row: on-next-swing (`& 0x404`)
            interrupt_flags: 0,   // no movement bit, also shipped
            ..Default::default()
        },
    )]));
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.world_mut()
        .resource_mut::<QueuedMeleeSpell>()
        .arm(HEROIC_STRIKE);

    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1),
        "the queued strike makes IsCasting true, so the binding EATS the press — which \
         is what spares the target three rungs further down the ladder"
    );
    apply_stops(&mut app);
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::net::ClientCommand::CancelCast {
                spell_id: HEROIC_STRIKE
            })
        ),
        "AbortCast names the queued strike on the wire, exactly as it would a cast"
    );
    assert_eq!(
        app.world().resource::<QueuedMeleeSpell>().current(),
        None,
        "the slot opens locally — the checked ring darkens this frame, not one RTT later"
    );
}

/// A cast nested over a queued strike (`PushPopNestedCast 0x6e4ad0`) dies on the first
/// Esc, the strike on the second; this pins the local order only.
#[test]
fn esc_takes_the_cast_first_and_the_strike_on_the_next_press() {
    let (mut app, rx) = harness(HashMap::new());
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.world_mut()
        .resource_mut::<QueuedMeleeSpell>()
        .arm(HEROIC_STRIKE);
    app.world_mut()
        .resource_mut::<PendingCast>()
        .arm(FIREBALL, Instant::now(), true);
    mark_casting(&mut app, FIREBALL);

    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1)
    );
    apply_stops(&mut app);
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::net::ClientCommand::CancelCast { spell_id: FIREBALL })
        ),
        "the nested cast is what sits in the inflight slot — it dies first"
    );
    assert!(rx.try_recv().is_err());
    assert_eq!(
        app.world().resource::<QueuedMeleeSpell>().current(),
        Some(HEROIC_STRIKE),
        "the strike is the SAVE underneath, untouched by the first press"
    );

    // Press 2: the pop put the strike back in the slot.
    app.world_mut().run_system_once(push_casting_flag).unwrap();
    assert_eq!(
        app.world()
            .non_send_resource::<UiScript>()
            .eval::<Option<i64>>("return SpellStopCasting()")
            .unwrap(),
        Some(1),
        "still stoppable — so this press is eaten too, and the target still survives"
    );
    apply_stops(&mut app);
    assert!(matches!(
        rx.try_recv(),
        Ok(crate::net::ClientCommand::CancelCast {
            spell_id: HEROIC_STRIKE
        })
    ));
    assert_eq!(app.world().resource::<QueuedMeleeSpell>().current(), None);
}
