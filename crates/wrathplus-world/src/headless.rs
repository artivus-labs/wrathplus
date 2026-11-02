//! `WOW_HEADLESS=1`: the client runs with no OS window (no Dock icon, nothing on screen, no audio
//! device), renders every frame into an offscreen image, and paces itself on a timer instead of
//! the window's event loop. For agent clients (issue #61): many can run on one machine, and the
//! agent control port reads their frames from the image.
//!
//! The `Window` entity stays, so everything that reads the primary window's size, scale or
//! cursor works unchanged; only winit, which would open the real window, is left out. Cameras
//! aimed at the primary window are re-aimed at [`HeadlessFrame`].
//!
//! | Env | Meaning |
//! |---|---|
//! | `WOW_HEADLESS=1` | headless mode |
//! | `WOW_WIN=WxH` | frame size in pixels (default 1280×720) |
//! | `WOW_HEADLESS_FPS=N` | frames a second (default 30, 1–240) |

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::{CameraUpdateSystems, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::window::{PrimaryWindow, WindowRef, WindowResolution};
use std::sync::OnceLock;
use std::time::Duration;

/// Whether this run is headless (`WOW_HEADLESS=1`), read once.
pub fn active() -> bool {
    static ACTIVE: OnceLock<bool> = OnceLock::new();
    *ACTIVE.get_or_init(|| {
        matches!(
            std::env::var("WOW_HEADLESS").as_deref(),
            Ok("1" | "true" | "yes" | "on")
        )
    })
}

/// The frame size: `WOW_WIN=WxH`, else 1280×720.
pub fn size() -> UVec2 {
    parse_size(std::env::var("WOW_WIN").ok().as_deref()).unwrap_or(UVec2::new(1280, 720))
}

fn parse_size(value: Option<&str>) -> Option<UVec2> {
    let (w, h) = value?.trim().split_once(['x', 'X'])?;
    let size = UVec2::new(w.parse().ok()?, h.parse().ok()?);
    (size.x >= 64 && size.y >= 64 && size.x <= 8192 && size.y <= 8192).then_some(size)
}

/// The frame pacing: `WOW_HEADLESS_FPS`, else 30 frames a second.
pub fn frame_interval() -> Duration {
    let fps = std::env::var("WOW_HEADLESS_FPS")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|f| (1.0..=240.0).contains(f))
        .unwrap_or(30.0);
    Duration::from_secs_f64(1.0 / fps)
}

/// The primary window a headless run keeps as an entity: the frame size at scale 1, so the UI's
/// logical pixels are the image's pixels.
pub fn window(mut window: Window) -> Window {
    let size = size();
    window.resolution = WindowResolution::new(size.x, size.y).with_scale_factor_override(1.0);
    window.mode = bevy::window::WindowMode::Windowed;
    window.focused = true;
    window
}

/// The image every primary-window camera renders into in a headless run.
#[derive(Resource, Clone)]
pub struct HeadlessFrame(pub Handle<Image>);

/// The offscreen frame and the frame timer; added by the boot plugins only when [`active`].
pub struct HeadlessPlugin;

impl Plugin for HeadlessPlugin {
    fn build(&self, app: &mut App) {
        let size = size();
        let mut image =
            Image::new_target_texture(size.x, size.y, TextureFormat::bevy_default(), None);
        // Screenshots read the frame back.
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(HeadlessFrame(handle))
            .add_plugins(ScheduleRunnerPlugin::run_loop(frame_interval()))
            .add_systems(PostUpdate, retarget.before(CameraUpdateSystems));
        info!(
            "headless: no window; rendering {}x{} offscreen at {:.0} fps",
            size.x,
            size.y,
            1.0 / frame_interval().as_secs_f64()
        );
    }
}

/// Re-aim cameras that draw to the primary window at the offscreen frame.
fn retarget(
    frame: Res<HeadlessFrame>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut cameras: Query<&mut RenderTarget, With<Camera>>,
) {
    let primary = primary.single().ok();
    for mut target in &mut cameras {
        let to_window = match &*target {
            RenderTarget::Window(WindowRef::Primary) => true,
            RenderTarget::Window(WindowRef::Entity(e)) => Some(*e) == primary,
            _ => false,
        };
        if to_window {
            *target = RenderTarget::Image(frame.0.clone().into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_and_reject_nonsense() {
        assert_eq!(parse_size(Some("800x600")), Some(UVec2::new(800, 600)));
        assert_eq!(
            parse_size(Some(" 1920X1080 ")),
            Some(UVec2::new(1920, 1080))
        );
        assert_eq!(parse_size(Some("10x10")), None);
        assert_eq!(parse_size(Some("wide")), None);
        assert_eq!(parse_size(None), None);
    }

    #[test]
    fn window_cameras_are_re_aimed_and_others_left_alone() {
        let mut app = App::new();
        let frame = Handle::<Image>::default();
        app.insert_resource(HeadlessFrame(frame.clone()))
            .add_systems(Update, retarget);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let primary = app
            .world_mut()
            .spawn((Camera::default(), RenderTarget::Window(WindowRef::Primary)))
            .id();
        let by_entity = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let atlas = app
            .world_mut()
            .spawn((Camera::default(), RenderTarget::None { size: UVec2::ONE }))
            .id();
        app.update();
        for camera in [primary, by_entity] {
            assert!(matches!(
                app.world().get::<RenderTarget>(camera),
                Some(RenderTarget::Image(target)) if target.handle == frame
            ));
        }
        assert!(matches!(
            app.world().get::<RenderTarget>(atlas),
            Some(RenderTarget::None { .. })
        ));
    }
}
