use super::{cargo, Report};
use std::env;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// Only native executable extensions: .cmd/.bat require a command interpreter.
// PATHEXT is case-insensitive on Windows, but never authorizes shell scripts here.
pub(super) fn names(name: &str, windows: bool, pathext: &str) -> Vec<String> {
    if !windows {
        return vec![name.into()];
    }
    let mut names = Vec::new();
    for ext in pathext.split(';') {
        if ext.eq_ignore_ascii_case(".exe") || ext.eq_ignore_ascii_case(".com") {
            names.push(format!("{name}{ext}"));
        }
    }
    names
}

pub(super) fn in_dirs(
    name: &str,
    dirs: &[PathBuf],
    windows: bool,
    pathext: &str,
) -> Option<PathBuf> {
    let names = names(name, windows, pathext);
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| executable(p))
}

pub(super) fn discover(name: &str, repo: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<_> = env::var_os("PATH")
        .map(|v| env::split_paths(&v).collect())
        .unwrap_or_default();
    if let Some(home) = env::var_os("CARGO_HOME").filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(home).join("bin"));
    }
    for key in ["HOME", "USERPROFILE"] {
        if let Some(home) = env::var_os(key).filter(|s| !s.is_empty()) {
            dirs.push(PathBuf::from(home).join(".cargo/bin"));
        }
    }
    // Relative PATH and CARGO_HOME entries refer to the command's working directory.
    let dirs: Vec<_> = dirs
        .into_iter()
        .map(|p| if p.is_absolute() { p } else { repo.join(p) })
        .collect();
    in_dirs(
        name,
        &dirs,
        cfg!(windows),
        &env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()),
    )
}

/// Small output limit, wall-clock deadline, kill + reap on timeout. Never uses a shell.
pub(super) fn run(repo: &Path, program: &Path, args: &[&OsStr]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(repo)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .env("RUSTUP_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let stdout = child.stdout.take().ok_or("Cannot capture probe output")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = String::new();
        let result = stdout
            .take(16 * 1024)
            .read_to_string(&mut output)
            .map(|_| output);
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!(
                        "{} exited {status}; check installed tools (nothing was installed)",
                        program.display()
                    ));
                }
                return rx
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| "Probe output timed out".to_string())?
                    .map(|s| s.trim().to_owned())
                    .map_err(|e| e.to_string());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(e) => format!("Probe wait failed: {e}"),
                    _ => format!("{} timed out after 5 seconds", program.display()),
                });
            }
        }
    }
}

pub(super) fn probe(repo: &Path, name: &str, args: &[&str]) -> Result<String, String> {
    let path =
        discover(name, repo).ok_or_else(|| format!("{name} executable not found in PATH"))?;
    run(
        repo,
        &path,
        &args.iter().map(OsStr::new).collect::<Vec<_>>(),
    )
}

pub(super) fn channel(text: &str) -> Result<String, String> {
    let mut toolchain = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            toolchain = line == "[toolchain]";
        }
        if !toolchain {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "channel" {
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .ok_or("Invalid toolchain channel string")?;
                if value.split('.').count() == 3
                    && value
                        .split('.')
                        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                {
                    return Ok(value.into());
                }
                return Err("Expected an exact numeric Rust toolchain channel".into());
            }
        }
    }
    Err("Missing [toolchain] channel in rust-toolchain.toml".into())
}

pub(super) fn version(output: &str, tool: &str, expected: &str) -> Result<String, String> {
    let mut words = output.split_whitespace();
    if words.next() == Some(tool) && words.next() == Some(expected) {
        Ok(output.into())
    } else {
        Err(format!(
            "Expected {tool} {expected}, got {output:?}; install/select the pinned toolchain"
        ))
    }
}

pub(super) fn rust(repo: &Path, report: &mut Report) {
    let expected = std::fs::read_to_string(repo.join("rust-toolchain.toml"))
        .map_err(|e| format!("Read rust-toolchain.toml: {e}"))
        .and_then(|s| channel(&s));
    report.result("Pinned Rust toolchain", expected.clone());
    let cargo = cargo(repo);
    let compiler = cargo
        .as_ref()
        .ok()
        .and_then(|p| p.parent())
        .and_then(|p| in_dirs("rustc", &[p.to_owned()], cfg!(windows), ".EXE"))
        .or_else(|| discover("rustc", repo));
    for (name, path) in [
        ("cargo", cargo),
        (
            "rustc",
            compiler.ok_or_else(|| "rustc executable not found".into()),
        ),
    ] {
        let result = path.and_then(|path| {
            let expected = expected.as_ref().map_err(Clone::clone)?;
            let output = run(repo, &path, &[OsStr::new("--version")])?;
            version(&output, name, expected).map(|output| format!("{} — {output}", path.display()))
        });
        report.result(name, result);
    }
}
