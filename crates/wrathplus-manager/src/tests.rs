use super::*;
#[test]
fn navigation_and_preview_actions_are_local_state_only() {
    let mut manager = Manager::default();
    for page in Page::ALL {
        let _ = manager.update(Message::Navigate(page));
        assert_eq!(manager.page, page);
    }
    let _ = manager.update(Message::Assets("/example/Data".into()));
    let _ = manager.update(Message::Notice("server setup is not connected yet."));
    assert_eq!(manager.assets, "/example/Data");
    assert_eq!(manager.events.len(), 2);
    assert!(manager.notice.starts_with("Preview only"));
}

#[test]
fn addon_scopes_navigate_without_recording_an_operation() {
    let mut manager = Manager::default();
    for scope in AddonScope::ALL {
        let _ = manager.update(Message::AddonScope(scope));
        assert_eq!(manager.page, Page::Addons);
        assert_eq!(manager.addon_scope, scope);
        assert_eq!(manager.events.len(), 1);
    }
}

#[test]
fn building_requires_a_completed_nonblocked_checklist() {
    let mut manager = Manager::default();
    assert!(!manager.build_ready());
    let _ = manager.update(Message::Build);
    assert!(!manager.busy);
    let report = preflight::Report {
        checks: vec![preflight::Check {
            label: "toolchain".into(),
            detail: "missing".into(),
            status: preflight::Status::Blocked,
        }],
    };
    let _ = manager.update(Message::PreflightFinished(report));
    assert!(!manager.build_ready());
    let report = preflight::Report {
        checks: vec![preflight::Check {
            label: "disk".into(),
            detail: "advisory".into(),
            status: preflight::Status::Warning,
        }],
    };
    let _ = manager.update(Message::PreflightFinished(report));
    assert!(manager.build_ready());
    manager.checking = true;
    assert!(!manager.build_ready());
    manager.checking = false;
    let _ = manager.update(Message::Repo("another checkout".into()));
    assert!(!manager.build_ready());
    assert!(manager.preflight.is_none());
    assert_eq!(
        manager.config.repo,
        std::path::PathBuf::from("another checkout")
    );
}

#[test]
fn cancelled_folder_dialog_preserves_path_and_validation() {
    let mut manager = Manager {
        assets: "/chosen/Data".into(),
        validated: true,
        choosing: true,
        ..Manager::default()
    };
    let _ = manager.update(Message::FolderChosen(None));
    assert!(!manager.choosing);
    assert!(manager.validated);
    assert_eq!(manager.assets, "/chosen/Data");
}

#[test]
fn build_directory_selection_updates_target_invalidates_checks_and_cancel_preserves_it() {
    let mut manager = Manager::default();
    let _ = manager.update(Message::BuildDirectory("/custom/build with spaces".into()));
    assert_eq!(
        manager.config.target,
        std::path::PathBuf::from("/custom/build with spaces")
    );
    assert!(manager.preflight.is_none());
    manager.choosing = true;
    let _ = manager.update(Message::BuildDirectoryChosen(None));
    assert!(!manager.choosing);
    assert_eq!(manager.build_directory, "/custom/build with spaces");
    manager.busy = true;
    let _ = manager.update(Message::BuildDirectory("/ignored".into()));
    assert_eq!(manager.build_directory, "/custom/build with spaces");
    manager.busy = false;
    let _ = manager.update(Message::BuildDirectoryChosen(Some("/chosen".into())));
    assert_eq!(manager.config.target, std::path::PathBuf::from("/chosen"));
}

#[test]
fn build_failure_remains_visible_and_expands_diagnostics() {
    let mut manager = Manager {
        busy: true,
        ..Default::default()
    };
    manager.build_feedback.started = Some(std::time::Instant::now());
    let _ = manager.update(Message::BuildFinished(Err("compiler failed".into())));
    assert!(!manager.busy);
    assert!(!manager.validated);
    assert!(manager.build_feedback.expanded);
    assert!(manager.build_feedback.finished.is_some());
    assert_eq!(
        manager.build_feedback.error.as_deref(),
        Some("compiler failed")
    );
    assert!(manager
        .build_feedback
        .copy_text()
        .contains("compiler failed"));
    let _ = manager.update(Message::ToggleBuildLog);
    assert!(!manager.build_feedback.expanded);
    let _ = manager.build_feedback();
}

#[test]
fn successful_build_stops_activity_and_keeps_log_available() {
    let mut manager = Manager {
        busy: true,
        ..Default::default()
    };
    manager.build_feedback.started = Some(std::time::Instant::now());
    let _ = manager.update(Message::BuildFinished(Ok("Installed".into())));
    assert!(!manager.busy);
    assert!(manager.validated);
    assert!(manager.build_feedback.error.is_none());
    assert!(manager.build_feedback.finished.is_some());
    let _ = manager.build_feedback();
}
