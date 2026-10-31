//! Build activity and bounded diagnostics; no estimated completion percentage.
use super::{style, Manager, Message};
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Element, Fill, Length};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

const LOG_LIMIT: u64 = 32 * 1024;

#[derive(Default)]
pub(super) struct Feedback {
    pub started: Option<Instant>,
    pub finished: Option<Instant>,
    pub receiver: Option<Receiver<PathBuf>>,
    pub path: Option<PathBuf>,
    pub log: String,
    pub error: Option<String>,
    pub expanded: bool,
    last_change: Option<Instant>,
    phase: u16,
    read_error: Option<String>,
}

fn tail(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let offset = size.saturating_sub(LOG_LIMIT);
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(LOG_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    // Drop a partial first line (including a possible partial UTF-8 character).
    let start = if offset > 0 {
        bytes.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1)
    } else {
        0
    };
    let body = String::from_utf8_lossy(&bytes[start..]);
    Ok(if offset > 0 {
        format!("[Earlier output omitted; full log is saved on disk.]\n{body}")
    } else {
        body.into_owned()
    })
}

impl Feedback {
    pub fn refresh(&mut self) {
        self.phase = (self.phase + 1) % 20;
        if let Some(receiver) = &self.receiver {
            if let Ok(path) = receiver.try_recv() {
                self.path = Some(path);
            }
        }
        if let Some(path) = &self.path {
            match tail(path) {
                Ok(log) => {
                    if log != self.log {
                        self.last_change = Some(Instant::now());
                    }
                    self.log = log;
                    self.read_error = None;
                }
                Err(error) => {
                    self.read_error = Some(format!("Couldn’t read the build log: {error}"))
                }
            }
        }
    }

    pub fn copy_text(&self) -> String {
        format!(
            "WrathPlus client build\n{}\nLog: {}\n{}{}",
            self.error.as_deref().unwrap_or("Build output"),
            self.path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Not yet available".into()),
            self.read_error.as_deref().unwrap_or(""),
            self.log
        )
    }
}

impl Manager {
    pub(super) fn build_feedback(&self) -> Element<'_, Message> {
        let feedback = &self.build_feedback;
        let Some(started) = feedback.started else {
            return column![].into();
        };
        let elapsed = feedback
            .finished
            .unwrap_or_else(Instant::now)
            .duration_since(started)
            .as_secs();
        let mut panel =
            column![text(format!("Elapsed: {}m {:02}s", elapsed / 60, elapsed % 60)).size(13)]
                .spacing(10);
        if self.busy {
            // A traveling pulse indicates activity, not an invented completion fraction.
            let left = feedback.phase.min(20 - feedback.phase) + 1;
            panel = panel.push(
                row![
                    space().width(Length::FillPortion(left)),
                    container(space().height(6))
                        .width(Length::FillPortion(3))
                        .style(|_| container::Style::default().background(style::ICE)),
                    space().width(Length::FillPortion(12 - left))
                ]
                .width(Fill),
            );
            let quiet = feedback.last_change.unwrap_or(started).elapsed().as_secs();
            panel = panel.push(text(if quiet >= 30 {
                format!("Still working · no new output for {quiet}s. Some build steps can take several minutes.")
            } else { "Build in progress…".into() }).size(12));
            if let Some(line) = feedback
                .log
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
            {
                panel = panel.push(text(line).size(12));
            }
        } else if feedback.error.is_none() {
            panel = panel.push(text("Your client is ready to launch.").color(style::ICE));
        }
        if let Some(error) = &feedback.error {
            panel = panel.push(text(format!("Build failed: {error}")).color(iced::Color::from_rgb8(222, 114, 114)))
                .push(text("Your previous client is unchanged. Copy the details below if you need help.").size(12));
        }
        panel = panel.push(
            row![
                button(if feedback.expanded {
                    "Hide build log"
                } else {
                    "Show build log"
                })
                .on_press(Message::ToggleBuildLog),
                button("Copy build details").on_press(Message::CopyBuildLog)
            ]
            .spacing(10),
        );
        if feedback.expanded {
            if let Some(path) = &feedback.path {
                panel = panel.push(text(format!("Full log: {}", path.display())).size(12));
            }
            if let Some(error) = &feedback.read_error {
                panel = panel.push(text(error).size(12));
            }
            panel = panel.push(
                scrollable(
                    text(if feedback.log.is_empty() {
                        "Preparing build…"
                    } else {
                        &feedback.log
                    })
                    .font(iced::Font::MONOSPACE)
                    .size(12),
                )
                .height(240),
            );
        }
        panel.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_is_bounded_and_handles_utf8_and_missing_files() {
        let path = std::env::temp_dir().join(format!("manager-tail-{}", std::process::id()));
        std::fs::write(
            &path,
            format!("{}\nCompiling client\nerror: example\n", "é".repeat(40_000)),
        )
        .unwrap();
        let log = tail(&path).unwrap();
        assert!(log.len() < LOG_LIMIT as usize + 100);
        assert!(log.contains("Earlier output omitted"));
        assert!(log.ends_with("error: example\n"));
        std::fs::write(&path, "small log\n").unwrap();
        assert_eq!(tail(&path).unwrap(), "small log\n");
        std::fs::remove_file(&path).unwrap();
        assert!(tail(&path).is_err());
    }
    #[test]
    fn feedback_tracks_exact_log_not_previous_builds() {
        let mut feedback = Feedback::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        feedback.receiver = Some(receiver);
        sender
            .send(PathBuf::from("/nonexistent/this-build.log"))
            .unwrap();
        feedback.refresh();
        assert!(feedback.read_error.is_some());
        feedback.error = Some("example failure".into());
        assert!(feedback.copy_text().contains("example failure"));
        assert!(feedback.copy_text().contains("this-build.log"));
    }
}
