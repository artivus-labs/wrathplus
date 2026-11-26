use super::*;
use bevy::ecs::system::RunSystemOnce;

fn offer(caster: u64, name: &str) -> ResurrectOffer {
    ResurrectOffer {
        caster,
        name: name.into(),
        sickness: false,
        has_timer: true,
    }
}

/// PLAYER_UNGHOST hides the popup with `StaticPopup_Hide`, which runs no `OnCancel`, so no
/// Decline clears `resurrect`.
#[test]
fn every_resurrect_message_announces_while_dead_and_never_while_alive() {
    let mut net = DeathNet::default();
    let mut memo = DeathAnnounced::default();
    let name = |o: &ResurrectOffer| Some(o.name.clone());

    net.resurrect = Some(offer(1, "Healer"));
    net.resurrect_generation += 1;
    assert_eq!(
        offer_announcement(&mut memo, &net, true, name).as_deref(),
        Some("Healer")
    );
    assert_eq!(
        offer_announcement(&mut memo, &net, true, name),
        None,
        "once per message"
    );

    // Alive with the offer unanswered: a fresh VM must not announce it.
    let mut reloaded = DeathAnnounced::default();
    assert_eq!(offer_announcement(&mut reloaded, &net, false, name), None);

    // Dead again, a new priest offers.
    net.resurrect = Some(offer(2, "Ptwo"));
    net.resurrect_generation += 1;
    assert_eq!(
        offer_announcement(&mut memo, &net, true, name).as_deref(),
        Some("Ptwo"),
        "the second offer announces"
    );

    // A name still resolving holds the announcement without consuming it.
    net.resurrect_generation += 1;
    assert_eq!(offer_announcement(&mut memo, &net, true, |_| None), None);
    assert_eq!(
        offer_announcement(&mut memo, &net, true, name).as_deref(),
        Some("Ptwo")
    );
}

/// A `/reload` must not restart the release window; a relog must clear it.
#[test]
fn the_session_edge_clears_the_world_scoped_death_memory_and_nothing_else_does() {
    let mut app = App::new();
    app.add_message::<crate::app_core::session::DisconnectedMessage>()
        .insert_resource(DeathFeedState {
            mirror: Some((0x1234, true, true)),
            release: Some(ReleaseTimer::Ends(12.0)),
            healer: Some(0x5678),
            ..Default::default()
        });

    // No session edge: the memory stands.
    app.world_mut()
        .run_system_once(end_session_death_feed)
        .expect("the teardown runs");
    assert_eq!(
        app.world().resource::<DeathFeedState>().mirror,
        Some((0x1234, true, true)),
        "nothing but the session edge may clear the body's own state machine"
    );

    // A `/logout` is a session edge although `session_over` is false for it.
    app.world_mut()
        .write_message(crate::app_core::session::DisconnectedMessage::new(
            "logged out".into(),
            wrathplus_protocol::SessionEnd::LoggedOut,
        ));
    app.world_mut()
        .run_system_once(end_session_death_feed)
        .expect("the teardown runs");
    let feed = app.world().resource::<DeathFeedState>();
    assert!(
        feed.mirror.is_none(),
        "a relog must find no mirror, or the login-while-ghost arm cannot fire"
    );
    assert!(
        feed.release.is_none() && feed.healer.is_none(),
        "the release window is the dead session's, and re-arms at the next login"
    );
}

/// `0x6dc070` reads the release bits once; `0x516210` and `0x516280` answer whole seconds.
#[test]
fn the_release_timer_arms_from_the_bits_and_answers_whole_seconds() {
    let at = |bits, flags| Some(arm_release(bits, flags, 100.0));
    assert_eq!(release_seconds(None, 100.0), 0, "before any death");
    assert_eq!(
        release_seconds(at(0x08, 0), 100.4),
        359,
        "the window, truncated"
    );
    assert_eq!(
        release_seconds(at(0x18, 0), 100.0),
        0,
        "NO_RELEASE_WINDOW wins"
    );
    assert_eq!(
        release_seconds(at(0x08, 0x4000), 100.0),
        -1,
        "out of bounds"
    );
    assert_eq!(release_seconds(at(0, 0), 100.0), -1, "no timer bit");
    assert_eq!(
        release_seconds(at(0x08, 0), 900.0),
        0,
        "expired, alive or not"
    );
    assert_eq!(recovery_seconds(Some(130.9), 100.0), 30, "rounded down");
    assert_eq!(recovery_seconds(Some(99.0), 100.0), 0);
}

/// `0x6d1fc0`: within reach + 4 yd, inclusive; `0x6db010` keeps only a ghost's ask in range.
#[test]
fn the_spirit_healer_reaches_four_yards_past_its_reach_and_asks_only_a_ghost() {
    let me = Vec3::ZERO;
    assert!(healer_in_range(me, Vec3::new(5.5, 0.0, 0.0), 1.5));
    assert!(!healer_in_range(me, Vec3::new(5.6, 0.0, 0.0), 1.5));
    assert!(
        healer_in_range(me, Vec3::new(0.0, 7.0, 0.0), 3.0),
        "a bigger reach"
    );
    assert_eq!(standing_ask(Some(9), true, |_| true), Some(9));
    assert_eq!(standing_ask(Some(9), false, |_| true), None, "alive");
    assert_eq!(standing_ask(Some(9), true, |_| false), None, "out of range");
}
