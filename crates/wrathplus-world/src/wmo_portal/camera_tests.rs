//! Camera-wide regressions exercise the publishing system, not just the per-model seed.
use super::*;
use bevy::ecs::system::RunSystemOnce;

fn world() -> World {
    let mut world = World::new();
    world.init_resource::<Assets<WmoModel>>();
    world.init_resource::<Assets<AdtTile>>();
    world.init_resource::<TerrainStreamer>();
    world.init_resource::<WmoCullProbe>();
    world.init_resource::<CameraWmoFog>();
    world.init_resource::<CameraInteriorClaim>();
    world.init_resource::<ExteriorWindows>();
    world.spawn((
        WorldCamera,
        GlobalTransform::from(Transform::from_translation(wow_to_bevy([-3.0, 0.0, 1.0]))),
        Projection::Perspective(PerspectiveProjection::default()),
    ));
    world
}

fn spawn(world: &mut World, depth: f32, exterior: bool, portal: bool) -> Entity {
    let mut model = tests::doorway_model();
    // A sealed interior with a valid resident graph, or its exterior-floor counterpart.
    // Move the actual floor, not merely its bounds; no height-cap shortcut can pass this test.
    let z = 1.0 - depth;
    for tri in &mut model.group_collision_tris[0] {
        for v in tri {
            v[2] = z;
        }
    }
    model.group_nav[0].bbox_min[2] = z;
    model.group_nav[0].bbox_max[2] = z + 3.0;
    model.group_nav[0].ref_count = 0;
    if exterior {
        model.group_nav[0].flags |= EXTERIOR;
    }
    if portal {
        model.group_nav[0].ref_count = 1;
        model.portal_infos[0].plane = [0.0, 0.0, 1.0, -z];
        model.portal_vertices[..4].copy_from_slice(&[
            [-10.0, -10.0, z],
            [0.0, -10.0, z],
            [0.0, 10.0, z],
            [-10.0, 10.0, z],
        ]);
        model.portal_refs[0].side = 1;
    }
    let handle = world.resource_mut::<Assets<WmoModel>>().add(model);
    world
        .spawn(WmoPortalInstance {
            handle,
            world_from_local: Affine3A::IDENTITY,
            name_set: 0,
            visible: Vec::new(),
            interior_fog: Vec::new(),
            liquid_visited: Vec::new(),
            flooded: Vec::new(),
        })
        .id()
}

fn run(world: &mut World) {
    world.run_system_once(compute_wmo_pvs).unwrap();
}

#[test]
fn near_exterior_defeats_deep_interior_in_both_orders_and_clears_old_claim() {
    for exterior_first in [false, true] {
        let mut w = world();
        let (near, deep) = if exterior_first {
            let near = spawn(&mut w, 4.26, true, false);
            (near, spawn(&mut w, 185.7, false, false))
        } else {
            let deep = spawn(&mut w, 185.7, false, false);
            run(&mut w);
            assert_eq!(
                w.resource::<CameraInteriorClaim>().0.unwrap().room.instance,
                deep
            );
            assert_eq!(
                *w.resource::<ExteriorWindows>(),
                ExteriorWindows::Windows(vec![])
            );
            (spawn(&mut w, 4.26, true, false), deep)
        };
        run(&mut w);
        assert!(w.resource::<CameraInteriorClaim>().0.is_none());
        assert!(w.resource::<CameraWmoFog>().0.is_none());
        assert_eq!(
            *w.resource::<ExteriorWindows>(),
            ExteriorWindows::Unrestricted
        );
        assert!(w.get::<WmoPortalInstance>(near).unwrap().visible[0]);
        assert!(w.get::<WmoPortalInstance>(deep).unwrap().visible[0]);
        // The same deep room is a valid interior when the nearby exterior is absent.
        w.despawn(near);
        run(&mut w);
        assert_eq!(
            w.resource::<CameraInteriorClaim>().0.unwrap().room.instance,
            deep
        );
        assert_eq!(
            *w.resource::<ExteriorWindows>(),
            ExteriorWindows::Windows(vec![])
        );
    }
}

#[test]
fn nearest_true_interior_wins_in_both_orders() {
    for near_first in [false, true] {
        let mut w = world();
        let near = if near_first {
            let near = spawn(&mut w, 2.0, false, false);
            spawn(&mut w, 185.7, false, false);
            near
        } else {
            spawn(&mut w, 185.7, false, false);
            spawn(&mut w, 2.0, false, false)
        };
        run(&mut w);
        assert_eq!(
            w.resource::<CameraInteriorClaim>().0.unwrap().room.instance,
            near
        );
        assert_eq!(
            *w.resource::<ExteriorWindows>(),
            ExteriorWindows::Windows(vec![])
        );
    }
}

#[test]
fn portal_crossing_participates_globally_and_keeps_across_seed() {
    let mut w = world();
    spawn(&mut w, 185.7, false, false);
    let door = spawn(&mut w, 1.0, false, true);
    let inst = w.get::<WmoPortalInstance>(door).unwrap();
    let model = w.resource::<Assets<WmoModel>>().get(&inst.handle).unwrap();
    let hit = camera_crossing(model, [-3.0, 0.0, 1.0], None).unwrap();
    assert!(hit.portal);
    assert_eq!(hit.seeds.across, Some(1));
    run(&mut w);
    assert_eq!(
        w.resource::<CameraInteriorClaim>().0.unwrap().room.instance,
        door
    );
}

#[test]
fn global_ties_are_total_and_order_independent() {
    // Exact portal/face ties prefer the portal; otherwise stable identity breaks ties.
    // Near but unequal depths retain the nearest, without epsilon non-transitivity.
    let keys = [
        (1.0, true, 1),
        (1.0, false, 3),
        (1.0, false, 2),
        (1.00001, false, 0),
    ];
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 0, 3, 2]] {
        let mut best = keys[order[0]];
        for i in order {
            if crossing_precedes(keys[i], best) {
                best = keys[i];
            }
        }
        assert_eq!(best, keys[2]);
    }
}
