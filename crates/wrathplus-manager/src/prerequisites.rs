//! Background prerequisite checks and the client setup checklist.
use super::{preflight, style, Manager, Message};
use iced::widget::{button, column, row, text, text_input, Column};
use iced::{Element, Fill};

impl Manager {
    pub(super) fn build_ready(&self) -> bool {
        !self.busy
            && !self.checking
            && self.child.is_none()
            && self.preflight.as_ref().is_some_and(|report| report.ready())
    }

    pub(super) fn check_prerequisites(&mut self) -> iced::Task<Message> {
        self.checking = true;
        self.preflight = None;
        self.record("Checking whether your computer is ready to build.".into());
        let config = self.config.clone();
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            // Always notify the GUI, including after an unexpected worker panic.
            let result = std::panic::catch_unwind(|| preflight::inspect(&config));
            let report = result.unwrap_or_else(|_| preflight::Report {
                checks: vec![preflight::Check {
                    label: "Preflight worker".into(),
                    detail: "Couldn’t complete the check. Please try again.".into(),
                    status: preflight::Status::Blocked,
                }],
            });
            let _ = sender.send(report);
        });
        iced::Task::perform(async move { receiver.await.ok() }, |report| {
            Message::PreflightFinished(report.unwrap_or_else(|| preflight::Report {
                checks: vec![preflight::Check {
                    label: "Preflight worker".into(),
                    detail: "Couldn’t complete the check. Please try again.".into(),
                    status: preflight::Status::Blocked,
                }],
            }))
        })
    }

    pub(super) fn prerequisite_list(&self) -> Element<'_, Message> {
        let recheck = button(if self.checking {
            "Checking…"
        } else {
            "Check readiness"
        });
        let recheck = if !self.checking && !self.busy {
            recheck.on_press(Message::Recheck)
        } else {
            recheck
        };
        let source = text_input("WrathPlus source folder", &self.repo).padding(10);
        let source = if !self.checking && !self.busy && self.child.is_none() {
            source.on_input(Message::Repo)
        } else {
            source
        };
        let mut checks = Column::new().spacing(10);
        if let Some(report) = &self.preflight {
            for check in &report.checks {
                let (label, color) = match check.status {
                    preflight::Status::Pass => ("OK", style::ICE),
                    preflight::Status::Warning => ("WARN", style::GOLD),
                    preflight::Status::Blocked => {
                        ("MISSING", iced::Color::from_rgb8(222, 114, 114))
                    }
                };
                let mut item = column![row![
                    text(label).color(color).width(80),
                    text(&check.label).width(Fill)
                ]
                .spacing(10)]
                .spacing(4);
                // Successful checks need no implementation detail; keep remediation visible.
                if check.status != preflight::Status::Pass {
                    item = item.push(text(&check.detail).size(12).color(style::MUTED));
                }
                checks = checks.push(item);
            }
        } else {
            checks = checks.push(text("Check your computer before starting a new build.").size(12));
        }
        column![
            text("Ready to build?").size(16),
            text("WrathPlus source folder").size(12).color(style::MUTED),
            source,
            recheck,
            checks,
        ]
        .spacing(12)
        .into()
    }
}
