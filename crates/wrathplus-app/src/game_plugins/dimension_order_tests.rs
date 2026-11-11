// Producer/consumer ordering regressions.

use super::*;

/// Height and model-scale Commands must land before the frame's depth and camera readers.
#[test]
fn unit_dimensions_are_stamped_before_their_frame_consumers() {
    let mut app = headless_client();
    let c = census(&mut app, Update, SyncPoints::Declared);
    if !type_names_available(&c) {
        eprintln!("skipped: this build carries no type names");
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
    let precedes = |from: &str, to: &str| {
        let target = one(to);
        let mut seen = HashSet::new();
        let mut stack = vec![one(from)];
        while let Some(n) = stack.pop() {
            for (_, next) in c.dependencies.iter().filter(|(a, _)| *a == n) {
                if seen.insert(*next) {
                    stack.push(*next);
                }
            }
        }
        assert!(seen.contains(&target), "{from} must precede {to}");
    };
    let height = "::collision_height::stamp_collision_heights";
    let scale = "::live_display::stamp_unit_model_scales";
    for stamp in [height, scale] {
        precedes("::entities::evict_display_caches", stamp);
        precedes("::net::apply::apply_net_updates", stamp);
    }
    precedes(height, "::entities::publish_world_units");
    let mirror = "::player::mirror_mover_collision_height";
    precedes(height, mirror);
    precedes("::land::land_here", mirror);
    precedes("::server_ride::drive_self_ride", mirror);
    precedes("::wire_in::release_on_session_end", mirror);
    precedes(mirror, "::follow::steer_follow");
    precedes(mirror, "::approach::steer_approach");
    precedes(scale, "::view_subject::publish_view_subject");
    precedes(scale, "::controller::control");
    precedes(scale, "::attach::attach_entity_visuals");
}

#[test]
fn the_client_builds_headless() {
    let app = headless_client();
    assert!(app.world().contains_resource::<Schedules>());
}
