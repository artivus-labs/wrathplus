//! The app icon, our own art (`packaging/icon`, drawn by `packaging/icon/gen`): every window's icon
//! through winit and, on macOS, the Dock's (`NSApplication.applicationIconImage`), which a bare
//! binary otherwise shows as the generic executable. Set once, the first frame the primary window
//! exists; a headless app (no `WinitPlugin`, no window) never reaches it.

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// The 256-pixel render of the icon, embedded so the bare binary needs no file beside it.
const ICON_PNG: &[u8] = include_bytes!("../../../packaging/icon/wrathplus-icon-256.png");

pub struct AppIconPlugin;

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        // No Dock or window icon to set in a headless run, and AppKit must stay untouched.
        if wrathplus_world::headless::active() {
            return;
        }
        app.add_systems(Update, set_app_icon);
    }
}

/// Main-thread only (`NonSendMarker`): winit's `set_window_icon` and AppKit both require it.
fn set_app_icon(
    window: Query<Entity, With<PrimaryWindow>>,
    mut done: Local<bool>,
    _main: NonSendMarker,
) {
    if *done {
        return;
    }
    let Ok(entity) = window.single() else {
        return;
    };
    let rgba = match image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            warn!("app icon: the embedded PNG did not decode ({e}); keeping the default");
            *done = true;
            return;
        }
    };
    let (w, h) = rgba.dimensions();
    // The winit window lands a frame after the entity; until then, try again.
    let window_up = bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        windows.get_window(entity).map(|win| {
            match winit::window::Icon::from_rgba(rgba.into_raw(), w, h) {
                Ok(icon) => win.set_window_icon(Some(icon)),
                Err(e) => warn!("app icon: winit refused the icon ({e})"),
            }
        })
    });
    if window_up.is_none() {
        return;
    }
    #[cfg(target_os = "macos")]
    macos::set_dock_icon(ICON_PNG);
    *done = true;
    info!("app icon: set ({w}x{h})");
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    /// The Dock and app-switcher icon, which winit's window icon does not reach on macOS.
    pub(super) fn set_dock_icon(png: &[u8]) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let data = NSData::with_bytes(png);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            bevy::log::warn!("app icon: AppKit could not read the embedded PNG");
            return;
        };
        // SAFETY: on the main thread (`mtm`), with a live `NSImage` the application retains.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }
}

#[cfg(test)]
mod tests {
    /// The embedded art decodes to the square RGBA the window icon takes.
    #[test]
    fn the_embedded_icon_is_a_square_rgba_png() {
        let img = image::load_from_memory_with_format(super::ICON_PNG, image::ImageFormat::Png)
            .expect("decodes")
            .to_rgba8();
        assert_eq!(img.dimensions(), (256, 256));
        assert!(winit::window::Icon::from_rgba(img.clone().into_raw(), 256, 256).is_ok());
        // Opaque at the centre, transparent in the corner outside the rounded plate.
        assert_eq!(img.get_pixel(128, 128)[3], 255);
        assert_eq!(img.get_pixel(1, 1)[3], 0);
    }
}
