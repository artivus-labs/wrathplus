//! Canonical output containment and configuration checks.
use super::*;
use std::path::Component;

/// Resolve every existing component (including symlinks), even for not-yet-created outputs.
/// Parent traversal is rejected instead of lexically folding it across a symlink.
pub(super) fn resolved(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err("Select an explicit, nonempty path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        env::current_dir()
            .map_err(|e| io_error("current directory", e))?
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => return Err("Paths must not contain '..'".into()),
            Component::CurDir => continue,
            other => result.push(other.as_os_str()),
        }
        match fs::symlink_metadata(&result) {
            Ok(_) => {
                result = fs::canonicalize(&result)
                    .map_err(|e| io_error(format!("resolve {}", result.display()), e))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(format!("inspect {}", result.display()), e)),
        }
    }
    Ok(result)
}

pub(super) fn protected_root(config: &Config) -> &Path {
    // A selected Data directory also protects its enclosing install (WTF, screenshots, etc.).
    if config
        .data
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Data"))
    {
        config.data.parent().unwrap_or(&config.data)
    } else {
        &config.data
    }
}

pub(super) fn writable(config: &Config, path: &Path) -> Result<PathBuf> {
    let path = resolved(path)?;
    let assets = protected_root(config);
    if path.starts_with(assets) || assets.starts_with(&path) {
        return Err(format!(
            "Output {} overlaps the read-only asset install {}",
            path.display(),
            assets.display()
        ));
    }
    Ok(path)
}

pub(super) fn checked_config(config: &Config) -> Result<Config> {
    if !matches!(
        config.locale.as_str(),
        "enUS" | "enGB" | "koKR" | "frFR" | "deDE" | "zhCN" | "zhTW" | "esES" | "esMX" | "ruRU"
    ) {
        return Err(format!("Unsupported Wrath locale {:?}", config.locale));
    }
    if config.server.is_empty()
        || config.server.starts_with('-')
        || config
            .server
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || config.server.contains(['/', '\\'])
    {
        return Err("Server must be a host[:port] without whitespace or path separators".into());
    }
    let mut config = config.clone();
    config.repo = resolved(&config.repo)?;
    config.data = resolved(&config.data)?;
    config.target = resolved(&config.target)?;
    if !config.repo.join("Cargo.toml").is_file()
        || !config.repo.join("crates/wrathplus/Cargo.toml").is_file()
    {
        return Err("Repository must contain Cargo.toml and crates/wrathplus/Cargo.toml".into());
    }
    if !config.data.is_dir() {
        return Err(format!(
            "Asset directory does not exist: {}",
            config.data.display()
        ));
    }
    writable(&config, &config.target)?;
    for path in [
        config.state_path(),
        config.binary_path(),
        config.repo.join("benilla-config"),
        config.target.join("manager"),
    ] {
        writable(&config, &path)?;
    }
    Ok(config)
}
