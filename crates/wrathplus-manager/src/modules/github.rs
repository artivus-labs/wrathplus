//! Explicit opt-in import, isolated Git configuration; no module scripts are executed.
use super::*;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 100_000;

pub(super) fn normalize(input: &str) -> Result<String> {
    let input = input.strip_prefix("https://github.com/").unwrap_or(input);
    let parts: Vec<_> = input.split('/').collect();
    if parts.len() != 2 {
        return Err("Use https://github.com/OWNER/REPO or OWNER/REPO".into());
    }
    for part in &parts {
        if part.is_empty()
            || part.len() > 100
            || !part.as_bytes()[0].is_ascii_alphanumeric()
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            || part.ends_with('.')
            || part.contains("..")
        {
            return Err("Invalid GitHub owner/repository".into());
        }
    }
    let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
    validate_name(repo)?;
    Ok(format!("{}/{repo}", parts[0]))
}
fn git(home: &Path) -> Command {
    let mut cmd = Command::new("git");
    // Inherited GIT_CONFIG_COUNT, URL rewrites, credentials, askpass, proxies and
    // loader variables must not reach the import process. Only PATH is retained.
    let path = std::env::var_os("PATH").unwrap_or_default();
    cmd.env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("absent-config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .arg("-c")
        .arg("credential.helper=")
        .arg("-c")
        .arg("core.askPass=")
        .arg("-c")
        .arg(format!("core.hooksPath={}", home.display()))
        .arg("-c")
        .arg("protocol.allow=never")
        .arg("-c")
        .arg("protocol.https.allow=always")
        .arg("-c")
        .arg("http.followRedirects=false")
        .arg("-c")
        .arg("http.lowSpeedLimit=1024")
        .arg("-c")
        .arg("http.lowSpeedTime=20")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}
fn size(root: &Path, reject_links: bool) -> Result<()> {
    let mut todo = vec![root.to_owned()];
    let (mut bytes, mut files) = (0u64, 0usize);
    while let Some(dir) = todo.pop() {
        for item in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let meta = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
            files += 1;
            bytes = bytes.saturating_add(meta.len());
            if files > MAX_FILES || bytes > MAX_BYTES {
                return Err("Import exceeds 256 MiB / 100,000 entries".into());
            }
            if meta.file_type().is_symlink() {
                if reject_links {
                    return Err(
                        "Imported module contains symlinks; review and register locally instead"
                            .into(),
                    );
                }
            } else if meta.is_dir() {
                todo.push(item.path());
            }
        }
    }
    Ok(())
}
fn run(mut command: Command, root: &Path, deadline: Instant) -> Result<()> {
    let mut child = command.spawn().map_err(|e| format!("Start git: {e}"))?;
    loop {
        let failure = if Instant::now() >= deadline {
            Some("Git import exceeded 120 seconds".to_owned())
        } else {
            size(root, false).err()
        };
        if let Some(error) = failure {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "Git import failed ({status}); no credentials or redirects allowed"
                    ))
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
}
pub(super) fn clone(config: &Config, core: &Path, repository: &str) -> Result<Source> {
    let root = storage::root(config, core)?;
    let dest = storage::clone_path(&root, repository);
    let (path, revision) = checkout(&dest, repository)?;
    let path = module_path(&path)?;
    Ok(Source::Github {
        repository: repository.to_owned(),
        revision,
        path,
    })
}
pub(super) fn checkout(dest: &Path, repository: &str) -> Result<(PathBuf, String)> {
    if setup::resolved(dest)? != dest {
        return Err("Clone path contains a symlink".into());
    }
    // create_dir is the exclusive reservation; never reuse, overwrite or delete.
    std::fs::create_dir_all(dest.parent().ok_or("Clone needs parent")?)
        .map_err(|e| e.to_string())?;
    std::fs::create_dir(dest)
        .map_err(|e| format!("Reserve new clone (existing imports are not overwritten): {e}"))?;
    let home = dest.join(".import-home");
    std::fs::create_dir(&home).map_err(|e| e.to_string())?;
    let checkout = dest.join("source");
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut cmd = git(&home);
    cmd.arg("clone")
        .arg("--depth=1")
        .arg("--single-branch")
        .arg("--no-tags")
        .arg("--no-checkout")
        .arg("--no-recurse-submodules")
        .arg(format!("--template={}", home.display()))
        .arg("--")
        .arg(format!("https://github.com/{repository}.git"))
        .arg(&checkout);
    run(cmd, dest, deadline)?;
    let mut cmd = git(&home);
    cmd.arg("-C")
        .arg(&checkout)
        .arg("checkout")
        .arg("--force")
        .arg("--detach")
        .arg("HEAD");
    run(cmd, dest, deadline)?;
    size(dest, true)?;
    let mut cmd = git(&home);
    cmd.arg("-C")
        .arg(&checkout)
        .arg("rev-parse")
        .arg("--verify")
        .arg("HEAD")
        .stdout(Stdio::piped());
    let output = cmd.output().map_err(|e| e.to_string())?;
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success()
        || revision.len() != 40
        || !revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Git did not resolve a SHA-1 revision".into());
    }
    Ok((checkout, revision))
}
