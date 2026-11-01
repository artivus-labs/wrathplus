//! macOS application identity is separate from the window title and window icon.
pub(super) const NAME: &str = "WrathPlus Manager";

pub(super) fn prepare() {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSProcessInfo, NSString};
        // SAFETY: called once on the main thread, before winit builds the application menu.
        unsafe { NSProcessInfo::processInfo().setProcessName(&NSString::from_str(NAME)) };
    }
}

pub(super) fn set_dock_icon() {
    #[cfg(target_os = "macos")]
    {
        use objc2::ClassType;
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::{MainThreadMarker, NSData};
        static SET: std::sync::Once = std::sync::Once::new();
        SET.call_once(|| {
            let mtm = MainThreadMarker::new().expect("Manager updates run on the main thread");
            let data = NSData::with_bytes(include_bytes!(
                "../../../packaging/icon/wrathplus-icon-256.png"
            ));
            let image = NSImage::initWithData(NSImage::alloc(), &data)
                .expect("embedded Manager icon must decode");
            // SAFETY: after event-loop startup on the main thread; AppKit retains the image.
            unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_application_icon_is_valid_rgba() {
        let image = image::load_from_memory(include_bytes!(
            "../../../packaging/icon/wrathplus-icon-256.png"
        ))
        .unwrap()
        .into_rgba8();
        assert_eq!(image.dimensions(), (256, 256));
        assert_eq!(image.get_pixel(128, 128)[3], 255);
        assert_eq!(image.get_pixel(1, 1)[3], 0);
    }
}
