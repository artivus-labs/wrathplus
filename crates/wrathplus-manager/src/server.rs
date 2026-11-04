//! Native managed disposable server workflow; no existing-realm adoption or reset.
mod build_feedback;
mod build_state;
mod characters;
#[cfg(test)]
mod characters_tests;
mod characters_ui;
mod chatter;
mod command;
mod data_prep;
mod docker;
mod environment;
#[cfg(test)]
mod environment_tests;
mod environment_ui;
mod guide;
mod module_settings;
mod settings;
mod setup_job;
mod source;
mod status;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod tests;
mod ui;
mod worker;

use crate::{Manager, Message};
pub(crate) use data_prep::Navmesh;
use settings::Settings;
pub use setup_job::SetupTask;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Save,
    Build,
    ForceBuild,
    Start,
    Stop,
    Status,
}
#[derive(Debug, Clone)]
pub enum Input {
    Source(String),
    Data(String),
    Browse,
    Folder(Option<PathBuf>),
    Run(Action),
    Finished(Result<String, String>),
    Connect,
    StartDocker,
    ToggleLogs,
    BuildLogs,
    Transfer(characters_ui::TransferInput),
    /// Start a first-install task (source, data or navigation meshes).
    Setup(SetupTask),
    SetupFinished(SetupTask, Result<String, String>),
    /// Edit the server settings (typed module settings and environment variables).
    Environment(environment_ui::EnvInput),
}
pub struct Server {
    settings: Settings,
    health: status::Health,
    docker: docker::Docker,
    build: build_state::BuildState,
    active: Option<Action>,
    source: String,
    data: String,
    busy: bool,
    choosing: bool,
    loaded: bool,
    stage: String,
    started: Option<Instant>,
    finished: Option<Instant>,
    receiver: Option<Receiver<worker::Event>>,
    log_path: Option<PathBuf>,
    log: String,
    show_logs: bool,
    error: Option<String>,
    build_feedback: build_feedback::Feedback,
    pulse: u16,
    next_refresh: Option<Instant>,
    /// An action pressed while a status check was in flight; it runs when the check returns.
    queued: Option<Action>,
    transfer: characters_ui::Transfer,
    /// The first-install task running beside server operations, if any.
    job: Option<setup_job::Job>,
    /// How the last first-install task ended.
    job_result: Option<setup_job::Finished>,
    env_form: environment_ui::EnvForm,
}
impl Default for Server {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            source: settings.source.to_string_lossy().into_owned(),
            data: String::new(),
            settings,
            health: status::Health::Unknown,
            docker: docker::Docker::Unknown,
            build: build_state::BuildState::Unknown,
            active: None,
            busy: false,
            choosing: false,
            loaded: false,
            stage: "Not checked · managed fresh instance only".into(),
            started: None,
            finished: None,
            receiver: None,
            log_path: None,
            log: String::new(),
            show_logs: false,
            error: None,
            build_feedback: build_feedback::Feedback::default(),
            pulse: 0,
            next_refresh: None,
            queued: None,
            transfer: characters_ui::Transfer::default(),
            job: None,
            job_result: None,
            env_form: environment_ui::EnvForm::default(),
        }
    }
}
impl Server {
    pub(crate) fn module_source(&self) -> &std::path::Path {
        &self.settings.source
    }
    pub(crate) fn module_editable(&self) -> bool {
        !self.locked() && !self.choosing
    }
    pub(crate) fn module_build_input(&self, enabled: bool) -> Option<Input> {
        self.build.input(enabled && self.module_editable())
    }
    pub(crate) fn module_build_label(&self) -> &'static str {
        if matches!(self.active, Some(Action::Build | Action::ForceBuild)) {
            "Building…"
        } else {
            self.build.label()
        }
    }
    pub fn refresh(&mut self) {
        if let Some(receiver) = &self.receiver {
            for event in receiver.try_iter() {
                match event {
                    worker::Event::Log(path) => self.log_path = Some(path),
                    worker::Event::Stage(stage) => self.stage = stage,
                    worker::Event::Health(health) => self.health = health,
                    worker::Event::Build(build) => self.build = build,
                    worker::Event::Docker(docker) => self.docker = docker,
                    worker::Event::Source(_) | worker::Event::Data(_) => {}
                }
            }
        }
        self.refresh_job();
        self.pulse = (self.pulse + 1) % 20;
        if let Some(path) = &self.log_path {
            match worker::tail(path) {
                Ok(log) => self.log = log,
                Err(e) => self.error = Some(format!("Read retained log: {e}")),
            }
        }
    }
}
impl Manager {
    pub(super) fn poll_server(&mut self) -> iced::Task<Message> {
        self.server_state.refresh();
        if self.server_state.refresh_due() {
            return self.server_update(Input::Run(Action::Status));
        }
        self.transfer_poll()
    }
    pub(super) fn load_server(&mut self) {
        if self.server_state.loaded {
            return;
        }
        self.server_state.loaded = true;
        match Settings::load(&self.config) {
            Ok(settings) => {
                self.server_state.source = settings.source.to_string_lossy().into_owned();
                self.server_state.data = settings.data.to_string_lossy().into_owned();
                self.server_state.settings = settings;
            }
            Err(e) => self.server_state.error = Some(e),
        }
    }
    pub(super) fn server_update(&mut self, input: Input) -> iced::Task<Message> {
        self.load_server();
        let state = &mut self.server_state;
        match input {
            Input::Source(value) if !state.locked() && !self.module_state.busy => {
                state.source = value.clone();
                state.settings.source = value.into();
                state.build = build_state::BuildState::Unknown;
                state.next_refresh = None;
            }
            Input::Data(value) if !state.locked() && !self.module_state.busy => {
                state.data = value.clone();
                state.settings.data = value.into();
            }
            Input::Browse if !state.locked() && !state.choosing => {
                state.choosing = true;
                return iced::Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Extracted server data (dbc, maps, vmaps) — read only")
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_owned())
                    },
                    |p| Message::Server(Input::Folder(p)),
                );
            }
            Input::Folder(path) => {
                state.choosing = false;
                if let Some(path) = path {
                    return self.server_update(Input::Data(path.to_string_lossy().into_owned()));
                }
            }
            Input::Run(action)
                if state.checking_quietly()
                    && !matches!(action, Action::Status)
                    && !state.choosing
                    && !self.module_state.busy =>
            {
                state.queued = Some(action);
            }
            Input::Run(action) if !state.busy && !state.choosing && !self.module_state.busy => {
                state.active = Some(action);
                state.busy = true;
                state.started = Some(Instant::now());
                state.finished = None;
                state.error = None;
                state.log.clear();
                state.log_path = None;
                state.stage = "Checking settings and Docker prerequisites".into();
                let (events, receiver) = std::sync::mpsc::channel();
                state.receiver = Some(receiver);
                let (sender, done) = iced::futures::channel::oneshot::channel();
                let config = self.config.clone();
                let settings = state.settings.clone();
                std::thread::spawn(move || {
                    let result =
                        std::panic::catch_unwind(|| worker::run(config, settings, action, events))
                            .unwrap_or_else(|_| {
                                Err(
                            "Server worker panicked. Inspect retained log; no reset performed."
                                .into(),
                        )
                            });
                    let _ = sender.send(result);
                });
                return iced::Task::perform(
                    async move {
                        done.await.unwrap_or_else(|_| Err("Server worker disconnected; inspect retained logs and refresh status.".into()))
                    },
                    |r| Message::Server(Input::Finished(r)),
                );
            }
            Input::Finished(result) => {
                state.refresh();
                state.busy = false;
                let action = state.active.take();
                if matches!(action, Some(Action::Build | Action::ForceBuild)) {
                    state.build_feedback.finish(
                        &result,
                        state.started.map(|t| t.elapsed()).unwrap_or_default(),
                        state.log_path.clone(),
                        state.log.clone(),
                    );
                }
                if !matches!(action, Some(Action::Status)) {
                    self.events.push(match &result {
                        Ok(message) => format!("Server: {message}"),
                        Err(error) => format!("Server failed: {error}"),
                    });
                }
                state.finished = Some(Instant::now());
                // While Docker is down, look again sooner so the page clears as soon as it answers.
                let wait = if state.docker.blocked() { 3 } else { 10 };
                state.next_refresh = Some(Instant::now() + std::time::Duration::from_secs(wait));
                match result {
                    Ok(message) => state.stage = message,
                    Err(error) => {
                        state.stage = "Failed · logs retained".into();
                        state.error = Some(error);
                    }
                }
                if let Some(queued) = state.queued.take() {
                    return self.server_update(Input::Run(queued));
                }
            }
            Input::StartDocker if !state.busy => {
                let started = docker::start();
                self.events.push(match &started {
                    Ok(message) => format!("Server: {message}"),
                    Err(error) => format!("Server: {error}"),
                });
                match started {
                    Ok(message) => {
                        state.docker = docker::Docker::Slow;
                        state.error = None;
                        state.stage = message;
                    }
                    Err(error) => state.error = Some(error),
                }
                state.next_refresh = Some(Instant::now() + std::time::Duration::from_secs(3));
            }
            Input::Environment(environment_ui::EnvInput::ToggleAdvanced) => {
                state.environment_update(environment_ui::EnvInput::ToggleAdvanced)
            }
            Input::Environment(input) if !state.locked() && !self.module_state.busy => {
                state.environment_update(input)
            }
            Input::Transfer(input) => return self.transfer_update(input),
            Input::Setup(task) => return self.setup_start(task, Navmesh::All),
            Input::SetupFinished(task, result) => self.setup_finished(task, result),
            Input::BuildLogs => state.build_feedback.toggle(),
            Input::ToggleLogs => state.show_logs = !state.show_logs,
            Input::Connect => {
                if self.child.is_some() {
                    self.record("Stop the client before changing its server endpoint.".into());
                } else {
                    self.host = "127.0.0.1:13724".into();
                    self.sync_config();
                    self.record("Client endpoint explicitly set to 127.0.0.1:13724. Use Client → Validate / save settings to persist. No game launched.".into());
                }
            }
            _ => {}
        }
        iced::Task::none()
    }
}

/// One worldserver command as the Manager's admin, over the realm's local command port (the same
/// path character transfer uses). Blocking; the realm must be running.
pub(crate) fn admin_command(
    config: &crate::setup::Config,
    command: &str,
) -> Result<String, String> {
    characters::soap(config, command)
}

/// The managed realm's navmesh (`<server data>/mmaps`), for agent clients' `AgentMoveTo`.
pub(crate) fn mmaps(config: &crate::setup::Config) -> Option<std::path::PathBuf> {
    let data = settings::Settings::load(config).ok()?.data;
    let dir = data.join("mmaps");
    (!data.as_os_str().is_empty() && dir.is_dir()).then_some(dir)
}

/// Run a first-install task from the command line (see `--get-server-source`).
pub(crate) fn run_setup_cli(
    config: crate::setup::Config,
    task: SetupTask,
    scope: Navmesh,
) -> Result<String, String> {
    setup_job::run_cli(config, task, scope)
}
