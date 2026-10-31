//! Stop only a retained Child handle, never an executable name or a stored PID.
use super::*;

pub(crate) fn stop(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .map_err(|e| io_error("poll client before stop", e))?
        .is_none()
    {
        child
            .kill()
            .map_err(|e| io_error("stop managed client", e))?;
    }
    // The regular poll reaps and releases the launch-time running marker after exit.
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn stop_terminates_only_retained_child_and_reap_releases_marker() {
        let fixture = super::super::tests::Fixture::new();
        let config = &fixture.config;
        for marker in running_paths(config) {
            fs::create_dir_all(marker.parent().unwrap()).unwrap();
        }
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        for marker in running_paths(config) {
            fs::write(marker, child.id().to_string()).unwrap();
        }
        stop(&mut child).unwrap();
        child.wait().unwrap();
        assert!(reap(config, &mut child).unwrap());
        for marker in running_paths(config) {
            assert!(!marker.exists());
        }
        // Already-exited children do not cause a second signal.
        stop(&mut child).unwrap();
    }
}
