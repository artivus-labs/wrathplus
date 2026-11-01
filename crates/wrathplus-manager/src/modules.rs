//! Metadata-only native server module catalog. Nothing here runs CMake or module code.
//! The hub must declare `mod modules;` and explicitly consume `prepare` at build time.
//! External selections require an isolated source context: upstream CMake hard-codes
//! `${CMAKE_SOURCE_DIR}/modules`; there is no supported external module path flag.
mod fingerprint;
mod github;
mod staging;
mod storage;
pub(crate) use fingerprint::fingerprint;
pub(crate) use staging::{stage, Staged};
#[cfg(test)]
mod build_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;

use crate::setup::{self, Config};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;
#[cfg(test)]
const REQUIRED: &str = "mod-playerbots";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum Source {
    Core,
    Local {
        path: PathBuf,
    },
    Github {
        repository: String,
        revision: String,
        path: PathBuf,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub name: String,
    pub source: Source,
    pub enabled: bool,
    pub required: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    core: PathBuf,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleInput {
    pub name: String,
    pub source: PathBuf,
}
/// Pass every argument as an individual process argument, never through a shell.
/// A fresh CMake build directory is required (old cache entries are not migrated).
/// Disabled directories must be omitted from an isolated context for a strict exclusion:
/// legacy loaders can force static linkage and module .cmake files run even if disabled.
#[derive(Clone, Debug)]
pub(crate) struct BuildPlan {
    pub core_path: PathBuf,
    pub cmake_args: Vec<String>,
    pub selected: Vec<ModuleInput>,
    pub disabled: Vec<String>,
    pub requires_staging: bool,
}

impl Catalog {
    /// Read only; missing catalog defaults to discovered upstream modules enabled.
    pub fn load(config: &Config, core_path: &Path) -> Result<Self> {
        let core = setup::resolved(core_path)?;
        if !core.join("modules").is_dir() || !core.join("CMakeLists.txt").is_file() {
            return Err("Select a core source containing CMakeLists.txt and modules/".into());
        }
        let saved = storage::load(config, &core)?;
        let mut entries = Vec::new();
        for item in std::fs::read_dir(core.join("modules")).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            if !path.join("src").is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Non-UTF8 module name")?
                .to_owned();
            validate_name(&name)?;
            // Do not follow an upstream module link outside the read-only core tree.
            let source = module_path(&path)?;
            if !source.starts_with(&core) {
                return Err(format!("Module {name} escapes core"));
            }
            let enabled = saved
                .iter()
                .find(|e| e.name == name && e.source == Source::Core)
                .is_none_or(|e| e.enabled);
            entries.push(Entry {
                required: false,
                enabled,
                name,
                source: Source::Core,
            });
        }
        // Keep unavailable external registrations visible; build planning fails explicitly.
        entries.extend(saved.into_iter().filter(|e| e.source != Source::Core));
        // Old catalogs marked Playerbots required. Selection is now profile-owned.
        for entry in &mut entries {
            entry.required = false;
        }
        validate_entries(&entries)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { core, entries })
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn register_local(&mut self, path: &Path) -> Result<()> {
        let path = module_path(path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Non-UTF8 module name")?
            .to_owned();
        self.insert(name, Source::Local { path })
    }
    /// Explicit network operation, never called by load/prepare. Synchronous; run off UI thread.
    /// A failed import retains its reserved directory rather than risk deleting source files.
    pub fn import_github(&mut self, config: &Config, repository: &str) -> Result<()> {
        let repository = github::normalize(repository)?;
        let name = repository
            .split('/')
            .next_back()
            .ok_or("Missing repository")?
            .to_owned();
        self.check_new(&name)?;
        let source = github::clone(config, &self.core, &repository)?;
        self.insert(name, source)
    }
    fn check_new(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        if self
            .entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("Module {name} already registered"));
        }
        Ok(())
    }
    fn insert(&mut self, name: String, source: Source) -> Result<()> {
        self.check_new(&name)?;
        self.entries.push(Entry {
            required: false,
            name,
            source,
            enabled: true,
        });
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.name == name)
            .ok_or("Unknown module")?;
        entry.enabled = enabled;
        Ok(())
    }
    pub fn save(&self, config: &Config) -> Result<()> {
        storage::save(config, &self.core, &self.entries)
    }
    pub fn build_plan(&self) -> Result<BuildPlan> {
        validate_entries(&self.entries)?;
        let mut plan = BuildPlan {
            core_path: self.core.clone(),
            cmake_args: vec!["-DMODULES=static".into()],
            selected: Vec::new(),
            disabled: Vec::new(),
            requires_staging: false,
        };
        for entry in &self.entries {
            // Actual ModuleNameToVariable: TOUPPER with hyphens unchanged.
            plan.cmake_args.push(format!(
                "-DMODULE_{}={}",
                entry.name.to_ascii_uppercase(),
                if entry.enabled { "static" } else { "disabled" }
            ));
            if !entry.enabled {
                plan.disabled.push(entry.name.clone());
                // Strict exclusion also avoids legacy loaders that override disabled flags.
                plan.requires_staging = true;
                continue;
            }
            let path = match &entry.source {
                Source::Core => self.core.join("modules").join(&entry.name),
                Source::Local { path } | Source::Github { path, .. } => {
                    plan.requires_staging = true;
                    path.clone()
                }
            };
            plan.selected.push(ModuleInput {
                name: entry.name.clone(),
                source: module_path(&path)?,
            });
        }
        Ok(plan)
    }
}
/// Reload persisted choices for each build. Does not write, fetch, stage or execute code.
pub(crate) fn prepare(config: &Config, core_path: &Path) -> Result<BuildPlan> {
    Catalog::load(config, core_path)?.build_plan()
}
pub(crate) fn clone_addon(config: &Config, repository: &str) -> Result<PathBuf> {
    let repository = github::normalize(repository)?;
    let root = config.state_root().join("managed-addon-sources");
    let dest = root.join(&repository);
    if setup::resolved(&root)? != root {
        return Err("Addon source root must not be a symlink".into());
    }
    let mut guard = config.clone();
    if guard.data.as_os_str().is_empty() {
        guard.data = config.repo.join(".unselected-client-assets");
    }
    setup::check_output(&guard, &dest)?;
    if let Ok(bytes) = std::fs::read(config.state_root().join("manager-server.json")) {
        let settings: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        for key in ["source", "data"] {
            if let Some(input) = settings[key].as_str().filter(|s| !s.is_empty()) {
                let input = setup::resolved(Path::new(input))?;
                if dest.starts_with(&input) || input.starts_with(&dest) {
                    return Err("Addon import overlaps selected server source or assets".into());
                }
            }
        }
    }
    let (path, revision) = github::checkout(&dest, &repository)?;
    std::fs::write(dest.join("revision.txt"), revision).map_err(|e| e.to_string())?;
    Ok(path)
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 100
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("Unsafe module name: {name:?}"));
    }
    Ok(())
}
fn module_path(path: &Path) -> Result<PathBuf> {
    let path = setup::resolved(path)?;
    let path =
        std::fs::canonicalize(path).map_err(|e| format!("Module source unavailable: {e}"))?;
    let src =
        std::fs::canonicalize(path.join("src")).map_err(|e| format!("Module needs src/: {e}"))?;
    if !src.is_dir() || !src.starts_with(&path) {
        return Err("Module src/ escapes source directory".into());
    }
    Ok(path)
}
fn validate_entries(entries: &[Entry]) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for e in entries {
        validate_name(&e.name)?;
        if !names.insert(e.name.to_ascii_uppercase()) {
            return Err("Duplicate CMake module identity".into());
        }
        if let Source::Github {
            repository,
            revision,
            ..
        } = &e.source
        {
            if github::normalize(repository)? != *repository
                || revision.len() != 40
                || !revision.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid GitHub provenance".into());
            }
        }
    }
    Ok(())
}
