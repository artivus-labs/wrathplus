//! Profiles: each one carries its own server module selection and server settings (environment
//! variables). The module catalog and `manager-server.json` always hold the active profile's;
//! the others wait in `manager-profiles.json` until selected. Module sources (what is imported or
//! registered) are shared; only which of them a build includes, and the settings, differ. A server
//! type therefore changes the active profile only.
use crate::server::Environment;
use crate::setup::{self, Config};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, String>;

/// The profiles on offer; creating, duplicating and sharing profiles is not connected yet.
pub(crate) const NAMES: [&str; 3] = ["Local adventures", "Classic essentials", "Bot workshop"];

/// One profile's server choices.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    /// Module name → in the build. `None`: there was no module catalog (no server source yet),
    /// so selecting the profile leaves the catalog alone.
    pub modules: Option<BTreeMap<String, bool>>,
    pub environment: Environment,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Store {
    /// Whose choices the catalog and the server settings hold now.
    pub active: String,
    /// The other profiles' choices, as they were when the player last left them.
    pub saved: BTreeMap<String, Selection>,
}

fn path(config: &Config) -> PathBuf {
    config.state_root().join("manager-profiles.json")
}

/// The same output guard the other Manager state files use.
fn guard(config: &Config) -> Result<Config> {
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    Ok(guard)
}

impl Store {
    pub(crate) fn load(config: &Config) -> Result<Self> {
        match std::fs::read(path(config)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Profiles: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                active: NAMES[0].into(),
                saved: BTreeMap::new(),
            }),
            Err(e) => Err(format!("Read profiles: {e}")),
        }
    }
    pub(crate) fn save(&self, config: &Config) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        setup::write_state(&guard(config)?, &path(config), &bytes)
    }
    /// Leave the active profile for `to`: keep the live choices as the active profile's and
    /// return `to`'s. A profile never selected before starts as a copy of the live choices.
    pub(crate) fn switch(&mut self, to: &str, live: Selection) -> Selection {
        let from = std::mem::replace(&mut self.active, to.into());
        let next = self.saved.remove(to).unwrap_or_else(|| live.clone());
        self.saved.insert(from, live);
        next
    }
}

/// The active profile's name, for the header at startup.
pub(crate) fn active(config: &Config) -> String {
    Store::load(config)
        .ok()
        .map(|store| store.active)
        .filter(|name| NAMES.contains(&name.as_str()))
        .unwrap_or_else(|| NAMES[0].into())
}

/// The catalog's selection as a profile records it.
pub(crate) fn modules_of(catalog: &crate::modules::Catalog) -> BTreeMap<String, bool> {
    catalog
        .entries()
        .iter()
        .map(|e| (e.name.clone(), e.enabled))
        .collect()
}

/// Make the catalog build what a profile chose: named modules as recorded, any module the profile
/// has not seen (added while another profile was active) left out.
pub(crate) fn apply_modules(
    catalog: &mut crate::modules::Catalog,
    modules: &BTreeMap<String, bool>,
) -> Result<()> {
    let names: Vec<String> = catalog.entries().iter().map(|e| e.name.clone()).collect();
    for name in names {
        catalog.set_enabled(&name, modules.get(&name).copied().unwrap_or(false))?;
    }
    Ok(())
}

impl crate::Manager {
    /// Select a profile: its module selection and server settings become the live ones (for the
    /// next build and start); the profile left keeps what it had.
    pub(crate) fn select_profile(&mut self, name: String) {
        if name == self.profile || !NAMES.contains(&name.as_str()) {
            return;
        }
        self.load_server();
        if self.module_state.busy || !self.server_state.module_editable() {
            self.notice =
                "Wait for the server operation to finish before switching profiles.".into();
            return;
        }
        match self.switch_profile(&name) {
            Ok(()) => {
                self.profile = name;
                self.record(format!(
                    "Profile {} selected: its server modules and settings apply to the next build and start.",
                    self.profile
                ));
            }
            Err(error) => self.record(format!("Profile not switched: {error}")),
        }
    }

    fn switch_profile(&mut self, name: &str) -> Result<()> {
        let config = self.config.clone();
        let mut store = Store::load(&config)?;
        store.active = self.profile.clone();
        let source = self.server_state.module_source().to_owned();
        let mut catalog = if source.as_os_str().is_empty() {
            None
        } else {
            Some(crate::modules::Catalog::load(&config, &source)?)
        };
        let live = Selection {
            modules: catalog.as_ref().map(modules_of),
            environment: self.server_state.environment().clone(),
        };
        let next = store.switch(name, live);
        if let (Some(catalog), Some(modules)) = (catalog.as_mut(), next.modules.as_ref()) {
            apply_modules(catalog, modules)?;
            catalog.save(&config)?;
        }
        self.server_state
            .set_environment(&config, next.environment)?;
        store.save(&config)
    }
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
