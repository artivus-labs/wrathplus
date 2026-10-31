//! Narrow local-state access for Manager addon registration.
use super::*;
/// Serialize addon registration with build/launch; never mutate a running client's namespace.
pub(crate) fn with_stopped_state<T>(
    config: &Config,
    action: impl FnOnce(&Config) -> Result<T>,
) -> Result<T> {
    let config = checked_config(config)?;
    let _lock = operation_lock(&config)?;
    require_stopped(&config)?;
    action(&config)
}
pub(crate) fn check_output(config: &Config, path: &Path) -> Result<()> {
    writable(config, path).map(|_| ())
}
pub(crate) fn write_state(config: &Config, path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(config, path, bytes)
}
