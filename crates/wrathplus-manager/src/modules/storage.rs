use super::*;
use std::io::Read;
const LIMIT: u64 = 256 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    core: PathBuf,
    entries: Vec<Entry>,
}
/// Validate before any mkdir/write; symlinked state directories cannot redirect outputs.
pub(super) fn root(config: &Config, core: &Path) -> Result<PathBuf> {
    let state = setup::resolved(&config.state_root())?;
    let root = state.join("managed-module-sources");
    if setup::resolved(&root)? != root {
        return Err("Module state root must not be a symlink".into());
    }
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    setup::check_output(&guard, &root)?;
    if root.starts_with(core) || core.starts_with(&root) {
        return Err("Module state overlaps core source".into());
    }
    // Protect the server's separately selected extracted assets too, without depending on UI.
    if let Some(bytes) = read_optional(&state.join("manager-server.json"))? {
        let settings: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Read server asset guard: {e}"))?;
        if let Some(data) = settings
            .get("data")
            .and_then(|d| d.as_str())
            .filter(|d| !d.is_empty())
        {
            let data = setup::resolved(Path::new(data))?;
            if root.starts_with(&data) || data.starts_with(&root) {
                return Err("Module state overlaps server assets".into());
            }
        }
    }
    Ok(root)
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Module metadata exceeds 256 KiB".into());
    }
    Ok(Some(bytes))
}
pub(super) fn load(config: &Config, core: &Path) -> Result<Vec<Entry>> {
    let root = root(config, core)?;
    let path = root.join("catalog.json");
    if setup::resolved(&path)? != path {
        return Err("Catalog must not be a symlink".into());
    }
    let Some(bytes) = read_optional(&path)? else {
        return Ok(Vec::new());
    };
    let doc: Document =
        serde_json::from_slice(&bytes).map_err(|e| format!("Read module catalog: {e}"))?;
    if doc.version != 1 {
        return Err("Unsupported module catalog version".into());
    }
    if doc.core != core {
        return Err("Catalog belongs to another core; use a separate state home".into());
    }
    validate_entries(&doc.entries)?;
    for entry in &doc.entries {
        if let Source::Github {
            repository, path, ..
        } = &entry.source
        {
            if *path != clone_path(&root, repository).join("source")
                || setup::resolved(path)? != *path
            {
                return Err("Managed clone escapes reserved module sources".into());
            }
        }
    }
    Ok(doc.entries)
}
pub(super) fn clone_path(root: &Path, repository: &str) -> PathBuf {
    // Separate owner/repo components avoid delimiter collisions; both are validated.
    root.join("github").join(repository.to_ascii_lowercase())
}
pub(super) fn save(config: &Config, core: &Path, entries: &[Entry]) -> Result<()> {
    validate_entries(entries)?;
    let root = root(config, core)?;
    let path = root.join("catalog.json");
    if setup::resolved(&path)? != path {
        return Err("Catalog must not be a symlink".into());
    }
    let bytes = serde_json::to_vec_pretty(&Document {
        version: 1,
        core: core.to_owned(),
        entries: entries.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Module metadata exceeds 256 KiB".into());
    }
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    setup::write_state(&guard, &path, &bytes)
}
