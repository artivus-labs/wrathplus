use super::*;
use crate::setup;

#[test]
fn all_commands_are_project_isolated_and_never_destructive() {
    let config = setup::Config::defaults();
    for args in [
        command::BUILD,
        command::MYSQL,
        command::BOOTSTRAP,
        command::START,
        command::STOP,
        command::STATUS,
        command::LOGS,
    ] {
        let cmd = command::compose(&config, &Settings::default(), args);
        let words = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(words
            .windows(2)
            .any(|w| w == ["--project-name", settings::PROJECT]));
        assert!(words.iter().any(|s| s == "--project-directory"));
        // Inspect the file argument as a native path, not a Unix string suffix.
        let compose_file = config.repo.join("packaging/server/compose.yaml");
        assert!(cmd
            .get_args()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| { w[0] == "-f" && std::path::Path::new(w[1]) == compose_file }));
        for forbidden in ["down", "-v", "--volumes", "rm", "exec", "3724", "8085"] {
            assert!(!words.iter().any(|s| s == forbidden));
        }
        let env = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(env.iter().any(|(k, v)| k == "COMPOSE_FILE" && v.is_none()));
        assert!(env.iter().any(|(k, v)| k == "SERVER_RECIPE" && v.is_some()));
        assert!(!env.iter().any(|(k, _)| k.contains("PASSWORD")));
    }
    assert_eq!(
        command::STOP,
        ["stop", "llm-chatter-bridge", "worldserver", "authserver"]
    );
}
#[test]
fn compose_arrays_and_ndjson_require_real_health() {
    let lines = "{\"Service\":\"mysql\",\"State\":\"running\",\"Health\":\"healthy\"}\n{\"Service\":\"authserver\",\"State\":\"running\",\"Health\":\"healthy\"}\n{\"Service\":\"worldserver\",\"State\":\"running\",\"Health\":\"healthy\"}";
    let rows = command::services(lines).unwrap();
    assert!(command::ready(&rows));
    assert!(command::ready(
        &command::services(&serde_json::to_string(&rows).unwrap()).unwrap()
    ));
    let mut rows = rows;
    rows[2]["Health"] = "starting".into();
    assert!(!command::ready(&rows));
    rows[2]["Health"] = "".into();
    assert!(!command::ready(&rows));
    assert!(!command::ready(&[]));
    assert!(command::services("warning: not JSON").is_err());
}
#[test]
fn exited_one_shots_are_expected_but_exited_servers_fail() {
    assert!(command::STATUS.contains(&"--all"));
    let mut rows = command::services(
        r#"[{"Service":"credentials","State":"exited","ExitCode":0},{"Service":"bootstrap","State":"exited","ExitCode":0}]"#,
    ).unwrap();
    assert!(!command::failed(&rows));
    rows.push(serde_json::json!({"Service":"worldserver","State":"exited","ExitCode":1}));
    assert!(command::failed(&rows));
}
#[test]
fn progress_only_accepts_reported_valid_counts() {
    assert_eq!(
        worker::count("#12 90.1 [28/1848] Building CXX"),
        Some((28, 1848))
    );
    assert_eq!(worker::count("still working…"), None);
    assert_eq!(worker::count("[2/0]"), None);
    assert_eq!(worker::count("[5/4]"), None);
}
#[test]
fn errors_finish_activity_and_preserve_logs() {
    let mut manager = Manager::default();
    manager.server_state.loaded = true;
    manager.server_state.busy = true;
    manager.server_state.log = "compiler failure".into();
    let _ = manager.server_update(Input::Finished(Err("disk full".into())));
    assert!(!manager.server_state.busy);
    assert!(manager.server_state.finished.is_some());
    assert_eq!(manager.server_state.error.as_deref(), Some("disk full"));
    assert_eq!(manager.server_state.log, "compiler failure");
}
#[test]
fn settings_are_separate_strict_and_have_no_secret_fields() {
    let config = setup::Config::defaults();
    assert_ne!(Settings::path(&config), config.state_path());
    assert!(Settings::state(&config).ends_with(settings::PROJECT));
    let json = serde_json::to_string(&Settings::default()).unwrap();
    assert!(!json.contains("password"));
    assert!(serde_json::from_str::<Settings>("{\"password\":\"secret\"}").is_err());
}
#[test]
fn bounded_tail_keeps_full_file_on_disk() {
    let path = std::env::temp_dir().join(format!("server-tail-{}", std::process::id()));
    std::fs::write(&path, format!("{}\nlast line\n", "é".repeat(50000))).unwrap();
    let tail = worker::tail(&path).unwrap();
    assert!(tail.len() <= 32768);
    assert!(tail.ends_with("last line\n"));
    assert!(std::fs::metadata(&path).unwrap().len() > 100000);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn connect_requires_explicit_input_and_never_launches() {
    let mut manager = Manager::default();
    let original = manager.host.clone();
    let _ = manager.update(Message::Navigate(crate::Page::Server));
    assert_eq!(manager.host, original);
    let _ = manager.server_update(Input::Connect);
    assert_eq!(manager.host, "127.0.0.1:13724");
    assert!(manager.child.is_none());
}
#[test]
fn settings_reject_output_overlap_without_needing_selected_client_assets() {
    let root = std::env::temp_dir().join(format!("server-settings-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut config = setup::Config::defaults();
    config.repo = root.clone();
    config.data = PathBuf::new();
    let mut settings = Settings {
        source: root.join("source"),
        data: root.join("server-assets"),
        reuse_image: true,
    };
    assert!(settings.validate(&config, false).is_ok());
    assert!(Settings::check_output(&config, &Settings::path(&config)).is_ok());
    settings.source = root.clone();
    assert!(settings.validate(&config, false).is_err());
    settings.source = root.join("source");
    settings.data = Settings::state(&config);
    assert!(settings.validate(&config, false).is_err());
    settings.data = root.join("server-assets");
    assert!(settings.validate(&config, true).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
