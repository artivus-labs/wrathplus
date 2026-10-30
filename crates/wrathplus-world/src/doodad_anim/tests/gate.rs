use super::*;

#[test]
fn gate_pauses_hidden_and_resumes_on_the_shared_clock() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    // The gate's params must resolve, though this meshed host never reads these.
    app.init_resource::<crate::view::ViewDistance>();
    app.init_resource::<crate::wmo_portal::ExteriorWindows>();
    app.init_resource::<crate::wmo_portal::CameraInteriorClaim>();
    app.init_resource::<crate::rig_palette::RigPalettes>();
    app.init_asset::<SkinnedMeshInverseBindposes>();
    app.add_systems(Update, gate_doodad_anim);

    let mesh = app.world_mut().spawn(Visibility::Inherited).id();
    let node = AnimationNodeIndex::new(1);
    let mut player = AnimationPlayer::default();
    player.play(node).repeat();
    let joint = app.world_mut().spawn(Transform::default()).id();
    let drive = GlobalSeqDrive::new(
        &anims(Vec::new(), None, true).global_bones,
        &[Entity::PLACEHOLDER, joint],
    )
    .expect("one gseq bone maps");
    let host = app
        .world_mut()
        .spawn((
            DoodadAnimHost {
                meshes: vec![mesh],
                fade: crate::particles::EmitterFade::sphere(1.0, Vec3::ZERO),
                clip: Some((node, 2.0)),
                armed_at: 0.0,
                // Far future: no window may expire and change the armed clip mid-test.
                window_hi: f32::INFINITY,
                anim_id: Some(0),
                active: true,
                parked_at: 0.0,
            },
            player,
            drive,
        ))
        .id();

    app.update();
    let playing = |app: &mut App, e: Entity| {
        app.world_mut()
            .entity(e)
            .get::<AnimationPlayer>()
            .unwrap()
            .playing_animations()
            .count()
    };
    assert_eq!(playing(&mut app, host), 1, "drawn ⇒ playing");

    *app.world_mut()
        .entity_mut(mesh)
        .get_mut::<Visibility>()
        .unwrap() = Visibility::Hidden;
    app.update();
    assert_eq!(playing(&mut app, host), 0, "hidden ⇒ stopped");
    assert!(
        !app.world()
            .entity(host)
            .get::<DoodadAnimHost>()
            .unwrap()
            .active
    );

    *app.world_mut()
        .entity_mut(mesh)
        .get_mut::<Visibility>()
        .unwrap() = Visibility::Inherited;
    app.update();
    assert_eq!(playing(&mut app, host), 1, "re-drawn ⇒ re-armed");
    let world = app.world();
    let p = world.entity(host).get::<AnimationPlayer>().unwrap();
    let active = p.animation(node).expect("the first-seq node is active");
    assert!(
        active.seek_time() >= 0.0 && active.seek_time() < 2.0,
        "seek lands inside the loop: {}",
        active.seek_time()
    );
}

/// The control is a map doodad at the same spot with no building, which the sealed room stops.
#[test]
fn a_sealed_room_keeps_its_own_meshless_props_animating() {
    use crate::wmo_portal::{
        CameraInteriorClaim, ExteriorWindows, InteriorClaim, WmoGroupVis, WmoPortalInstance,
        WmoRoom,
    };
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_resource::<crate::view::ViewDistance>();
    app.init_resource::<crate::rig_palette::RigPalettes>();
    app.init_asset::<SkinnedMeshInverseBindposes>();
    app.add_systems(Update, gate_doodad_anim);

    // The camera stands inside a sealed room of `building`, with no exterior window.
    let building = app
        .world_mut()
        .spawn(WmoPortalInstance {
            handle: Handle::default(),
            world_from_local: bevy::math::Affine3A::IDENTITY,
            name_set: 0,
            visible: vec![true],
            interior_fog: vec![false],
            liquid_visited: vec![false],
            flooded: vec![None],
        })
        .id();
    app.insert_resource(ExteriorWindows::Windows(Vec::new()));
    app.insert_resource(CameraInteriorClaim(Some(InteriorClaim {
        room: WmoRoom {
            instance: building,
            group: 0,
        },
        exterior_visible: false,
    })));

    // Looking straight at both hosts, 20 yd out and well inside the wall.
    let seat = Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Vec3::new(0.0, 0.0, -20.0), Vec3::Y);
    let projection = bevy::camera::Projection::from(bevy::camera::PerspectiveProjection {
        far: 5000.0,
        ..default()
    });
    let frustum = bevy::camera::primitives::Frustum::from_clip_from_world(
        &(projection.get_clip_from_view() * GlobalTransform::from(seat).affine().inverse()),
    );
    app.world_mut().spawn((
        crate::view::WorldCamera,
        GlobalTransform::from(seat),
        seat,
        frustum,
        projection,
    ));

    let node = AnimationNodeIndex::new(1);
    let mut meshless = |fade: crate::particles::EmitterFade| {
        app.world_mut()
            .spawn(DoodadAnimHost {
                meshes: Vec::new(), // 0 render batches
                fade,
                clip: Some((node, 2.0)),
                armed_at: 0.0,
                window_hi: f32::INFINITY,
                anim_id: Some(0),
                active: false,
                parked_at: 0.0,
            })
            .id()
    };
    let center = Vec3::new(0.0, 0.0, -20.0);
    let prop = meshless(crate::particles::EmitterFade {
        instance: Some(building),
        room: Some(WmoGroupVis {
            instance: building,
            groups: [0u16].as_slice().into(),
        }),
        ..crate::particles::EmitterFade::sphere(2.0, center)
    });
    let map_doodad = meshless(crate::particles::EmitterFade::sphere(2.0, center));

    app.update();
    let active = |app: &App, e: Entity| {
        app.world()
            .entity(e)
            .get::<DoodadAnimHost>()
            .unwrap()
            .active
    };
    assert!(
        active(&app, prop),
        "a prop of the camera's OWN building is not exterior to it — it keeps animating, and \
         its $DSL keeps firing"
    );
    assert!(
        !active(&app, map_doodad),
        "the control: a map doodad carries no building, so the sealed room stops it"
    );
}

/// The gate runs before transform propagation, so on a teleport frame only the camera's local
/// has moved, and the verdict reuse must still re-judge every host.
#[test]
fn a_reseated_camera_is_not_a_still_scene() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_resource::<crate::view::ViewDistance>();
    app.init_resource::<crate::wmo_portal::ExteriorWindows>();
    app.init_resource::<crate::wmo_portal::CameraInteriorClaim>();
    app.init_resource::<crate::rig_palette::RigPalettes>();
    app.init_asset::<SkinnedMeshInverseBindposes>();
    app.add_systems(Update, gate_doodad_anim);

    // A camera 1000 yd off, looking away: not drawn. With no propagation, `GlobalTransform`
    // never follows `Transform`, as on the real snap frame.
    let far =
        Transform::from_xyz(1000.0, 0.0, 0.0).looking_at(Vec3::new(2000.0, 0.0, 0.0), Vec3::Y);
    let projection = bevy::camera::Projection::from(bevy::camera::PerspectiveProjection {
        far: 5000.0,
        ..default()
    });
    let frustum = bevy::camera::primitives::Frustum::from_clip_from_world(
        &(projection.get_clip_from_view() * GlobalTransform::from(far).affine().inverse()),
    );
    let cam = app
        .world_mut()
        .spawn((
            crate::view::WorldCamera,
            GlobalTransform::from(far),
            far,
            frustum,
            projection,
        ))
        .id();
    let node = AnimationNodeIndex::new(1);
    let mut player = AnimationPlayer::default();
    player.play(node).repeat();
    let joint = app.world_mut().spawn(Transform::default()).id();
    let drive = GlobalSeqDrive::new(
        &anims(Vec::new(), None, true).global_bones,
        &[Entity::PLACEHOLDER, joint],
    )
    .expect("one gseq bone maps");
    let host = app
        .world_mut()
        .spawn((
            DoodadAnimHost {
                meshes: Vec::new(),
                fade: crate::particles::EmitterFade::sphere(1.0, Vec3::ZERO),
                clip: Some((node, 2.0)),
                armed_at: 0.0,
                window_hi: f32::INFINITY,
                anim_id: Some(0),
                active: true,
                parked_at: 0.0,
            },
            player,
            drive,
        ))
        .id();
    let active = |app: &App| {
        app.world()
            .entity(host)
            .get::<DoodadAnimHost>()
            .unwrap()
            .active
    };
    app.update();
    assert!(!active(&app), "far and facing away ⇒ parked");
    app.update();
    app.update();
    assert!(!active(&app), "still ⇒ the verdict is reused, still parked");

    // The snap: the seat writes the local next to the host, facing it; the global stays stale.
    let near = Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
    let near_frustum = bevy::camera::primitives::Frustum::from_clip_from_world(
        &(bevy::camera::Projection::from(bevy::camera::PerspectiveProjection {
            far: 5000.0,
            ..default()
        })
        .get_clip_from_view()
            * GlobalTransform::from(near).affine().inverse()),
    );
    {
        let mut e = app.world_mut().entity_mut(cam);
        *e.get_mut::<Transform>().unwrap() = near;
        *e.get_mut::<bevy::camera::primitives::Frustum>().unwrap() = near_frustum;
        // Written without a change tick, so the local is this frame's only moved signal.
        *e.get_mut::<GlobalTransform>()
            .unwrap()
            .bypass_change_detection() = GlobalTransform::from(near);
    }
    app.update();
    assert!(active(&app), "the snap frame re-judges the host: drawn");
}

/// Bevy's frustum culling writes `ViewVisibility` only, so a meshed doodad off-screen keeps an
/// `Inherited` part; the gate judges its fade sphere against the frustum like a meshless one.
#[test]
fn a_meshed_host_outside_the_frustum_is_parked() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_resource::<crate::view::ViewDistance>();
    app.init_resource::<crate::wmo_portal::ExteriorWindows>();
    app.init_resource::<crate::wmo_portal::CameraInteriorClaim>();
    app.init_resource::<crate::rig_palette::RigPalettes>();
    app.init_asset::<SkinnedMeshInverseBindposes>();
    app.add_systems(Update, gate_doodad_anim);

    // Looking down -Z from the origin.
    let seat = Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Vec3::new(0.0, 0.0, -20.0), Vec3::Y);
    let projection = bevy::camera::Projection::from(bevy::camera::PerspectiveProjection {
        far: 5000.0,
        ..default()
    });
    let frustum = bevy::camera::primitives::Frustum::from_clip_from_world(
        &(projection.get_clip_from_view() * GlobalTransform::from(seat).affine().inverse()),
    );
    app.world_mut().spawn((
        crate::view::WorldCamera,
        GlobalTransform::from(seat),
        seat,
        frustum,
        projection,
    ));
    let mut meshed = |center: Vec3| {
        let mesh = app.world_mut().spawn(Visibility::Inherited).id();
        app.world_mut()
            .spawn(DoodadAnimHost {
                meshes: vec![mesh],
                fade: crate::particles::EmitterFade::sphere(2.0, center),
                clip: Some((AnimationNodeIndex::new(1), 2.0)),
                armed_at: 0.0,
                window_hi: f32::INFINITY,
                anim_id: Some(0),
                active: true,
                parked_at: 0.0,
            })
            .id()
    };
    let behind = meshed(Vec3::new(0.0, 0.0, 30.0));
    let ahead = meshed(Vec3::new(0.0, 0.0, -30.0));
    app.update();
    let world = app.world();
    assert!(
        world.entity(behind).contains::<AnimParked>(),
        "behind the camera ⇒ outside the frustum ⇒ parked, whatever its part's Visibility"
    );
    assert!(
        !world.entity(ahead).contains::<AnimParked>(),
        "in front of the camera ⇒ drawn"
    );
}

#[test]
fn arm_clock_wraps_on_the_shared_clock() {
    let mut host = DoodadAnimHost {
        meshes: Vec::new(),
        fade: crate::particles::EmitterFade::sphere(0.0, Vec3::ZERO),
        clip: Some((AnimationNodeIndex::new(1), 3.333)),
        armed_at: 10.0,
        window_hi: f32::NEG_INFINITY,
        anim_id: Some(0),
        // Parked, and the clock still answers: honouring the animate set is the caller's job.
        active: false,
        parked_at: 0.0,
    };

    let (_, seek) = host.arm_clock(10.0).expect("armed");
    assert!(seek.abs() < 1e-6, "at the arm it sits at t = 0");

    let (_, seek) = host.arm_clock(12.0).expect("armed");
    assert!((seek - 2.0).abs() < 1e-4, "2 s in, {seek}");

    let (_, seek) = host.arm_clock(14.0).expect("armed");
    assert!((seek - (4.0 - 3.333)).abs() < 1e-4, "wrapped to {seek}");

    host.clip = Some((AnimationNodeIndex::new(1), 0.0));
    let (_, seek) = host.arm_clock(99.0).expect("armed");
    assert!(
        seek.abs() < 1e-6,
        "a zero-length clip sits at its own t = 0"
    );

    host.clip = None;
    assert!(host.arm_clock(12.0).is_none());
}
