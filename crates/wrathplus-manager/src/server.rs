//! Native managed disposable server workflow; no existing-realm adoption or reset.
mod build_state;
mod command;
mod settings;
mod status;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod tests;
mod ui;
mod worker;

use crate::{Manager, Message};
use settings::Settings;
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
    ToggleLogs,
}
pub struct Server {
    settings: Settings,
    health: status::Health,
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
    pulse: u16,
    next_refresh: Option<Instant>,
}
impl Default for Server {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            source: settings.source.to_string_lossy().into_owned(),
            data: String::new(),
            settings,
            health: status::Health::Unknown,
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
            pulse: 0,
            next_refresh: None,
        }
    }
}
impl Server {
    pub fn refresh(&mut self) {
        if let Some(receiver) = &self.receiver {
            for event in receiver.try_iter() {
                match event {
                    worker::Event::Log(path) => self.log_path = Some(path),
                    worker::Event::Stage(stage) => self.stage = stage,
                    worker::Event::Health(health) => self.health = health,
                    worker::Event::Build(build) => self.build = build,
                }
            }
        }
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
        iced::Task::none()
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
            Input::Source(value) if !state.busy => {
                state.source = value.clone();
                state.settings.source = value.into();
                state.build = build_state::BuildState::Unknown;
                state.next_refresh = None;
            }
            Input::Data(value) if !state.busy => {
                state.data = value.clone();
                state.settings.data = value.into();
            }
            Input::Browse if !state.busy && !state.choosing => {
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
            Input::Run(action) if !state.busy && !state.choosing => {
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
                if !matches!(action, Some(Action::Status)) {
                    self.events.push(match &result {
                        Ok(message) => format!("Server: {message}"),
                        Err(error) => format!("Server failed: {error}"),
                    });
                }
                state.finished = Some(Instant::now());
                state.next_refresh = Some(Instant::now() + std::time::Duration::from_secs(10));
                match result {
                    Ok(message) => state.stage = message,
                    Err(error) => {
                        state.stage = "Failed · logs retained".into();
                        state.error = Some(error);
                    }
                }
            }
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
