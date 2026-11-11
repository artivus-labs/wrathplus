// Producer/consumer ordering regressions.

use super::*;

/// A follow requested by this tick must reach the resolver in the same frame.
#[test]
fn follow_requests_are_drained_before_the_target_chain_consumes_them() {
    let mut app = headless_client();
    let c = census(&mut app, Update, SyncPoints::Declared);
    if !type_names_available(&c) {
        return;
    }
    let one = |suffix: &str| -> SystemKey {
        let hits: Vec<_> = c
            .systems
            .iter()
            .filter(|(_, s)| s.name.ends_with(suffix))
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(hits.len(), 1, "exactly one system named …{suffix}");
        hits[0]
    };
    let mut seen = HashSet::new();
    let mut stack = vec![one("::ui_follow::drain_follow")];
    while let Some(n) = stack.pop() {
        for (_, next) in c.dependencies.iter().filter(|(a, _)| *a == n) {
            if seen.insert(*next) {
                stack.push(*next);
            }
        }
    }
    assert!(seen.contains(&one("::by_name::follow::follow_requests")));
}

#[test]
fn the_client_builds_headless() {
    let app = headless_client();
    assert!(app.world().contains_resource::<Schedules>());
}
