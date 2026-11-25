//! Whole-client tests of `crate::net::wrath_achievement_capture`, from `net/wrath_achievement_capture_tests.rs`.
//! They build the client with `headless_client`, so they live in the root layer.

use crate::net::wrath_achievement_capture::*;
use bevy::prelude::*;
use wrathplus_protocol::wrath_achievement::Packet;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
use wrathplus_protocol::SessionEvent;
use wrathplus_protocol::SessionEventKind;

fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, opcode: u16, body: &[u8]) {
    if let WrathDispatch::Handled { session_events, .. } =
        adapter.dispatch(opcode, body, 0).unwrap()
    {
        crate::net::handlers::dispatch(app.world_mut(), session_events);
    }
}

fn captured(app: &App) -> Vec<Packet> {
    app.world()
        .resource::<NativeAchievementPackets>()
        .0
        .iter()
        .cloned()
        .collect()
}

#[test]
fn the_capture_and_the_counters_own_receipts_in_order_and_both_boundaries_clear() {
    let mut app = crate::game_plugins::schedule_tests::headless_client();
    let census = app
        .world()
        .resource::<crate::net::handlers::NetHandlers>()
        .census();
    let owners = &census[&SessionEventKind::WrathAchievement];
    // The counters take the snapshot and the deletions in wire order beside the capture.
    assert_eq!(owners.len(), 2, "{owners:?}");
    for owner in [
        "wrath_achievement_capture::receive",
        "wrath_notices::on_progress",
    ] {
        assert!(owners.iter().any(|o| o.contains(owner)), "{owners:?}");
    }
    let mut adapter = WrathEventAdapter::new();
    let deleted = Packet::AchievementDeleted { achievement: 6 };
    for event in [
        SessionEvent::Connected {
            self_guid: 1,
            name: "test".into(),
            billing_time_rested: 0,
            tutorial_flags: None,
            addon_info: None,
        },
        SessionEvent::Disconnected {
            reason: "test".into(),
            end: wrathplus_protocol::SessionEnd::Lost,
        },
    ] {
        dispatch(&mut app, &mut adapter, 0x49f, &[6, 0, 0, 0]);
        dispatch(&mut app, &mut adapter, 0x49f, &[6, 0, 0]);
        dispatch(&mut app, &mut adapter, 0x49f, &[6, 0, 0, 0, 0]);
        dispatch(&mut app, &mut adapter, 0x49f, &[6, 0, 0, 0]);
        assert_eq!(captured(&app), vec![deleted.clone(), deleted.clone()]);
        crate::net::handlers::dispatch(app.world_mut(), vec![event]);
        assert!(captured(&app).is_empty());
    }
}
