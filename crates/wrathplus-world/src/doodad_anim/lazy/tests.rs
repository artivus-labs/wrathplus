use super::super::{gate_doodad_anim, DoodadAnimHost};
use super::*;
use bevy::animation::graph::AnimationNodeIndex;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;

fn app() -> App {
    let mut app = App::new();
    // `Time` is manual: the real-clock driver would clobber the tests' own advances.
    app.add_plugins((
        bevy::MinimalPlugins
            .build()
            .disable::<bevy::time::TimePlugin>(),
        AssetPlugin::default(),
    ));
    app.init_resource::<Time>();
    app.init_resource::<crate::view::ViewDistance>();
    app.init_resource::<crate::wmo_portal::ExteriorWindows>();
    app.init_resource::<crate::wmo_portal::CameraInteriorClaim>();
    app.init_resource::<RigPalettes>();
    app.init_asset::<SkinnedMeshInverseBindposes>();
    app.init_asset::<Mesh>();
    app.add_systems(Update, (gate_doodad_anim, reap_parked_rigs).chain());
    app
}

/// A host with a one-bone [`LazyRig`] whose single [`SkinnedTwin`] part carries its visibility.
fn lazy_host(app: &mut App, visible: bool) -> (Entity, Entity) {
    let stat = Handle::<Mesh>::default();
    let skinned = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .reserve_handle();
    let part = app
        .world_mut()
        .spawn((
            Mesh3d(stat.clone()),
            MeshTag(crate::mesh_tag::alpha_bits(1.0)),
            SkinnedTwin {
                skinned,
                stat: stat.clone(),
            },
            if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ))
        .id();
    let ibp = app
        .world_mut()
        .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
        .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY]));
    let host = app
        .world_mut()
        .spawn((
            DoodadAnimHost {
                meshes: vec![part],
                fade: crate::particles::EmitterFade::sphere(1.0, Vec3::ZERO),
                clip: Some((AnimationNodeIndex::new(1), 2.0)),
                armed_at: 0.0,
                window_hi: f32::INFINITY,
                anim_id: Some(0),
                active: false,
                parked_at: 0.0,
            },
            LazyRig {
                bones: 1,
                ibp,
                parts: vec![part],
            },
        ))
        .id();
    let skeleton = wrathplus_assets::ModelSkeleton {
        joints: vec![wrathplus_assets::ModelJoint {
            parent: -1,
            local_translation: Vec3::ZERO,
            billboard: None,
            parent_arm: None,
        }],
        spine_bone: None,
        head_bone: None,
    };
    let pose = crate::rig_anim::RigPose::new(host, &skeleton);
    app.world_mut().entity_mut(host).insert(pose);
    (host, part)
}

fn rig_slot(app: &App, host: Entity) -> Option<u16> {
    app.world().entity(host).get::<RigSkin>().map(|r| r.slot)
}

fn part_state(app: &mut App, part: Entity) -> (Handle<Mesh>, u16) {
    let e = app.world().entity(part);
    (
        e.get::<Mesh3d>().unwrap().0.clone(),
        crate::mesh_tag::rig_of(e.get::<MeshTag>().unwrap().0),
    )
}

/// The production gate must not reuse a stationary-camera verdict for a moving WMO parent.
#[test]
fn moving_parent_bounds_promote_park_and_resume_without_double_placement() {
    use super::super::ParentSpaceAnimBound;
    let mut app = app();
    let origin = Vec3::new(10000.0, 200.0, 8000.0);
    let seat = Transform::from_translation(origin);
    let projection = bevy::camera::Projection::from(bevy::camera::PerspectiveProjection {
        far: 5000.0,
        ..default()
    });
    let frustum = bevy::camera::primitives::Frustum::from_clip_from_world(
        &(projection.get_clip_from_view() * GlobalTransform::from(seat).affine().inverse()),
    );
    app.world_mut().spawn((
        crate::view::WorldCamera,
        seat,
        GlobalTransform::from(seat),
        projection,
        frustum,
    ));
    let parent = app
        .world_mut()
        .spawn(GlobalTransform::from_translation(origin))
        .id();
    let (host, part) = lazy_host(&mut app, true);
    // Placement is included once, in the parent frame, NOT host/model local space.
    let local = Vec3::new(0.0, 0.0, -30.0);
    app.world_mut().entity_mut(host).insert((
        Transform::from_translation(local),
        GlobalTransform::from_translation(origin + local),
        ChildOf(parent),
        ParentSpaceAnimBound {
            parent,
            center: local,
            radius: 2.0,
        },
    ));
    let (stat, _) = lazy_host(&mut app, true);
    app.world_mut()
        .entity_mut(stat)
        .get_mut::<DoodadAnimHost>()
        .unwrap()
        .fade = crate::particles::EmitterFade::sphere(2.0, origin + local);
    // Even a nonidentity static host transform must NOT transform its world sphere again.
    app.world_mut()
        .entity_mut(stat)
        .insert(GlobalTransform::from_translation(origin));
    let active = |app: &App, e| app.world().get::<DoodadAnimHost>(e).unwrap().active;
    app.update();
    assert!(
        active(&app, host),
        "far-from-origin parent is in view; local origin is not"
    );
    assert!(
        rig_slot(&app, host).is_none(),
        "first drawn frame must not promote"
    );
    app.update();
    let slot = rig_slot(&app, host).expect("second drawn frame promotes");
    assert_eq!(part_state(&mut app, part).1, slot);
    assert!(active(&app, stat), "static world sphere is unchanged");
    // Unchanged camera, scene, visibility and bounds.
    app.update();
    // Rotate parent 180 degrees: local -Z goes behind the FIXED camera.
    *app.world_mut().get_mut::<GlobalTransform>(parent).unwrap() = GlobalTransform::from(
        Transform::from_translation(origin)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
    );
    app.update();
    assert!(
        !active(&app, host),
        "parent rotation invalidates cached drawn verdict"
    );
    assert!(app
        .world()
        .get::<crate::rig_anim::AnimParked>(host)
        .is_some());
    assert_eq!(rig_slot(&app, host), Some(slot), "parking retains palette");
    assert!(
        active(&app, stat),
        "moving parent cannot disturb static world bounds"
    );
    app.update();
    // Translate parent back in front, with nonuniform parent scale. Placement scale has
    // already been included in the stored sphere and must not be applied twice.
    *app.world_mut().get_mut::<GlobalTransform>(parent).unwrap() = GlobalTransform::from(
        Transform::from_translation(origin + Vec3::new(0.0, 0.0, -10.0))
            .with_scale(Vec3::new(2.0, 3.0, 1.0)),
    );
    app.update();
    assert!(
        active(&app, host),
        "parent motion invalidates cached parked verdict"
    );
    assert!(app
        .world()
        .get::<crate::rig_anim::AnimParked>(host)
        .is_none());
    assert_eq!(rig_slot(&app, host), Some(slot));
    // Part visibility remains authoritative; the mover lane is NOT a visibility bypass.
    *app.world_mut().get_mut::<Visibility>(part).unwrap() = Visibility::Hidden;
    app.update();
    assert!(!active(&app, host));
    assert!(active(&app, stat));
}

#[test]
fn a_host_rigs_at_first_wake_not_at_spawn() {
    let mut app = app();
    let (host, part) = lazy_host(&mut app, false);
    app.update();
    assert_eq!(rig_slot(&app, host), None, "hidden ⇒ no slot claimed");
    assert_eq!(
        app.world().resource::<RigPalettes>().occupancy().0,
        0,
        "the table is untouched while the host is parked"
    );

    *app.world_mut()
        .entity_mut(part)
        .get_mut::<Visibility>()
        .unwrap() = Visibility::Inherited;
    app.update();
    assert_eq!(
        rig_slot(&app, host),
        None,
        "one drawn frame is the spawn-race lie — no slot yet"
    );
    app.update();
    let slot = rig_slot(&app, host).expect("second drawn frame ⇒ slot");
    let (mesh, tag_rig) = part_state(&mut app, part);
    let twin = app
        .world()
        .entity(part)
        .get::<SkinnedTwin>()
        .unwrap()
        .skinned
        .clone();
    assert_eq!(mesh, twin, "the part swapped to its skinned twin");
    assert_eq!(tag_rig, slot, "the tag's rig field names the new slot");
    assert_eq!(
        app.world().resource::<RigPalettes>().computed_rigs(),
        1,
        "the promote seeded the rows — never a zeroed (origin-collapsed) first frame"
    );
}

#[test]
fn a_denied_wake_retries_until_the_table_has_room() {
    let mut app = app();
    // Fill all 2047 slots with one-bone rigs, held so they stay live.
    let hoard: Vec<RigSkin> = {
        let mut palettes = app.world_mut().resource_mut::<RigPalettes>();
        std::iter::from_fn(|| RigSkin::allocate_bones(&mut palettes, 1, Handle::default()))
            .collect()
    };
    assert_eq!(app.world().resource::<RigPalettes>().slot_headroom(), 0);

    let (host, _part) = lazy_host(&mut app, true);
    app.update(); // frame 1: the spawn-race frame never promotes
    app.update(); // frame 2: drawn, denied by the full table
    assert_eq!(
        rig_slot(&app, host),
        None,
        "full table ⇒ denied, static mesh"
    );

    let freed = hoard[0].slot;
    app.world_mut().resource_mut::<RigPalettes>().free(freed);
    app.update();
    assert!(rig_slot(&app, host).is_some(), "room ⇒ the retry lands");
}

#[test]
fn pressure_reaps_the_parked_and_the_next_wake_re_rigs() {
    let mut app = app();
    let (host, part) = lazy_host(&mut app, true);
    app.update();
    app.update(); // the second drawn frame promotes
    assert!(rig_slot(&app, host).is_some(), "drawn ⇒ rigged");

    // Park and age past the hysteresis: with headroom, the slot stays.
    *app.world_mut()
        .entity_mut(part)
        .get_mut::<Visibility>()
        .unwrap() = Visibility::Hidden;
    app.update();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(
            REAP_MIN_PARKED_SECS + 1.0,
        ));
    app.update();
    assert!(
        rig_slot(&app, host).is_some(),
        "parked with headroom ⇒ the slot is kept (zero churn)"
    );

    // Choke the table below the low-water mark.
    let _hoard: Vec<RigSkin> = {
        let mut palettes = app.world_mut().resource_mut::<RigPalettes>();
        (0..(crate::mesh_tag::MAX_RIG_SLOTS - 1 - REAP_LOW_WATER))
            .filter_map(|_| RigSkin::allocate_bones(&mut palettes, 1, Handle::default()))
            .collect()
    };
    app.update();
    app.update(); // the queued removal applies
    assert_eq!(
        rig_slot(&app, host),
        None,
        "pressure ⇒ the parked slot is reaped"
    );
    let (mesh, tag_rig) = part_state(&mut app, part);
    assert_eq!(
        mesh,
        app.world()
            .entity(part)
            .get::<SkinnedTwin>()
            .unwrap()
            .stat
            .clone(),
        "the part demoted to its static form"
    );
    assert_eq!(tag_rig, 0, "and its tag's rig field cleared");

    // Re-wake: the reaped slot is free again, and two drawn frames re-promote.
    *app.world_mut()
        .entity_mut(part)
        .get_mut::<Visibility>()
        .unwrap() = Visibility::Inherited;
    app.update();
    app.update();
    assert!(rig_slot(&app, host).is_some(), "the next wake re-rigs");
}

#[test]
fn a_host_without_skinned_parts_never_takes_a_slot() {
    let mut app = app();
    let (host, _part) = lazy_host(&mut app, true);
    app.world_mut().entity_mut(host).remove::<LazyRig>();
    app.update();
    assert_eq!(rig_slot(&app, host), None);
    assert_eq!(app.world().resource::<RigPalettes>().occupancy().0, 0);
}
