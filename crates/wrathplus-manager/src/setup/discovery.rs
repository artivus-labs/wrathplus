//! Discover only known checkout build locations; never execute a candidate to identify it.
use super::*;

fn executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

pub(crate) fn discover(config: &Config) -> Option<PathBuf> {
    let repo = resolved(&config.repo).ok()?;
    let target = resolved(&config.target).ok()?;
    let candidates = [
        config.binary_path(),
        config.target.join(binary_name()),
        config.target.join("bin").join(binary_name()),
        config.target.join("play").join(binary_name()),
        config.target.join("release").join(binary_name()),
        config.repo.join("target/azb-live/bin").join(binary_name()),
    ];
    candidates
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter(|path| path.starts_with(&repo) || path.starts_with(&target))
        .find(|path| executable(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_directory_wins_over_checkout_fallback_and_persists() {
        let f = super::super::tests::Fixture::new();
        let selected = f.config.target.join(binary_name());
        let fallback = f
            .config
            .repo
            .join("target/azb-live/bin")
            .join(binary_name());
        for path in [&selected, &fallback] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic fixture, never executed").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        assert_eq!(discover(&f.config), Some(selected.clone()));
        save_config(&f.config, &f.config.state_path()).unwrap();
        let restored = load_config(&f.config.state_path()).unwrap();
        assert_eq!(restored.target, f.config.target);
        assert_eq!(
            launch_command(&restored, &restored.repo).get_program(),
            selected.as_os_str()
        );
    }

    #[test]

    fn known_build_order_and_missing_or_empty_candidates() {
        let f = super::super::tests::Fixture::new();
        assert!(discover(&f.config).is_none());
        let live = f
            .config
            .repo
            .join("target/azb-live/bin")
            .join(binary_name());
        fs::create_dir_all(live.parent().unwrap()).unwrap();
        fs::write(&live, []).unwrap();
        assert!(discover(&f.config).is_none());
        fs::write(&live, b"test executable fixture; never run").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(discover(&f.config).is_none());
            fs::set_permissions(&live, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(discover(&f.config), Some(live));
        fs::create_dir_all(f.config.binary_path().parent().unwrap()).unwrap();
        fs::write(f.config.binary_path(), b"managed fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(f.config.binary_path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(discover(&f.config), Some(f.config.binary_path()));
    }
}
