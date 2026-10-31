//! Where benilla keeps local state: one folder, `wrathplus-config/`, never inside the install, and
//! this module is the only place that computes a path into it; each path fn's doc says what it
//! holds. The 1.12 client is portable the same way, its `WTF/` and `Cache/` in its own folder.
//!
//! Resolution, in order, the same shape as [`wrathplus_formats::wow_data`]:
//! 1. `$BENILLA_HOME`.
//! 2. `<project folder>/wrathplus-config/`, dev builds only ([`wrathplus_formats::project_folder`], the
//!    launcher's): a shipped binary must not carry the build machine's source tree.
//! 3. `<exe dir>/wrathplus-config/`.
//!
//! At either resident root, an existing `benilla-config` wins (even if both names exist).
//! No state is moved or merged; see [`wrathplus_formats::default_state_home`].
//!
//! With `$WOW_CAPTURE` set every path resolves to `None`: a capture neither reads nor writes
//! player state.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The folder's name: not `benilla`, which beside the binary is the executable itself, and not
/// `WTF`, which a real install already uses.
#[cfg(test)]
const STATE_DIR: &str = "wrathplus-config";

/// The state folder, or `None` when persistence is off (a capture run, or no executable path).
/// It may not exist yet: [`write_atomic`] creates it, and a reader treats a missing file as
/// defaults.
pub fn home() -> Option<PathBuf> {
    if std::env::var_os("WOW_CAPTURE").is_some() {
        return None; // hermetic: captures neither read nor write player state
    }
    if let Some(over) = std::env::var_os("BENILLA_HOME") {
        return Some(PathBuf::from(over));
    }
    if cfg!(test) {
        return None; // a unit test reaches no real state folder unless it pins `$BENILLA_HOME`
    }
    resident_home()
}

/// Steps 2 and 3 of [`home`]: the folder the build itself lives in.
fn resident_home() -> Option<PathBuf> {
    // 2 · the project folder, dev builds only.
    if let Some(root) = dev_project_root() {
        return Some(wrathplus_formats::default_state_home(&root));
    }
    // 3 · beside the binary; a dev build never reaches here.
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .map(|dir| wrathplus_formats::default_state_home(&dir))
}

/// The project folder a dev build keeps its state in ([`wrathplus_formats::project_folder`]), through
/// [`shared_root`].
fn dev_project_root() -> Option<PathBuf> {
    Some(shared_root(wrathplus_formats::project_folder()?))
}

/// `here`, or for a linked worktree its primary checkout, whichever worktree built the binary, so
/// every worktree shares one settings folder. Anything unexpected keeps `here`.
fn shared_root(here: &Path) -> PathBuf {
    let dot_git = here.join(".git");
    // A linked worktree: `.git` is a file pointing at `<primary>/.git/worktrees/<name>`.
    if dot_git.is_file() {
        if let Some(gitdir) = std::fs::read_to_string(&dot_git).ok().and_then(|s| {
            s.trim()
                .strip_prefix("gitdir:")
                .map(|p| p.trim().to_owned())
        }) {
            // …/.git/worktrees/<name> → …/.git → the primary checkout.
            if let Some(primary) = Path::new(&gitdir)
                .ancestors()
                .nth(2)
                .and_then(|d| d.parent())
            {
                if primary.is_dir() {
                    return primary.to_path_buf();
                }
            }
        }
    }
    here.to_path_buf()
}

/// `wrathplus-config/config.toml`: the CVar overrides, the `Config.wtf` analog.
pub fn config_path() -> Option<PathBuf> {
    home().map(|h| h.join("config.toml"))
}

/// `wrathplus-config/macros/account.txt`: the account-wide macro tab (indices 1..=36).
pub fn macros_account_path() -> Option<PathBuf> {
    home().map(|h| h.join("macros/account.txt"))
}

/// `wrathplus-config/macros/<realm>-<character>.txt`: the per-character macro tab (37..=54), the
/// reference's `WTF/Account/<ACC>/<REALM>/<CHAR>/macros-cache.txt` flattened into one folder.
pub fn macros_character_path(realm: &str, character: &str) -> Option<PathBuf> {
    let key = format!("{}-{}", file_token(realm), file_token(character));
    home().map(|h| h.join("macros").join(format!("{key}.txt")))
}

/// `wrathplus-config/bindings/account.txt`: the account-wide key bindings (the `bindings-cache.wtf`
/// analog), a diff against the defaults.
pub fn bindings_account_path() -> Option<PathBuf> {
    home().map(|h| h.join("bindings/account.txt"))
}

/// `wrathplus-config/bindings/<realm>-<character>.txt`: the character-specific binding set. Its
/// existence is the "character specific key bindings" state; going back to general deletes it.
pub fn bindings_character_path(realm: &str, character: &str) -> Option<PathBuf> {
    let key = format!("{}-{}", file_token(realm), file_token(character));
    home().map(|h| h.join("bindings").join(format!("{key}.txt")))
}

/// `wrathplus-config/saved-variables.lua`: the flat channel our FrameXML saves through
/// `RegisterForSave` (the `SavedVariables.lua` analog), written whole at logout, run at UI load.
pub fn saved_variables_path() -> Option<PathBuf> {
    home().map(|h| h.join("saved-variables.lua"))
}

/// `wrathplus-config/addons/<Realm>-<Character>.txt`: the AddOn enable state, per character in the
/// reference's `AddOns.txt` format (`<AddOnName>: enabled|disabled`), which it writes at the tail
/// of its UI shutdown (`0x490bd0`). An addon absent from the file is enabled.
pub fn addons_state_path(realm: &str, character: &str) -> Option<PathBuf> {
    let key = format!("{}-{}", file_token(realm), file_token(character));
    home().map(|h| h.join("addons").join(format!("{key}.txt")))
}

/// `wrathplus-config/saved/`: per-addon saved variables, account scope, one `<Addon>.lua` per addon
/// declaring `## SavedVariables` (the reference's `WTF/Account/<ACC>/SavedVariables/<Addon>.lua`).
pub fn addon_saved_account_dir() -> Option<PathBuf> {
    home().map(|h| h.join("saved"))
}

/// `wrathplus-config/saved/<Realm>-<Character>/`: per-addon saved variables, character scope
/// (`## SavedVariablesPerCharacter`), loaded after the account file so it wins.
pub fn addon_saved_character_dir(realm: &str, character: &str) -> Option<PathBuf> {
    let key = format!("{}-{}", file_token(realm), file_token(character));
    home().map(|h| h.join("saved").join(key))
}

/// `wrathplus-config/camera/<realm>-<character>.txt`: the third-person camera pose (the
/// `<Char>/camera-settings.txt` analog), two lines in the reference's keys and order.
pub fn camera_character_path(realm: &str, character: &str) -> Option<PathBuf> {
    let key = format!("{}-{}", file_token(realm), file_token(character));
    home().map(|h| h.join("camera").join(format!("{key}.txt")))
}

/// `wrathplus-config/account`: the account name the login screen remembers
/// (`GetSavedAccountName`/`SetSavedAccountName`; the reference's is `Config.wtf`'s `accountName`).
pub fn saved_account_path() -> Option<PathBuf> {
    home().map(|h| h.join("account"))
}

/// `wrathplus-config/chat/<realm>-<character>.txt`: the chat windows' tint, alpha, font size and
/// lock, the four the reference keeps in its per-character `chat-cache.txt`.
pub fn chat_character_path(realm: &str, character: &str) -> Option<PathBuf> {
    Some(home()?.join("chat").join(format!(
        "{}-{}.txt",
        file_token(realm),
        file_token(character)
    )))
}

/// `wrathplus-config/layout/<realm>-<character>.txt`: the layout cache, the geometry of every
/// user-placed frame, as the reference's per-character `layout-cache.txt`; [`crate::ui_layout`]
/// owns the shape.
pub fn layout_character_path(realm: &str, character: &str) -> Option<PathBuf> {
    Some(home()?.join("layout").join(format!(
        "{}-{}.txt",
        file_token(realm),
        file_token(character)
    )))
}

/// `wrathplus-config/cache/<realm>.tsv`: the player, creature and pet names the server has answered,
/// one file for the reference's `WDB/namecache.wdb`, `creaturecache.wdb` and `petnamecache.wdb`.
///
/// Realm-scoped, where the reference's is not: guids, creature entries and pet numbers are
/// realm-local, so one shared file would serve a realm another realm's names.
pub fn name_cache_path(realm: &str) -> Option<PathBuf> {
    Some(
        home()?
            .join("cache")
            .join(format!("{}.tsv", file_token(realm))),
    )
}

/// `wrathplus-config/Logs/`: `WoWChatLog.txt` and `WoWCombatLog.txt`, the reference's names.
pub fn logs_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Logs"))
}

/// `wrathplus-config/shots.txt`: the framing instrument's camera poses (`/shot`, dev builds).
pub fn shots_path() -> Option<PathBuf> {
    home().map(|h| h.join("shots.txt"))
}

/// `wrathplus-config/Screenshots/`: where the print-screen key writes, each image once, not through
/// [`write_atomic`].
///
/// Deviation: the reference writes `Screenshots\\` inside the install, which benilla never writes
/// to; the folder keeps the reference's name.
pub fn screenshots_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Screenshots"))
}

/// `wrathplus-config/Diagnostics/`: the stuck-thread sampler's profiles (`perf::stall`) and
/// the FPS journal; `None` on a capture run like every path here, so a capture has no sampler.
pub fn diagnostics_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Diagnostics"))
}

/// `wrathplus-config/Diagnostics/fps-journal.csv`: the FPS journal's rows while the `fpsJournal`
/// CVar is on; a capture names its own path through `WOW_FPS_JOURNAL`.
pub fn fps_journal_path() -> Option<PathBuf> {
    diagnostics_dir().map(|d| d.join("fps-journal.csv"))
}

/// A realm or character name as one path component: anything not a letter or digit becomes `_`.
/// Letters of any script stay, as in the reference's raw-name folders: vmangos accepts non-Latin
/// names (`StrictPlayerNames = 0`) and a character name is letters only, so no two share a file.
fn file_token(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    if t.is_empty() {
        "unknown".into()
    } else {
        t
    }
}

/// Write a state file atomically, through `<path>.tmp` and a rename, so a crash leaves the old
/// file intact.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic_bytes(path, contents.as_bytes())
}

/// [`write_atomic`] for bytes: the saved-variables files carry Lua byte strings as they are.
pub fn write_atomic_bytes(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// The addon folder's name, the reference's own spelling.
const ADDONS_DIR: &str = "AddOns";

/// The addon folder, `<wrathplus-config>/AddOns/`, or `None` (always under `$WOW_CAPTURE`); addon
/// textures and fonts resolve onto it too (`ui_script::addons`, the font engine). Deviation: the
/// reference reads the install's `Interface\AddOns\`; benilla reads an install and does not live
/// in one, so its addons live in its own state folder, and only there.
pub fn addons_root() -> Option<PathBuf> {
    addons_root_from(home())
}

/// [`addons_root`] with its one environment fact passed in, so a test need not set the
/// environment.
fn addons_root_from(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|h| h.join(ADDONS_DIR))
}

/// Test plumbing for the persistence env vars: `set_var` is process-global, so every such test
/// takes [`ENV_LOCK`] and scopes its overrides in [`EnvGuard`]s.
#[cfg(any(test, feature = "test-support"))]
pub mod test_env {
    pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub struct EnvGuard(&'static str, Option<std::ffi::OsString>);
    impl EnvGuard {
        pub fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self(key, old)
        }
        pub fn unset(key: &'static str) -> Self {
            let old = std::env::var_os(key);
            std::env::remove_var(key);
            Self(key, old)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }
}

#[cfg(test)]
#[path = "local_state_tests.rs"]
mod tests;
