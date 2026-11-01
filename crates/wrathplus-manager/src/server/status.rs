//! Health is observed from Compose, never inferred from launching a process.
use super::{command, Action, Server};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Health {
    #[default]
    Unknown,
    Running,
    Stopped,
    Starting,
    Unhealthy,
}
impl Health {
    pub fn observed(rows: &[serde_json::Value]) -> Self {
        if command::ready(rows) {
            return Self::Running;
        }
        let servers: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r["Service"].as_str(), Some("authserver" | "worldserver")))
            .collect();
        if rows.iter().any(|r| r["Health"] == "unhealthy") {
            Self::Unhealthy
        } else if servers.is_empty() || servers.iter().all(|r| r["State"] == "exited") {
            Self::Stopped
        } else if command::failed(rows) {
            Self::Unhealthy
        } else {
            Self::Starting
        }
    }
}
impl Server {
    pub(crate) fn open_logs(&mut self) {
        self.show_logs = true;
    }
    pub(super) fn refresh_due(&self) -> bool {
        !self.busy
            && !self.choosing
            && self
                .next_refresh
                .is_none_or(|t| std::time::Instant::now() >= t)
    }
    pub(crate) fn label(&self) -> &'static str {
        match self.active {
            Some(Action::Start) => "Starting…",
            Some(Action::Stop) => "Stopping…",
            Some(Action::Build | Action::ForceBuild) => "Building…",
            _ if self.error.is_some() => "Check failed",
            _ => match self.health {
                Health::Unknown => "Checking…",
                Health::Running => "Running",
                Health::Stopped => "Stopped",
                Health::Starting => "Not ready",
                Health::Unhealthy => "Unhealthy",
            },
        }
    }
    pub(crate) fn detail(&self) -> &str {
        self.error.as_deref().unwrap_or(&self.stage)
    }
    pub(crate) fn control_input(&self) -> Option<super::Input> {
        (!self.busy && !self.choosing).then_some(super::Input::Run(
            if self.health == Health::Running {
                Action::Stop
            } else {
                Action::Start
            },
        ))
    }
    pub(crate) fn control_label(&self) -> &'static str {
        match self.active {
            Some(Action::Start) => "Starting…",
            Some(Action::Stop) => "Stopping…",
            _ if self.health == Health::Running => "Stop",
            _ => "Start",
        }
    }
}
