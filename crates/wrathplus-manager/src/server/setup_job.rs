//! First-install tasks that run beside the server's own operations: getting the server source and
//! preparing the server data. Each runs on its own thread with a saved log, like a build, and
//! never touches the realm's containers, so the server can be started or stopped meanwhile.
use super::data_prep::{self, Navmesh, Part};
use super::settings::Settings;
use super::worker::{self, Event};
use crate::setup::{self, Config};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupTask {
    /// Download the pinned AzerothCore and mod-playerbots.
    Source,
    /// dbc, maps and vmaps from the player's client: enough to start the realm.
    Data,
    /// Navigation meshes, after the data; long, and continues where it stopped.
    Navmesh,
}

impl SetupTask {
    pub(super) fn running_label(self) -> &'static str {
        match self {
            Self::Source => "Downloading server source…",
            Self::Data => "Preparing server data…",
            Self::Navmesh => "Making navigation meshes…",
        }
    }
}

/// How the last task ended, and its saved log.
pub(super) struct Finished {
    pub result: Result<String>,
    pub log: Option<PathBuf>,
}

pub(super) struct Job {
    pub task: SetupTask,
    pub receiver: Receiver<Event>,
    pub log_path: Option<PathBuf>,
    pub log: String,
    pub stage: String,
    pub started: Instant,
}

impl Job {
    pub(super) fn new(task: SetupTask, receiver: Receiver<Event>) -> Self {
        Self {
            task,
            receiver,
            log_path: None,
            log: String::new(),
            stage: "Starting".into(),
            started: Instant::now(),
        }
    }
    /// What the task's tools last reported, as (label, done, total).
    pub(super) fn progress(&self) -> Option<(&'static str, u64, u64)> {
        match self.task {
            SetupTask::Source => {
                super::source::progress(&self.log).map(|(label, pct)| (label, pct, 100))
            }
            SetupTask::Data => data_prep::progress(&self.log).map(|(d, t)| ("Maps", d, t)),
            SetupTask::Navmesh => data_prep::progress(&self.log).map(|(d, t)| ("Tiles", d, t)),
        }
    }
}

/// Run `task` to the end. Blocking: call it off the UI thread.
pub(crate) fn run(
    mut config: Config,
    task: SetupTask,
    scope: Navmesh,
    sender: Sender<Event>,
) -> Result<String> {
    config.repo = setup::resolved(&config.repo)?;
    if !config.data.as_os_str().is_empty() {
        config.data = setup::resolved(&config.data)?;
    }
    if task != SetupTask::Source {
        let docker = super::docker::check();
        let blocked = docker.blocked();
        let advice = docker.advice();
        let _ = sender.send(Event::Docker(docker));
        if blocked {
            return Err(advice.into());
        }
    }
    Settings::check_output(&config, &Settings::state(&config).join("manager-logs"))?;
    let (mut log, path) = worker::open_log(&config)?;
    let _ = sender.send(Event::Log(path));
    let stage = |label: String| {
        let _ = sender.send(Event::Stage(label));
    };
    match task {
        SetupTask::Source => {
            let core = super::source::get(&config, &mut log, &stage)?;
            let _ = sender.send(Event::Source(core));
            Ok("Server source ready: AzerothCore with Playerbots, at the revisions the build expects. Next: choose a server type and build.".into())
        }
        SetupTask::Data => {
            let out = data_prep::run(&config, &[Part::Maps, Part::Vmaps], scope, &mut log, &stage)?;
            let _ = sender.send(Event::Data(out));
            Ok("Server data ready. You can start the server now; navigation meshes can follow later.".into())
        }
        SetupTask::Navmesh => {
            data_prep::run(&config, &[Part::Mmaps], scope, &mut log, &stage)?;
            Ok(if scope == Navmesh::All {
                "Navigation meshes ready. Restart the server so bots and agents walk with them."
                    .into()
            } else {
                format!(
                    "Navigation mesh made for {}. The rest of the world is still to do.",
                    scope.describe()
                )
            })
        }
    }
}

impl super::Server {
    /// Take the running task's events and its log tail; a folder it made becomes the selection.
    pub(super) fn refresh_job(&mut self) {
        let Some(job) = &mut self.job else { return };
        let mut adopted = Vec::new();
        for event in job.receiver.try_iter() {
            match event {
                Event::Log(path) => job.log_path = Some(path),
                Event::Stage(stage) => job.stage = stage,
                Event::Docker(docker) => self.docker = docker,
                Event::Source(path) => adopted.push((true, path)),
                Event::Data(path) => adopted.push((false, path)),
                Event::Health(_) | Event::Build(_) => {}
            }
        }
        if let Some(path) = &job.log_path {
            if let Ok(log) = worker::tail(path) {
                job.log = log;
            }
        }
        for (source, path) in adopted {
            let value = path.to_string_lossy().into_owned();
            if source {
                self.source = value;
                self.settings.source = path;
                self.build = super::build_state::BuildState::Unknown;
                self.next_refresh = None;
            } else {
                self.data = value;
                self.settings.data = path;
            }
        }
    }
    pub(super) fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }
    pub(super) fn job_result(&self) -> Option<&Finished> {
        self.job_result.as_ref()
    }
    /// Whether `task` may start now: one task at a time, and a new source never under a build.
    pub(super) fn can_setup(&self, task: SetupTask) -> bool {
        self.job.is_none() && !self.choosing && (task != SetupTask::Source || !self.locked())
    }
}

impl crate::Manager {
    pub(crate) fn setup_start(
        &mut self,
        task: SetupTask,
        scope: Navmesh,
    ) -> iced::Task<crate::Message> {
        self.load_server();
        if !self.server_state.can_setup(task)
            || (task == SetupTask::Source && self.module_state.busy)
        {
            return iced::Task::none();
        }
        let (events, receiver) = std::sync::mpsc::channel();
        self.server_state.job = Some(Job::new(task, receiver));
        self.server_state.job_result = None;
        let (sender, done) = iced::futures::channel::oneshot::channel();
        let config = self.config.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| run(config, task, scope, events))
                .unwrap_or_else(|_| Err("The setup task stopped unexpectedly. Inspect its saved log; nothing was deleted.".into()));
            let _ = sender.send(result);
        });
        iced::Task::perform(
            async move {
                done.await.unwrap_or_else(|_| {
                    Err("The setup task disconnected; inspect its saved log.".into())
                })
            },
            move |r| crate::Message::Server(super::Input::SetupFinished(task, r)),
        )
    }
    pub(super) fn setup_finished(&mut self, task: SetupTask, result: Result<String>) {
        let state = &mut self.server_state;
        state.refresh_job();
        let log = state.job.take().and_then(|job| job.log_path);
        let adopted = result.is_ok() && matches!(task, SetupTask::Source | SetupTask::Data);
        if adopted {
            // Saved now so the next build or start uses the new folder; a running operation
            // keeps the settings it started with.
            if let Err(error) = state.settings.save(&self.config) {
                state.error = Some(error);
            }
        }
        self.events.push(match &result {
            Ok(message) => format!("Server setup: {message}"),
            Err(error) => format!("Server setup failed: {error}"),
        });
        self.server_state.job_result = Some(Finished { result, log });
    }
}

/// The same task from the command line: stages and the saved log path on stdout, the folder it
/// made selected in the server settings. Blocking.
pub(crate) fn run_cli(config: Config, task: SetupTask, scope: Navmesh) -> Result<String> {
    let (events, receiver) = std::sync::mpsc::channel();
    let worker = {
        let config = config.clone();
        std::thread::spawn(move || run(config, task, scope, events))
    };
    let mut adopted = None;
    for event in receiver {
        match event {
            Event::Log(path) => println!("Saved log: {}", path.display()),
            Event::Stage(stage) => println!("{stage}"),
            Event::Docker(docker) if docker.blocked() => println!("{}", docker.advice()),
            Event::Source(path) => adopted = Some((true, path)),
            Event::Data(path) => adopted = Some((false, path)),
            Event::Docker(_) | Event::Health(_) | Event::Build(_) => {}
        }
    }
    let message = worker
        .join()
        .unwrap_or_else(|_| Err("The setup task stopped unexpectedly.".into()))?;
    if let Some((source, path)) = adopted {
        let mut settings = Settings::load(&config)?;
        if source {
            settings.source = path;
        } else {
            settings.data = path;
        }
        settings.save(&config)?;
    }
    Ok(message)
}
