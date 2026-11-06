//! The portable state-directory naming contract shared by the client and Manager.
use std::path::{Path, PathBuf};

/// The state folder's name.
const NAME: &str = "wrathplus-config";

/// Select the whole home under `root`, `wrathplus-config`.
/// The caller selects the resident root (primary checkout for dev, executable dir for players)
/// and handles explicit overrides and hermetic runs before calling this function.
pub fn default_state_home(root: &Path) -> PathBuf {
    root.join(NAME)
}
