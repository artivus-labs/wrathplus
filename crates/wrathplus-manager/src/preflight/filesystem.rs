use super::{tools, Report, Status};
use crate::setup;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn resolve(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Select a nonempty path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut result = PathBuf::new();
    for part in absolute.components() {
        match part {
            // A Windows prefix (e.g. a verbatim drive) is not queryable until its root is appended.
            Component::Prefix(prefix) => {
                result.push(prefix.as_os_str());
                continue;
            }
            Component::ParentDir => return Err("Paths must not contain '..'".into()),
            Component::CurDir => continue,
            part => result.push(part.as_os_str()),
        }
        match fs::symlink_metadata(&result) {
            Ok(_) => {
                result = fs::canonicalize(&result)
                    .map_err(|e| format!("Resolve {}: {e}", result.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Inspect {}: {e}", result.display())),
        }
    }
    Ok(result)
}

pub(super) fn overlaps(a: &Path, b: &Path) -> bool {
    // Windows canonicalization does not guarantee uniform case for nonexistent suffixes.
    if cfg!(windows) {
        let a = PathBuf::from(a.to_string_lossy().to_lowercase());
        let b = PathBuf::from(b.to_string_lossy().to_lowercase());
        a.starts_with(&b) || b.starts_with(&a)
    } else {
        a.starts_with(b) || b.starts_with(a)
    }
}

pub(super) fn nearest(path: &Path) -> Result<PathBuf, String> {
    let mut parent = path;
    loop {
        match fs::metadata(parent) {
            Ok(meta) if meta.is_dir() => return Ok(parent.to_owned()),
            Ok(_) => return Err(format!("{} is not a directory", parent.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                parent = parent.parent().ok_or("No existing parent directory")?;
            }
            Err(e) => return Err(format!("Inspect {}: {e}", parent.display())),
        }
    }
}

static SERIAL: AtomicU64 = AtomicU64::new(0);

fn write_probe(path: &Path, protected: &Path) -> Result<String, String> {
    let path = resolve(path)?;
    if overlaps(&path, protected) {
        return Err(format!(
            "{} overlaps the read-only asset install {}",
            path.display(),
            protected.display()
        ));
    }
    let parent = resolve(&nearest(&path)?)?;
    if overlaps(&parent, protected) {
        return Err(format!(
            "Nearest existing parent {} overlaps the read-only install; no probe written",
            parent.display()
        ));
    }
    // create_new never follows/overwrites a preexisting probe or symlink.
    let name = format!(
        ".manager-preflight-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    );
    let probe = parent.join(name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&probe)
        .map_err(|e| format!("Write probe in {}: {e}", parent.display()))?;
    let written = file
        .write_all(b"manager preflight\n")
        .and_then(|_| file.sync_all());
    drop(file);
    let removed = fs::remove_file(&probe);
    written.map_err(|e| format!("Write probe: {e}"))?;
    removed.map_err(|e| format!("Remove probe {}: {e}", probe.display()))?;
    Ok(format!(
        "{} (probed {}; no directories created)",
        path.display(),
        parent.display()
    ))
}

pub(super) fn df_available(text: &str) -> Option<u64> {
    // Filesystem names can contain spaces. Read backwards from capacity/mount boundary.
    text.lines().skip(1).find_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let capacity = fields
            .iter()
            .position(|s| s.ends_with('%') && s[..s.len() - 1].parse::<u32>().is_ok())?;
        fields
            .get(capacity.checked_sub(1)?)?
            .parse::<u64>()
            .ok()?
            .checked_mul(1024)
    })
}

fn disk(repo: &Path, path: &Path, label: &str, report: &mut Report) {
    if cfg!(unix) {
        let result = nearest(path).and_then(|path| {
            let df = tools::discover("df", repo).ok_or("df executable not found")?;
            tools::run(repo, &df, &[std::ffi::OsStr::new("-Pk"), path.as_os_str()]).and_then(|s| {
                df_available(&s).ok_or_else(|| "Cannot parse df available space".into())
            })
        });
        match result {
            Ok(bytes) => report.add(label, if bytes < 20 * 1024 * 1024 * 1024 { Status::Warning } else { Status::Pass },
                format!("{:.1} GiB available; a cold Bevy build can need tens of GiB (20 GiB advisory, not a guarantee)", bytes as f64 / 1073741824.0)),
            Err(e) => report.add(label, Status::Warning, format!("Free space unmeasured: {e}")),
        }
    } else {
        report.add(
            label,
            Status::Warning,
            "Free space unmeasured on this platform; allow tens of GiB for a cold Bevy build",
        );
    }
}

pub(super) fn inspect(config: &setup::Config, repo: &Path, report: &mut Report) {
    // An unknown install cannot be safely excluded from scratch writes. Tools still run.
    let protected = if config.data.as_os_str().is_empty() {
        Err("No asset path selected: write probes skipped to preserve the unknown install".into())
    } else {
        resolve(&config.data).map(|data| {
            if data
                .file_name()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("Data"))
            {
                data.parent().unwrap_or(&data).to_owned()
            } else {
                data
            }
        })
    };
    for (label, path) in [
        ("Repository writable", Ok(repo.to_owned())),
        ("Target writable", resolve(&config.target)),
    ] {
        match (&protected, path) {
            (Ok(protected), Ok(path)) => report.result(label, write_probe(&path, protected)),
            (Err(e), _) => report.add(label, Status::Warning, e.clone()),
            (_, Err(e)) => report.add(label, Status::Blocked, e),
        }
    }
    disk(repo, repo, "Repository disk space", report);
    match resolve(&config.target) {
        Ok(target) => disk(repo, &target, "Target disk space", report),
        Err(e) => report.add("Target disk space", Status::Warning, e),
    }
}
