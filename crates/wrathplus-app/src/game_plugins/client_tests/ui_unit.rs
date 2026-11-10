//! Whole-client tests of `crate::ui_unit`, from `ui_unit.rs` (`tests`).
//! They build the client with `headless_client`, so they live in the root layer.

use bevy::prelude::*;

/// The reach feed measures the guids the aura feed pushes, so a frame's feeds must run in that
/// order: read off the declared graph, the aura feed's set comes before the reach feed. Systems
/// are found by the type they run under, as a build without `bevy/debug` names none.
#[test]
fn the_reach_feed_runs_after_the_aura_feed_it_reads() {
    use crate::game_plugins::schedule_tests::{census, headless_client, SyncPoints};

    let mut app = headless_client();
    let c = census(&mut app, Update, SyncPoints::Declared);
    let one = |what: &str, hit: &dyn Fn(&_) -> bool| {
        let mut hits = c.systems.iter().filter(|(_, s)| hit(s));
        match (hits.next(), hits.next()) {
            (Some((k, _)), None) => *k,
            _ => panic!("exactly one {what} in Update"),
        }
    };
    let aura = one("system in AuraEvents", &|s| {
        s.sets.iter().any(|set| set == "AuraEvents")
    });
    let reach_type = crate::ui_unit::aura_readers::feed_unit_reach();
    let reach = one("feed_unit_reach", &|s| s.type_id == reach_type);
    assert!(
        c.dependencies.contains(&(aura, reach)),
        "feed_unit_reach is declared after feed_auras"
    );
}
