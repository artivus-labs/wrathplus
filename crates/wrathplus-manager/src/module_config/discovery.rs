//! Per-module discovery is read-only; opening a template never creates a config.
use super::*;
use crate::modules::{Entry, Source};
use std::path::Path;

#[derive(Clone)]
pub(crate) struct ConfigFile {
    pub module: String,
    pub name: String,
    pub template: PathBuf,
    pub target: PathBuf,
}
pub(crate) fn discover(config: &Config, core: &Path, entry: &Entry) -> Result<Vec<ConfigFile>> {
    let source = match &entry.source {
        Source::Core => core.join("modules").join(&entry.name),
        Source::Local { path } | Source::Github { path, .. } => path.clone(),
    };
    let mut result = Vec::new();
    for template in paths::list(&source.join("conf"), ".conf.dist")? {
        let name = template
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".dist"))
            .ok_or("Invalid config filename")?
            .to_owned();
        let target = directory(config).join(&name);
        validate(config, core, &target)?;
        result.push(ConfigFile {
            module: entry.name.clone(),
            name,
            template,
            target,
        });
    }
    Ok(result)
}

pub(super) fn resolve(
    config: &Config,
    core: &Path,
    module: &str,
    name: &str,
) -> Result<ConfigFile> {
    let catalog = crate::modules::Catalog::load(config, core)?;
    let entry = catalog
        .entries()
        .iter()
        .find(|e| e.name == module)
        .ok_or("Module no longer available")?;
    discover(config, core, entry)?
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| "Config no longer available; reopen the editor".into())
}
