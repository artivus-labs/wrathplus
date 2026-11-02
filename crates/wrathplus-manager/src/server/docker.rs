//! Is Docker installed and its engine answering? Checked before every server action, so a stopped
//! Docker Desktop reads as "start Docker", not as a failed Compose command.
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Docker {
    #[default]
    Unknown,
    Ready,
    /// No `docker` command on this machine.
    Missing,
    /// The command exists but the engine does not answer (Docker Desktop not started yet).
    Stopped,
    /// The engine did not answer in time, usually because Docker Desktop is still starting.
    Slow,
}

impl Docker {
    pub(crate) fn blocked(&self) -> bool {
        matches!(self, Self::Missing | Self::Stopped | Self::Slow)
    }
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Missing => "Docker missing",
            Self::Stopped => "Docker stopped",
            Self::Slow => "Docker starting",
            Self::Unknown | Self::Ready => "",
        }
    }
    pub(crate) fn advice(&self) -> &'static str {
        match self {
            Self::Missing => "Docker is not installed. The server runs in Docker: install Docker Desktop from docker.com, start it, then press Refresh.",
            Self::Stopped if cfg!(target_os = "linux") => "Docker is installed but not running. Start it (for example `sudo systemctl start docker`), then press Refresh.",
            Self::Stopped => "Docker is installed but not running. Press Start Docker (or open Docker Desktop yourself) and wait for it to finish starting.",
            Self::Slow => "Docker is not answering yet. If Docker Desktop is still starting, wait a moment; this page checks again by itself.",
            Self::Unknown | Self::Ready => "",
        }
    }
    /// Whether this platform can open Docker Desktop for the player.
    pub(crate) fn can_start(&self) -> bool {
        matches!(self, Self::Stopped | Self::Slow) && !cfg!(target_os = "linux")
    }
}

/// `docker info` with a deadline; never a shell, never a container verb.
pub(crate) fn check() -> Docker {
    let mut command = Command::new("docker");
    command
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Docker::Missing,
        Err(_) => return Docker::Stopped,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut version = String::new();
                if let Some(out) = child.stdout.as_mut() {
                    let _ = out.take(1024).read_to_string(&mut version);
                }
                return classify(status.success(), &version);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Docker::Slow;
            }
        }
    }
}

/// `docker info` exits non-zero, or prints no server version, when the engine is down.
pub(super) fn classify(success: bool, version: &str) -> Docker {
    if success && !version.trim().is_empty() {
        Docker::Ready
    } else {
        Docker::Stopped
    }
}

/// Open Docker Desktop. Returns what to tell the player.
pub(crate) fn start() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.args(["-a", "Docker"]);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let base = std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
        Command::new(std::path::PathBuf::from(base).join(r"Docker\Docker\Docker Desktop.exe"))
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    return Err(Docker::Stopped.advice().into());
    #[cfg(any(target_os = "macos", windows))]
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| "Starting Docker Desktop · this can take a minute; the server page checks again by itself.".into())
        .map_err(|e| format!("Could not open Docker Desktop ({e}). Start it yourself, then press Refresh."))
}
