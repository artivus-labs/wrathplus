//! The frames: the camera a capture carries, its pixel rays, and the frame numbering.

use super::*;
use bevy::camera::{CameraProjection, RenderTargetInfo};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
use wrathplus_assets::coords::wow_to_bevy;
use wrathplus_world::view::CAM_FOVY;

/// The world camera at `eye` (world coordinates) looking at `yaw` (as `AgentPosition().facing`)
/// and `pitch` (up), with the client's projection over a `size` viewport, as `camera_system`
/// computes it.
pub(crate) fn view_at(eye: [f32; 3], yaw: f32, pitch: f32, size: UVec2) -> View {
    let projection = PerspectiveProjection {
        fov: CAM_FOVY,
        aspect_ratio: size.x as f32 / size.y as f32,
        near: 0.1,
        ..default()
    };
    let mut camera = Camera::default();
    camera.computed.clip_from_view = projection.get_clip_from_view();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: size,
        scale_factor: 1.0,
    });
    // Bevy yaw turns -Z toward -X, which is WoW's counter-clockwise from north.
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    View {
        camera,
        transform: GlobalTransform::from(
            Transform::from_translation(wow_to_bevy(eye)).with_rotation(rotation),
        ),
        fov_v: CAM_FOVY,
    }
}

pub(crate) const SIZE: UVec2 = UVec2::new(1280, 720);

fn frame(view: Option<View>) -> Frame {
    Frame {
        id: 0,
        time: 1.5,
        width: 640,
        height: 360,
        pose: ShotPose { view, body: None },
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn the_camera_reads_as_world_yaw_and_pitch() {
    let north = view_at([0.0, 0.0, 10.0], 0.0, 0.0, SIZE);
    assert_eq!(north.yaw_pitch(), (0.0, 0.0));
    let (yaw, pitch) = view_at([0.0, 0.0, 10.0], FRAC_PI_2, -FRAC_PI_4, SIZE).yaw_pitch();
    // West is a quarter turn counter-clockwise, as `GetPlayerFacing` counts it.
    assert!(
        close(yaw, FRAC_PI_2) && close(pitch, -FRAC_PI_4),
        "{yaw} {pitch}"
    );
    // 45 degrees tall at 16:9 is about 73 wide.
    assert!(close(
        north.fov_h(),
        2.0 * ((CAM_FOVY / 2.0).tan() * 16.0 / 9.0).atan()
    ));
    assert!((north.fov_h().to_degrees() - 72.73).abs() < 0.01);
}

/// The image's centre is the camera's forward, its edges are half the field of view off it, and
/// a pixel right of centre turns clockwise (the yaw falls), up the image the pitch rises. The
/// image may be scaled: its fractions are the viewport's.
#[test]
fn a_pixel_unprojects_through_the_frames_camera() {
    let view = view_at([100.0, 50.0, 20.0], 1.0, 0.0, SIZE);
    let centre = view.ray(320.0, 180.0, 640, 360).unwrap();
    assert!(centre.direction.dot(*view.transform.forward()) > 0.999_999);
    assert!(centre.origin.distance(view.transform.translation()) < 0.2);
    let (yaw, _) = yaw_pitch(*view.ray(640.0, 180.0, 640, 360).unwrap().direction);
    assert!(close(yaw, 1.0 - view.fov_h() / 2.0), "{yaw}");
    let (yaw, _) = yaw_pitch(*view.ray(0.0, 180.0, 640, 360).unwrap().direction);
    assert!(close(yaw, 1.0 + view.fov_h() / 2.0), "{yaw}");
    let (_, pitch) = yaw_pitch(*view.ray(320.0, 0.0, 640, 360).unwrap().direction);
    assert!(close(pitch, CAM_FOVY / 2.0), "{pitch}");
    // The full-size image's pixel and the half-size one's are the same ray.
    let full = view.ray(960.0, 200.0, 1280, 720).unwrap();
    let half = view.ray(480.0, 100.0, 640, 360).unwrap();
    assert!(full.direction.dot(*half.direction) > 0.999_999);
}

#[test]
fn frames_are_numbered_kept_sixteen_deep_and_found_by_number() {
    let mut frames = Frames::default();
    assert!(frames.get(None).unwrap_err().contains("no screenshot yet"));
    for n in 1..=20u64 {
        assert_eq!(frames.add(frame(None)), n);
    }
    assert_eq!(frames.get(None).unwrap().id, 20);
    assert_eq!(frames.get(Some(5)).unwrap().id, 5);
    assert!(frames.get(Some(4)).unwrap_err().contains("last 16"));
    assert!(frames.get(Some(21)).is_err());
}

#[test]
fn the_frame_header_has_the_camera_and_the_player_in_world_terms() {
    let mut f = frame(Some(view_at([1.0, 2.0, 3.0], 0.5, -0.25, SIZE)));
    f.id = 7;
    f.pose.body = Some(Body {
        pos: [4.0, 5.0, 6.0],
        facing: 0.5,
        map: 0,
    });
    let meta = f.meta();
    assert_eq!(meta["frame"], 7);
    assert_eq!(
        (meta["width"].as_u64(), meta["height"].as_u64()),
        (Some(640), Some(360))
    );
    let camera = &meta["camera"];
    assert_eq!(
        (
            camera["x"].as_f64(),
            camera["y"].as_f64(),
            camera["z"].as_f64()
        ),
        (Some(1.0), Some(2.0), Some(3.0))
    );
    assert_eq!(
        (camera["yaw"].as_f64(), camera["pitch"].as_f64()),
        (Some(0.5), Some(-0.25))
    );
    // Four places of 45 degrees.
    assert_eq!(
        camera["fov_v"].as_f64(),
        Some((f64::from(FRAC_PI_4) * 1e4).round() / 1e4)
    );
    assert_eq!(meta["player"]["facing"].as_f64(), Some(0.5));
    assert_eq!(frame(None).meta()["camera"], Value::Null);
}

#[test]
fn scaled_sizes_match_the_encoder() {
    assert_eq!(scaled(1280, 720, 1.0), (1280, 720));
    assert_eq!(scaled(1280, 720, 0.5), (640, 360));
    assert_eq!(scaled(1281, 721, 0.5), (641, 361));
    assert_eq!(scaled(100, 100, 0.001), (1, 1));
}

#[derive(Resource)]
struct Spawned(Entity);

/// A camera that moves in `Update` the frame a capture is asked for is stamped where it ends that
/// frame, not where it was when the capture was asked for: the render reads the final pose.
#[test]
fn a_capture_is_stamped_with_the_pose_its_frame_renders() {
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin);
    add_stamp(&mut app);
    let view = view_at([0.0, 0.0, 0.0], 0.0, 0.0, SIZE);
    app.world_mut().spawn((
        view.camera,
        Transform::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        WorldCamera,
    ));
    app.insert_resource(State::new(ClientState::InWorld));
    app.insert_resource(wrathplus_app_core::app_core::player::Player::default());
    app.add_systems(
        Update,
        |mut commands: Commands, mut cam: Query<&mut Transform, With<WorldCamera>>| {
            let shot = commands.spawn(Screenshot::image(Handle::default())).id();
            commands.insert_resource(Spawned(shot));
            cam.single_mut().unwrap().translation = wow_to_bevy([10.0, 20.0, 30.0]);
        },
    );
    app.update();
    let shot = app.world().resource::<Spawned>().0;
    let pose = app.world().get::<ShotPose>(shot).expect("stamped");
    let view = pose.view.as_ref().expect("world camera");
    assert_eq!(
        bevy_to_wow(view.transform.translation()),
        [10.0, 20.0, 30.0]
    );
    assert!(view.camera.logical_viewport_size().is_some());
    assert_eq!(pose.body.map(|b| b.map), Some(0));
}
