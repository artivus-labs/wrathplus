//! Whole-client tests of `crate::ui_unit::held`, from `ui_unit/held.rs` (`tests`).
//! They build the client with `headless_client`, so they live in the root layer.

use bevy::prelude::*;

/// The chain feed reads the resolver inputs the aura feed pushes in the same frame, so the
/// declared order puts the feed after it: read off the schedule graph, by the type each system
/// runs under (a build without `bevy/debug` names none).
#[test]
fn the_chain_feed_runs_after_the_aura_feed_it_reads() {
    use crate::game_plugins::schedule_tests::{census, headless_client, SyncPoints};
    use crate::ui_unit::aura_readers;

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
    for (feed, ty) in [
        ("feed_chain_units", aura_readers::feed_chain_units()),
        ("feed_targeting_to_vm", aura_readers::feed_targeting_to_vm()),
    ] {
        assert!(
            c.dependencies
                .contains(&(aura, one(feed, &|s| s.type_id == ty))),
            "{feed} is declared after feed_auras"
        );
    }
}
