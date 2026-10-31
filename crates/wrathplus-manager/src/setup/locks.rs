//! Fence both the resolved player-state home and the managed binary target.
use super::*;

pub(super) struct OperationLock(pub(super) Vec<PathBuf>);
impl Drop for OperationLock {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}
pub(super) fn operation_lock(config: &Config) -> Result<OperationLock> {
    // State and installed binary are independent resources: fence both. Holding the state
    // lock first also serializes configurations with different Cargo target directories.
    let mut guard = OperationLock(Vec::new());
    for dir in [
        config.state_root().join("manager"),
        config.target.join("manager"),
    ] {
        let dir = ensure_dir(config, &dir)?;
        let path = writable(config, &dir.join("operation.lock"))?;
        if guard.0.contains(&path) {
            continue;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|e| io_error("Manager operation locked (after a crash, confirm no setup is running before removing operation.lock)", e))?;
        guard.0.push(path);
        writeln!(file, "{}", std::process::id())
            .map_err(|e| io_error("write operation lock", e))?;
    }
    Ok(guard)
}
pub(super) fn running_path(config: &Config) -> PathBuf {
    config.state_root().join("manager/running.lock")
}
pub(super) fn running_paths(config: &Config) -> [PathBuf; 2] {
    // Keep the target marker for older Managers and shared installed binaries.
    [
        running_path(config),
        config.target.join("manager/running.lock"),
    ]
}
pub(super) fn require_stopped(config: &Config) -> Result<()> {
    for path in running_paths(config) {
        let path = writable(config, &path)?;
        if path
            .try_exists()
            .map_err(|e| io_error("inspect running marker", e))?
        {
            return Err(format!(
                "Managed client is running or not yet reaped. After confirming it has stopped, remove stale {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "lock_tests.rs"]
mod tests;
