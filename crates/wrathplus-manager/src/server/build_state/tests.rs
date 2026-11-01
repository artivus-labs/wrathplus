use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn fixture() -> (std::path::PathBuf, setup::Config, Settings) {
    let root = std::env::temp_dir().join(format!(
        "manager-build-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let source = root.join("core");
    std::fs::create_dir_all(source.join("modules/mod-playerbots")).unwrap();
    std::fs::write(source.join("modules/CMakeLists.txt"), "not a module").unwrap();
    let mut config = setup::Config::defaults();
    config.repo = root.join("client");
    config.data = root.join("assets");
    std::fs::create_dir_all(&config.repo).unwrap();
    std::fs::create_dir_all(&config.data).unwrap();
    let settings = Settings {
        source,
        data: root.join("server-data"),
        reuse_image: true,
    };
    (root, config, settings)
}
#[test]
fn single_build_action_requires_observation_and_never_reuses_instead_of_building() {
    assert!(BuildState::Unknown.input(true).is_none());
    assert!(BuildState::Unavailable("Docker offline".into())
        .input(true)
        .is_none());
    assert_eq!(BuildState::Missing.label(), "Build");
    assert!(matches!(
        BuildState::Missing.input(true),
        Some(Input::Run(Action::Build))
    ));
    let built = BuildState::Present { warning: None };
    assert_eq!(built.label(), "Rebuild");
    assert!(matches!(
        built.input(true),
        Some(Input::Run(Action::ForceBuild))
    ));
    assert!(built.input(false).is_none());
    assert!(BuildState::Missing
        .warning()
        .unwrap()
        .contains("before its first start"));
}
#[test]
fn inventory_detects_new_removed_modules_and_replaced_image_without_false_freshness() {
    let (root, config, settings) = fixture();
    assert!(matches!(
        BuildState::observe("\n", &config, &settings),
        BuildState::Missing
    ));
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .unwrap()
        .contains("not verified"));
    let receipt = Receipt::snapshot("sha256:one".into(), &settings.source).unwrap();
    assert_eq!(receipt.modules, ["mod-playerbots"]);
    receipt.save(&config).unwrap();
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .is_none());
    std::fs::create_dir(settings.source.join("modules/mod-example")).unwrap();
    let state = BuildState::observe("sha256:one", &config, &settings);
    assert!(state
        .warning()
        .unwrap()
        .contains("New modules: mod-example"));
    assert!(state.warning().unwrap().contains("not included"));
    // Only a successful-build receipt updates the baseline, not an observation.
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .is_some());
    Receipt::snapshot("sha256:two".into(), &settings.source)
        .unwrap()
        .save(&config)
        .unwrap();
    assert!(BuildState::observe("sha256:two", &config, &settings)
        .warning()
        .is_none());
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .unwrap()
        .contains("not verified"));
    std::fs::remove_dir(settings.source.join("modules/mod-example")).unwrap();
    assert!(BuildState::observe("sha256:two", &config, &settings)
        .warning()
        .unwrap()
        .contains("modules changed"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn missing_source_and_corrupt_receipt_are_not_reported_current() {
    let (root, config, mut settings) = fixture();
    let path = Settings::state(&config).join("build-receipt.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "broken").unwrap();
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .unwrap()
        .contains("not verified"));
    settings.source = root.join("missing");
    assert!(BuildState::observe("sha256:one", &config, &settings)
        .warning()
        .unwrap()
        .contains("unavailable"));
    std::fs::remove_dir_all(root).unwrap();
}
