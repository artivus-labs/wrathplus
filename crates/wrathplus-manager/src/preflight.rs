//! Read-only tool diagnostics plus disposable write probes outside the selected install.
use crate::setup;
use std::path::{Path, PathBuf};

mod filesystem;
mod platform;
#[cfg(test)]
mod tests;
mod tools;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Pass,
    Warning,
    Blocked,
}

#[derive(Clone, Debug)]
pub(crate) struct Check {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) status: Status,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Report {
    pub(crate) checks: Vec<Check>,
}

impl Report {
    pub(crate) fn ready(&self) -> bool {
        !self.checks.iter().any(|c| c.status == Status::Blocked)
    }

    pub(crate) fn summary(&self) -> String {
        self.checks
            .iter()
            .map(|c| format!("[{:?}] {}: {}", c.status, c.label, c.detail))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn add(&mut self, label: &str, status: Status, detail: impl Into<String>) {
        self.checks.push(Check {
            label: label.into(),
            status,
            detail: detail.into(),
        });
    }

    fn result(&mut self, label: &str, result: Result<String, String>) {
        match result {
            Ok(detail) => self.add(label, Status::Pass, detail),
            Err(detail) => self.add(label, Status::Blocked, detail),
        }
    }
}

/// Discovers an executable, not a shell alias. Does not invoke cargo or install Rust.
pub(crate) fn cargo(repo: &Path) -> Result<PathBuf, String> {
    tools::discover("cargo", repo).ok_or_else(||
        "Cargo executable not found in PATH, CARGO_HOME/bin, or HOME/USERPROFILE/.cargo/bin; install the repository's pinned Rust toolchain".into())
}

/// Match rustc to the discovered Cargo, including Finder/minimal-PATH launches.
pub(crate) fn rustc(repo: &Path) -> Result<PathBuf, String> {
    let cargo = cargo(repo)?;
    cargo
        .parent()
        .and_then(|parent| tools::in_dirs("rustc", &[parent.to_owned()], cfg!(windows), ".EXE"))
        .or_else(|| tools::discover("rustc", repo))
        .ok_or_else(|| "rustc executable not found alongside Cargo or in tool search paths".into())
}

pub(crate) fn inspect(config: &setup::Config) -> Report {
    let mut report = Report::default();
    if cfg!(windows) && platform::wsl_path(&config.repo.to_string_lossy()) {
        report.add("Windows repository path", Status::Blocked,
            "WSL/Linux path selected in Windows Manager; choose a native Windows checkout or run Manager inside WSL");
    }
    let repo = match filesystem::resolve(&config.repo) {
        Ok(repo) => repo,
        Err(e) => {
            report.add("Repository", Status::Blocked, e);
            return report;
        }
    };
    report.result(
        "Repository",
        if repo.join("Cargo.toml").is_file() && repo.join("crates/wrathplus/Cargo.toml").is_file() {
            Ok(repo.display().to_string())
        } else {
            Err("Select the client workspace checkout".into())
        },
    );
    tools::rust(&repo, &mut report);
    platform::native(&repo, &mut report);
    filesystem::inspect(config, &repo, &mut report);
    report
}
