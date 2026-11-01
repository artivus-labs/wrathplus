//! Offline import contract tests: no Git invocation or network access.
use super::*;
use crate::setup::tests::Fixture;

fn checkout(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("Demo.toc"), "## Interface: 30300\nmain.lua\n").unwrap();
    std::fs::write(path.join("main.lua"), "error('must never execute')").unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[test]
fn root_checkout_is_discovered_without_activation_or_execution() {
    let f = Fixture::new();
    let path = checkout(&f.config.repo, "imported");
    let note = github::import_with(&f.config, "owner/repo", |config, repository| {
        assert_eq!(config.repo, f.config.repo);
        assert_eq!(repository, "owner/repo");
        Ok(path.clone())
    })
    .unwrap();
    let settings = load(&f.config).unwrap();
    assert_eq!(settings.folders, [path]);
    assert!(settings.selected.is_empty());
    assert_eq!(discover(&f.config, &settings).len(), 1);
    assert!(!f.config.state_root().join("AddOns/Demo").exists());
    assert!(note.contains("No addon was automatically enabled"));
}

#[test]
fn collection_checkout_is_supported_and_registration_is_idempotent() {
    let f = Fixture::new();
    let addon = checkout(&f.config.repo, "collection/Demo");
    let root = addon.parent().unwrap().to_owned();
    for _ in 0..2 {
        github::import_with(&f.config, "https://github.com/owner/repo", |_, _| {
            Ok(root.clone())
        })
        .unwrap();
    }
    let settings = load(&f.config).unwrap();
    assert_eq!(settings.folders, [root]);
    assert_eq!(discover(&f.config, &settings)[0].source, addon);
    assert!(settings.selected.is_empty());
}

#[test]
fn invalid_checkout_or_helper_failure_does_not_persist() {
    let f = Fixture::new();
    let empty = f.config.repo.join("empty-checkout");
    std::fs::create_dir(&empty).unwrap();
    let error = github::import_with(&f.config, "owner/repo", |_, _| Ok(empty.clone())).unwrap_err();
    assert!(error.contains("No addon TOCs"));
    assert!(empty.exists());
    assert_eq!(
        github::import_with(&f.config, "bad", |_, _| Err("Invalid GitHub input".into()))
            .unwrap_err(),
        "Invalid GitHub input"
    );
    assert!(!f.config.state_root().join("manager-addons.json").exists());
}

#[test]
fn settings_are_reloaded_after_checkout_not_taken_from_ui_snapshot() {
    let f = Fixture::new();
    let existing = checkout(&f.config.repo, "existing");
    let imported = checkout(&f.config.repo, "imported");
    let selection = scan_folder(&existing)[0].selection();
    github::import_with(&f.config, "owner/repo", |config, _| {
        save_and_apply(
            config,
            &Settings {
                folders: vec![existing.clone()],
                selected: vec![selection.clone()],
            },
        )?;
        Ok(imported.clone())
    })
    .unwrap();
    let settings = load(&f.config).unwrap();
    assert_eq!(settings.folders, [existing, imported]);
    assert_eq!(settings.selected, [selection]);
}

#[test]
fn client_starting_during_checkout_fences_registration_and_retains_source() {
    let f = Fixture::new();
    let imported = checkout(&f.config.repo, "imported");
    let error = github::import_with(&f.config, "owner/repo", |config, _| {
        std::fs::create_dir_all(config.target.join("manager")).unwrap();
        std::fs::write(config.target.join("manager/running.lock"), "fixture").unwrap();
        Ok(imported.clone())
    })
    .unwrap_err();
    assert!(error.contains("running"), "{error}");
    assert!(load(&f.config).unwrap().folders.is_empty());
    assert!(imported.join("Demo.toc").exists());
}

#[test]
fn choosing_serializes_addon_actions_and_import_requires_editable_state() {
    let f = Fixture::new();
    let mut manager = crate::Manager {
        config: f.config.clone(),
        choosing: true,
        ..Default::default()
    };
    assert!(!manager.addon_editable());
    // No worker is started while a picker/import owns the shared flag.
    let _ = manager.import_addon_github();
    assert!(manager.choosing);
    manager.choosing = false;
    let _ = manager.import_addon_github();
    assert!(!manager.choosing);
    assert!(manager.notice.contains("Enter a GitHub"));
}
