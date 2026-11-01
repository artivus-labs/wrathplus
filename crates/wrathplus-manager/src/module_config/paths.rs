use super::*;
use crate::setup;
use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

/// Reject links in every existing component, including dangling links and special files.
/// Like other Manager path guards this assumes no hostile concurrent directory replacement.
pub(super) fn plain(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err("Empty config path".into());
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|_| "Cannot resolve config path")?
            .join(path)
    };
    let mut cursor = PathBuf::new();
    for part in path.components() {
        if matches!(part, Component::ParentDir) {
            return Err("Config traversal rejected".into());
        }
        if matches!(part, Component::CurDir) {
            continue;
        }
        cursor.push(part);
        // A Windows drive/verbatim prefix is queryable only after its root is appended.
        // Do not resolve links: every rooted existing component is still inspected below.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&cursor) {
            Ok(m) if m.file_type().is_symlink() || (!m.is_dir() && !m.is_file()) => {
                return Err("Config paths must not contain links or special files".into())
            }
            Ok(m) if cursor != path && !m.is_dir() => {
                return Err("Config parent is not a directory".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect config path".into()),
        }
    }
    Ok(path)
}

/// Output is a direct .conf child of the reserved managed-instance config directory.
pub(crate) fn validate(config: &Config, core: &Path, path: &Path) -> Result<PathBuf> {
    let root = plain(&directory(config))?;
    let path = plain(path)?;
    if path.parent() != Some(root.as_path()) || path.extension().is_none_or(|s| s != "conf") {
        return Err("Only direct .conf files in module-config are editable".into());
    }
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    setup::check_output(&guard, &root)?;
    let mut protected = Vec::new();
    if !core.as_os_str().is_empty() {
        protected.push(setup::resolved(core)?);
    }
    let settings = config.state_root().join("manager-server.json");
    plain(&settings)?;
    match fs::File::open(&settings) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(65537)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read server path guards")?;
            if bytes.len() > 65536 {
                return Err("Server path guards exceed size limit".into());
            }
            let doc: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| "Cannot decode server path guards")?;
            for key in ["source", "data"] {
                if let Some(value) = doc[key].as_str().filter(|s| !s.is_empty()) {
                    protected.push(setup::resolved(Path::new(value))?);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot read server path guards".into()),
    }
    for input in protected {
        if root.starts_with(&input) || input.starts_with(&root) {
            return Err("Config directory overlaps server source or assets".into());
        }
    }
    Ok(path)
}

pub(super) fn list(dir: &Path, suffix: &str) -> Result<Vec<PathBuf>> {
    plain(dir)?;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Cannot list config directory".into()),
    };
    let mut paths = Vec::new();
    for (count, entry) in entries.enumerate() {
        if count >= ENTRIES {
            return Err("Config directory exceeds 512 entries".into());
        }
        let entry = entry.map_err(|_| "Cannot inspect config entry")?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(suffix))
        {
            plain(&path)?;
            if !entry
                .file_type()
                .map_err(|_| "Cannot inspect config type")?
                .is_file()
            {
                return Err("Config entry is not a regular file".into());
            }
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}
