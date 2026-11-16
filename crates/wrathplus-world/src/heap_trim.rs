//! glibc heap trim: hand freed heap pages back to the OS on a timer. glibc returns only the top of
//! each arena on its own, so the pages streamed art frees mid-arena stay resident and RSS climbs
//! with ground covered while the live heap does not. `malloc_trim` is glibc's; a no-op elsewhere.

/// Start the trim thread, once per process: the heap is the process's, not an `App`'s.
pub(crate) fn start() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        /// Seconds between trims with `$WOW_HEAP_TRIM` unset; `WOW_HEAP_TRIM=0` turns the trim off.
        const DEFAULT_SECS: u64 = 30;
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let secs = std::env::var("WOW_HEAP_TRIM")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_SECS);
            if secs == 0 {
                bevy::log::warn!("heap-trim: OFF ($WOW_HEAP_TRIM=0), freed heap stays resident");
                return;
            }
            unsafe extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            // Its own thread: a trim holds each arena's lock in turn for tens of milliseconds.
            let spawned =
                std::thread::Builder::new()
                    .name("heap-trim".into())
                    .spawn(move || loop {
                        std::thread::sleep(std::time::Duration::from_secs(secs));
                        unsafe { malloc_trim(0) };
                    });
            match spawned {
                Ok(_) => bevy::log::info!("heap-trim: freed heap returns to the OS every {secs} s"),
                Err(why) => bevy::log::warn!("heap-trim: no thread ({why})"),
            }
        });
    }
}
