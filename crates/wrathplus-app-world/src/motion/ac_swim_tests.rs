//! Actual sampler -> room-water marker -> clamp -> animation-unify regression, not a predicate.
use super::*;
use crate::app_core::movement::{move_flags, CANNOT_SWIM};
use crate::app_core::session::Embodied;
use crate::app_core::session::ObjectStore;
use crate::app_core::state::CollisionHeight;
use crate::creature_anim::{select::unify, MovementState};
use crate::motion::{RemoteMotion, UnitMoveModes};
use avian3d::prelude::*;
use wrathplus_protocol::types::{Field, ObjectFields};
use wrathplus_world::collision::ColliderEpoch;
use wrathplus_world::wmo_portal::UnitWmoRoom;

const H: f32 = 2.031;
const WATER: f32 = 10.0;
const FEET: f32 = WATER - 0.5922368 * H;

fn fixture(floor: Option<f32>, feet: f32) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<wrathplus_world::collision::MoverTraceExclusions>();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        PhysicsPlugins::new(PostUpdate),
    ));
    app.init_asset::<Mesh>().init_resource::<ColliderEpoch>();
    wrathplus_world::world_point::init_world_point_resources(app.world_mut());
    app.insert_resource(crate::app_core::movement::PlayerCapsule(Collider::capsule(
        crate::app_core::movement::CAPSULE_RADIUS,
        crate::app_core::movement::CAPSULE_HEIGHT - 2.0 * crate::app_core::movement::CAPSULE_RADIUS,
    )));
    app.finish();
    app.cleanup();
    if let Some(y) = floor {
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::trimesh(
                vec![
                    Vec3::new(-20., y, -20.),
                    Vec3::new(20., y, -20.),
                    Vec3::new(20., y, 20.),
                    Vec3::new(-20., y, 20.),
                ],
                vec![[0, 2, 1], [0, 3, 2]],
            ),
            Transform::default(),
        ));
    }
    app.world_mut()
        .spawn(wrathplus_world::liquid::WaterChunkInfo::new(
            wrathplus_world::liquid::LiquidSource::AdtChunk,
            wrathplus_formats::LiquidKind::Still,
            [2, 2],
            vec![
                [-20., -20., WATER],
                [20., -20., WATER],
                [-20., 20., WATER],
                [20., 20., WATER],
            ],
            vec![true],
        ));
    wrathplus_world::world_point::index_fixture_liquids(app.world_mut());
    let body = app
        .world_mut()
        .spawn((
            NetEntity {
                kind: EntityKind::Player,
                display_id: None,
                scale: 1.0,
            },
            Transform::from_translation(wow_to_bevy([0., 0., feet])),
            CollisionHeight(H),
            ObjectStore(ObjectFields::from_pairs(&[(Field::UnitFlags.index(), 8)])),
            UnitWmoRoom::default(),
            Spline {
                deck: None,
                points: vec![[0., -1., feet], [0., 1., feet]],
                start: Instant::now(),
                duration: Duration::from_secs(60),
                id: 7,
                grounded: true,
                interpolation: SplineInterpolation::Linear,
                run_mode: Some(true),
            },
        ))
        .id();
    app.update(); // build the real collider trees before running the production chain
    app.add_systems(
        Update,
        (
            sample_splines,
            mark_swimming_creatures,
            ground_clamp_creatures,
        )
            .chain(),
    );
    (app, body)
}

fn state(app: &App, e: Entity) -> MovementState {
    let w = app.world();
    unify(
        w.get::<MovementState>(e),
        w.get::<RemoteMotion>(e),
        w.get::<Spline>(e),
        w.get::<CreatureSwimming>(e).is_some(),
        w.get::<UnitMoveModes>(e),
    )
}

#[test]
fn ac_surface_seat_over_deep_support_swims_but_same_seat_over_shore_wades() {
    for (floor, swims) in [(Some(6.0), true), (Some(FEET), false), (None, false)] {
        let (mut app, e) = fixture(floor, FEET);
        for _ in 0..3 {
            app.update();
            assert_eq!(
                app.world().get::<CreatureSwimming>(e).is_some(),
                swims,
                "AC feet {FEET} at water {WATER} over support {floor:?}"
            );
            assert_eq!(state(&app, e).flags & move_flags::SWIMMING != 0, swims);
            let y = app.world().get::<Transform>(e).unwrap().translation.y;
            assert!(
                (y - FEET).abs() < 1e-3,
                "seat preserved for surface swim or shallow wade: {y}"
            );
        }
    }
}

#[test]
fn ac_surface_swim_policy_negative_controls_keep_their_authority() {
    for case in [
        "cannot",
        "no_permit",
        "waterwalk",
        "local",
        "relay",
        "flying",
        "deck",
        "bed",
        "not_seat",
        "unknown_room",
    ] {
        let feet = if case == "bed" {
            6.0
        } else if case == "not_seat" {
            WATER - 0.4 * H
        } else {
            FEET
        };
        let (mut app, e) = fixture(Some(6.0), feet);
        match case {
            "cannot" | "no_permit" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert(ObjectStore(ObjectFields::from_pairs(&[(
                        Field::UnitFlags.index(),
                        if case == "cannot" { 8 | CANNOT_SWIM } else { 0 },
                    )])));
            }
            "waterwalk" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert(UnitMoveModes(move_flags::WATER_WALKING));
            }
            "local" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert((Embodied, MovementState::default()));
            }
            "relay" => {
                app.world_mut().entity_mut(e).insert(RemoteMotion {
                    wow_pos: [0., 0., FEET],
                    orientation: 0.,
                    flags: move_flags::FORWARD,
                    pitch: 0.,
                    speed: 7.,
                    vertical_velocity: 0.,
                    jump_xy_vel: [0.; 2],
                    fall_start_z: None,
                    pending: Default::default(),
                    relay: Default::default(),
                    last_apply_ms: 0.,
                    last_apply_pos: [0.; 3],
                });
            }
            "flying" => app.world_mut().get_mut::<Spline>(e).unwrap().grounded = false,
            "deck" => app.world_mut().get_mut::<Spline>(e).unwrap().deck = Some(123),
            "unknown_room" => {
                app.world_mut().entity_mut(e).remove::<UnitWmoRoom>();
            }
            "bed" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert(ObjectStore(ObjectFields::from_pairs(&[(
                        Field::UnitFlags.index(),
                        0,
                    )])));
            }
            _ => {}
        }
        app.update();
        assert!(
            app.world().get::<CreatureSwimming>(e).is_none(),
            "{case} must not acquire inferred swim"
        );
        assert_eq!(state(&app, e).flags & move_flags::SWIMMING, 0, "{case}");
        if matches!(case, "local" | "relay" | "flying" | "deck") {
            assert!(
                (app.world().get::<Transform>(e).unwrap().translation.y - feet).abs() < 1e-3,
                "{case} owns Z"
            );
        }
    }
}

#[test]
fn ac_surface_swim_releases_on_shore_and_permit_mode_or_path_loss() {
    for case in ["permit", "waterwalk", "path", "shore", "height"] {
        let (mut app, e) = fixture(Some(6.0), FEET);
        app.update();
        assert!(app.world().get::<CreatureSwimming>(e).is_some());
        match case {
            "permit" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert(ObjectStore(ObjectFields::from_pairs(&[(
                        Field::UnitFlags.index(),
                        0,
                    )])));
            }
            "waterwalk" => {
                app.world_mut()
                    .entity_mut(e)
                    .insert(UnitMoveModes(move_flags::WATER_WALKING));
            }
            "path" => {
                app.world_mut().entity_mut(e).remove::<Spline>();
            }
            "height" => app.world_mut().get_mut::<CollisionHeight>(e).unwrap().0 = H * 2.0,
            "shore" => {
                // Replace only support; same wire feet and room water.
                let mut q = app.world_mut().query_filtered::<Entity, With<RigidBody>>();
                let floors: Vec<_> = q.iter(app.world()).collect();
                for f in floors {
                    app.world_mut().despawn(f);
                }
                app.world_mut().spawn((
                    RigidBody::Static,
                    Collider::cuboid(40.0, 0.1, 40.0),
                    Transform::from_xyz(0.0, FEET - 0.05, 0.0),
                ));
                app.world_mut().resource_mut::<ColliderEpoch>().bump();
                // Publish the new collider tree; next sample must reclassify.
                app.update();
            }
            _ => unreachable!(),
        }
        app.update();
        assert!(
            app.world().get::<CreatureSwimming>(e).is_none(),
            "{case} releases compatibility swim"
        );
    }
}
