//! `realm`'s tests: the route source against a stubbed transport (what it would send is read from
//! its outbox; what the realm would answer is handed to `receive`).

use super::*;

const HELLO: &str = "h 1 1500 256 64 5 240";

fn ask(purpose: Purpose, to: [f32; 3]) -> Ask {
    Ask {
        purpose,
        from: [100.0, 200.0, 30.0],
        to,
        avoid: Vec::new(),
    }
}

fn walk(ticket: u32) -> Purpose {
    Purpose::Walk { ticket }
}

/// In the world, with the realm's answer to the probe in.
fn available() -> Remote {
    let mut remote = Remote::new();
    assert!(remote.world(true, 0.0).is_empty());
    assert_eq!(std::mem::take(&mut remote.outbox), ["H 1"]);
    assert!(remote.receive(HELLO, 0.1).is_empty());
    assert!(matches!(remote.realm, Realm::Available(l) if l.max_distance == 1500.0));
    remote
}

#[test]
fn local_mmaps_come_first_then_a_realm_that_answers() {
    let limits = match available().realm {
        Realm::Available(l) => l,
        other => panic!("{other:?}"),
    };
    let probing = Realm::Probing {
        since: 0.0,
        tries: 1,
    };
    for (local, realm, source) in [
        (true, Realm::Available(limits), Source::Local),
        (true, Realm::Absent, Source::Local),
        (false, Realm::Available(limits), Source::Realm),
        // Asked while the probe is out: the request waits for its answer.
        (false, probing, Source::Realm),
        (false, Realm::Absent, Source::None),
        (false, Realm::Unknown, Source::None),
    ] {
        assert_eq!(choose(local, &realm), source, "{local} {realm:?}");
    }
}

#[test]
fn a_realm_without_the_module_is_known_by_the_echo_and_says_both_ways_to_a_navmesh() {
    let mut remote = Remote::new();
    remote.world(true, 0.0);
    // Asked before the probe is answered: it waits.
    remote.ask(ask(walk(1), [150.0, 200.0, 30.0]), 0.5);
    assert_eq!(remote.outbox, ["H 1"]);
    // Stock realms deliver a whisper to oneself: our own probe comes back.
    let out = remote.receive("H 1", 0.6);
    assert_eq!(remote.realm, Realm::Absent);
    let [Outcome::Failed { ask, note }] = &out[..] else {
        panic!("{out:?}");
    };
    assert_eq!(ask.purpose, walk(1));
    assert!(
        note.contains("mod-agent-nav") && note.contains("WOW_NAV_MMAPS"),
        "{note}"
    );
    assert_eq!(remote.describe(), *note);
    assert!(remote
        .no_navmesh(Some("000.mmap: No such file"))
        .contains("the local one failed (000.mmap: No such file"));
}

#[test]
fn an_unanswered_probe_is_sent_three_times_before_the_realm_counts_as_absent() {
    let mut remote = Remote::new();
    remote.world(true, 0.0);
    remote.ask(ask(Purpose::Path, [150.0, 200.0, 30.0]), 1.0);
    assert!(remote.tick(4.9).is_empty());
    assert!(remote.tick(5.1).is_empty() && remote.tick(10.2).is_empty());
    assert_eq!(remote.outbox, ["H 1", "H 1", "H 1"]);
    assert!(matches!(remote.realm, Realm::Probing { tries: 3, .. }));
    let out = remote.tick(15.3);
    assert_eq!(remote.realm, Realm::Absent);
    assert!(matches!(&out[..], [Outcome::Failed { .. }]));
    // A late answer still counts.
    remote.receive(HELLO, 16.0);
    assert!(matches!(remote.realm, Realm::Available(_)));
    // Out of the world nothing is known, and the next entry asks again.
    remote.outbox.clear();
    remote.world(false, 20.0);
    assert_eq!(remote.realm, Realm::Unknown);
    assert!(remote.no_navmesh(None).contains("once in the world"));
    remote.world(true, 21.0);
    assert_eq!(remote.outbox, ["H 1"]);
}

#[test]
fn a_request_goes_out_and_its_route_comes_back_in_parts() {
    let mut remote = available();
    let mut asked = ask(walk(3), [150.0, 260.0, 31.0]);
    asked.avoid = vec![[120.0, 230.0, 23.0]];
    remote.ask(asked.clone(), 1.0);
    assert_eq!(
        std::mem::take(&mut remote.outbox),
        [
            "A 1 120.0 230.0 23.0",
            "P 1 - 100.0 200.0 30.0 150.0 260.0 31.0"
        ]
    );
    assert!(remote
        .receive("r 1 1 2 cs 1000,2000,300 100,100,0", 1.1)
        .is_empty());
    let out = remote.receive("r 1 2 2 cs 400,500,10", 1.2);
    let [Outcome::Route { ask, route, leg }] = &out[..] else {
        panic!("{out:?}");
    };
    assert_eq!((ask, *leg), (&asked, false));
    assert!(route.complete && route.steep);
    assert_eq!(
        route.points,
        [
            [100.0, 200.0, 30.0],
            [110.0, 210.0, 30.0],
            [150.0, 260.0, 31.0]
        ]
    );
    // A reply for a request no longer in flight is dropped.
    assert!(remote.receive("r 1 1 1 c 1000,2000,300", 1.3).is_empty());
    assert!(remote.receive("e 9 noroute x", 1.3).is_empty());
    assert!(remote.receive("gibberish", 1.3).is_empty());
}

#[test]
fn one_request_flies_at_a_time_and_only_the_newest_ask_of_a_kind_waits() {
    let mut remote = available();
    remote.ask(ask(walk(1), [110.0, 200.0, 30.0]), 1.0);
    // A chase asks again as its target moves; AgentPath asks too.
    remote.ask(ask(Purpose::Path, [300.0, 200.0, 30.0]), 1.01);
    remote.ask(ask(walk(2), [120.0, 200.0, 30.0]), 1.02);
    remote.ask(ask(walk(3), [130.0, 200.0, 30.0]), 1.03);
    assert_eq!(remote.outbox.len(), 1, "{:?}", remote.outbox);
    // The answer lands at once, but the next request keeps its distance from the last.
    let out = remote.receive("r 1 1 1 c 1000,2000,300 100,0,0", 1.05);
    assert!(matches!(&out[..], [Outcome::Route { ask, .. }] if ask.purpose == walk(1)));
    assert!(remote.tick(1.2).is_empty());
    assert_eq!(remote.outbox.len(), 1);
    remote.tick(1.0 + SPACING);
    // The newest walk goes before the path question; walk 2 was replaced.
    assert_eq!(remote.outbox[1], "P 2 - 100.0 200.0 30.0 130.0 200.0 30.0");
    let out = remote.receive("e 2 noroute start or end is not on walkable ground", 1.3);
    let [Outcome::Failed { ask, note }] = &out[..] else {
        panic!("{out:?}");
    };
    assert_eq!(ask.purpose, walk(3));
    assert_eq!(note, "no route: start or end is not on walkable ground");
    remote.tick(1.6);
    assert_eq!(remote.outbox[2], "P 3 - 100.0 200.0 30.0 300.0 200.0 30.0");
    assert_eq!(remote.outbox.len(), 3);
}

#[test]
fn an_unanswered_request_times_out_with_a_clear_note() {
    let mut remote = available();
    remote.ask(ask(walk(1), [110.0, 200.0, 30.0]), 1.0);
    assert!(remote.tick(1.0 + REPLY_TIMEOUT - 0.1).is_empty());
    let out = remote.tick(1.1 + REPLY_TIMEOUT);
    let [Outcome::Failed { note, .. }] = &out[..] else {
        panic!("{out:?}");
    };
    assert_eq!(
        note,
        "the realm did not answer the route request within 8 s"
    );
    // Its answer, should it still come, belongs to nothing.
    assert!(remote.receive("r 1 1 1 c 1000,2000,300", 10.0).is_empty());
    // The realm's refusals read as sentences.
    remote.ask(ask(walk(2), [110.0, 200.0, 30.0]), 20.0);
    let out = remote.receive("e 2 rate 5 a second", 20.1);
    assert!(
        matches!(&out[..], [Outcome::Failed { note, .. }] if note.contains("too many requests (it allows 5 a second")),
        "{out:?}"
    );
}

#[test]
fn a_goal_past_the_realm_limit_is_asked_for_a_leg_at_a_time() {
    let mut remote = available();
    // 3000 yd east: the realm routes 1500 at most.
    remote.ask(ask(walk(1), [3100.0, 200.0, 77.0]), 1.0);
    assert_eq!(remote.outbox, ["P 1 n 100.0 200.0 30.0 1450.0 200.0 -"]);
    let out = remote.receive("r 1 1 1 c 1000,2000,300 13400,10,-50", 1.1);
    let [Outcome::Route { route, leg, .. }] = &out[..] else {
        panic!("{out:?}");
    };
    // A leg is never the whole way, whatever the realm says of it.
    assert!(*leg && !route.complete);
    assert_eq!(route.points[1], [1440.0, 201.0, 25.0]);
}

#[test]
fn only_the_nearest_circles_are_sent_when_the_realm_takes_fewer() {
    let mut remote = Remote::new();
    remote.world(true, 0.0);
    remote.receive("h 1 1500 256 2 5 240", 0.1);
    remote.outbox.clear();
    let mut asked = ask(walk(1), [150.0, 200.0, 30.0]);
    asked.avoid = vec![
        [400.0, 200.0, 20.0],
        [110.0, 200.0, 20.0],
        [100.0, 230.0, 20.0],
    ];
    remote.ask(asked, 1.0);
    assert_eq!(remote.outbox[0], "A 1 110.0 200.0 20.0 100.0 230.0 20.0");
}

#[test]
fn leaving_the_world_ends_what_was_asked() {
    let mut remote = available();
    remote.ask(ask(walk(1), [110.0, 200.0, 30.0]), 1.0);
    remote.ask(ask(Purpose::Path, [120.0, 200.0, 30.0]), 1.0);
    let out = remote.world(false, 2.0);
    assert_eq!(out.len(), 2);
    assert!(out
        .iter()
        .all(|o| matches!(o, Outcome::Failed { note, .. } if note.contains("left the world"))));
    assert!(remote.outbox.is_empty());
}
