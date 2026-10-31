//! Only managed links are changed. Source folders and per-character enable files are read-only.
use super::*;
use crate::setup;

fn path(config: &Config) -> PathBuf {
    config.state_root().join("manager-addons.json")
}
pub(crate) fn load(config: &Config) -> Result<Settings, String> {
    match std::fs::read(path(config)) {
        Ok(bytes) if bytes.len() <= 65536 => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Read addon settings: {e}"))
        }
        Ok(_) => Err("Addon settings exceed 64 KiB".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("Read addon settings: {e}")),
    }
}
pub(super) fn target_matches(path: &Path, source: &Path) -> bool {
    std::fs::read_link(path).is_ok_and(|p| same_target(&p, source))
}
#[cfg(not(windows))]
fn same_target(target: &Path, source: &Path) -> bool {
    target == source
}
#[cfg(windows)]
fn same_target(target: &Path, source: &Path) -> bool {
    use std::path::{Component, Prefix};
    // read_link returns DOS/UNC spelling where possible; canonicalize stores verbatim paths.
    // Compare only these namespace aliases, without following links (sources may be gone).
    fn key(path: &Path) -> Option<PathBuf> {
        let mut parts = path.components();
        let Component::Prefix(prefix) = parts.next()? else {
            return None;
        };
        let mut result = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                PathBuf::from(format!(r"\\?\{}:", char::from(drive)))
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                PathBuf::from(r"\\?\UNC").join(server).join(share)
            }
            _ => return None,
        };
        if parts.next()? != Component::RootDir {
            return None;
        }
        result.push(r"\");
        for part in parts {
            let Component::Normal(name) = part else {
                return None;
            };
            // DOS trims these suffixes; they are not namespace-only spelling changes.
            if name.to_string_lossy().ends_with(['.', ' ']) {
                return None;
            }
            result.push(name);
        }
        Some(result)
    }
    target == source || matches!((key(target), key(source)), (Some(a), Some(b)) if a == b)
}
#[cfg(unix)]
fn link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}
#[cfg(windows)]
fn link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
}
#[cfg(windows)]
fn unlink(target: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(target)
}
#[cfg(not(windows))]
fn unlink(target: &Path) -> std::io::Result<()> {
    std::fs::remove_file(target)
}

pub(crate) fn save_and_apply(config: &Config, next: &Settings) -> Result<(), String> {
    setup::with_stopped_state(config, |config| apply(config, next))
}
fn apply(config: &Config, next: &Settings) -> Result<(), String> {
    let old = load(config)?;
    let bytes = serde_json::to_vec_pretty(next).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Addon settings exceed 64 KiB; remove unused source folders".into());
    }
    let root = config.state_root().join("AddOns");
    setup::check_output(config, &root)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("Create addon link directory: {e}"))?;
    // Resolve the directory itself, never the individual links (they intentionally target sources).
    setup::check_output(config, &root)?;
    let mut names = std::collections::HashSet::new();
    for item in &next.selected {
        if !safe_name(&item.name) || !names.insert(item.name.to_ascii_lowercase()) {
            return Err("Invalid or duplicate addon identity".into());
        }
        // Unavailable sources must not prevent removing other registrations one at a time.
        // Only exact prior registrations bypass manifest validation; ownership is checked below.
        if !old.selected.contains(item) {
            let entry =
                manifest(&item.source).ok_or_else(|| format!("No valid TOC for {}", item.name))?;
            if entry.name != item.name || entry.source != item.source {
                return Err("Addon source must be canonical and match its TOC name".into());
            }
        }
        if item.source.starts_with(&root) {
            return Err("An existing local addon is already available; manage its enable state in the client's AddOns screen".into());
        }
        let target = root.join(&item.name);
        if std::fs::symlink_metadata(&target).is_ok()
            && !old
                .selected
                .iter()
                .any(|s| s.name == item.name && target_matches(&target, &s.source))
        {
            return Err(format!(
                "{} already exists and is not a Manager-owned link; it was not modified",
                item.name
            ));
        }
    }
    for item in &old.selected {
        if !safe_name(&item.name) {
            return Err("Invalid saved addon identity".into());
        }
        let target = root.join(&item.name);
        if std::fs::symlink_metadata(&target).is_ok() && !target_matches(&target, &item.source) {
            return Err(format!(
                "Managed link {} changed outside Manager; refusing to replace it",
                item.name
            ));
        }
    }
    let mut removed: Vec<&Selection> = Vec::new();
    let mut created: Vec<&Selection> = Vec::new();
    let result = (|| {
        for item in &old.selected {
            if next.selected.contains(item) {
                continue;
            }
            let target = root.join(&item.name);
            if target_matches(&target, &item.source) {
                unlink(&target).map_err(|e| format!("Remove addon link: {e}"))?;
                removed.push(item);
            }
        }
        for item in &next.selected {
            let target = root.join(&item.name);
            if target_matches(&target, &item.source) {
                continue;
            }
            link(&item.source, &target).map_err(|e| {
                format!(
                    "Link addon {}: {e}. Windows may require Developer Mode or symlink permission",
                    item.name
                )
            })?;
            created.push(item);
        }

        setup::write_state(config, &path(config), &bytes)
    })();
    if let Err(error) = &result {
        let mut rollback_errors = Vec::new();
        for item in created.iter().rev() {
            if let Err(e) = unlink(&root.join(&item.name)) {
                rollback_errors.push(e.to_string());
            }
        }
        for item in &removed {
            if let Err(e) = link(&item.source, &root.join(&item.name)) {
                rollback_errors.push(e.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            return Err(format!(
                "{}; rollback errors: {}",
                error,
                rollback_errors.join("; ")
            ));
        }
    }
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn windows_target_spelling_only_equates_absolute_namespace_aliases() {
        for (dos, verbatim) in [
            (r"C:\addons\Demo", r"\\?\C:\addons\Demo"),
            (r"\\server\share\Demo", r"\\?\UNC\server\share\Demo"),
        ] {
            assert!(same_target(Path::new(dos), Path::new(verbatim)));
            assert!(same_target(Path::new(verbatim), Path::new(dos)));
        }
        for other in [
            r"C:\addons\Other",
            r"D:\addons\Demo",
            r"C:addons\Demo",
            r"addons\Demo",
            r"C:\addons\..\Demo",
            r"C:\addons\Demo.",
            r"C:\addons\Demo ",
            r"\\.\C:\addons\Demo",
            r"\\server\other\Demo",
        ] {
            assert!(
                !same_target(Path::new(other), Path::new(r"\\?\C:\addons\Demo")),
                "{other}"
            );
        }
    }
}
