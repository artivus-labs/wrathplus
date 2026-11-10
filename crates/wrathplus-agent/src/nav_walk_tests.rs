//! A walk routed by the realm, end to end without a server: the Lua functions, the walker driven
//! by a stand-in for click-to-move, and the realm played by handing its lines to `pump`.

use super::rig::{Rig, HELLO};
use super::walker::SETTLE;
use super::*;

#[test]
fn a_walk_waits_for_the_realm_route_then_walks_it() {
    let mut rig = Rig::in_world();
    assert_eq!(status(&rig.nav)["source"], "realm");
    let pending: (bool, Option<String>) = rig
        .lua
        .load("local r, why = AgentMoveTo(150, 260, 31) return r.pending, why")
        .eval()
        .unwrap();
    assert_eq!(pending, (true, None));
    assert_eq!(rig.status(), "routing");
    assert_eq!(rig.realm(&[]), ["P 1 - 100.0 200.0 30.0 150.0 260.0 31.0"]);
    // Callers ask again every tick the walk is not "walking": the same walk is one request.
    rig.later(1.0);
    rig.lua
        .load("AgentMoveTo(150, 260, 31) AgentMoveTo(150.4, 260, 31)")
        .exec()
        .unwrap();
    rig.walk();
    assert!(rig.realm(&[]).is_empty() && rig.feet.clicks.is_empty());
    assert_eq!(rig.status(), "routing");
    // The route arrives in two parts; the walk starts with the last.
    rig.realm(&["r 1 1 2 c 1000,2000,300 100,100,0"]);
    assert_eq!(rig.status(), "routing");
    rig.realm(&["r 1 2 2 c 400,500,10"]);
    assert_eq!(rig.walk_status(), ("walking".into(), None));
    rig.walk();
    assert_eq!(
        rig.feet.clicks,
        [[110.0, 210.0, 30.0], [150.0, 260.0, 31.0]]
    );
    assert_eq!(rig.status(), "arrived");
    assert_eq!(rig.events(), ["AGENT_WALK_DONE [\"arrived\"]"]);
}

#[test]
fn a_refused_or_unanswered_route_fails_the_walk_and_says_why() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    rig.realm(&["e 1 noroute start or end is not on walkable ground"]);
    assert_eq!(
        rig.walk_status(),
        (
            "failed".into(),
            Some("no route: start or end is not on walkable ground".into())
        )
    );
    // No answer at all: the walk ends when the request times out.
    rig.later(1.0);
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    assert_eq!(rig.realm(&[]).len(), 1);
    rig.later(realm::REPLY_TIMEOUT - 0.5);
    rig.realm(&[]);
    assert_eq!(rig.status(), "routing");
    rig.later(1.0);
    rig.realm(&[]);
    assert_eq!(
        rig.walk_status(),
        (
            "failed".into(),
            Some("the realm did not answer the route request within 8 s".into())
        )
    );
    rig.walk();
    assert!(rig.feet.clicks.is_empty());
    assert_eq!(
        rig.events(),
        [
            "AGENT_WALK_DONE [\"failed\"]",
            "AGENT_WALK_DONE [\"failed\"]"
        ]
    );
}

#[test]
fn a_stopped_or_replaced_walk_ignores_the_route_it_no_longer_wants() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    rig.lua.load("AgentStop()").exec().unwrap();
    rig.walk();
    assert_eq!(rig.status(), "stopped");
    rig.realm(&["r 1 1 1 c 1000,2000,300 500,600,10"]);
    rig.walk();
    assert_eq!(rig.status(), "stopped");
    assert!(rig.feet.clicks.is_empty());
    // A chase: the target moved, so a new walk replaces the one still being routed. The first
    // route arrives late and is dropped; the second is walked.
    rig.later(1.0);
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 2 - 100.0 200.0 30.0 150.0 260.0 31.0"]);
    rig.lua.load("AgentMoveTo(160, 260, 31)").exec().unwrap();
    assert!(rig.realm(&[]).is_empty(), "one request in flight at a time");
    rig.later(1.0);
    assert_eq!(
        rig.realm(&["r 2 1 1 c 1000,2000,300 500,600,10"]),
        ["P 3 - 100.0 200.0 30.0 160.0 260.0 31.0"]
    );
    assert_eq!(rig.status(), "routing");
    rig.realm(&["r 3 1 1 c 1000,2000,300 600,600,10"]);
    rig.walk();
    assert_eq!(rig.feet.clicks, [[160.0, 260.0, 31.0]]);
    // Lines that are not our own whisper are nobody's route.
    rig.lua.load("AgentMoveTo(170, 260, 31)").exec().unwrap();
    rig.later(1.0);
    rig.realm(&[]);
    let spoofed = vec![("e 4 noroute x".to_string(), "Mallory".to_string())];
    pump(&rig.nav, spoofed, Some("Me"));
    assert_eq!(rig.status(), "routing");
}

#[test]
fn a_corner_not_reached_is_routed_again_by_the_realm() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    rig.feet.wall = Some([150.0, 260.0, 31.0]);
    rig.realm(&["r 1 1 1 c 1000,2000,300 100,100,0 400,500,10"]);
    rig.walk();
    // Tried twice, then a fresh route is asked from where the walk stands.
    assert_eq!(rig.feet.clicks.len(), 3);
    assert_eq!(rig.status(), "routing");
    rig.later(1.0);
    // The fresh route goes round the bump, a step ahead toward the corner.
    assert_eq!(
        rig.realm(&[]),
        [
            "A 2 110.9 211.2 3.0",
            "P 2 - 110.0 210.0 30.0 150.0 260.0 31.0"
        ]
    );
    assert_eq!(rig.nav.walk.lock().unwrap().bumps.len(), 1);
    rig.feet.wall = None;
    rig.realm(&["r 2 1 1 c 1100,2100,300 200,500,0 200,0,10"]);
    rig.walk();
    assert_eq!(rig.status(), "arrived");
    assert_eq!(rig.nav.walk.lock().unwrap().replans, 1);
    assert_eq!(
        rig.feet.clicks[3..],
        [[130.0, 260.0, 30.0], [150.0, 260.0, 31.0]]
    );
}

#[test]
fn a_far_goal_is_walked_a_leg_at_a_time() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(3100, 200, 77)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 1 n 100.0 200.0 30.0 1450.0 200.0 -"]);
    rig.realm(&["r 1 1 1 c 1000,2000,300 13400,10,-50"]);
    let (status, note) = rig.walk_status();
    assert_eq!(status, "walking");
    assert!(note.unwrap().contains("a leg at a time"));
    rig.later(1.0);
    rig.walk();
    // The leg's end is not the goal: the next leg is asked from there, toward the same goal.
    assert_eq!(rig.status(), "routing");
    assert_eq!(rig.realm(&[]), ["P 2 n 1440.0 201.0 25.0 2790.0 200.2 -"]);
    rig.realm(&["r 2 1 1 c 14400,2010,250 13400,0,0"]);
    rig.later(1.0);
    rig.walk();
    // Within the realm's reach now: the last request is for the goal itself.
    assert_eq!(
        rig.realm(&[]),
        ["P 3 - 2780.0 201.0 25.0 3100.0 200.0 77.0"]
    );
    rig.realm(&["r 3 1 1 c 27800,2010,250 3200,-10,520"]);
    rig.walk();
    assert_eq!(rig.status(), "arrived");
    assert_eq!(rig.feet.at, [3100.0, 200.0, 77.0]);
    assert_eq!(rig.events(), ["AGENT_WALK_DONE [\"arrived\"]"]);
}

#[test]
fn legs_that_stop_getting_nearer_end_the_walk_as_stuck() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(3100, 200, 77)").exec().unwrap();
    rig.realm(&[]);
    rig.realm(&["r 1 1 1 c 1000,2000,300 13400,10,-50"]);
    rig.later(1.0);
    rig.walk();
    rig.realm(&[]);
    // The next leg ends where it began (a river with no way across).
    rig.realm(&["r 2 1 1 - 14400,2010,250 30,0,0"]);
    rig.walk();
    let (status, note) = rig.walk_status();
    assert_eq!(status, "stuck");
    assert_eq!(
        note.unwrap(),
        "the realm's routes stopped getting nearer the goal"
    );
}

#[test]
fn a_far_interaction_walks_the_realm_route_or_falls_back_to_the_straight_approach() {
    let mut rig = Rig::in_world();
    rig.world.snapshot.lock().unwrap().objects = vec![world::Seen {
        guid: 0x10,
        distance: 24.0,
        pos: [124.0, 200.0, 30.0],
        ..Default::default()
    }];
    let call = "return AgentInteract('0x0000000000000010')";
    assert!(rig.lua.load(call).eval::<bool>().unwrap());
    assert_eq!(rig.realm(&[]).len(), 1);
    // Asked again while routing: still the one request.
    assert!(rig.lua.load(call).eval::<bool>().unwrap());
    assert!(rig.realm(&[]).is_empty());
    rig.realm(&["r 1 1 1 c 1000,2000,300 240,0,0"]);
    rig.walk();
    assert_eq!(rig.status(), "arrived");
    assert!(matches!(
        rig.orders.try_recv(),
        Ok(world::Order::Interact(0x10))
    ));
    // The realm has no route: the straight approach, as with no navmesh at all.
    rig.later(1.0);
    assert!(rig.lua.load(call).eval::<bool>().unwrap());
    rig.realm(&[]);
    assert!(rig.orders.try_recv().is_err());
    rig.realm(&["e 2 noroute start or end is not on walkable ground"]);
    assert!(matches!(
        rig.orders.try_recv(),
        Ok(world::Order::Interact(0x10))
    ));
    assert_eq!(rig.status(), "idle");
}

#[test]
fn agent_path_asks_the_realm_and_the_next_call_reads_the_answer() {
    let rig = Rig::in_world();
    let ask = "local r, why = AgentPath(150, 260, 31) return r and r.pending, r and r.length, r and r.complete, why";
    type Answer = (Option<bool>, Option<f32>, Option<bool>, Option<String>);
    let first: Answer = rig.lua.load(ask).eval().unwrap();
    assert_eq!(first, (Some(true), None, None, None));
    assert_eq!(rig.realm(&[]), ["P 1 - 100.0 200.0 30.0 150.0 260.0 31.0"]);
    rig.realm(&["r 1 1 1 c 1000,2000,300 500,600,10"]);
    assert_eq!(rig.events(), ["AGENT_PATH_READY [true,true]"]);
    let second: Answer = rig.lua.load(ask).eval().unwrap();
    assert_eq!(second, (None, Some(78.1), Some(true), None));
    // A walk is not started by asking for a path.
    assert_eq!(rig.status(), "idle");
    // A refusal is read the same way.
    rig.later(1.0);
    let _: Answer = rig.lua.load(ask).eval().unwrap();
    rig.realm(&[]);
    rig.realm(&["e 2 far 1500"]);
    let refused: Answer = rig.lua.load(ask).eval().unwrap();
    assert_eq!(
        refused,
        (
            None,
            None,
            None,
            Some("the realm routes at most 1500 yd at a time".into())
        )
    );
}

#[test]
fn realm_lines_are_taken_out_of_the_event_stream() {
    use wrathplus_ui::script::ScriptValue::{Int, Str};
    let addon = |prefix: &str, text: &str| {
        (
            "CHAT_MSG_ADDON".to_string(),
            vec![
                Str(prefix.into()),
                Str(text.into()),
                Str("WHISPER".into()),
                Str("Me".into()),
            ],
        )
    };
    let mut events = vec![
        ("PLAYER_LEVEL_UP".to_string(), vec![Int(2)]),
        addon("WPNAV", HELLO),
        addon("MBOT", "HELLO~4.0"),
        addon("WPNAV", "r 1 1 1 c 1,2,3"),
    ];
    assert_eq!(
        take_lines(&mut events),
        [
            (HELLO.to_string(), "Me".to_string()),
            ("r 1 1 1 c 1,2,3".to_string(), "Me".to_string())
        ]
    );
    assert_eq!(events.len(), 2);
    assert_eq!(events[1], addon("MBOT", "HELLO~4.0"));
}

#[test]
fn status_names_the_source_in_use() {
    let rig = Rig::new();
    assert_eq!(status(&rig.nav)["source"], "none");
    rig.realm(&[]);
    let probing = status(&rig.nav);
    assert_eq!(probing["source"], "realm");
    assert!(probing["detail"]
        .as_str()
        .unwrap()
        .contains("asking the realm"));
    rig.realm(&[HELLO]);
    assert_eq!(
        status(&rig.nav)["detail"],
        "routes come from the realm (mod-agent-nav): up to 1500 yd a request, 5 requests a second and 240 a minute"
    );
    // A local mmaps folder that exists is preferred, whatever the realm offers.
    let dir = std::env::temp_dir();
    rig.nav.mesh.lock().unwrap().dir = Some(dir.clone());
    let local = status(&rig.nav);
    assert_eq!(local["source"], "local");
    assert!(local["detail"]
        .as_str()
        .unwrap()
        .contains(dir.to_str().unwrap()));
    // But a map it does not cover is still routed by the realm.
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    assert_eq!(rig.status(), "routing");
    // A folder that is not there counts as none.
    rig.nav.mesh.lock().unwrap().dir = Some(dir.join("no-such-mmaps-folder"));
    assert_eq!(status(&rig.nav)["source"], "realm");
}

#[test]
fn a_target_without_a_height_asks_the_realm_to_find_the_floor() {
    let rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 1 h 100.0 200.0 30.0 150.0 260.0 30.0"]);
    // The floor it found becomes the goal: a later fresh route asks for it, height known.
    rig.realm(&["r 1 1 1 c 1000,2000,300 500,600,650"]);
    let walk = rig.nav.walk.lock().unwrap();
    assert_eq!(
        (walk.goal, walk.any_height),
        (Some([150.0, 260.0, 95.0]), false)
    );
    drop(walk);
    // AgentPath too; a guid's height is known.
    rig.later(1.0);
    rig.lua.load("AgentPath(150, 260)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 2 h 100.0 200.0 30.0 150.0 260.0 30.0"]);
    // Legs toward a far flat goal are not searched; the last request is.
    rig.later(1.0);
    rig.realm(&["e 2 noroute x"]);
    rig.lua.load("AgentMoveTo(3100, 200)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 3 n 100.0 200.0 30.0 1450.0 200.0 -"]);
}

#[test]
fn a_realm_route_gets_the_push_and_the_first_step_grace() {
    let mut rig = Rig::in_world();
    rig.feet.vm = true;
    // The first click is refused while autoInteract comes through; the walk waits for it.
    rig.feet.refuse = 3;
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    assert_eq!(rig.nav.walk.lock().unwrap().settle, SETTLE);
    rig.feet.wall = Some([150.0, 260.0, 31.0]);
    rig.realm(&["r 1 1 1 c 1000,2000,300 100,100,0 400,500,10"]);
    rig.walk();
    // Short of the walled corner: a push, held until its time is up.
    assert_eq!(rig.feet.keys, ["push"]);
    assert_eq!(rig.status(), "walking");
    let clicks = rig.feet.clicks.len();
    rig.walk();
    assert_eq!(
        rig.feet.clicks.len(),
        clicks,
        "nothing clicked while pushing"
    );
    rig.later(1.0);
    rig.walk();
    assert_eq!(rig.feet.keys, ["push", "let go"]);
    // The second try fell short too: routed round the bump, by the realm.
    assert_eq!(rig.status(), "routing");
    let sent = rig.realm(&[]);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].starts_with("A 2 ") && sent[1].starts_with("P 2 - "));
}

/// A cinematic starts while the realm works out a route: the answer is kept, but the walker takes
/// no step until the cinematic ends (click-to-move would stall and end the walk `stuck` with no
/// reason); the walk says why meanwhile, and carries on after.
#[test]
fn a_realm_route_that_arrives_under_a_cinematic_waits_for_its_end() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    assert_eq!(rig.realm(&[]), ["P 1 - 100.0 200.0 30.0 150.0 260.0 31.0"]);
    rig.world.snapshot.lock().unwrap().cinematic = true;
    rig.held = true;
    assert_eq!(rig.status(), "routing");
    rig.realm(&["r 1 1 1 c 1000,2000,300 500,600,10"]);
    rig.walk();
    assert!(rig.feet.clicks.is_empty(), "no step under the cinematic");
    let (status, note) = rig.walk_status();
    assert_eq!(status, "walking");
    assert_eq!(note.as_deref(), Some(world::CINEMATIC_NOTE));
    // A new walk asked for meanwhile is refused, announced once; the held one is replaced.
    rig.lua
        .load("AgentMoveTo(120, 200, 30) AgentMoveTo(121, 200, 30)")
        .exec()
        .unwrap();
    rig.walk();
    assert_eq!(rig.events(), ["AGENT_WALK_DONE [\"failed\"]"]);
    // Over: a walk asked for now goes, and nothing else was left half-walked.
    rig.world.snapshot.lock().unwrap().cinematic = false;
    rig.held = false;
    rig.later(1.0);
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    rig.realm(&["r 2 1 1 c 1000,2000,300 500,600,10"]);
    rig.walk();
    assert_eq!(rig.feet.clicks, [[150.0, 260.0, 31.0]]);
    assert_eq!(rig.status(), "arrived");
}

/// The same with no new walk asked: the route that arrived under the cinematic is walked after it.
#[test]
fn a_walk_held_by_a_cinematic_carries_on_after_it() {
    let mut rig = Rig::in_world();
    rig.lua.load("AgentMoveTo(150, 260, 31)").exec().unwrap();
    rig.realm(&[]);
    rig.world.snapshot.lock().unwrap().cinematic = true;
    rig.held = true;
    rig.realm(&["r 1 1 1 c 1000,2000,300 500,600,10"]);
    rig.walk();
    assert!(rig.feet.clicks.is_empty());
    rig.world.snapshot.lock().unwrap().cinematic = false;
    rig.held = false;
    assert_eq!(rig.walk_status(), ("walking".into(), None));
    rig.walk();
    assert_eq!(rig.feet.clicks, [[150.0, 260.0, 31.0]]);
    assert_eq!(rig.status(), "arrived");
    assert_eq!(rig.events(), ["AGENT_WALK_DONE [\"arrived\"]"]);
}
