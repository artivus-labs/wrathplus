//! Build results outlive automatic status polls and other server operations.
use super::{worker, Action, Input, Server};
use crate::{style, Message};
use iced::{
    widget::{button, column, container, progress_bar, scrollable, text},
    Element, Fill,
};
use std::{path::PathBuf, time::Duration};

#[derive(Default)]
pub(super) struct Feedback {
    result: Option<Result<(), String>>,
    elapsed: Duration,
    path: Option<PathBuf>,
    log: String,
    expanded: bool,
}
impl Feedback {
    pub(super) fn finish(
        &mut self,
        result: &Result<String, String>,
        elapsed: Duration,
        path: Option<PathBuf>,
        log: String,
    ) {
        self.result = Some(result.as_ref().map(|_| ()).map_err(Clone::clone));
        self.elapsed = elapsed;
        self.path = path;
        self.log = log;
        self.expanded = false;
    }
    pub(super) fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }
    fn title(&self) -> Option<&'static str> {
        self.result.as_ref().map(|r| {
            if r.is_ok() {
                "Build complete — ready to restart"
            } else {
                "Build failed — changes were not applied"
            }
        })
    }
}
fn duration(value: Duration) -> String {
    let seconds = value.as_secs();
    format!(
        "{}h {:02}m {:02}s",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
impl Server {
    pub(crate) fn build_feedback_view(&self) -> Element<'_, Message> {
        let building = matches!(self.active, Some(Action::Build | Action::ForceBuild));
        let feedback = &self.build_feedback;
        let mut content = column![].spacing(10);
        if building {
            content = content.push(text("Building server…").size(19).color(style::ICE))
                .push(text("Please keep Manager open. Your current server and characters are unchanged.").size(13))
                .push(text(&self.stage).size(13))
                .push(text(format!("Elapsed {}", duration(self.started.map(|t| t.elapsed()).unwrap_or_default()))).size(13));
            if let Some((done, total)) = worker::count(&self.log) {
                content = content
                    .push(
                        text(format!(
                            "Compiling: {done} of {total} tasks (not a time estimate)"
                        ))
                        .size(13),
                    )
                    .push(progress_bar(0.0..=total as f32, done as f32));
            } else {
                content = content.push(text("Preparing, compiling or packaging the server. The first build can take over an hour.").size(13).color(style::MUTED));
            }
        } else if let Some(title) = feedback.title() {
            let success = matches!(feedback.result, Some(Ok(())));
            content = content.push(text(title).size(19).color(if success { iced::Color::from_rgb8(128, 210, 157) } else { style::GOLD }))
                .push(text(format!("Last build · {}", duration(feedback.elapsed))).size(12))
                .push(text(if success {
                    "The new build is saved. If the server is running, Stop it and then Start it from Server setup to use the changes. If it is stopped, just Start it. Nothing was restarted automatically."
                } else {
                    "Manager could not finish preparing the new server. Your running server and characters have not been changed. Check that Docker Desktop is running and there is free disk space, then try Rebuild. If it fails again, share the build details below."
                }).size(14))
                .push(button(if feedback.expanded { "Hide technical details" } else { "Show build details (optional)" }).on_press(Message::Server(Input::BuildLogs)));
            if feedback.expanded {
                if let Some(Err(error)) = &feedback.result {
                    content = content.push(text(error).size(12));
                }
                if let Some(path) = &feedback.path {
                    content = content.push(text(format!("Build log: {}", path.display())).size(12));
                }
                content = content.push(
                    scrollable(
                        text(if feedback.log.is_empty() {
                            "No build output was recorded."
                        } else {
                            &feedback.log
                        })
                        .font(iced::Font::MONOSPACE)
                        .size(12),
                    )
                    .height(200),
                );
            }
        } else {
            return column![].into();
        }
        container(content)
            .padding(18)
            .width(Fill)
            .style(style::panel)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manager;
    #[test]
    fn success_and_failure_survive_status_polls_and_other_actions() {
        let mut manager = Manager::default();
        manager.server_state.loaded = true;
        for result in [Ok("built".into()), Err("compiler failed".into())] {
            manager.server_state.active = Some(Action::ForceBuild);
            manager.server_state.busy = true;
            manager.server_state.started = Some(std::time::Instant::now());
            manager.server_state.log_path = Some("retained-build.log".into());
            manager.server_state.log = "build-specific output".into();
            let _ = manager.server_update(Input::Finished(result.clone()));
            let expected = if result.is_ok() {
                "Build complete"
            } else {
                "Build failed"
            };
            assert!(manager
                .server_state
                .build_feedback
                .title()
                .unwrap()
                .starts_with(expected));
            for action in [Action::Status, Action::Save, Action::Start] {
                manager.server_state.active = Some(action);
                manager.server_state.log_path = None;
                manager.server_state.log.clear();
                let _ = manager.server_update(Input::Finished(Ok("unrelated result".into())));
                assert!(manager
                    .server_state
                    .build_feedback
                    .title()
                    .unwrap()
                    .starts_with(expected));
                assert_eq!(
                    manager.server_state.build_feedback.path.as_deref(),
                    Some(std::path::Path::new("retained-build.log"))
                );
                assert_eq!(
                    manager.server_state.build_feedback.log,
                    "build-specific output"
                );
            }
            let _ = manager.server_update(Input::BuildLogs);
            assert!(manager.server_state.build_feedback.expanded);
        }
    }
}
