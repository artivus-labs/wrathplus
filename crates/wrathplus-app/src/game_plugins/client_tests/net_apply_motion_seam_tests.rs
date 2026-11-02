//! Whole-client tests of `crate::net::apply`'s movement apply. They build the client with
//! `headless_client`, so they live in the root layer.
//!
//! The movement apply's ordering laws on the real drain, pinned before the apply leaves `net`
//! (`https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN`, item 3 part A): a handler later in a drain sees exactly the
//! movement state the earlier events left; a create, a move and a destroy in one drain apply in
//! wire order; the movement kinds' other handlers and the movement chain keep their order.

use std::time::Duration;

use bevy::prelude::*;
use wrathplus_protocol::types::{MonsterMoveFacing, ObjectType, RelayVerb};
use wrathplus_protocol::{
    EntityKind, ObjectFields, SessionEvent, SessionEventKind, SplineInterpolation,
};

use crate::motion::RemoteMotion;
use crate::motion::Spline;
use crate::motion::UnitMoveModes;
use crate::net::{GuidIndex, NetEvents, NetHandlerApp};

const GUID: u64 = 0xF130_0000_4321_0001;
const FIELD_UNIT_HEALTH: u16 = wrathplus_protocol::types::Field::UnitHealth.index();

fn create() -> SessionEvent {
    SessionEvent::ObjectCreate {
        guid: GUID,
        kind: EntityKind::Unit,
        display_id: None,
        position: [1.0, 2.0, 3.0],
        orientation: 0.0,
        rotation: None,
        scale: 1.0,
        speeds: None,
        mover: None,
        transport_progress: None,
        transport: None,
        spline: None,
        fields: ObjectFields::from_pairs(&[(FIELD_UNIT_HEALTH, 100)])
            .into_created(ObjectType::Unit),
    }
}

/// A relayed move forward to `position`.
fn walk_to(position: [f32; 3]) -> SessionEvent {
    SessionEvent::UnitMove {
        guid: GUID,
        position,
        orientation: 1.0,
        flags: crate::app_core::movement::move_flags::FORWARD,
        pitch: 0.0,
        time: 1000,
        verb: RelayVerb::Pose,
        fall_time: 0,
        jump: None,
        transport: None,
    }
}

/// A two-point ground path from the create's spot.
fn path() -> SessionEvent {
    SessionEvent::MonsterMove {
        guid: GUID,
        transport: None,
        start: [1.0, 2.0, 3.0],
        spline_id: 7,
        path: vec![[10.0, 2.0, 3.0], [20.0, 2.0, 3.0]],
        facing: MonsterMoveFacing::None,
        stop: false,
        duration_ms: 2000,
        flying: false,
        interpolation: SplineInterpolation::Linear,
        cyclic: false,
        effect: None,
        run_mode: None,
    }
}

fn pong(sequence: u32) -> SessionEvent {
    SessionEvent::Pong { sequence }
}

/// `RemoteMotion`'s pose, flags, queue length and last applied position.
type RemoteView = ([f32; 3], f32, u32, usize, [f32; 3]);

/// What a reader can see of the unit's movement. The splines' start `Instant`s are left out: they
/// are wall-clock, not state an event wrote.
#[derive(Debug, Default, PartialEq)]
struct Pose {
    indexed: bool,
    translation: Option<Vec3>,
    rotation: Option<Quat>,
    remote: Option<RemoteView>,
    spline: Option<(Vec<[f32; 3]>, Duration, u32, bool)>,
    modes: Option<u32>,
}

fn pose(world: &World) -> Pose {
    let Some(&e) = world.resource::<GuidIndex>().0.get(&GUID) else {
        return Pose::default();
    };
    let transform = world.get::<Transform>(e);
    Pose {
        indexed: true,
        translation: transform.map(|t| t.translation),
        rotation: transform.map(|t| t.rotation),
        remote: world.get::<RemoteMotion>(e).map(|r| {
            (
                r.wow_pos,
                r.orientation,
                r.flags,
                r.pending.len(),
                r.last_apply_pos,
            )
        }),
        spline: world
            .get::<Spline>(e)
            .map(|s| (s.points.clone(), s.duration, s.id, s.grounded)),
        modes: world.get::<UnitMoveModes>(e).map(|m| m.0),
    }
}

/// The poses the `Pong` probe saw, in drain order.
#[derive(Resource, Default)]
struct Seen(Vec<Pose>);

/// A handler on a kind no object packet uses, so it runs between whatever events surround it.
fn probe(In(_): In<SessionEvent>, world: &mut World) {
    let seen = pose(world);
    world.resource_mut::<Seen>().0.push(seen);
}

/// One frame of the real drain over `events`: what the probe saw, then the unit at the end.
fn drained(events: Vec<SessionEvent>) -> (Vec<Pose>, Pose) {
    let mut app = crate::game_plugins::schedule_tests::headless_client();
    app.init_resource::<Seen>();
    // Test-only: bound first, so the handler census's source scan (`net_handlers.rs`), which
    // counts the plugins' registrations, does not count it.
    let register = <App as NetHandlerApp>::net_handler;
    register(&mut app, SessionEventKind::Pong, probe);
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetEvents(rx));
    for ev in events {
        tx.send(ev).unwrap();
    }
    let world = app.world_mut();
    crate::net::apply::apply_net_updates(world);
    let seen = std::mem::take(&mut world.resource_mut::<Seen>().0);
    (seen, pose(world))
}

/// A relayed move and a spline path, each seen by a later handler in the same drain exactly as
/// the drain leaves it: nothing the move wrote waits for a later system.
#[test]
fn a_later_handler_sees_the_movement_the_earlier_events_applied() {
    let (_, created) = drained(vec![create()]);
    for (name, mv) in [("relay", walk_to([30.0, 2.0, 3.0])), ("spline", path())] {
        let (seen, _) = drained(vec![create(), mv.clone(), pong(1)]);
        let (_, after) = drained(vec![create(), mv]);
        assert_ne!(after, created, "{name}: the move changed the unit");
        assert_eq!(seen, vec![after], "{name}");
    }
}

/// Three events in one drain, each fully applied before the next, in wire order.
#[test]
fn create_move_and_destroy_in_one_drain_apply_in_wire_order() {
    let move_to = walk_to([30.0, 2.0, 3.0]);
    let (seen, end) = drained(vec![
        create(),
        pong(1),
        move_to.clone(),
        pong(2),
        SessionEvent::ObjectDestroyed(GUID),
        pong(3),
    ]);
    let (_, created) = drained(vec![create()]);
    let (_, moved) = drained(vec![create(), move_to]);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0], created);
    assert_eq!(seen[1], moved);
    assert_eq!(seen[2], Pose::default(), "the destroy dropped the unit");
    assert_eq!(end, Pose::default());
}

/// The movement kinds' handlers other than the object layer's and the movement apply's: none of
/// them reads movement state, so where the apply sits among them cannot change what they see.
/// The object layer's or the movement handlers' handler comes first on every kind.
#[test]
fn the_movement_kinds_other_handlers_are_pinned() {
    let app = crate::game_plugins::schedule_tests::headless_client();
    let census = app
        .world()
        .resource::<crate::net::handlers::NetHandlers>()
        .census();
    use SessionEventKind as K;
    for (kind, others) in [
        (K::ObjectCreate, vec!["net::wrath_auras::on_auras"]),
        (K::ObjectMove, vec![]),
        (K::MoveTimeSkipped, vec![]),
        (K::UnitMove, vec![]),
        (K::MonsterMove, vec![]),
        (K::SplineMoveMode, vec![]),
        (K::ForceSpeedChange, vec![]),
        (K::SpeedChanged, vec![]),
    ] {
        let names = census.get(&kind).cloned().unwrap_or_default();
        assert!(
            names
                .first()
                .is_some_and(|n| n.contains("::net::objects::") || n.contains("::motion::")),
            "{kind:?}: {names:?}"
        );
        // By module path: the crate prefix names whichever layer crate the handler lives in.
        let rest: Vec<&str> = names
            .into_iter()
            .filter(|n| !n.contains("::net::objects::") && !n.contains("::motion::"))
            .map(|n| n.split_once("::").map_or(n, |(_, path)| path))
            .collect();
        assert_eq!(rest, others, "{kind:?}");
    }
}

/// The movement chain's order in `Update`, by function name: the drain, then the movers in their
/// fixed order, with the NPC token feed seated between the drain and the facing driver.
///
/// Dev builds only: Bevy keeps system and observer names only with its `debug` feature, which only
/// the `dev` build turns on (through `wrathplus-world/dev` and `bevy_egui`); a player build reads
/// empty names, so there is nothing to compare and the test returns at once.
#[test]
fn the_movement_chain_keeps_its_order() {
    if !crate::run_mode::dev_affordances() {
        return;
    }
    let mut app = crate::game_plugins::schedule_tests::headless_client();
    let names: Vec<String> = app.world_mut().schedule_scope(Update, |world, schedule| {
        schedule.initialize(world).expect("Update initializes");
        schedule
            .systems()
            .expect("initialized")
            .map(|(_, s)| {
                let name = s.name().to_string();
                name.rsplit("::").next().unwrap_or(&name).to_string()
            })
            .collect()
    });
    let at = |name: &str| {
        let found: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, n)| *n == name)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(found.len(), 1, "{name} once in Update");
        found[0]
    };
    let chain = [
        "apply_net_updates",
        "tag_self_player",
        "sample_splines",
        "mark_swimming_creatures",
        "ground_clamp_creatures",
        "drain_pending_moves",
        "extrapolate_remote_units",
        "drive_display_facing",
    ];
    let order: Vec<usize> = chain.iter().map(|n| at(n)).collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "{chain:?} at {order:?}"
    );
    let npc = at("feed_interact_npc");
    assert!(
        order[0] < npc && npc < order[7],
        "feed_interact_npc at {npc}, chain {order:?}"
    );
}

/// Every indexed object's movement, one line each, in guid order.
fn poses(world: &World) -> Vec<String> {
    let mut rows: Vec<(u64, Entity)> = world
        .resource::<GuidIndex>()
        .0
        .iter()
        .map(|(g, e)| (*g, *e))
        .collect();
    rows.sort_unstable();
    rows.into_iter()
        .map(|(guid, e)| {
            let transform = world.get::<Transform>(e);
            let remote = world.get::<RemoteMotion>(e).map(|r| {
                (
                    r.wow_pos,
                    r.orientation,
                    r.flags,
                    r.pending.len(),
                    r.last_apply_pos,
                )
            });
            let spline = world
                .get::<Spline>(e)
                .map(|s| (s.points.clone(), s.duration, s.id, s.grounded));
            let modes = world.get::<UnitMoveModes>(e).map(|m| m.0);
            let rider = world
                .get::<crate::app_core::movement::TransportRider>(e)
                .map(|r| (r.transport_guid, r.local_pos, r.local_orientation));
            format!(
                "{guid:#018x} t={:?} r={:?} remote={remote:?} spline={spline:?} modes={modes:?} \
                 rider={rider:?}",
                transform.map(|t| t.translation),
                transform.map(|t| t.rotation),
            )
        })
        .collect()
}

/// Replays a recorded build-12340 capture (`WOW_WRATH_PACKET_RECORD`) through the live reader's
/// decode routes and the real drain, one drain per 60 Hz frame of record time, and writes every
/// indexed object's movement state after each frame. No movement system runs, so nothing reads the
/// wall clock: two runs on one tree write the same file, and a run before and after a refactor of
/// the apply diff to nothing. Each frame is its number and its objects in guid order ([`poses`]);
/// how many events it carried goes to stderr, not the file, so a newly decoded opcode that changes
/// no object moves nothing. Captures hold private gameplay data; the output stays local.
///
/// Accepted baseline for capture 144407 (`target/azb-live/144407/packets.cap`): sha256
/// `16bc666b3ef442c2…`, 201,656 lines (wrath-hud b00e901a; the same object state as the earlier
/// `dd066b6e`/`75e998a3` dumps, which also wrote each frame's event count).
///
/// `WOW_REPLAY_CAPTURE=<capture> WOW_REPLAY_POSES=<file> cargo test -p wrathplus-app --lib
/// replay_capture_poses -- --ignored`
#[test]
#[ignore = "needs a local capture: WOW_REPLAY_CAPTURE and WOW_REPLAY_POSES"]
fn replay_capture_poses() {
    use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
    use wrathplus_protocol::{wrath_capture, wrath_movement_misc, wrath_relay};
    let capture = std::env::var("WOW_REPLAY_CAPTURE").expect("WOW_REPLAY_CAPTURE");
    let out = std::env::var("WOW_REPLAY_POSES").expect("WOW_REPLAY_POSES");
    let bytes = std::fs::read(&capture).expect("read the capture");
    let records = wrath_capture::read(&bytes)
        .expect("a build-12340 capture")
        .records;

    // The live reader's routes, as `wrath_capture::census` takes them, keeping each event's time.
    let mut adapter = WrathEventAdapter::new();
    let mut timed: Vec<(u64, SessionEvent)> = Vec::new();
    let mut queue: std::collections::VecDeque<(u16, Vec<u8>, u64)> = records
        .into_iter()
        .map(|r| (r.opcode, r.body, r.elapsed_us))
        .collect();
    while let Some((opcode, body, us)) = queue.pop_front() {
        if opcode == wrath_movement_misc::SMSG_MULTIPLE_MOVES {
            if let Ok(inner) = wrath_movement_misc::split_multiple_moves(&body) {
                for (o, b) in inner.into_iter().rev() {
                    queue.push_front((o, b, us));
                }
            }
        } else if wrath_relay::handles(opcode) {
            if let Ok(event) = wrath_relay::decode(opcode, &body) {
                timed.push((us, event));
            }
        } else if let Ok(WrathDispatch::Handled { session_events, .. }) =
            adapter.dispatch(opcode, &body, (us / 1000) as u32)
        {
            timed.extend(session_events.into_iter().map(|e| (us, e)));
        }
    }

    let mut app = crate::game_plugins::schedule_tests::headless_client();
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetEvents(rx));
    let mut lines = Vec::new();
    let mut frame = 0u64;
    let mut total = 0usize;
    let mut events = timed.into_iter().peekable();
    while events.peek().is_some() {
        let frame_end = (frame + 1) * 16_667;
        let mut sent = 0usize;
        while let Some((_, ev)) = events.next_if(|(us, _)| *us < frame_end) {
            tx.send(ev).unwrap();
            sent += 1;
        }
        if sent > 0 {
            let world = app.world_mut();
            crate::net::apply::apply_net_updates(world);
            total += sent;
            lines.push(format!("frame {frame}"));
            lines.extend(poses(world));
        }
        frame += 1;
    }
    std::fs::write(&out, lines.join("\n") + "\n").expect("write the pose dump");
    eprintln!("replay: {total} events, {} lines to {out}", lines.len());
}
