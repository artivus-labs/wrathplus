//! Bounded, explicit client setup. Assets are read-only; no downloads or realm operations.
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use wrathplus_mpq::Archive;

mod assets;
mod state_access;
pub(crate) use state_access::{check_output, with_stopped_state, write_state};
mod discovery;
mod locks;
mod login;
pub(crate) use discovery::discover;
#[cfg(test)]
use locks::running_path;
use locks::{operation_lock, require_stopped, running_paths, OperationLock};
mod paths;
mod process;
pub(crate) use assets::validate;
#[cfg(test)]
use assets::{check_component, check_dbc};
use paths::{checked_config, resolved, writable};
pub(crate) use process::stop;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) repo: PathBuf,
    pub(crate) data: PathBuf,
    pub(crate) locale: String,
    pub(crate) server: String,
    pub(crate) target: PathBuf,
}

impl Config {
    pub(crate) fn defaults() -> Self {
        let repo = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            data: env::var_os("WOW_DATA")
                .map(PathBuf::from)
                .unwrap_or_default(),
            target: env::var_os("CARGO_TARGET_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| repo.join("target")),
            repo,
            locale: "enUS".into(),
            server: "127.0.0.1:3724".into(),
        }
    }

    /// Same non-moving legacy fallback as the client. Manager deliberately ignores inherited
    /// BENILLA_HOME and pins this checkout's selected home when launching.
    pub(crate) fn state_root(&self) -> PathBuf {
        wrathplus_formats::default_state_home(&self.repo)
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.state_root().join("manager.json")
    }

    pub(crate) fn binary_path(&self) -> PathBuf {
        self.target.join("manager/bin").join(binary_name())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Validated {
    pub(crate) config: Config,
    pub(crate) summary: String,
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "wrathplus.exe"
    } else {
        "wrathplus"
    }
}

pub(super) fn io_error(context: impl std::fmt::Display, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

fn ensure_dir(config: &Config, path: &Path) -> Result<PathBuf> {
    let path = writable(config, path)?;
    fs::create_dir_all(&path).map_err(|e| io_error(path.display(), e))?;
    writable(config, &path)
}

static SERIAL: AtomicU64 = AtomicU64::new(0);
fn stage_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

fn atomic_write(config: &Config, path: &Path, bytes: &[u8]) -> Result<()> {
    let path = writable(config, path)?;
    ensure_dir(config, path.parent().ok_or("State path needs a parent")?)?;
    let stage = writable(config, &stage_path(&path))?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&stage)
            .map_err(|e| io_error(stage.display(), e))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| io_error("write staged settings", e))?;
        fs::rename(&stage, &path).map_err(|e| io_error("promote staged settings", e))
    })();
    let _ = fs::remove_file(stage);
    result
}

/// Explicit state path, no environment or implicit discovery. A configuration is usable only
/// with an installed managed binary; build invokes this after successful promotion.
pub(crate) fn save_config(config: &Config, state_path: &Path) -> Result<()> {
    let config = validate(config)?.config;
    if discover(&config).is_none() {
        return Err("Build the managed client before saving setup".into());
    }
    let bytes =
        serde_json::to_vec_pretty(&config).map_err(|e| io_error("encode manager settings", e))?;
    atomic_write(&config, state_path, &bytes)
}

pub(crate) fn load_config(state_path: &Path) -> Result<Config> {
    let mut bytes = Vec::new();
    File::open(state_path)
        .and_then(|f| f.take(64 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| io_error(format!("load {}", state_path.display()), e))?;
    if bytes.len() > 64 * 1024 {
        return Err("Manager settings exceed 64 KiB".into());
    }
    let config: Config =
        serde_json::from_slice(&bytes).map_err(|e| io_error("decode manager settings", e))?;
    let config = checked_config(&config)?;
    writable(&config, state_path)?;
    if discover(&config).is_none() {
        return Err("Saved managed binary is missing; rebuild the client".into());
    }
    Ok(config)
}

/// Convenience for the UI; explicit-path persistence remains available to callers and tests.
pub(crate) fn load(config: &Config) -> Result<Config> {
    let saved = load_config(&config.state_path())?;
    if saved.repo != resolved(&config.repo)? {
        return Err("Saved Manager configuration belongs to another checkout".into());
    }
    Ok(saved)
}

/// Cheap readiness indicator, not asset certification; launch always revalidates the archives.
pub(crate) fn available(config: &Config) -> bool {
    checked_config(config).is_ok_and(|canonical| {
        discover(&canonical).is_some() && require_stopped(&canonical).is_ok()
    })
}

fn sanitize(command: &mut Command) {
    for (key, _) in env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("WOW_") || upper.starts_with("BENILLA_") || upper.contains("PROBE") {
            command.env_remove(key);
        }
    }
}

fn build_command(config: &Config, cargo: &Path) -> Command {
    let mut command = Command::new(cargo);
    command
        .args([
            "build",
            "--locked",
            "-p",
            "wrathplus",
            "--bin",
            "wrathplus",
            "--no-default-features",
            "--profile",
            "play",
            "--jobs",
            "2",
            "--manifest-path",
        ])
        .arg(config.repo.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&config.target)
        .current_dir(&config.repo)
        .env("CARGO_INCREMENTAL", "0")
        .stdin(Stdio::null());
    sanitize(&mut command);
    // Prevent an inherited cross target from changing the expected play/wrathplus artifact.
    command.env_remove("CARGO_BUILD_TARGET");
    command
}

/// Validate, build to a logged staging artifact, atomically promote and then persist setup.
/// A failed build never touches either the prior binary or prior successful configuration.
pub(crate) fn build(config: &Config) -> Result<String> {
    build_with_log(config, |_| {})
}

pub(crate) fn build_with_log(config: &Config, log_ready: impl FnOnce(PathBuf)) -> Result<String> {
    let report = crate::preflight::inspect(config);
    if !report.ready() {
        return Err(report.summary());
    }
    let cargo = crate::preflight::cargo(&config.repo)?;
    let validated = validate(config)?;
    let config = &validated.config;
    let _lock = operation_lock(config)?;
    require_stopped(config)?;
    let state_root = ensure_dir(config, &config.state_root())?;
    let log_path = writable(config, &stage_path(&state_root.join("manager-build.log")))?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)
        .map_err(|e| io_error("create build log", e))?;
    log_ready(log_path.clone());
    let mut command = build_command(config, &cargo);
    command.env("RUSTC", crate::preflight::rustc(&config.repo)?);
    command
        .stdout(
            log.try_clone()
                .map_err(|e| io_error("clone build log", e))?,
        )
        .stderr(log);
    let status = command
        .status()
        .map_err(|e| io_error(format!("start cargo; log {}", log_path.display()), e))?;
    if !status.success() {
        return Err(format!(
            "Client build failed ({status}); prior installation preserved. Log: {}",
            log_path.display()
        ));
    }
    require_stopped(config)?;
    let source = writable(config, &config.target.join("play").join(binary_name()))?;
    install(config, &source)?;
    Ok(format!(
        "{}\nInstalled {}\nBuild log: {}",
        validated.summary,
        config.binary_path().display(),
        log_path.display()
    ))
}

fn install(config: &Config, source: &Path) -> Result<()> {
    let destination = writable(config, &config.binary_path())?;
    ensure_dir(
        config,
        destination
            .parent()
            .ok_or("Binary needs a parent directory")?,
    )?;
    let stage = writable(config, &stage_path(&destination))?;
    let backup = writable(config, &destination.with_extension("backup"))?;
    if backup.exists() {
        return Err(format!(
            "Recovery backup already exists: {}; resolve it before rebuilding",
            backup.display()
        ));
    }
    let result = (|| {
        let mut input = File::open(source).map_err(|e| io_error("open built client", e))?;
        let metadata = input
            .metadata()
            .map_err(|e| io_error("built client metadata", e))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err("Built client is not a nonempty regular file".into());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|e| io_error("create staged binary", e))?;
        std::io::copy(&mut input, &mut output)
            .and_then(|_| output.sync_all())
            .map_err(|e| io_error("stage built client", e))?;
        fs::set_permissions(&stage, metadata.permissions())
            .map_err(|e| io_error("set client permissions", e))?;
        let had_previous = destination
            .try_exists()
            .map_err(|e| io_error("inspect prior client", e))?;
        if had_previous {
            fs::rename(&destination, &backup).map_err(|e| io_error("back up prior client", e))?;
        }
        let promoted = fs::rename(&stage, &destination)
            .map_err(|e| io_error("install client", e))
            .and_then(|_| save_config(config, &config.state_path()));
        if let Err(error) = promoted {
            let _ = fs::remove_file(&destination);
            if had_previous {
                fs::rename(&backup, &destination).map_err(|e| {
                    format!(
                        "{error}; rollback failed: {e}; prior binary retained at {}",
                        backup.display()
                    )
                })?;
            }
            return Err(error);
        }
        if had_previous {
            let _ = fs::remove_file(&backup);
        }
        Ok(())
    })();
    let _ = fs::remove_file(stage);
    result
}

fn launch_command(config: &Config, state_root: &Path) -> Command {
    let mut command = Command::new(discover(config).unwrap_or_else(|| config.binary_path()));
    sanitize(&mut command);
    command
        .current_dir(state_root)
        .env("WOW_DATA", &config.data)
        .env("WOW_CLIENT_PROFILE", "wrath")
        .env("WOW_LOCALE", &config.locale)
        .env("WOW_HOST", &config.server)
        .env("BENILLA_HOME", state_root)
        .args(["--server", &config.server])
        .stdin(Stdio::null());
    command
}

/// Called only in response to an explicit Play action. Parent retains Child and polls via reap.
pub(crate) fn launch(config: &Config) -> Result<Child> {
    let config = validate(config)?.config;
    let _lock = operation_lock(&config)?;
    require_stopped(&config)?;
    if discover(&config).is_none() {
        return Err("No playable build found; build the client first".into());
    }
    save_config(&config, &config.state_path())?;
    let state_root = ensure_dir(&config, &config.state_root())?;
    let log_path = writable(&config, &stage_path(&state_root.join("manager-client.log")))?;
    let mut markers = OperationLock(Vec::new());
    let mut files = Vec::new();
    for marker in running_paths(&config) {
        let marker = writable(&config, &marker)?;
        if markers.0.contains(&marker) {
            continue;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .map_err(|e| io_error("create running marker", e))?;
        markers.0.push(marker);
        files.push(file);
    }
    let result = (|| {
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(log_path)
            .map_err(|e| io_error("create client log", e))?;
        let mut command = launch_command(&config, &state_root);
        login::configure(&mut command, &config)?;
        command
            .stdout(
                log.try_clone()
                    .map_err(|e| io_error("clone client log", e))?,
            )
            .stderr(log);
        command
            .spawn()
            .map_err(|e| io_error("launch managed client", e))
    })();
    match result {
        Ok(child) => {
            // The already-created marker fences replacement even if recording the PID fails.
            for mut file in files {
                let _ = writeln!(file, "{}", child.id());
            }
            markers.0.clear();
            Ok(child)
        }
        Err(error) => Err(error),
    }
}

/// true = exited and marker released; false = still running. Use the launch-time Config, even
/// if the UI fields have since changed. Never clear a marker just because the Manager exits.
pub(crate) fn reap(config: &Config, child: &mut Child) -> Result<bool> {
    if child
        .try_wait()
        .map_err(|e| io_error("poll managed client", e))?
        .is_none()
    {
        return Ok(false);
    }
    let config = checked_config(config)?;
    let _lock = operation_lock(&config)?;
    let mut markers = running_paths(&config)
        .iter()
        .map(|path| writable(&config, path))
        .collect::<Result<Vec<_>>>()?;
    markers.sort();
    markers.dedup();
    // Verify every marker before releasing either resource.
    for marker in &markers {
        let contents =
            fs::read_to_string(marker).map_err(|e| io_error("read running marker", e))?;
        if contents.trim() != child.id().to_string() {
            return Err("Running marker belongs to another child; refusing to remove it".into());
        }
    }
    for marker in markers {
        fs::remove_file(marker).map_err(|e| io_error("release running marker", e))?;
    }
    Ok(true)
}

#[cfg(test)]
#[path = "setup_tests.rs"]
pub(crate) mod tests;
