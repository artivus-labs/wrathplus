//! Local module configuration only; no realm actions and no config contents in activity logs.
mod discovery;
#[cfg(test)]
mod editor_tests;
pub(crate) use discovery::discover;
mod files;
mod paths;
mod rows;
#[cfg(test)]
mod tests;
mod ui;

use crate::setup::Config;
use iced::widget::text_editor;
use std::path::PathBuf;

pub(crate) use paths::validate;
type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 256 * 1024;
const ENTRIES: usize = 512;

pub(crate) fn directory(config: &Config) -> PathBuf {
    config
        .state_root()
        .join("managed-server/wrathplus-managed-local/module-config")
}

#[derive(Default)]
pub(crate) struct State {
    module: String,
    name: String,
    pub(crate) open: bool,
    selected: Option<PathBuf>,
    original: Option<files::Snapshot>,
    content: text_editor::Content,
    dirty: bool,
    notice: String,
}

// Do not derive Debug: editor actions (including paste) may contain secrets.
#[derive(Clone)]
pub(crate) enum Input {
    Open { module: String, name: String },
    Cancel,
    Edit(text_editor::Action),
    Save,
    Discard,
}
impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModuleConfig input (contents redacted)")
    }
}
