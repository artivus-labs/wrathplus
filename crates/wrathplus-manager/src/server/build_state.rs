//! Local image observation and successful-build module inventory, not runtime activation proof.
use super::{settings::Settings, Action, Input};
use crate::setup;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub(super) enum BuildState {
    #[default]
    Unknown,
    Missing,
    Present {
        warning: Option<String>,
    },
    Unavailable(String),
}
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Receipt {
    pub(super) image: String,
    source: std::path::PathBuf,
    modules: Vec<String>,
}
impl Receipt {
    pub fn snapshot(image: String, source: &Path) -> Result<Self, String> {
        let source = source
            .canonicalize()
            .map_err(|e| format!("Read source folder: {e}"))?;
        let mut modules = Vec::new();
        for entry in
            std::fs::read_dir(source.join("modules")).map_err(|e| format!("Read modules: {e}"))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().is_dir() {
                modules.push(
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "Module name is not UTF-8")?,
                );
            }
        }
        modules.sort();
        Ok(Self {
            image,
            source,
            modules,
        })
    }
    pub fn save(&self, config: &setup::Config) -> Result<(), String> {
        let path = Settings::state(config).join("build-receipt.json");
        Settings::check_output(config, &path)?;
        let mut guard = config.clone();
        if guard.data.as_os_str().is_empty() {
            guard.data = config.repo.join(".unselected-client-assets");
        }
        setup::write_state(
            &guard,
            &path,
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
}
impl BuildState {
    pub fn observe(image: &str, config: &setup::Config, settings: &Settings) -> Self {
        let image = image.trim();
        if image.is_empty() {
            return Self::Missing;
        }
        let current = Receipt::snapshot(image.into(), &settings.source);
        let recorded = std::fs::read(Settings::state(config).join("build-receipt.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Receipt>(&bytes).ok());
        let warning = match (current, recorded) {
            (Err(error), _) => Some(format!("Module inventory unavailable: {error}")),
            (Ok(current), Some(previous)) if current.image == previous.image && current.source == previous.source => {
                if current.modules == previous.modules { None } else {
                    let added: Vec<_> = current.modules.iter().filter(|m| !previous.modules.contains(m)).cloned().collect();
                    Some(if added.is_empty() {
                        "Server modules changed. Rebuild to apply the selected module list.".into()
                    } else {
                        format!("New modules: {}. They are not included in the recorded build. Rebuild, then restart the server to activate them.", added.join(", "))
                    })
                }
            }
            _ => Some("Existing build found; its module inventory is not verified. Rebuild to record the selected modules.".into()),
        };
        Self::Present { warning }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Present { .. } => "Rebuild",
            Self::Missing => "Build",
            _ => "Checking build…",
        }
    }
    pub fn input(&self, enabled: bool) -> Option<Input> {
        if !enabled {
            return None;
        }
        match self {
            Self::Missing => Some(Input::Run(Action::Build)),
            Self::Present { .. } => Some(Input::Run(Action::ForceBuild)),
            _ => None,
        }
    }
    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Unknown => Some("Checking for a local server build…"),
            Self::Missing => Some("The server needs to be built before its first start."),
            Self::Present { warning } => warning.as_deref(),
            Self::Unavailable(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests;
