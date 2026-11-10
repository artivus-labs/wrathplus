//! `nav`'s unit tests: targets, the no-navmesh answers and the straight-approach fallback.

use super::api::{height_given, target};
use super::realm::Purpose;
use super::walker::*;
use super::*;
use mlua::{Lua, MultiValue, Table};

#[test]
fn targets_come_from_coordinates_or_a_nearby_guid() {
    let lua = Lua::new();
    let snapshot = world::Snapshot {
        me: None,
        objects: vec![world::Seen {
            guid: 0x10,
            pos: [1.0, 2.0, 3.0],
            ..Default::default()
        }],
        corpse: None,
        cinematic: false,
    };
    let args = |code: &str| lua.load(code).eval::<MultiValue>().unwrap();
    assert_eq!(
        target(&args("return 4, 5, 6"), &snapshot),
        Ok([4.0, 5.0, 6.0])
    );
    assert_eq!(
        target(&args("return '0x0000000000000010'"), &snapshot),
        Ok([1.0, 2.0, 3.0])
    );
    assert!(target(&args("return '0x99'"), &snapshot).is_err());
    assert!(target(&args("return true"), &snapshot).is_err());
}

#[test]
fn without_mmaps_a_route_says_how_to_get_one() {
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        crossbeam_channel::unbounded().0,
    );
    handles.mesh.lock().unwrap().dir = None;
    let me = world::Me {
        pos: [0.0; 3],
        facing: 0.0,
        map: 0,
    };
    // Out of the world the realm has not been asked; in it, a realm that does not answer.
    let why = route_to(&handles, me, [1.0, 1.0, 0.0], false, Purpose::Path).unwrap_err();
    assert!(
        why.contains("WOW_NAV_MMAPS") && why.contains("once in the world"),
        "{why}"
    );
    assert_eq!(pump(&handles, Vec::new(), Some("Me")), ["H 1"]);
    pump(&handles, vec![("H 1".into(), "Me".into())], Some("Me"));
    let why = route_to(&handles, me, [1.0, 1.0, 0.0], false, Purpose::Path).unwrap_err();
    assert!(
        why.contains("WOW_NAV_MMAPS") && why.contains("mod-agent-nav"),
        "{why}"
    );
    assert_eq!(status(&handles)["source"], "none");
    assert_eq!(status(&handles)["detail"], why.as_str());
}

#[test]
fn a_far_interaction_without_a_navmesh_falls_back_to_the_straight_approach() {
    let (world_handles, world_link) = world::link();
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        world_handles.orders.clone(),
    );
    handles.mesh.lock().unwrap().dir = None;
    {
        let mut s = world_handles.snapshot.lock().unwrap();
        s.me = Some(world::Me {
            pos: [0.0; 3],
            facing: 0.0,
            map: 0,
        });
        s.objects = vec![world::Seen {
            guid: 0x10,
            distance: 24.0,
            pos: [24.0, 0.0, 0.0],
            ..Default::default()
        }];
    }
    let lua = Lua::new();
    world::install(&lua, &world_handles).unwrap();
    install(&lua, &handles, &world_handles).unwrap();
    let sent: bool = lua
        .load("return AgentInteract('0x0000000000000010')")
        .eval()
        .unwrap();
    assert!(sent);
    assert!(matches!(
        world_link.orders.try_recv(),
        Ok(world::Order::Interact(0x10))
    ));
    assert_eq!(handles.walk.lock().unwrap().then_interact, None);
}

impl Plan {
    /// The route of a plan made from local mmaps.
    fn ready(self) -> Route {
        match self {
            Plan::Ready(route) => route,
            Plan::Asked => panic!("asked the realm, with local mmaps set"),
        }
    }
}

/// A local route, as the walker asks for one.
fn local(handles: &Handles, me: world::Me, to: [f32; 3], bumps: &[[f32; 3]]) -> Route {
    route_in(&handles.mesh, me, to, bumps, false, Purpose::Path, 0.0)
        .unwrap()
        .ready()
}

/// A bump is marked a step ahead toward the corner the walk fell short of, or on the corner
/// itself when that is nearer, with the radius a fresh route goes round.
#[test]
fn a_bump_is_marked_just_ahead_toward_the_corner() {
    assert_eq!(
        bump([0.0, 0.0, 5.0], [10.0, 0.0, 9.0]),
        [BUMP_AHEAD, 0.0, BUMP_RADIUS]
    );
    let diagonal = bump([-3.0, -4.0, 0.0], [0.0, 0.0, 0.0]);
    assert!((diagonal[0] + 2.1).abs() < 1e-4 && (diagonal[1] + 2.8).abs() < 1e-4);
    assert_eq!(
        bump([1.0, 1.0, 0.0], [1.5, 1.0, 0.0]),
        [1.5, 1.0, BUMP_RADIUS]
    );
    assert_eq!(
        bump([1.0, 1.0, 0.0], [1.0, 1.0, 3.0]),
        [1.0, 1.0, BUMP_RADIUS]
    );
}

/// The stall this walker had at Goldshire's inn (#82, live on the beta realm): the navmesh's
/// route into the Lion's Pride hugs the timber foot at the building's corner, which the client's
/// collision does not let a player past, from either side. A fresh route from the same spot was
/// the same route; told of the bump, it leaves the corner out and still arrives. Each walk keeps
/// its own bumps, and `AgentAvoid`'s circles stay as the agent set them.
///
/// Needs real mmaps: `WRATHPLUS_MMAPS=<server data>/mmaps cargo test -p wrathplus-agent -- --ignored`.
#[test]
#[ignore = "needs the server's extracted mmaps"]
fn a_walk_stuck_on_the_inn_corner_routes_round_it() {
    let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        crossbeam_channel::unbounded().0,
    );
    {
        let mut mesh = handles.mesh.lock().unwrap();
        mesh.dir = Some(PathBuf::from(dir));
        mesh.avoid = vec![[0.0, 0.0, 1.0]];
    }
    let near = |route: &Route, p: [f32; 2]| {
        route
            .points
            .iter()
            .skip(1)
            .any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1.0)
    };
    // (where the player stood, the goal, the corner the walk fell short of), as recorded live:
    // into the inn from the east, and past it from the west.
    let cases = [
        (
            [-9454.67, 44.46, 56.62],
            [-9462.66, 16.19, 56.96],
            [-9457.6, 44.8, 57.1],
        ),
        (
            [-9456.88, 45.22, 56.65],
            [-9440.0, 38.0, 58.0],
            [-9450.7, 42.7, 57.4],
        ),
    ];
    for (at, goal, corner) in cases {
        let me = world::Me {
            pos: at,
            facing: 0.0,
            map: 0,
        };
        let plain = local(&handles, me, goal, &[]);
        assert!(
            plain.complete && near(&plain, [corner[0], corner[1]]),
            "the plain route takes the corner: {plain:?}"
        );
        let mut bumps = Vec::new();
        let round = route_round(
            &handles.mesh,
            &mut bumps,
            me,
            corner,
            goal,
            Purpose::Path,
            0.0,
        )
        .unwrap()
        .ready();
        assert_eq!(bumps, [bump(at, corner)]);
        assert!(round.complete, "{round:?}");
        assert!(
            !near(&round, [corner[0], corner[1]]) && round != plain,
            "the fresh route leaves the corner out: {round:?}"
        );
        // The walk's bumps are its own: the next plain route is the first again.
        assert_eq!(local(&handles, me, goal, &[]), plain);
    }
    assert_eq!(handles.mesh.lock().unwrap().avoid, [[0.0, 0.0, 1.0]]);
}

/// The push before a second try is the forward key held with one press of the jump key, on a VM
/// an agent drives, and its end lets forward go: nothing stays down after it. It lasts the run to
/// the corner, between a jump's worth and a few yards'.
#[test]
fn the_push_holds_forward_with_a_jump_and_lets_go() {
    use wrathplus_ui::script::{BindingInput, FiredInput, HeldInput, UiScript};
    let mut script = UiScript::new().unwrap();
    script.set_agent_input(true);
    script.run(PUSH).unwrap();
    assert_eq!(
        script.take_binding_input(),
        [
            BindingInput::Start(HeldInput::MoveForward),
            BindingInput::Fire(FiredInput::Jump),
            BindingInput::Start(HeldInput::Ascend),
            BindingInput::Stop(HeldInput::Ascend),
        ]
    );
    script.run(PUSH_END).unwrap();
    assert_eq!(
        script.take_binding_input(),
        [BindingInput::Stop(HeldInput::MoveForward)]
    );
    assert_eq!(push_secs(0.5), 0.3);
    assert_eq!(push_secs(3.5), 0.5);
    assert_eq!(push_secs(235.0), 0.7);
}

#[test]
fn a_target_without_a_height_is_looked_for_on_other_levels_nearest_first() {
    let lua = Lua::new();
    let args = |code: &str| lua.load(code).eval::<MultiValue>().unwrap();
    assert!(!height_given(&args("return -8387, 685")));
    assert!(!height_given(&args("return -8387, 685, nil")));
    assert!(height_given(&args("return -8387, 685, 95")));
    assert!(height_given(&args("return '0x0000000000000010'")));
    let tried: Vec<f32> = heights(58.0).collect();
    assert_eq!(tried[..4], [73.0, 43.0, 88.0, 28.0]);
    assert_eq!(tried.len(), 20);
    assert_eq!(tried[18..], [208.0, -92.0]);
}

/// Needs real mmaps: `WRATHPLUS_MMAPS=<server data>/mmaps cargo test -p wrathplus-agent -- --ignored`.
/// Elmore's Task is handed in at the quest area (-8387, 685), in Stormwind's Dwarven District on
/// a floor at z 95; from Goldshire (z 58) the flat point alone gave a route ending on another level.
#[test]
#[ignore = "needs the server's extracted mmaps"]
fn a_quest_hand_in_in_stormwind_is_reached_from_goldshire_without_its_height() {
    let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        crossbeam_channel::unbounded().0,
    );
    handles.mesh.lock().unwrap().dir = Some(dir.into());
    let me = world::Me {
        pos: [-9462.0, 88.0, 58.0],
        facing: 0.0,
        map: 0,
    };
    let flat = [-8387.0, 685.0, 58.0];
    let at_my_height = route_to(&handles, me, flat, false, Purpose::Path)
        .unwrap()
        .ready();
    assert!(!at_my_height.complete, "{:?}", at_my_height.points.last());
    let found = route_to(&handles, me, flat, true, Purpose::Path)
        .unwrap()
        .ready();
    assert!(found.complete, "{:?}", found.points.last());
    let end = found.points.last().unwrap();
    assert!(
        (end[0] + 8387.0).abs() < 2.0
            && (end[1] - 685.0).abs() < 2.0
            && (end[2] - 95.0).abs() < 3.0,
        "{end:?}"
    );
}

/// A new character's intro cinematic: the walk is refused with the reason instead of ending
/// `stuck` with none, and `AgentWalk()` carries it while the cinematic lasts.
#[test]
fn a_cinematic_refuses_the_walk_and_says_how_to_end_it() {
    let (world_handles, world_link) = world::link();
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        world_handles.orders.clone(),
    );
    {
        let mut s = world_handles.snapshot.lock().unwrap();
        s.me = Some(world::Me {
            pos: [0.0; 3],
            facing: 0.0,
            map: 0,
        });
        s.objects = vec![world::Seen {
            guid: 0x10,
            distance: 24.0,
            pos: [24.0, 0.0, 0.0],
            ..Default::default()
        }];
        s.cinematic = true;
    }
    let lua = Lua::new();
    world::install(&lua, &world_handles).unwrap();
    install(&lua, &handles, &world_handles).unwrap();
    let (route, why, status, note, flagged): (Option<Table>, String, String, String, bool) = lua
        .load(
            "local route, why = AgentMoveTo(10, 0, 0) local w = AgentWalk() \
             return route, why, w.status, w.note, w.cinematic",
        )
        .eval()
        .unwrap();
    assert!(route.is_none());
    assert_eq!(why, world::CINEMATIC_NOTE);
    assert!(why.contains("StopCinematic()"));
    assert_eq!(
        (status.as_str(), note.as_str(), flagged),
        ("failed", why.as_str(), true)
    );
    // Announced like any refused walk, so whoever waits on it is woken: once, however often a
    // loop asks again while the cinematic lasts.
    assert!(std::mem::take(&mut handles.walk.lock().unwrap().failed));
    lua.load("AgentMoveTo(10, 0, 0) AgentMoveTo(12, 0, 0)")
        .exec()
        .unwrap();
    assert!(!handles.walk.lock().unwrap().failed);
    // A far interaction does not start a walk either: it is the plain order.
    let sent: bool = lua
        .load("return AgentInteract('0x0000000000000010')")
        .eval()
        .unwrap();
    assert!(sent);
    assert!(matches!(
        world_link.orders.try_recv(),
        Ok(world::Order::Interact(0x10))
    ));
    assert_eq!(handles.walk.lock().unwrap().status, "failed");
    // Over: the walk's own note is left, and the flag is down.
    world_handles.snapshot.lock().unwrap().cinematic = false;
    let (flagged, note): (bool, String) = lua
        .load("local w = AgentWalk() return w.cinematic, w.note")
        .eval()
        .unwrap();
    assert!(!flagged);
    assert_eq!(note, world::CINEMATIC_NOTE, "the refused walk's own reason");
    // A walk a cinematic caught under way keeps its note, after the cinematic's.
    {
        let mut walk = handles.walk.lock().unwrap();
        walk.status = "walking";
        walk.note = Some("partial route".into());
    }
    world_handles.snapshot.lock().unwrap().cinematic = true;
    let note: String = lua.load("return AgentWalk().note").eval().unwrap();
    assert!(note.starts_with(world::CINEMATIC_NOTE) && note.ends_with("; partial route"));
}
