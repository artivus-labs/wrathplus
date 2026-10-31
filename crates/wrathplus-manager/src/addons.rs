//! Metadata-only addon discovery. Selecting a folder never executes addon code.
mod storage;
#[cfg(test)]
mod tests;
mod ui;

use crate::setup::Config;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Selection {
    pub name: String,
    pub source: PathBuf,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub folders: Vec<PathBuf>,
    pub selected: Vec<Selection>,
}
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub name: String,
    pub title: String,
    pub source: PathBuf,
    pub version: String,
    pub interface: u32,
    pub dependencies: Vec<String>,
    pub optional: Vec<String>,
    pub saved: Vec<String>,
    pub lod: bool,
    pub files: usize,
    pub missing: usize,
}
impl Entry {
    pub fn selection(&self) -> Selection {
        Selection {
            name: self.name.clone(),
            source: self.source.clone(),
        }
    }
    pub fn selected(&self, settings: &Settings) -> bool {
        settings.selected.contains(&self.selection())
    }
    pub fn requirements(&self, entries: &[Entry]) -> String {
        let mut notes = vec![format!(
            "Interface {} · {} files · {}",
            self.interface,
            self.files,
            if self.lod {
                "load on demand"
            } else {
                "startup"
            }
        )];
        if self.interface != 30300 {
            notes.push("Version gate: requires Interface 30300".into());
        }
        if self.missing > 0 {
            notes.push(format!("{} missing/unsafe manifest paths", self.missing));
        }
        for dep in &self.dependencies {
            notes.push(format!(
                "Requires {dep}{}",
                if entries.iter().any(|e| e.name.eq_ignore_ascii_case(dep)) {
                    " (discovered; must also be added)"
                } else {
                    " (not discovered; may be archive-provided)"
                }
            ));
        }
        if !self.optional.is_empty() {
            notes.push(format!("Optional: {}", self.optional.join(", ")));
        }
        if !self.saved.is_empty() {
            notes.push(format!("Saved variables: {}", self.saved.join(", ")));
        }
        if self.name.eq_ignore_ascii_case("MultiBot") {
            notes.push("Playerbot features require server mod-playerbots; Chatless bridge features also require mod-multibot-bridge. Runtime compatibility not verified.".into());
        }
        notes.join(" · ")
    }
}

fn manifest(path: &Path) -> Option<Entry> {
    let source = std::fs::canonicalize(path).ok()?;
    if !source.is_dir() {
        return None;
    }
    let mut tocs: Vec<_> = std::fs::read_dir(&source)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toc")))
        .collect();
    tocs.sort();
    let basename = source.file_name()?.to_string_lossy();
    let toc = tocs
        .iter()
        .find(|p| {
            p.file_stem()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&basename))
        })
        .or_else(|| (tocs.len() == 1).then(|| &tocs[0]))?;
    let name = toc.file_stem()?.to_str()?.to_owned();
    if !safe_name(&name) {
        return None;
    }
    let bytes = std::fs::read(toc).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let text = wrathplus_ui::source::decode(&bytes);
    let toc = wrathplus_ui::toc::Toc::parse(&text);
    let files: Vec<_> = toc.files.iter().map(|p| p.replace('\\', "/")).collect();
    let dependencies = toc.dependencies().into_iter().map(str::to_owned).collect();
    let optional = toc
        .optional_dependencies()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let saved = toc
        .list("SavedVariables")
        .into_iter()
        .chain(toc.list("SavedVariablesPerCharacter"))
        .map(str::to_owned)
        .collect();
    let missing = files
        .iter()
        .filter(|p| !manifest_file_exists(&source, Path::new(p)))
        .count();
    Some(Entry {
        title: toc
            .directive("Title")
            .map(str::to_owned)
            .unwrap_or_else(|| name.clone()),
        name,
        source,
        version: toc.directive("Version").unwrap_or_default().to_owned(),
        interface: toc.interface_version(),
        dependencies,
        optional,
        saved,
        lod: toc.load_on_demand(),
        files: files.len(),
        missing,
    })
}
// Match the client's case-insensitive loose-file lookup without accepting traversal.
// Resolve the source root before calling this; it may itself be a managed directory link.
fn manifest_file_exists(source: &Path, path: &Path) -> bool {
    let mut current = source.to_owned();
    if path.as_os_str().is_empty() {
        return false;
    }
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        if name.to_string_lossy().contains(':') {
            return false;
        }
        let Some(entry) = std::fs::read_dir(&current).ok().and_then(|entries| {
            entries.filter_map(Result::ok).find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            })
        }) else {
            return false;
        };
        current = entry.path();
    }
    current.is_file()
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
        && !name.to_ascii_lowercase().starts_with("blizzard_")
}

pub(crate) fn scan_folder(path: &Path) -> Vec<Entry> {
    if let Some(entry) = manifest(path) {
        return vec![entry];
    }
    let mut entries: Vec<_> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| manifest(&e.path()))
        .collect();
    entries.sort_by_key(|e| (e.name.to_lowercase(), e.source.clone()));
    entries
}
pub(crate) fn discover(config: &Config, settings: &Settings) -> Vec<Entry> {
    let mut roots = vec![
        config.state_root().join("AddOns"),
        config.repo.join("addons"),
    ];
    if let Some(install) = config.data.parent() {
        roots.push(install.join("Interface/AddOns"));
    }
    roots.extend(settings.folders.clone());
    roots.extend(settings.selected.iter().map(|s| s.source.clone()));
    let mut entries: Vec<Entry> = Vec::new();
    for entry in roots.iter().flat_map(|p| scan_folder(p)) {
        if !entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&entry.name) && e.source == entry.source)
        {
            entries.push(entry);
        }
    }
    // Keep unavailable registrations removable; absence must not hide an owned dangling link.
    for selected in &settings.selected {
        if !entries.iter().any(|e| e.selection() == *selected) {
            entries.push(Entry {
                name: selected.name.clone(),
                title: format!("{} (source unavailable)", selected.name),
                source: selected.source.clone(),
                version: String::new(),
                interface: 0,
                dependencies: vec![],
                optional: vec![],
                saved: vec![],
                lod: false,
                files: 0,
                missing: 1,
            });
        }
    }
    entries.sort_by_key(|e| (e.title.to_lowercase(), e.source.clone()));
    entries
}
pub(crate) use storage::{load, save_and_apply};
