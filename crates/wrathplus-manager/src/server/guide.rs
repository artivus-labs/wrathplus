//! The first-install path on the Server page: source → build → data → start, then navigation
//! meshes. Each step says where it stands and offers the one action that moves it on.
use super::build_state::BuildState;
use super::data_prep::{self, Part};
use super::setup_job::SetupTask;
use super::{Action, Input, Server};
use crate::{style, Manager, Message};
use iced::widget::{button, column, container, progress_bar, row, text, Column};
use iced::{Element, Fill};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    Done,
    /// The step to do now (or in progress).
    Now,
    /// Waits for an earlier step.
    Later,
}

pub(super) struct Step {
    pub title: &'static str,
    pub mark: Mark,
    pub detail: String,
    pub action: Option<(&'static str, Option<Message>)>,
}

impl Step {
    fn new(title: &'static str, mark: Mark, detail: impl Into<String>) -> Self {
        Self {
            title,
            mark,
            detail: detail.into(),
            action: None,
        }
    }
    fn with(mut self, label: &'static str, message: Option<Message>) -> Self {
        self.action = Some((label, message));
        self
    }
}

impl Server {
    /// A Source folder the build can use: a core checkout with mod-playerbots in modules/. The
    /// build itself checks revisions and cleanliness.
    fn source_ready(&self) -> bool {
        let source = &self.settings.source;
        !source.as_os_str().is_empty()
            && source.join("CMakeLists.txt").is_file()
            && source.join("modules/mod-playerbots/src").is_dir()
    }
    fn data_ready(&self) -> bool {
        let data = &self.settings.data;
        !data.as_os_str().is_empty()
            && ["dbc", "maps", "vmaps"]
                .iter()
                .all(|d| data.join(d).is_dir())
    }
    fn navmesh_ready(&self) -> bool {
        data_prep::done(&self.settings.data, Part::Mmaps)
    }
    fn running_task(&self, task: SetupTask) -> Option<String> {
        let job = self.job().filter(|job| job.task == task)?;
        let elapsed = job.started.elapsed().as_secs();
        Some(format!(
            "{} · {}h {:02}m {:02}s",
            job.stage,
            elapsed / 3600,
            elapsed / 60 % 60,
            elapsed % 60
        ))
    }
    fn setup(&self, task: SetupTask) -> Option<Message> {
        self.can_setup(task)
            .then_some(Message::Server(Input::Setup(task)))
    }

    /// The steps, in order, for a player with `client` (whether the Client page holds a WoW
    /// client folder).
    pub(super) fn guide(&self, client: bool) -> [Step; 5] {
        let source = if let Some(stage) = self.running_task(SetupTask::Source) {
            Step::new("Get the server source", Mark::Now, stage)
        } else if self.source_ready() {
            Step::new(
                "Get the server source",
                Mark::Done,
                format!("AzerothCore with Playerbots · {}", self.source),
            )
        } else {
            Step::new("Get the server source", Mark::Now, "WrathPlus downloads AzerothCore with Playerbots (about 1 GB) into its own folder. It takes a few minutes.")
                .with("Get server source", self.setup(SetupTask::Source))
        };
        let building = matches!(self.active, Some(Action::Build | Action::ForceBuild));
        let build = if source.mark != Mark::Done {
            Step::new(
                "Build the server",
                Mark::Later,
                "After the source is in place.",
            )
        } else if building {
            Step::new(
                "Build the server",
                Mark::Now,
                "Building · the first build takes about an hour. Progress and the log are below.",
            )
        } else {
            match &self.build {
                BuildState::Present { .. } => Step::new("Build the server", Mark::Done, "Server built."),
                BuildState::Missing => Step::new("Build the server", Mark::Now, "Choose a server type below, then build. The first build takes about an hour; your computer stays usable.")
                    .with("Build", self.build.input(!self.locked() && !self.choosing).map(Message::Server)),
                BuildState::Unknown => Step::new("Build the server", Mark::Now, "Checking for a server build…"),
                BuildState::Unavailable(error) => Step::new("Build the server", Mark::Now, error.clone()),
            }
        };
        let built = matches!(self.build, BuildState::Present { .. });
        let data = if let Some(stage) = self.running_task(SetupTask::Data) {
            Step::new("Prepare the server data", Mark::Now, stage)
        } else if self.data_ready() {
            Step::new(
                "Prepare the server data",
                Mark::Done,
                format!("Maps and line-of-sight data · {}", self.data),
            )
        } else if !client {
            Step::new("Prepare the server data", Mark::Now, "The server reads its maps from your own WoW 3.3.5a client. Choose the client folder on the Client page first.")
                .with("Open Client page", Some(Message::Navigate(crate::Page::Client)))
        } else if !built {
            Step::new(
                "Prepare the server data",
                Mark::Later,
                "After the build: the server's own tools read the maps from your client.",
            )
        } else {
            Step::new("Prepare the server data", Mark::Now, "Reads maps and game tables from your client; nothing is downloaded and your client is not changed. A few minutes and about 1 GB.")
                .with("Prepare server data", self.setup(SetupTask::Data))
        };
        let start = if self.running() {
            Step::new(
                "Start and play",
                Mark::Done,
                "The server is running. Press Use this server for the client, then play.",
            )
        } else if data.mark != Mark::Done || !built {
            Step::new(
                "Start and play",
                Mark::Later,
                "After the build and the data.",
            )
        } else {
            Step::new("Start and play", Mark::Now, "Start the server (the first start sets up its database and takes a few minutes), then press Use this server for the client.")
                .with(self.control_label(), self.control_input().map(Message::Server))
        };
        let navmesh = if let Some(stage) = self.running_task(SetupTask::Navmesh) {
            Step::new("Navigation meshes (recommended)", Mark::Now, stage)
        } else if self.navmesh_ready() {
            Step::new(
                "Navigation meshes (recommended)",
                Mark::Done,
                "Bots and agents find their way with them.",
            )
        } else if data.mark != Mark::Done {
            Step::new(
                "Navigation meshes (recommended)",
                Mark::Later,
                "After the data. They help bots and agents find their way.",
            )
        } else {
            Step::new("Navigation meshes (recommended)", Mark::Now, "Help bots and agents find their way around. Making them takes several hours and about 6 GB; you can play meanwhile, and a stopped run continues where it left off. Restart the server afterwards.")
                .with("Make navigation meshes", self.setup(SetupTask::Navmesh))
        };
        [source, build, data, start, navmesh]
    }
}

impl Manager {
    /// The first-install card, while any step is still to do.
    pub(crate) fn server_guide(&self) -> Option<Element<'_, Message>> {
        let state = &self.server_state;
        let steps = state.guide(!self.config.data.as_os_str().is_empty());
        if steps.iter().all(|s| s.mark == Mark::Done) {
            return None;
        }
        let mut list = Column::new().spacing(14);
        for (index, step) in steps.into_iter().enumerate() {
            let (badge, colour) = match step.mark {
                Mark::Done => ("Done".to_owned(), style::ICE),
                Mark::Now => (format!("{}", index + 1), style::GOLD),
                Mark::Later => (format!("{}", index + 1), style::MUTED),
            };
            let mut body = column![
                text(step.title)
                    .size(16)
                    .color(if step.mark == Mark::Later {
                        style::MUTED
                    } else {
                        style::TEXT
                    }),
                text(step.detail).size(13).color(style::MUTED),
            ]
            .spacing(4)
            .width(Fill);
            if let Some(job) = state.job().filter(|_| step.mark == Mark::Now) {
                if let Some((label, done, total)) = job.progress().filter(|_| {
                    matches!(
                        (index, job.task),
                        (0, SetupTask::Source) | (2, SetupTask::Data) | (4, SetupTask::Navmesh)
                    )
                }) {
                    body = body
                        .push(text(format!("{label}: {done} / {total}")).size(12))
                        .push(progress_bar(0.0..=total as f32, done as f32).girth(5));
                }
            }
            let mut line = row![text(badge).size(13).color(colour).width(44), body]
                .spacing(12)
                .align_y(iced::Center);
            if let Some((label, message)) = step.action {
                line = line.push(button(label).style(style::primary).on_press_maybe(message));
            }
            list = list.push(line);
        }
        let mut card = column![
            text("Set up your own server").size(20),
            text("A private realm on this computer, with bots to play alongside. Follow the steps in order; each one says what it needs.").size(13).color(style::MUTED),
            list,
        ]
        .spacing(14);
        if let Some(job) = state.job() {
            card = card.push(text(job.task.running_label()).size(12).color(style::ICE));
            if let Some(path) = &job.log_path {
                card = card.push(
                    text(format!("Saved log: {}", path.display()))
                        .size(12)
                        .color(style::MUTED),
                );
            }
        } else if let Some(finished) = state.job_result() {
            let (message, colour) = match &finished.result {
                Ok(message) => (message.as_str(), style::ICE),
                Err(error) => (error.as_str(), style::GOLD),
            };
            card = card.push(text(message).size(13).color(colour));
            if let (Err(_), Some(path)) = (&finished.result, &finished.log) {
                card = card.push(
                    text(format!("Saved log: {}", path.display()))
                        .size(12)
                        .color(style::MUTED),
                );
            }
        }
        Some(
            container(card)
                .padding(18)
                .width(Fill)
                .style(style::panel)
                .into(),
        )
    }
}

#[cfg(test)]
#[path = "guide_tests.rs"]
mod tests;
