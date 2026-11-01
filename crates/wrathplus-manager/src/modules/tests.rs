use super::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(super) struct Fixture {
    root: PathBuf,
    pub(super) config: Config,
    pub(super) core: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "module-catalog-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let core = root.join("core");
        fs::create_dir_all(core.join("modules/mod-playerbots/src")).unwrap();
        fs::create_dir_all(core.join("modules/mod-unknown/src")).unwrap();
        fs::create_dir_all(root.join("repo")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(core.join("CMakeLists.txt"), "").unwrap();
        let config = Config {
            repo: root.join("repo"),
            data: root.join("assets"),
            target: root.join("target"),
            locale: "enUS".into(),
            server: "localhost".into(),
        };
        Self { root, config, core }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn discovery_selection_roundtrip_and_real_cmake_flags() {
    let f = Fixture::new();
    let mut catalog = Catalog::load(&f.config, &f.core).unwrap();
    assert!(catalog.entries().iter().all(|e| e.enabled));
    assert!(!f.config.state_root().exists());
    assert!(
        catalog
            .entries()
            .iter()
            .find(|e| e.name == REQUIRED)
            .unwrap()
            .enabled
    );
    catalog.set_enabled(REQUIRED, false).unwrap();
    catalog.save(&f.config).unwrap();
    assert!(
        !Catalog::load(&f.config, &f.core)
            .unwrap()
            .entries()
            .iter()
            .find(|e| e.name == REQUIRED)
            .unwrap()
            .enabled
    );
    catalog.set_enabled(REQUIRED, true).unwrap();
    catalog.set_enabled("mod-unknown", false).unwrap();
    catalog.save(&f.config).unwrap();
    let plan = prepare(&f.config, &f.core).unwrap();
    assert!(plan.requires_staging);
    assert!(plan
        .cmake_args
        .contains(&"-DMODULE_MOD-UNKNOWN=disabled".into()));
    assert!(plan
        .cmake_args
        .contains(&"-DMODULE_MOD-PLAYERBOTS=static".into()));
    assert_eq!(plan.disabled, vec!["mod-unknown"]);
    assert_eq!(plan.selected.len(), 1);
    assert_eq!(plan.selected[0].name, REQUIRED);
    assert_eq!(
        plan.selected[0].source,
        fs::canonicalize(f.core.join("modules/mod-playerbots")).unwrap()
    );
    assert_eq!(plan.core_path, fs::canonicalize(&f.core).unwrap());
    fs::create_dir_all(f.core.join("modules/mod-new/src")).unwrap();
    let loaded = Catalog::load(&f.config, &f.core).unwrap();
    assert!(
        loaded
            .entries()
            .iter()
            .find(|e| e.name == "mod-new")
            .unwrap()
            .enabled
    );
    assert!(
        !loaded
            .entries()
            .iter()
            .find(|e| e.name == "mod-unknown")
            .unwrap()
            .enabled
    );
}
#[test]
fn local_registration_is_read_only_and_unavailable_sources_fail_build() {
    let f = Fixture::new();
    let local = f.root.join("mod-local");
    fs::create_dir_all(local.join("src")).unwrap();
    fs::write(local.join("marker"), "unchanged").unwrap();
    let mut catalog = Catalog::load(&f.config, &f.core).unwrap();
    catalog.register_local(&local).unwrap();
    assert!(catalog.register_local(&local).is_err());
    assert!(catalog.build_plan().unwrap().requires_staging);
    catalog.save(&f.config).unwrap();
    assert_eq!(
        fs::read_to_string(local.join("marker")).unwrap(),
        "unchanged"
    );
    fs::remove_dir_all(&local).unwrap();
    let mut loaded = Catalog::load(&f.config, &f.core).unwrap();
    assert!(loaded.build_plan().is_err());
    loaded.set_enabled("mod-local", false).unwrap();
    assert!(loaded.build_plan().is_ok());
}
#[test]
fn github_input_is_strict_and_offline() {
    for input in [
        "ac/mod-example",
        "https://github.com/ac/mod-example",
        "ac/mod-example.git",
    ] {
        assert_eq!(github::normalize(input).unwrap(), "ac/mod-example");
    }
    for input in [
        "http://github.com/ac/mod-x",
        "https://user:pass@github.com/ac/mod-x",
        "https://github.com.evil/ac/mod-x",
        "ac/mod-x/",
        "ac/../mod-x",
        "ac/mod-x?ref=x",
        "ac/mod-x#branch",
        "git@github.com:ac/mod-x",
        "ac/mod-x\n",
        "ac/-x",
        "ac/mod.x",
        "ac/mod-x;touch",
    ] {
        assert!(github::normalize(input).is_err(), "{input:?}");
    }
}
#[test]
fn malformed_catalog_and_core_switch_fail_closed() {
    let f = Fixture::new();
    let mut catalog = Catalog::load(&f.config, &f.core).unwrap();
    catalog.entries.push(catalog.entries[0].clone());
    assert!(catalog.save(&f.config).is_err());
    Catalog::load(&f.config, &f.core)
        .unwrap()
        .save(&f.config)
        .unwrap();
    let path = storage::root(&f.config, &f.core)
        .unwrap()
        .join("catalog.json");
    fs::write(&path, b"not json").unwrap();
    assert!(Catalog::load(&f.config, &f.core).is_err());
    fs::write(&path, vec![b' '; 262145]).unwrap();
    assert!(Catalog::load(&f.config, &f.core).is_err());
}
#[test]
fn playerbots_free_build_is_valid() {
    let f = Fixture::new();
    fs::remove_dir_all(f.core.join("modules/mod-playerbots")).unwrap();
    assert!(prepare(&f.config, &f.core).is_ok());
}
#[cfg(unix)]
#[test]
fn symlink_output_and_source_escapes_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir_all(f.config.state_root()).unwrap();
    symlink(
        &f.config.data,
        f.config.state_root().join("managed-module-sources"),
    )
    .unwrap();
    assert!(Catalog::load(&f.config, &f.core).is_err());
    fs::remove_file(f.config.state_root().join("managed-module-sources")).unwrap();
    symlink(&f.root, f.core.join("modules/mod-escape")).unwrap();
    fs::create_dir_all(f.root.join("src")).unwrap();
    assert!(Catalog::load(&f.config, &f.core).is_err());
}
#[test]
fn protected_client_core_and_server_data_are_never_outputs() {
    let mut f = Fixture::new();
    let core = fs::canonicalize(&f.core).unwrap();
    f.config.data = f.config.repo.clone();
    assert!(storage::root(&f.config, &core).is_err());
    f.config.data = f.root.join("assets");
    fs::create_dir_all(f.config.state_root()).unwrap();
    let output = f.config.state_root().join("managed-module-sources");
    fs::write(
        f.config.state_root().join("manager-server.json"),
        serde_json::to_vec(&serde_json::json!({"data": output})).unwrap(),
    )
    .unwrap();
    assert!(storage::root(&f.config, &core).is_err());
    fs::remove_file(f.config.state_root().join("manager-server.json")).unwrap();
    assert!(storage::root(&f.config, &fs::canonicalize(f.config.repo.clone()).unwrap()).is_err());
}
