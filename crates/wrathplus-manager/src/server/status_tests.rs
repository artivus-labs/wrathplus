use super::status::Health;
use super::*;

fn healthy() -> Vec<serde_json::Value> {
    ["mysql", "authserver", "worldserver"]
        .into_iter()
        .map(|service| serde_json::json!({"Service":service,"State":"running","Health":"healthy"}))
        .collect()
}
#[test]
fn play_control_toggles_start_stop_and_disables_duplicates() {
    let mut server = Server::default();
    assert_eq!(server.control_label(), "Start");
    assert!(matches!(
        server.control_input(),
        Some(Input::Run(Action::Start))
    ));
    server.active = Some(Action::Start);
    server.busy = true;
    assert_eq!(server.label(), "Starting…");
    assert_eq!(server.control_label(), "Starting…");
    assert!(server.control_input().is_none());
    server.busy = false;
    server.active = None;
    server.health = Health::Running;
    assert_eq!(server.control_label(), "Stop");
    assert!(matches!(
        server.control_input(),
        Some(Input::Run(Action::Stop))
    ));
    server.busy = true;
    server.active = Some(Action::Stop);
    assert_eq!(server.control_label(), "Stopping…");
    assert!(server.control_input().is_none());
    server.busy = false;
    server.active = None;
    server.health = Health::Stopped;
    assert_eq!(server.control_label(), "Start");
    assert!(matches!(
        server.control_input(),
        Some(Input::Run(Action::Start))
    ));
    server.choosing = true;
    assert!(server.control_input().is_none());
}
#[test]
fn logs_are_optional_without_changing_server_or_client_state() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    assert!(!manager.server_state.show_logs);
    let _ = manager.server_update(Input::ToggleLogs);
    assert!(manager.server_state.show_logs);
    let _ = manager.server_update(Input::ToggleLogs);
    assert!(!manager.server_state.show_logs);
    assert!(!manager.server_state.busy);
    assert!(manager.child.is_none());
}
#[test]
fn activity_opens_logs_and_records_operations_not_poll_results() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    let _ = manager.update(Message::Navigate(crate::Page::Activity));
    assert!(manager.server_state.show_logs);
    assert!(manager.build_feedback.expanded);
    let count = manager.events.len();
    manager.server_state.active = Some(Action::Status);
    let _ = manager.server_update(Input::Finished(Ok("healthy".into())));
    assert_eq!(manager.events.len(), count);
    manager.server_state.active = Some(Action::Stop);
    let _ = manager.server_update(Input::Finished(Ok("Stopped; progress retained".into())));
    assert_eq!(manager.events.len(), count + 1);
    assert!(manager.events.last().unwrap().contains("Stopped"));
    manager.server_state.active = Some(Action::Start);
    let _ = manager.server_update(Input::Finished(Err("Docker unavailable".into())));
    assert!(manager
        .events
        .last()
        .unwrap()
        .contains("Docker unavailable"));
    assert!(manager.child.is_none());
}
#[test]
fn status_summary_excludes_setup_containers() {
    let mut rows = healthy();
    rows.push(serde_json::json!({"Service":"credentials","State":"exited","ExitCode":0}));
    let summary = command::summary(&rows);
    assert!(summary.contains("worldserver"));
    assert!(!summary.contains("credentials"));
    assert!(!summary.contains("exited"));
}
#[test]
fn polling_is_due_on_startup_and_across_navigation_without_start() {
    let mut manager = Manager::default();
    for page in crate::Page::ALL {
        manager.page = page;
        assert!(manager.server_state.refresh_due());
    }
    manager.server_state.next_refresh = Some(Instant::now() + std::time::Duration::from_secs(10));
    assert!(!manager.server_state.refresh_due());
    manager.server_state.next_refresh = Some(Instant::now());
    assert!(manager.server_state.refresh_due());
    manager.server_state.busy = true;
    assert!(!manager.server_state.refresh_due());
}
#[test]
fn health_requires_all_three_services_and_ignores_successful_one_shots() {
    let mut rows = healthy();
    rows.push(serde_json::json!({"Service":"credentials","State":"exited","ExitCode":0}));
    assert_eq!(Health::observed(&rows), Health::Running);
    rows[2]["Health"] = "starting".into();
    assert_eq!(Health::observed(&rows), Health::Starting);
    rows[2]["Health"] = "unhealthy".into();
    assert_eq!(Health::observed(&rows), Health::Unhealthy);
    rows[2]["Health"] = "".into();
    rows[2]["State"] = "exited".into();
    assert_eq!(Health::observed(&rows), Health::Unhealthy);
    rows[1]["State"] = "exited".into();
    assert_eq!(Health::observed(&rows), Health::Stopped);
    assert_eq!(Health::observed(&[]), Health::Stopped);
}
#[test]
fn worker_health_and_failure_remain_visible_on_play_and_server() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    let (sender, receiver) = std::sync::mpsc::channel();
    manager.server_state.receiver = Some(receiver);
    sender.send(worker::Event::Health(Health::Running)).unwrap();
    let _ = manager.server_update(Input::Finished(Ok("all healthy".into())));
    for page in [crate::Page::Play, crate::Page::Server] {
        let _ = manager.update(Message::Navigate(page));
        assert_eq!(manager.server_state.label(), "Running");
        assert_eq!(manager.server_state.detail(), "all healthy");
    }
    let _ = manager.server_update(Input::Finished(Err("Docker unavailable".into())));
    assert_eq!(manager.server_state.label(), "Check failed");
    assert_eq!(manager.server_state.detail(), "Docker unavailable");
    assert!(manager.server_state.next_refresh.is_some());
}
#[test]
fn a_status_check_never_locks_the_page_and_queues_a_pressed_action() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    manager.server_state.busy = true;
    manager.server_state.active = Some(Action::Status);
    let state = &manager.server_state;
    assert!(state.checking_quietly());
    assert!(!state.locked());
    assert!(state.module_editable());
    assert!(state.control_input().is_some());
    let _ = manager.server_update(Input::Data("/extracted/data".into()));
    assert_eq!(manager.server_state.data, "/extracted/data");
    let _ = manager.server_update(Input::Run(Action::Status));
    assert!(manager.server_state.queued.is_none());
    let _ = manager.server_update(Input::Run(Action::Stop));
    assert!(matches!(manager.server_state.queued, Some(Action::Stop)));
    assert!(matches!(manager.server_state.active, Some(Action::Status)));
}
#[test]
fn a_real_operation_still_locks_the_page() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    manager.server_state.busy = true;
    manager.server_state.active = Some(Action::Start);
    assert!(manager.server_state.locked());
    assert!(!manager.server_state.module_editable());
    assert!(manager.server_state.control_input().is_none());
    let _ = manager.server_update(Input::Data("/ignored".into()));
    assert_ne!(manager.server_state.data, "/ignored");
    let _ = manager.server_update(Input::Run(Action::Stop));
    assert!(manager.server_state.queued.is_none());
}
