use super::*;
use crate::server::status::Health;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "server-guide-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("core/modules/mod-playerbots/src")).unwrap();
    std::fs::write(root.join("core/CMakeLists.txt"), "").unwrap();
    for dir in ["dbc", "maps", "vmaps"] {
        std::fs::create_dir_all(root.join("data").join(dir)).unwrap();
    }
    root
}

fn marks(server: &Server, client: bool) -> Vec<Mark> {
    server.guide(client).iter().map(|s| s.mark).collect()
}

fn action(step: &Step) -> Option<(&'static str, bool)> {
    step.action.as_ref().map(|(label, m)| (*label, m.is_some()))
}

#[test]
fn a_fresh_install_starts_with_the_source_and_the_client() {
    let server = Server::default();
    let steps = server.guide(false);
    assert_eq!(
        marks(&server, false),
        [
            Mark::Now,
            Mark::Later,
            Mark::Now,
            Mark::Later,
            Mark::Later,
            Mark::Later
        ]
    );
    assert_eq!(action(&steps[0]), Some(("Get server source", true)));
    assert_eq!(action(&steps[2]), Some(("Open Client page", true)));
    assert!(steps[1].action.is_none() && steps[3].action.is_none());
    // With a client chosen, the data waits for the build that carries the extractors.
    assert_eq!(server.guide(true)[2].mark, Mark::Later);
}

#[test]
fn each_finished_step_hands_on_to_the_next() {
    let root = scratch();
    let mut server = Server::default();
    server.settings.source = root.join("core");
    server.build = BuildState::Missing;
    let steps = server.guide(true);
    assert_eq!(steps[0].mark, Mark::Done);
    assert_eq!(action(&steps[1]), Some(("Build", true)));
    assert_eq!(steps[2].mark, Mark::Later);

    server.build = BuildState::Present { warning: None };
    let steps = server.guide(true);
    assert_eq!(action(&steps[2]), Some(("Prepare server data", true)));
    assert_eq!(steps[3].mark, Mark::Later);

    server.settings.data = root.join("data");
    server.health = Health::Stopped;
    let steps = server.guide(true);
    assert_eq!(steps[2].mark, Mark::Done);
    assert_eq!(action(&steps[3]), Some(("Start", true)));
    assert_eq!(steps[4].mark, Mark::Later);
    assert_eq!(action(&steps[5]), Some(("Make navigation meshes", true)));

    server.health = Health::Running;
    assert_eq!(
        action(&server.guide(true)[4]),
        Some(("Create account", true))
    );
    server.account.created = Some("PLAYER".into());
    std::fs::write(root.join("data/.wrathplus-mmaps-done"), "done").unwrap();
    assert!(marks(&server, true).iter().all(|m| *m == Mark::Done));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn one_setup_task_at_a_time_and_no_new_source_under_a_build() {
    let root = scratch();
    let mut server = Server::default();
    server.settings.source = root.join("core");
    server.build = BuildState::Present { warning: None };
    server.settings.data = root.join("data");
    let (_events, receiver) = std::sync::mpsc::channel();
    server.job = Some(super::super::setup_job::Job::new(
        SetupTask::Navmesh,
        receiver,
    ));
    let steps = server.guide(true);
    assert_eq!(steps[5].mark, Mark::Now);
    assert!(steps[5].action.is_none());
    assert!(!server.can_setup(SetupTask::Data) && !server.can_setup(SetupTask::Source));
    server.job = None;
    server.busy = true;
    server.active = Some(Action::Build);
    assert!(!server.can_setup(SetupTask::Source));
    // Preparing data does not touch the realm, so it may run beside a build or a start.
    assert!(server.can_setup(SetupTask::Data));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_finished_task_selects_the_folder_it_made() {
    let mut server = Server::default();
    let (events, receiver) = std::sync::mpsc::channel();
    server.job = Some(super::super::setup_job::Job::new(
        SetupTask::Source,
        receiver,
    ));
    events
        .send(super::super::worker::Event::Source("/state/core".into()))
        .unwrap();
    events
        .send(super::super::worker::Event::Stage("Checking".into()))
        .unwrap();
    server.refresh();
    assert_eq!(server.settings.source, PathBuf::from("/state/core"));
    assert_eq!(server.source, "/state/core");
    assert_eq!(server.job().unwrap().stage, "Checking");
    events
        .send(super::super::worker::Event::Data("/state/data".into()))
        .unwrap();
    server.refresh();
    assert_eq!(server.settings.data, PathBuf::from("/state/data"));
}
