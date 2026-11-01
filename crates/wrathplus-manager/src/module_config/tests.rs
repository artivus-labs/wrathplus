use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) config: Config,
    pub(super) core: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "module-config-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let core = root.join("core");
        fs::create_dir_all(core.join("modules/mod-playerbots/src")).unwrap();
        fs::write(core.join("CMakeLists.txt"), "").unwrap();
        let config = Config {
            repo: root.clone(),
            data: root.join("client-assets"),
            target: root.join("target"),
            locale: "enUS".into(),
            server: "localhost:3724".into(),
        };
        Self { root, config, core }
    }
    pub(super) fn path(&self) -> PathBuf {
        directory(&self.config).join("bots.conf")
    }
    fn create(&self) {
        files::write(
            &self.config,
            &self.core,
            &self.path(),
            "Secret = local\n",
            None,
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn creation_save_conflict_and_no_overwrite() {
    let f = Fixture::new();
    f.create();
    assert!(files::write(&f.config, &f.core, &f.path(), "replacement", None).is_err());
    let original = files::read(&f.path()).unwrap();
    files::write(&f.config, &f.core, &f.path(), "changed", Some(&original)).unwrap();
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "changed");
    assert!(files::write(&f.config, &f.core, &f.path(), "stale", Some(&original)).is_err());
    let snapshot = files::read(&f.path()).unwrap();
    fs::write(f.path(), "external").unwrap();
    assert!(files::write(&f.config, &f.core, &f.path(), "stale", Some(&snapshot)).is_err());
    assert_eq!(fs::read_to_string(f.path()).unwrap(), "external");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn bounded_utf8_and_traversal() {
    let f = Fixture::new();
    f.create();
    assert!(validate(
        &f.config,
        &f.core,
        &directory(&f.config).join("../escape.conf")
    )
    .is_err());
    assert!(validate(
        &f.config,
        &f.core,
        &directory(&f.config).join("nested/file.conf")
    )
    .is_err());
    assert!(validate(
        &f.config,
        &f.core,
        &directory(&f.config).join("file.conf.dist")
    )
    .is_err());
    fs::write(f.path(), vec![b'a'; LIMIT + 1]).unwrap();
    assert!(files::read(&f.path()).is_err());
    fs::write(f.path(), [0xff]).unwrap();
    assert!(files::read(&f.path()).is_err());
    fs::write(f.path(), vec![b'a'; LIMIT]).unwrap();
    assert_eq!(files::read(&f.path()).unwrap().text.len(), LIMIT);
}

#[test]
fn protects_client_core_and_persisted_server_paths() {
    let f = Fixture::new();
    let mut config = f.config.clone();
    config.data = config.repo.clone();
    assert!(validate(&config, &f.core, &f.path()).is_err());
    assert!(validate(&f.config, &f.root, &f.path()).is_err());
    fs::create_dir_all(f.config.state_root()).unwrap();
    for key in ["data", "source"] {
        fs::write(
            f.config.state_root().join("manager-server.json"),
            format!("{{\"{key}\":{}}}", serde_json::to_string(&f.root).unwrap()),
        )
        .unwrap();
        assert!(validate(&f.config, &f.core, &f.path()).is_err());
    }
}

#[test]
fn selected_immediate_templates_only_and_explicit_creation() {
    let f = Fixture::new();
    let conf = f.core.join("modules/mod-playerbots/conf");
    fs::create_dir_all(conf.join("nested")).unwrap();
    fs::write(conf.join("bots.conf.dist"), "Bots = 1\n").unwrap();
    fs::write(conf.join("nested/ignored.conf.dist"), "ignored").unwrap();
    fs::write(conf.join("ignored.txt"), "ignored").unwrap();
    let templates = files::templates(&f.config, &f.core).unwrap();
    assert_eq!(templates, vec![conf.join("bots.conf.dist")]);
    assert!(!directory(&f.config).exists());
    let target = files::create(&f.config, &f.core, &templates[0]).unwrap();
    assert_eq!(target, f.path());
    assert_eq!(fs::read_to_string(&target).unwrap(), "Bots = 1\n");
    assert!(files::create(&f.config, &f.core, &templates[0]).is_err());
    assert!(files::create(&f.config, &f.core, &conf.join("nested/ignored.conf.dist")).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_special_files_and_parent_links_rejected() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let f = Fixture::new();
    fs::create_dir_all(directory(&f.config)).unwrap();
    let outside = f.root.join("outside");
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, f.path()).unwrap();
    assert!(files::read(&f.path()).is_err());
    assert!(validate(&f.config, &f.core, &f.path()).is_err());
    fs::remove_file(f.path()).unwrap();
    let socket = f.root.join("socket");
    let _socket = UnixListener::bind(&socket).unwrap();
    assert!(files::read(&socket).is_err());
    fs::remove_dir(directory(&f.config)).unwrap();
    symlink(&f.root, directory(&f.config)).unwrap();
    assert!(validate(&f.config, &f.core, &f.path()).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "outside");
}

#[test]
fn listing_is_bounded() {
    let f = Fixture::new();
    fs::create_dir_all(directory(&f.config)).unwrap();
    for i in 0..=ENTRIES {
        fs::write(directory(&f.config).join(format!("{i}.txt")), "").unwrap();
    }
    assert!(paths::list(&directory(&f.config), ".conf").is_err());
}

#[cfg(windows)]
#[test]
fn canonical_windows_paths_are_plain_and_editable() {
    let f = Fixture::new();
    // canonicalize returns a verbatim Windows prefix (e.g. \\?\C:).
    let root = fs::canonicalize(&f.root).unwrap();
    assert_eq!(paths::plain(&root).unwrap(), root);
    let missing = root.join("not-created.conf");
    assert_eq!(paths::plain(&missing).unwrap(), missing);
    f.create();
    let path = fs::canonicalize(f.path()).unwrap();
    assert_eq!(paths::plain(&path).unwrap(), path);
    assert_eq!(validate(&f.config, &f.core, &path).unwrap(), path);
}
