use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(super) struct Snapshot {
    pub text: String,
    metadata: fs::Metadata,
}
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    let common = a.len() == b.len() && a.modified().ok() == b.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        common
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        common && a.created().ok() == b.created().ok()
    }
}
pub(super) fn read(path: &Path) -> Result<Snapshot> {
    paths::plain(path)?;
    let before = fs::symlink_metadata(path).map_err(|_| "Cannot inspect config file")?;
    if !before.is_file() || before.len() > LIMIT as u64 {
        return Err("Config must be a regular UTF-8 file no larger than 256 KiB".into());
    }
    let file = fs::File::open(path).map_err(|_| "Cannot open config file")?;
    let metadata = file.metadata().map_err(|_| "Cannot inspect open config")?;
    if !same(&before, &metadata) {
        return Err("Config changed while opening".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read config file")?;
    if bytes.len() > LIMIT {
        return Err("Config exceeds 256 KiB".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Config is not UTF-8")?;
    let after = fs::symlink_metadata(path).map_err(|_| "Cannot recheck config file")?;
    if !same(&metadata, &after) {
        return Err("Config changed while reading".into());
    }
    Ok(Snapshot { text, metadata })
}

#[cfg(test)]
pub(super) fn templates(config: &Config, core: &Path) -> Result<Vec<PathBuf>> {
    let selected = crate::modules::Catalog::load(config, core)?
        .build_plan()?
        .selected;
    if selected.len() > ENTRIES {
        return Err("Too many selected modules".into());
    }
    let mut result = Vec::new();
    for module in selected {
        let conf = module.source.join("conf");
        result.extend(paths::list(&conf, ".conf.dist")?);
        if result.len() > ENTRIES {
            return Err("Too many config templates".into());
        }
    }
    result.sort();
    Ok(result)
}

#[cfg(test)]
pub(super) fn create(config: &Config, core: &Path, template: &Path) -> Result<PathBuf> {
    // Re-discover rather than trusting a stale UI path or arbitrary message payload.
    if !templates(config, core)?.iter().any(|p| p == template) {
        return Err("Template is no longer selected; refresh".into());
    }
    let name = template
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_suffix(".dist"))
        .ok_or("Invalid config template name")?;
    let target = directory(config).join(name);
    let snapshot = read(template)?;
    write(config, core, &target, &snapshot.text, None)?;
    Ok(target)
}

pub(super) fn write(
    config: &Config,
    core: &Path,
    path: &Path,
    text: &str,
    original: Option<&Snapshot>,
) -> Result<()> {
    if text.len() > LIMIT {
        return Err("Config exceeds 256 KiB; edits retained".into());
    }
    let path = validate(config, core, path)?;
    let check = || -> Result<()> {
        if let Some(original) = original {
            let current = read(&path)?;
            if current.text != original.text || !same(&current.metadata, &original.metadata) {
                return Err("Config changed externally; edits retained. Copy them before Discard and reload.".into());
            }
        } else if fs::symlink_metadata(&path).is_ok() {
            return Err("Config already exists; never overwritten by template creation".into());
        }
        Ok(())
    };
    check()?;
    let parent = path.parent().ok_or("Missing config parent")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create config directory")?;
    validate(config, core, &path)?;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let stage = parent.join(format!(
        ".config-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&stage).map_err(|_| "Cannot stage config")?;
    let result = (|| {
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot write staged config")?;
        validate(config, core, &path)?;
        check()?;
        if original.is_some() {
            fs::rename(&stage, &path).map_err(|_| "Cannot atomically save config")?;
        } else {
            // Atomic publication with no replacement, even if another creator wins the race.
            fs::hard_link(&stage, &path)
                .map_err(|_| "Cannot create config (possibly already exists)")?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&stage);
    result
}
