//! The portable state-directory naming contract shared by the client and Manager.
use std::path::{Path, PathBuf};

/// The state folder's name.
const NAME: &str = "wrathplus-config";
/// Its name before WrathPlus; a home still under it is moved on first use.
const LEGACY: &str = "benilla-config";

/// Select the whole home under `root`, `wrathplus-config`.
/// A home still under the old name, `benilla-config`, is renamed to `wrathplus-config` once, and
/// the absolute paths Manager stored in its JSON files are rewritten to match. The old home is
/// used as it is, unmoved, when that is not safe or not possible: a `wrathplus-config` already
/// exists beside it (a partial new home must not hide settings, AddOns, login state or Manager
/// JSON), the old name is a symlink, or the rename fails. Unreadable entries and dangling
/// symlinks count as present, so writes fail rather than reset.
/// The caller selects the resident root (primary checkout for dev, executable dir for players)
/// and handles explicit overrides and hermetic runs before calling this function.
pub fn default_state_home(root: &Path) -> PathBuf {
    let home = root.join(NAME);
    let legacy = root.join(LEGACY);
    let legacy_dir = match std::fs::symlink_metadata(&legacy) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return home,
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    };
    let home_absent = matches!(std::fs::symlink_metadata(&home),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    if legacy_dir && home_absent && migrate(&legacy, &home).is_ok() {
        return home;
    }
    legacy
}

/// Rename the old home, then point the paths inside its JSON files at the new name.
fn migrate(legacy: &Path, home: &Path) -> std::io::Result<()> {
    // Recorded paths may be the canonical ones (Manager resolves them) or as given.
    let mut pairs = vec![(legacy.to_path_buf(), home.to_path_buf())];
    if let Ok(real) = legacy.canonicalize() {
        let new = real.with_file_name(NAME);
        if real != legacy {
            pairs.push((real, new));
        }
    }
    std::fs::rename(legacy, home)?;
    rewrite_json(home, &pairs, 0);
    Ok(())
}

/// Best effort: a file that cannot be rewritten keeps the old path, which then no longer
/// resolves, as if that source had been removed.
fn rewrite_json(dir: &Path, pairs: &[(PathBuf, PathBuf)], depth: u32) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth < 4 && entry.file_name() != "AddOns" {
            rewrite_json(&path, pairs, depth + 1);
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "json") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut out = text.clone();
            for (old, new) in pairs {
                // As written raw, and as JSON escapes it (backslashes doubled on Windows).
                let (old, new) = (old.to_string_lossy(), new.to_string_lossy());
                out = out.replace(&*old, &new);
                out = out.replace(&old.replace('\\', "\\\\"), &new.replace('\\', "\\\\"));
            }
            if out != text {
                let tmp = path.with_extension("json.migrating");
                if std::fs::write(&tmp, out).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("wrathplus-home-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn dangling_legacy_link_does_not_silently_reset_state() {
        let root = root("link");
        let legacy = root.join(LEGACY);
        std::os::unix::fs::symlink(root.join("missing"), &legacy).unwrap();
        assert_eq!(default_state_home(&root), legacy);
        assert!(!root.join(NAME).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fresh_roots_use_the_name_and_an_old_home_moves_with_its_paths() {
        let root = root("move");
        let new = root.join(NAME);
        let old = root.join(LEGACY);
        assert_eq!(default_state_home(&root), new);
        assert!(!new.exists(), "resolving never creates the home");
        std::fs::create_dir_all(old.join("AddOns/Demo")).unwrap();
        std::fs::create_dir_all(old.join("managed-module-sources")).unwrap();
        std::fs::write(old.join("account"), "account").unwrap();
        let source = old.join("managed-module-sources/github/a/mod-x/source");
        let catalog = format!("{{\"path\":{}}}", serde_json_string(&source));
        std::fs::write(old.join("managed-module-sources/catalog.json"), &catalog).unwrap();
        std::fs::write(old.join("AddOns/Demo/x.json"), &catalog).unwrap();

        assert_eq!(default_state_home(&root), new);
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("account")).unwrap(),
            "account"
        );
        let moved = new.join("managed-module-sources/github/a/mod-x/source");
        assert_eq!(
            std::fs::read_to_string(new.join("managed-module-sources/catalog.json")).unwrap(),
            format!("{{\"path\":{}}}", serde_json_string(&moved))
        );
        // AddOns hold the player's own files: never rewritten.
        assert_eq!(
            std::fs::read_to_string(new.join("AddOns/Demo/x.json")).unwrap(),
            catalog
        );
        // Settled: the next run just uses it.
        assert_eq!(default_state_home(&root), new);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn with_both_names_present_the_old_home_wins_unmoved() {
        let root = root("both");
        let new = root.join(NAME);
        let old = root.join(LEGACY);
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("manager.json"), "{}").unwrap();
        std::fs::create_dir(&new).unwrap();
        assert_eq!(default_state_home(&root), old);
        assert!(old.join("manager.json").is_file());
        assert_eq!(std::fs::read_dir(&new).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A path as a JSON string, the way serde writes it.
    fn serde_json_string(path: &Path) -> String {
        format!("\"{}\"", path.to_string_lossy().replace('\\', "\\\\"))
    }
}
