//! Real claim producers on overlapping resident geometry, independent of the camera claim.
use super::*;
use crate::world_unit::WorldUnit;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<Assets<WmoModel>>()
        .init_resource::<Assets<AdtTile>>()
        .init_resource::<TerrainStreamer>()
        .init_resource::<crate::interior::WmoResidency>()
        .init_resource::<CurrentAreaInterior>()
        .init_resource::<CurrentWmoInterior>()
        .init_resource::<PlayerWmoRoom>()
        .insert_resource(crate::view::Viewer {
            at: Some(wow_to_bevy([-3.0, 0.0, 0.0])),
            ..default()
        })
        .insert_resource(super::super::ExteriorWindows::Windows(vec![]))
        .add_plugins(crate::exterior_cull::ExteriorCullPlugin);
    app.world_mut().spawn((
        WorldCamera,
        GlobalTransform::from_translation(Vec3::new(0.0, 1.0, 10.0)),
        Projection::Perspective(PerspectiveProjection::default()),
    ));
    let unit = app
        .world_mut()
        .spawn((
            // Deliberately disagreeing local/world transforms: mounted poses use GlobalTransform.
            Transform::from_xyz(500.0, 500.0, 500.0),
            GlobalTransform::from_translation(wow_to_bevy([-3.0, 0.0, 0.0])),
            WorldUnit {
                wades: true,
                scale: 1.0,
                height: 2.0,
                bound: Some(bevy::camera::primitives::Aabb::from_min_max(
                    Vec3::new(-0.5, 0.0, -0.5),
                    Vec3::new(0.5, 2.0, 0.5),
                )),
            },
            Visibility::Inherited,
        ))
        .id();
    (app, unit)
}

fn spawn(app: &mut App, z: f32, flags: u32, id: u32) -> Entity {
    let mut model = super::super::tests::doorway_model();
    model.wmo_id = id;
    model.group_nav.truncate(1);
    model.group_nav[0].flags = flags;
    model.group_nav[0].ref_count = 0;
    model.group_nav[0].area_table_id = id + 100;
    model.group_nav[0].bbox_min[2] = z;
    model.group_nav[0].bbox_max[2] = z + 3.0;
    model.group_collision_tris.truncate(1);
    for tri in &mut model.group_collision_tris[0] {
        for v in tri {
            v[2] = z;
        }
    }
    (model.group_collision_bounds, model.collision_bounds) =
        wrathplus_assets::collision_tri_bounds(&model.group_collision_tris);
    let handle = app
        .world_mut()
        .resource_mut::<Assets<WmoModel>>()
        .add(model);
    app.world_mut()
        .spawn(WmoPortalInstance {
            handle,
            world_from_local: Affine3A::IDENTITY,
            name_set: 7,
            visible: vec![true],
            interior_fog: vec![false],
            liquid_visited: vec![false],
            flooded: vec![None],
        })
        .id()
}

fn edit_model(app: &mut App, entity: Entity, f: impl FnOnce(&mut WmoModel)) {
    let handle = app
        .world()
        .get::<WmoPortalInstance>(entity)
        .unwrap()
        .handle
        .clone();
    f(app
        .world_mut()
        .resource_mut::<Assets<WmoModel>>()
        .get_mut(&handle)
        .unwrap());
}

fn run(app: &mut App) {
    let handles: Vec<_> = app
        .world_mut()
        .query::<&WmoPortalInstance>()
        .iter(app.world())
        .map(|inst| inst.handle.id())
        .collect();
    app.world_mut()
        .resource_mut::<crate::interior::WmoResidency>()
        .update(handles);
    app.world_mut()
        .run_system_once(track_area_interior)
        .unwrap();
    app.world_mut()
        .run_system_once(track_current_interior)
        .unwrap();
    app.world_mut()
        .run_system_once(track_unit_interiors)
        .unwrap();
}

fn unit_room(app: &App, unit: Entity) -> Option<WmoRoom> {
    app.world().get::<UnitWmoRoom>(unit).unwrap().room()
}

fn assert_claims(app: &App, unit: Entity, expected: Option<(Entity, u32)>) {
    let room = expected.map(|(instance, _)| WmoRoom { instance, group: 0 });
    assert_eq!(app.world().resource::<PlayerWmoRoom>().0, room);
    assert_eq!(unit_room(app, unit), room);
    let keys = expected.map(|(_, id)| WmoInteriorKeys {
        wmo_id: id,
        name_set: 7,
        group_area_id: id + 100,
    });
    assert_eq!(app.world().resource::<CurrentAreaInterior>().0, keys);
    assert_eq!(app.world().resource::<CurrentWmoInterior>().0, keys);
}

#[test]
fn multi_placement_systems_nearest_interior_and_exterior_both_orders() {
    for near_first in [false, true] {
        for exterior in [false, true] {
            let (mut app, unit) = fixture();
            let (near, deep) = if near_first {
                let near = spawn(&mut app, -0.5, if exterior { EXTERIOR } else { 0 }, 600);
                (near, spawn(&mut app, -185.0, 0, 1218))
            } else {
                let deep = spawn(&mut app, -185.0, 0, 1218);
                run(&mut app);
                assert_claims(&app, unit, Some((deep, 1218)));
                (
                    spawn(&mut app, -0.5, if exterior { EXTERIOR } else { 0 }, 600),
                    deep,
                )
            };
            run(&mut app);
            assert_claims(&app, unit, (!exterior).then_some((near, 600)));
            // Stationary residency removal must invalidate the cached unit claim, with no cap.
            app.world_mut().despawn(near);
            run(&mut app);
            assert_claims(&app, unit, Some((deep, 1218)));
        }
    }
}

#[test]
fn metadata_never_exposes_deeper_geometry() {
    for flags in [0, EXTERIOR] {
        let (mut app, unit) = fixture();
        spawn(&mut app, -185.0, 0, 1218);
        let near = spawn(&mut app, -0.5, flags, 0);
        run(&mut app);
        assert!(app.world().resource::<CurrentAreaInterior>().0.is_none());
        assert!(app.world().resource::<CurrentWmoInterior>().0.is_none());
        let room = (flags == 0).then_some(WmoRoom {
            instance: near,
            group: 0,
        });
        assert_eq!(app.world().resource::<PlayerWmoRoom>().0, room);
        assert_eq!(unit_room(&app, unit), room);
    }
}

#[test]
fn unit_retry_only_after_true_global_miss() {
    // A reachable floor above the feet is only adopted when the complete normal pass misses.
    for lower in [None, Some(EXTERIOR), Some(0)] {
        let (mut app, unit) = fixture();
        let above = spawn(&mut app, 1.0, 0, 600);
        let below = lower.map(|flags| spawn(&mut app, -185.0, flags, 1218));
        run(&mut app);
        let expected = match lower {
            None => Some(above),
            Some(EXTERIOR) => None,
            Some(_) => below,
        };
        assert_eq!(unit_room(&app, unit).map(|r| r.instance), expected);
    }
    // Reach is the existing bound-centre/minimum-two policy, not an unlimited upward search.
    for (floor, centre, expected) in [(2.2, 1.0, false), (3.0, 4.0, true)] {
        let (mut app, unit) = fixture();
        spawn(&mut app, floor, 0, 600);
        app.world_mut().get_mut::<WorldUnit>(unit).unwrap().bound =
            Some(bevy::camera::primitives::Aabb::from_min_max(
                Vec3::new(-0.5, centre - 0.5, -0.5),
                Vec3::new(0.5, centre + 0.5, 0.5),
            ));
        run(&mut app);
        assert_eq!(unit_room(&app, unit).is_some(), expected);
    }
}

#[test]
fn faces_only_masks_range_and_render_fallback_remain_separate() {
    let (mut app, unit) = fixture();
    let porch = spawn(&mut app, -0.5, EXTERIOR_LIT, 600);
    run(&mut app);
    assert_claims(&app, unit, Some((porch, 600)));
    let outdoors = app
        .world_mut()
        .run_system_once(
            |wmos: Res<Assets<WmoModel>>,
             instances: Query<&WmoPortalInstance>,
             streamer: Res<TerrainStreamer>,
             tiles: Res<Assets<AdtTile>>| {
                indoors_at(
                    &wmos,
                    instances,
                    &streamer,
                    &tiles,
                    wow_to_bevy([-3.0, 0.0, 0.0]),
                )
            },
        )
        .unwrap();
    assert!(!outdoors, "0x40 is outdoor for lighting only");

    for detail in [false, true] {
        let (mut app, unit) = fixture();
        let entity = spawn(&mut app, if detail { -0.5 } else { -1100.0 }, 0, 600);
        if detail {
            edit_model(&mut app, entity, |m| {
                m.group_camera_only_tris = m.group_collision_tris.clone();
                m.group_collision_tris = vec![vec![]];
                (m.group_collision_bounds, m.collision_bounds) =
                    wrathplus_assets::collision_tri_bounds(&m.group_collision_tris);
            });
        }
        run(&mut app);
        assert!(app.world().resource::<CurrentAreaInterior>().0.is_none());
        assert!(unit_room(&app, unit).is_none());
        assert_eq!(
            app.world().resource::<PlayerWmoRoom>().0.unwrap().instance,
            entity
        );
    }
    // Portal-only vertical doorway at the probe column claims render, never position faces.
    let (mut app, unit) = fixture();
    let entity = spawn(&mut app, 0.0, 0, 600);
    edit_model(&mut app, entity, |m| {
        m.group_collision_tris = vec![vec![]];
        (m.group_collision_bounds, m.collision_bounds) =
            wrathplus_assets::collision_tri_bounds(&m.group_collision_tris);
        m.group_nav[0].ref_count = 1;
        m.portal_refs[0].side = 1;
        m.portal_infos[0].plane = [1.0, 0.0, 0.0, 3.0];
        m.portal_vertices[..4].copy_from_slice(&[
            [-3.0, -1.0, 0.0],
            [-3.0, 1.0, 0.0],
            [-3.0, 1.0, 3.0],
            [-3.0, -1.0, 3.0],
        ]);
    });
    run(&mut app);
    assert!(app.world().resource::<CurrentAreaInterior>().0.is_none());
    assert!(unit_room(&app, unit).is_none());
    assert_eq!(
        app.world().resource::<PlayerWmoRoom>().0.unwrap().instance,
        entity
    );
}

#[test]
fn real_claim_to_root_visibility_and_animation_room_predicate() {
    let (mut app, unit) = fixture();
    let deep = spawn(&mut app, -185.0, 0, 1218);
    app.world_mut()
        .get_mut::<WmoPortalInstance>(deep)
        .unwrap()
        .visible[0] = false;
    run(&mut app);
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Hidden
    );
    let near = spawn(&mut app, -0.5, 0, 600);
    run(&mut app);
    assert_claims(&app, unit, Some((near, 600)));
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Inherited,
        "correct visible local room admits root even with sealed exterior windows"
    );
    let visible = app
        .world_mut()
        .run_system_once(
            |units: Query<&UnitWmoRoom>,
             instances: Query<&WmoPortalInstance>,
             wmos: Res<Assets<WmoModel>>| {
                super::super::room_pvs_visible(units.iter().next(), &instances, &wmos)
            },
        )
        .unwrap();
    assert!(visible, "animation LOD consumes this same predicate");
    app.world_mut()
        .get_mut::<WmoPortalInstance>(near)
        .unwrap()
        .visible[0] = false;
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Hidden
    );
    // Genuine outdoor classification follows the normal exterior gate, not a fail-open bypass.
    edit_model(&mut app, near, |m| m.group_nav[0].flags = EXTERIOR);
    // Force the existing movement gate rather than mutating private cache state.
    *app.world_mut().get_mut::<GlobalTransform>(unit).unwrap() =
        GlobalTransform::from_translation(wow_to_bevy([-3.3, 0.0, 0.0]));
    run(&mut app);
    assert!(unit_room(&app, unit).is_none());
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Hidden
    );
    *app.world_mut()
        .resource_mut::<super::super::ExteriorWindows>() =
        super::super::ExteriorWindows::Unrestricted;
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Inherited
    );
}

#[test]
fn global_faces_terrain_segment_and_exact_tie_controls() {
    for (terrain, expected) in [
        (None, true),
        (Some(-0.5), true),
        (Some(-0.4), false),
        (Some(1.0), true),
    ] {
        let (mut app, _) = fixture();
        spawn(&mut app, -185.0, 0, 1218);
        let near = spawn(&mut app, -0.5, 0, 600);
        let hit = app
            .world_mut()
            .run_system_once(
                move |wmos: Res<Assets<WmoModel>>,
                      instances: Query<(Entity, &WmoPortalInstance)>| {
                    nearest_crossing(
                        &wmos,
                        &instances,
                        wow_to_bevy([-3.0, 0.0, 0.1]),
                        terrain,
                        ClaimRay::Faces,
                    )
                },
            )
            .unwrap();
        assert_eq!(hit.map(|h| h.instance), expected.then_some(near));
    }
}

#[test]
fn unit_movement_threshold_and_visible_room_frustum_control() {
    let (mut app, unit) = fixture();
    let near = spawn(&mut app, -0.5, 0, 600);
    run(&mut app);
    edit_model(&mut app, near, |m| m.group_nav[0].flags = EXTERIOR);
    // Asset mutation without residency churn: below-threshold motion keeps the cached claim.
    *app.world_mut().get_mut::<GlobalTransform>(unit).unwrap() =
        GlobalTransform::from_translation(wow_to_bevy([-3.1, 0.0, 0.0]));
    run(&mut app);
    assert_eq!(unit_room(&app, unit).unwrap().instance, near);
    *app.world_mut().get_mut::<GlobalTransform>(unit).unwrap() =
        GlobalTransform::from_translation(wow_to_bevy([-3.25, 0.0, 0.0]));
    run(&mut app);
    assert!(unit_room(&app, unit).is_none(), "exact threshold re-rays");
    edit_model(&mut app, near, |m| m.group_nav[0].flags = 0);
    *app.world_mut().get_mut::<GlobalTransform>(unit).unwrap() =
        GlobalTransform::from_translation(wow_to_bevy([-3.0, 0.0, 0.0]));
    run(&mut app);
    // A correct visible room never bypasses the root frustum test.
    let mut cameras = app
        .world_mut()
        .query_filtered::<&mut GlobalTransform, With<WorldCamera>>();
    *cameras.single_mut(app.world_mut()).unwrap() =
        GlobalTransform::from_translation(Vec3::new(0.0, 1.0, -10.0));
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(unit).unwrap(),
        Visibility::Hidden
    );
}
