//! Explicit GitHub import. Git isolation/limits belong to the shared modules checkout helper.
use super::*;
use crate::{modules, Manager, Message};

/// Run on a worker: the helper validates GitHub input and returns an isolated checkout,
/// not a server module (`src/` is not required). No addon code is executed here.
fn import(config: &Config, repository: &str) -> Result<String, String> {
    import_with(config, repository, modules::clone_addon)
}

// Injection keeps validation/persistence tests entirely offline, without a second Git implementation.
pub(super) fn import_with(
    config: &Config,
    repository: &str,
    checkout: impl FnOnce(&Config, &str) -> Result<PathBuf, String>,
) -> Result<String, String> {
    let path = checkout(config, repository)?;
    let path = std::fs::canonicalize(&path)
        .map_err(|e| format!("Imported addon checkout unavailable: {e}"))?;
    if scan_folder(&path).is_empty() {
        return Err("No addon TOCs found in the imported repository root or its immediate subfolders. Checkout retained for review; no addon source registered.".into());
    }
    // Reload after the network operation; never persist a stale UI selection snapshot.
    let mut settings = load(config)?;
    if !settings.folders.contains(&path) {
        settings.folders.push(path);
    }
    // This is the authoritative stopped/build fence, even if a build began during clone.
    // Failure retains the checkout but must not register or activate it.
    save_and_apply(config, &settings)?;
    Ok("GitHub addon source imported. Review code before selecting Add to client; runtime compatibility is not verified. No addon was automatically enabled.".into())
}

impl Manager {
    pub(crate) fn import_addon_github(&mut self) -> iced::Task<Message> {
        if !self.addon_editable() {
            return iced::Task::none();
        }
        let repository = self.addon_path.trim().to_owned();
        if repository.is_empty() {
            self.record("Enter a GitHub owner/repo or https://github.com/owner/repo.".into());
            return iced::Task::none();
        }
        self.choosing = true;
        self.record("Importing GitHub addon source… Only import code you trust; addon code is not executed during discovery.".into());
        let config = self.config.clone();
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                import(&config, &repository)
            }))
            .unwrap_or_else(|_| {
                Err("Addon import worker failed; any checkout is retained for review.".into())
            });
            let _ = sender.send(result);
        });
        iced::Task::perform(
            async move {
                receiver
                    .await
                    .unwrap_or_else(|_| Err("Addon import worker disconnected.".into()))
            },
            Message::AddonImportFinished,
        )
    }
}
