//! Server environment variables: saved with the server settings, written on start to a private
//! env file that Compose hands to the auth, world and bootstrap services. AzerothCore lets an
//! environment variable override any config key, module configs included (`AC_` + the key in
//! upper snake case), so this one list carries every server setting the Manager surfaces.
//! Values never go into argv, logs or `Debug` output.
use super::module_settings;
use crate::setup;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Saved variables by name. A typed setting that is missing here means its default.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Environment(BTreeMap<String, String>);

impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Names only: values may be keys or passwords.
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl Environment {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }
    pub fn remove(&mut self, name: &str) {
        self.0.remove(name);
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
    /// What the server gets: every saved variable, plus the default of each typed setting the
    /// list does not name.
    pub fn effective(&self) -> Self {
        let mut all = self.clone();
        for module in module_settings::MODULES {
            for setting in module.settings {
                all.0
                    .entry(setting.env())
                    .or_insert_with(|| setting.default.into());
            }
        }
        all
    }
    /// Every name and value well formed, and every typed setting in its module's range.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in &self.0 {
            check_name(name)?;
            check_value(name, value)?;
        }
        module_settings::validate(self)
    }
    /// The env file text, one `NAME='value'` per line. Compose takes single-quoted values
    /// literally (no `$` interpolation, no escapes), which `check_value` keeps possible.
    pub fn render(&self) -> String {
        self.0.iter().map(|(k, v)| format!("{k}='{v}'\n")).collect()
    }
    /// Validate, then write the effective list privately (0600, atomic) for Compose.
    pub fn write(&self, config: &setup::Config, path: &std::path::Path) -> Result<(), String> {
        self.validate()?;
        let mut safe = config.clone();
        if safe.data.as_os_str().is_empty() {
            safe.data = config.repo.join(".unselected-client-assets");
        } else {
            safe.data = setup::resolved(&config.data)?;
        }
        setup::write_state(&safe, path, self.effective().render().as_bytes())
    }
}

/// An upper-snake identifier (`AC_*`, `MANAGED_*` or any other), as a shell would accept.
pub fn check_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    let first_ok = bytes
        .next()
        .is_some_and(|b| b.is_ascii_uppercase() || b == b'_');
    if first_ok
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && name.len() <= 128
    {
        Ok(())
    } else {
        Err(format!("Environment variable name {name:?}: use capital letters, digits and _ (for example AC_PLAYERBOTS_MIN_RANDOM_BOTS)."))
    }
}

/// One line, and representable in a single-quoted env-file value.
pub fn check_value(name: &str, value: &str) -> Result<(), String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(format!("{name}: the value must be a single line."));
    }
    if value.contains('\'') || value.ends_with('\\') {
        return Err(format!(
            "{name}: the value cannot contain ' or end with \\."
        ));
    }
    if value.len() > 4096 {
        return Err(format!("{name}: the value is longer than 4096 characters."));
    }
    Ok(())
}

/// AzerothCore's environment name for a config key: `AC_` + `IniKeyToEnvVarKey(key)`, a
/// byte-for-byte port of `src/common/Configuration/Config.cpp` in the pinned core.
pub fn env_name(key: &str) -> String {
    let bytes = key.as_bytes();
    let mut out = String::from("AC_");
    for (i, &curr) in bytes.iter().enumerate() {
        if matches!(curr, b' ' | b'.' | b'-') {
            out.push('_');
            continue;
        }
        let upper = curr.to_ascii_uppercase() as char;
        if let Some(&next) = bytes.get(i + 1) {
            // "aB" to "A_B"
            if !curr.is_ascii_uppercase() && next.is_ascii_uppercase() {
                out.push(upper);
                out.push('_');
                continue;
            }
            // "a1" to "A_1" and "1a" to "1_A"
            if curr.is_ascii_digit() != next.is_ascii_digit() {
                out.push(upper);
                out.push('_');
                continue;
            }
        }
        out.push(upper);
    }
    out
}
