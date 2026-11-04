//! Get the server source: the Playerbot branch of AzerothCore and mod-playerbots, each fetched at
//! the revision the build checks (packaging/server/compose.yaml), into the Manager's state folder.
//! An explicit network operation, run off the UI thread; it never deletes or overwrites a change.
use crate::setup::{self, Config};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

type Result<T> = std::result::Result<T, String>;

/// The Playerbot branch of AzerothCore (docs/CLIENT-SETUP.md); mod-playerbots does not build on
/// stock AzerothCore.
pub(crate) const CORE_REPOSITORY: &str = "mod-playerbots/azerothcore-wotlk";
pub(crate) const CORE_REV: &str = "93333d6a9eb00fdf0179adde30da3c17618207d6";
pub(crate) const PLAYERBOTS_REPOSITORY: &str = "mod-playerbots/mod-playerbots";
pub(crate) const PLAYERBOTS_REV: &str = "05a1f6fceb8a32aa879b13f5f48934c6e46a774c";

/// One repository at one revision, in its folder.
pub(super) struct Pin<'a> {
    pub repository: &'a str,
    pub revision: &'a str,
    pub path: PathBuf,
}

/// Where the Manager keeps the server source it fetched.
pub(crate) fn folder(config: &Config) -> PathBuf {
    config
        .state_root()
        .join("managed-server-source")
        .join("azerothcore-wotlk")
}

/// The core, then mod-playerbots inside its modules/ folder.
pub(super) fn pins(core: &Path) -> [Pin<'static>; 2] {
    [
        Pin {
            repository: CORE_REPOSITORY,
            revision: CORE_REV,
            path: core.to_owned(),
        },
        Pin {
            repository: PLAYERBOTS_REPOSITORY,
            revision: PLAYERBOTS_REV,
            path: core.join("modules").join("mod-playerbots"),
        },
    ]
}

/// The git arguments that bring `pin` to its revision: a one-commit fetch of exactly that
/// revision, then a detached checkout. `fresh` (no commit checked out yet) lets the checkout
/// replace files an interrupted earlier checkout left; otherwise git refuses to overwrite a change.
pub(super) fn fetch_args(pin: &Pin<'_>, fresh: bool) -> [Vec<String>; 2] {
    let path = pin.path.to_string_lossy().into_owned();
    let mut checkout = vec![
        "-C".into(),
        path.clone(),
        "checkout".into(),
        "--detach".into(),
    ];
    if fresh {
        checkout.push("--force".into());
    }
    checkout.push(pin.revision.into());
    [
        vec![
            "-C".into(),
            path,
            "fetch".into(),
            "--depth=1".into(),
            "--no-tags".into(),
            "--progress".into(),
            "--".into(),
            format!("https://github.com/{}.git", pin.repository),
            pin.revision.into(),
        ],
        checkout,
    ]
}

/// The private HOME the source's git commands run with.
fn home(core: &Path) -> Result<PathBuf> {
    let home = core
        .parent()
        .ok_or("The server source folder needs a parent")?
        .join(".git-home");
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    Ok(home)
}

/// Run one isolated git command, its output into the saved log.
fn git(home: &Path, args: &[String], log: &File) -> Result<()> {
    let mut command = crate::modules::isolated_git(home);
    command
        .args(args)
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log.try_clone().map_err(|e| e.to_string())?);
    let status = command
        .status()
        .map_err(|e| format!("Could not run git: {e}. Install Git, then try again."))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed ({status}). Check your internet connection and the saved log, then press Get server source again; it continues where it stopped.",
            args.get(2).map_or("", String::as_str)
        ))
    }
}

/// What a git query prints, trimmed. Isolated like the fetch, so a user's git config cannot
/// change the answer.
fn query(home: &Path, path: &Path, args: &[&str]) -> Result<String> {
    let mut command = crate::modules::isolated_git(home);
    command
        .arg("-C")
        .arg(path)
        .args(args)
        .stdout(Stdio::piped());
    let output = command
        .output()
        .map_err(|e| format!("Could not run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {}",
            args.join(" "),
            path.display()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The checks the server build makes before it compiles (packaging/server/Dockerfile): each
/// repository at its pinned revision, no tracked core file changed and mod-playerbots unchanged.
pub(super) fn verify(core: &Path, pins: &[Pin<'_>]) -> Result<()> {
    let home = home(core)?;
    for pin in pins {
        let head = query(&home, &pin.path, &["rev-parse", "HEAD"])
            .map_err(|_| format!("{} is not checked out yet.", pin.repository))?;
        if head != pin.revision {
            return Err(format!(
                "{} is at {}, but the server build needs {}.",
                pin.repository,
                head.get(..12).unwrap_or(&head),
                &pin.revision[..12]
            ));
        }
    }
    let changed = query(&home, core, &["diff", "--name-only", "HEAD"])?;
    if !changed.is_empty() {
        return Err(format!(
            "The server source has changed files ({}). The build needs a clean copy: undo those changes or choose another Source folder.",
            changed.lines().take(3).collect::<Vec<_>>().join(", ")
        ));
    }
    for pin in pins.iter().skip(1) {
        let status = query(&home, &pin.path, &["status", "--porcelain"])?;
        if !status.is_empty() {
            return Err(format!(
                "{} has changed files. The build needs it unchanged.",
                pin.repository
            ));
        }
    }
    Ok(())
}

/// Fetch whatever is missing and check the result. Safe to run again: a finished copy is only
/// checked, an interrupted one continues. Returns the Source folder.
pub(super) fn get(config: &Config, log: &mut File, stage: &dyn Fn(String)) -> Result<PathBuf> {
    let core = folder(config);
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    setup::check_output(&guard, &core)?;
    let core = setup::resolved(&core)?;
    let home = home(&core)?;
    let pins = pins(&core);
    for (step, pin) in pins.iter().enumerate() {
        let name = pin.repository.rsplit('/').next().unwrap_or(pin.repository);
        if query(&home, &pin.path, &["rev-parse", "HEAD"]).as_deref() == Ok(pin.revision) {
            writeln!(log, "--- {name} already at {} ---", pin.revision)
                .map_err(|e| e.to_string())?;
            continue;
        }
        stage(format!(
            "Downloading {name} ({} of 2) · about 1 GB in all",
            step + 1
        ));
        std::fs::create_dir_all(&pin.path).map_err(|e| e.to_string())?;
        let fresh = !pin.path.join(".git").exists()
            || query(&home, &pin.path, &["rev-parse", "--verify", "HEAD"]).is_err();
        if !pin.path.join(".git").exists() {
            let template = format!("--template={}", home.display());
            let init = [
                "init".into(),
                "--quiet".into(),
                template,
                pin.path.to_string_lossy().into_owned(),
            ];
            git(&home, &init, log)?;
        }
        for args in fetch_args(pin, fresh) {
            writeln!(log, "\n--- git {} {name} ---", args[2]).map_err(|e| e.to_string())?;
            git(&home, &args, log)?;
        }
    }
    stage("Checking the server source".into());
    verify(&core, &pins)?;
    writeln!(log, "\nServer source verified: {CORE_REPOSITORY} {CORE_REV}, {PLAYERBOTS_REPOSITORY} {PLAYERBOTS_REV}")
        .map_err(|e| e.to_string())?;
    Ok(core)
}

/// The last git progress figure in a log ("Receiving objects:  45% (…)"), as percent done.
pub(crate) fn progress(log: &str) -> Option<(&'static str, u64)> {
    log.rsplit(['\n', '\r']).find_map(|line| {
        let (label, rest) = [
            ("Receiving files", "Receiving objects:"),
            ("Unpacking files", "Resolving deltas:"),
            ("Writing files", "Updating files:"),
        ]
        .into_iter()
        .find_map(|(label, prefix)| Some((label, line.trim().strip_prefix(prefix)?)))?;
        let percent = rest.trim().split('%').next()?.trim().parse().ok()?;
        (percent <= 100).then_some((label, percent))
    })
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
