//! Managed fresh-instance settings; deliberately separate from client manager.json.
use crate::setup;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROJECT: &str = "wrathplus-managed-local";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub source: PathBuf,
    pub data: PathBuf,
    pub reuse_image: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            data: PathBuf::new(),
            reuse_image: true,
        }
    }
}
impl Settings {
    pub fn path(config: &setup::Config) -> PathBuf {
        config.state_root().join("manager-server.json")
    }
    pub fn state(config: &setup::Config) -> PathBuf {
        config.state_root().join("managed-server").join(PROJECT)
    }
    pub fn load(config: &setup::Config) -> Result<Self, String> {
        let path = Self::path(config);
        Self::check_output(config, &path)?;
        let bytes = match std::fs::File::open(path) {
            Ok(file) => {
                use std::io::Read;
                let mut bytes = Vec::new();
                file.take(65537)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 65536 {
                    return Err("Server settings exceed 64 KiB; restore the separate manager-server.json file.".into());
                }
                Ok(bytes)
            }
            Err(e) => Err(e),
        };
        match bytes {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Server settings: {e}"))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Read server settings: {e}")),
        }
    }
    pub fn check_output(config: &setup::Config, path: &std::path::Path) -> Result<(), String> {
        setup::resolved(path)?;
        if !config.data.as_os_str().is_empty() {
            let mut guard = config.clone();
            guard.data = setup::resolved(&config.data)?;
            setup::check_output(&guard, path)?;
        }
        Ok(())
    }
    pub fn save(&self, config: &setup::Config) -> Result<(), String> {
        let mut safe_config = config.clone();
        if safe_config.data.as_os_str().is_empty() {
            safe_config.data = config.repo.join(".unselected-client-assets");
        } else {
            safe_config.data = setup::resolved(&config.data)?;
        }
        setup::write_state(
            &safe_config,
            &Self::path(config),
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
    pub fn validate(&self, config: &setup::Config, needs_data: bool) -> Result<(), String> {
        for output in [
            Self::path(config),
            Self::state(config),
            Self::state(config).join("manager-logs"),
        ] {
            Self::check_output(config, &output)?;
            for input in [&self.source, &self.data] {
                if !input.as_os_str().is_empty() {
                    let input = setup::resolved(input)?;
                    let mut guard = config.clone();
                    guard.data = input;
                    setup::check_output(&guard, &output)?;
                }
            }
        }
        if needs_data
            && !["dbc", "maps", "vmaps"]
                .iter()
                .all(|dir| self.data.join(dir).is_dir())
        {
            return Err("Select extracted server data containing dbc/, maps/ and vmaps/ (not client MPQs). Data is mounted read-only.".into());
        }
        Ok(())
    }
}
