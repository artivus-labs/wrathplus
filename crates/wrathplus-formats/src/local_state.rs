//! The portable state-directory naming contract shared by the client and Manager.
use std::path::{Path, PathBuf};

/// Select a whole home without creating, moving, copying or merging any player state.
/// Existing legacy homes win, even when both names exist: a partial new home must not hide
/// settings, AddOns, login state or Manager JSON. Fresh roots use `wrathplus-config`.
/// Treat unreadable entries and dangling symlinks as present so writes fail rather than reset.
/// The caller selects the resident root (primary checkout for dev, executable dir for players)
/// and handles explicit overrides and hermetic runs before calling this function.
pub fn default_state_home(root: &Path) -> PathBuf {
    let legacy = root.join("benilla-config");
    match std::fs::symlink_metadata(&legacy) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => root.join("wrathplus-config"),
        _ => legacy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn dangling_legacy_link_does_not_silently_reset_state() {
        let root = std::env::temp_dir().join(format!("wrathplus-home-link-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let legacy = root.join("benilla-config");
        std::os::unix::fs::symlink(root.join("missing"), &legacy).unwrap();
        assert_eq!(default_state_home(&root), legacy);
        assert!(!root.join("wrathplus-config").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fresh_roots_use_new_name_and_legacy_homes_win_without_mutation() {
        let root = std::env::temp_dir().join(format!("wrathplus-home-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let new = root.join("wrathplus-config");
        let old = root.join("benilla-config");
        assert_eq!(default_state_home(&root), new);
        assert!(!new.exists());
        std::fs::create_dir(&old).unwrap();
        for file in [
            "config.toml",
            "account",
            "realmlist.wtf",
            "manager.json",
            "manager-addons.json",
        ] {
            std::fs::write(old.join(file), file).unwrap();
        }
        std::fs::create_dir_all(old.join("AddOns/Demo")).unwrap();
        assert_eq!(default_state_home(&root), old);
        std::fs::create_dir(&new).unwrap();
        assert_eq!(default_state_home(&root), old);
        assert_eq!(
            std::fs::read_to_string(old.join("account")).unwrap(),
            "account"
        );
        assert_eq!(std::fs::read_dir(&new).unwrap().count(), 0);
        // Only this synthetic fixture is renamed; production resolution never migrates state.
        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&old, &new).unwrap();
        assert_eq!(default_state_home(&root), new);
        assert!(new.join("AddOns/Demo").is_dir());
        assert!(new.join("manager-addons.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
