//! Blocking Docker work and file-backed output, never executed on the Iced event thread.
use super::{command, settings::Settings, Action};
use crate::setup;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum Event {
    Log(PathBuf),
    Stage(String),
    Health(super::status::Health),
    Build(super::build_state::BuildState),
    Docker(super::docker::Docker),
    /// A setup task made this folder the Source folder.
    Source(PathBuf),
    /// A setup task made this folder the Server data folder.
    Data(PathBuf),
}
type Result<T> = std::result::Result<T, String>;
pub(super) fn execute(mut command: Command, log: &File) -> Result<()> {
    let status = command.stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log.try_clone().map_err(|e| e.to_string())?)
        .status().map_err(|e| format!("Could not run Docker: {e}. Install/start Docker Desktop and check docker compose version."))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Docker exited with {status}. Inspect the saved log; check Docker, disk space, source revisions and extracted data. No volumes were removed."
        ))
    }
}
pub(super) fn capture(command: Command, log: &mut File) -> Result<String> {
    let start = log.metadata().map_err(|e| e.to_string())?.len();
    execute(command, log)?;
    let mut reader = log.try_clone().map_err(|e| e.to_string())?;
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    reader
        .take(128 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    // Cloned descriptors share an offset: restore append position after capture.
    log.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
pub fn tail(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let offset = file
        .metadata()
        .map_err(|e| e.to_string())?
        .len()
        .saturating_sub(32 * 1024);
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(32 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let start = if offset > 0 {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1)
    } else {
        0
    };
    Ok(String::from_utf8_lossy(&bytes[start..]).into_owned())
}
/// A new private log file under the managed state folder, as every server operation keeps one.
pub(super) fn open_log(config: &setup::Config) -> Result<(File, PathBuf)> {
    let dir = Settings::state(config).join("manager-logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = dir.join(format!("{stamp}-{}.log", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(&path).map_err(|e| e.to_string())?;
    Ok((log, path))
}
pub fn run(
    mut config: setup::Config,
    mut settings: Settings,
    action: Action,
    sender: Sender<Event>,
) -> Result<String> {
    if !matches!(action, Action::Save) {
        let docker = super::docker::check();
        let blocked = docker.blocked();
        let advice = docker.advice();
        let _ = sender.send(Event::Docker(docker));
        if blocked {
            return Err(advice.into());
        }
    }
    config.repo = setup::resolved(&config.repo)?;
    if !config.data.as_os_str().is_empty() {
        config.data = setup::resolved(&config.data)?;
    }
    if !settings.source.as_os_str().is_empty() {
        settings.source = setup::resolved(&settings.source)?;
    }
    // Compose interpolates the build context even for runtime-only commands.
    // An unselected source may use the recipe folder for status/stop, never a build.
    if settings.source.as_os_str().is_empty()
        && matches!(action, Action::Build | Action::ForceBuild)
    {
        return Err("Select a separate clean core checkout containing modules/mod-playerbots before building.".into());
    }
    if !settings.data.as_os_str().is_empty() {
        settings.data = setup::resolved(&settings.data)?;
    }
    settings.validate(&config, matches!(action, Action::Start))?;
    settings.save(&config)?;
    let config_dir = crate::module_config::directory(&config);
    crate::module_config::validate(&config, &settings.source, &config_dir.join("probe.conf"))?;
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let (mut log, path) = open_log(&config)?;
    let _ = sender.send(Event::Log(path));
    let stage = |label: &str| {
        let _ = sender.send(Event::Stage(label.into()));
    };
    let run = |args: &[&str], log: &File| execute(command::compose(&config, &settings, args), log);
    if matches!(action, Action::Status | Action::Start) {
        let build = match capture(command::local_image(), &mut log) {
            Ok(image) => super::build_state::BuildState::observe(&image, &config, &settings),
            Err(error) => super::build_state::BuildState::Unavailable(format!(
                "Could not check the server build: {error}"
            )),
        };
        let missing = matches!(build, super::build_state::BuildState::Missing);
        let _ = sender.send(Event::Build(build));
        if missing && matches!(action, Action::Start) {
            return Err("Build the server before starting it.".into());
        }
    }
    match action {
        Action::Save => Ok(
            "Server settings saved separately from client settings. No Docker services changed."
                .into(),
        ),
        Action::Build | Action::ForceBuild => {
            let plan = crate::modules::prepare(&config, &settings.source)?;
            let before =
                super::build_state::Receipt::selected(String::new(), &settings.source, &plan)?;
            let staged = crate::modules::stage(&config, &plan)?;
            for module in &plan.selected {
                if crate::modules::fingerprint(&module.source)?
                    != crate::modules::fingerprint(&staged.path.join(&module.name))?
                {
                    return Err("Module sources changed during staging; retry build".into());
                }
            }
            if before
                != super::build_state::Receipt::selected(String::new(), &settings.source, &plan)?
            {
                return Err("Module sources changed during staging; retry build".into());
            }
            stage("Building selected server modules · first build can take over an hour");
            let mut build = command::compose(&config, &settings, command::BUILD);
            build.env("MODULE_SOURCES", &staged.path);
            execute(build, &log)?;
            let image = capture(command::local_image(), &mut log)?;
            if image.trim().is_empty() {
                return Err(
                    "Build completed but no local server image was found; refresh status.".into(),
                );
            }
            let next = crate::modules::prepare(&config, &settings.source)?;
            let mut after =
                super::build_state::Receipt::selected(String::new(), &settings.source, &next)?;
            if before != after {
                return Err("Server image built, but modules changed during the build. Refresh and rebuild; module inventory was not recorded.".into());
            }
            after.image = image.trim().into();
            after.save(&config)?;
            let _ = sender.send(Event::Build(super::build_state::BuildState::observe(
                &image, &config, &settings,
            )));
            Ok("Server image built. Restart the server to use this build; running services are unchanged.".into())
        }
        Action::Start => {
            // Saved server settings reach auth/world as environment variables. Written before any
            // `up`: Compose recreates a service whose environment changed.
            settings
                .environment
                .write(&config, &Settings::env_file(&config))?;
            stage("Checking whether managed server is already healthy");
            let output = capture(
                command::compose(&config, &settings, command::STATUS),
                &mut log,
            )?;
            let rows = command::services(&output)?;
            if command::ready(&rows) {
                start_chatter(&config, &settings, &mut log, &sender)?;
                let _ = sender.send(Event::Health(super::status::Health::Running));
                return Ok(
                    "Already running · MySQL/auth/world healthy · auth 127.0.0.1:13724".into(),
                );
            }
            for (label, args) in [
                ("Starting isolated MySQL", command::MYSQL),
                ("Bootstrapping managed schemas", command::BOOTSTRAP),
                ("Starting auth/world", command::START),
            ] {
                stage(label);
                writeln!(log, "\n--- {label} ---").map_err(|e| e.to_string())?;
                run(args, &log)?;
            }
            stage("Waiting for healthy MySQL/auth/world · up to 10 minutes");
            let deadline = Instant::now() + Duration::from_secs(600);
            loop {
                let output = capture(
                    command::compose(&config, &settings, command::STATUS),
                    &mut log,
                )?;
                let rows = command::services(&output)?;
                let _ = sender.send(Event::Health(super::status::Health::observed(&rows)));
                if command::ready(&rows) {
                    start_chatter(&config, &settings, &mut log, &sender)?;
                    return Ok("Managed MySQL/auth/world healthy · auth 127.0.0.1:13724 · world 127.0.0.1:18085".into());
                }
                if command::failed(&rows) || Instant::now() >= deadline {
                    let _ = run(command::LOGS, &log);
                    return Err(format!(
                        "Not ready: {}. Inspect logs, data files and Docker health checks; services/DB retained.",
                        command::summary(&rows)
                    ));
                }
                run(command::LOGS, &log)?;
                std::thread::sleep(Duration::from_secs(5));
            }
        }
        Action::Stop => {
            stage("Stopping auth/world (database and volumes retained)");
            run(command::STOP, &log)?;
            let _ = sender.send(Event::Health(super::status::Health::Stopped));
            Ok("Managed auth/world stopped; MySQL and persistent volumes retained.".into())
        }
        Action::Status => {
            stage("Checking managed services and collecting logs");
            let output = capture(
                command::compose(&config, &settings, command::STATUS),
                &mut log,
            )?;
            let rows = command::services(&output)?;
            let _ = sender.send(Event::Health(super::status::Health::observed(&rows)));
            run(command::LOGS, &log)?;
            Ok(super::chatter::summary(
                &rows,
                super::chatter::source(&config, &settings)?.is_some(),
            ))
        }
    }
}

fn start_chatter(
    config: &setup::Config,
    settings: &Settings,
    log: &mut File,
    sender: &Sender<Event>,
) -> Result<()> {
    if super::chatter::source(config, settings)?.is_none() {
        // Deselecting Chatter must not leave a previously managed bridge generating work.
        return execute(
            command::compose(config, settings, &["stop", "llm-chatter-bridge"]),
            log,
        );
    }
    let _ = sender.send(Event::Stage(
        "Provisioning bundled LLM Chatter bridge (realm unchanged)".into(),
    ));
    let staged = super::chatter::stage(config, settings)?;
    let mut start = command::compose(config, settings, super::chatter::START);
    start.env("CHATTER_SOURCE", staged.path.join("mod-llm-chatter"));
    execute(start, log)?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let rows = command::services(&capture(
            command::compose(config, settings, command::STATUS),
            log,
        )?)?;
        if super::chatter::ready(&rows) {
            return Ok(());
        }
        if super::chatter::failed(&rows) || Instant::now() >= deadline {
            let _ = execute(command::compose(config, settings, command::LOGS), log);
            return Err(format!("Realm retained, but LLM Chatter bridge is not ready: {}. Check provider/model/key in mod_llm_chatter.conf and bridge logs.", command::summary(&rows)));
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Ninja reports completed/total tasks, not time remaining or whole-operation percent.
pub fn count(log: &str) -> Option<(u64, u64)> {
    log.lines().rev().find_map(|line| {
        let start = line.find('[')? + 1;
        let pair = line.get(start..)?.split(']').next()?;
        let (done, total) = pair.split_once('/')?;
        let (done, total) = (done.parse().ok()?, total.parse().ok()?);
        (total > 0 && done <= total).then_some((done, total))
    })
}
