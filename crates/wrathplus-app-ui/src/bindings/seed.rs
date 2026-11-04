//! The key sets' seeding at a VM's birth, from the chain's defaults and the saved files.

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;

use super::{store, BindingFiles};

/// Seed the key sets at the VM's birth, before any interface file runs: the defaults off the
/// player's chain (`WTF\DefaultBindings.wtf`), the account set, and the character's set if it has
/// one. The keys are their own table, so a command a later file declares finds its keys there,
/// and stock `ActionButton_OnLoad` paints its hotkey corner from `GetBindingKey` at load.
///
/// The reference reads the defaults later in `UI_Init` (`0x4b62b0` at `0x4900c7`); here they are
/// read first because the stored sets are diffs against them ([`store`]'s deviation), and nothing
/// reads set 0 before `LoadBindings(0)`.
pub(crate) fn seed_bindings_for_vm(world: &mut World, script: &mut UiScript) {
    let defaults = crate::ui_script::default_bindings();
    script.set_default_bindings(defaults);
    script.seed_binding_layers(0, crate::ui_script::default_binding_layers());
    let defaults = script.default_bindings();
    let account = crate::local_state::bindings_account_path();
    let text = read_file(&account);
    let overrides = text.as_deref().map(store::from_diff).unwrap_or_default();
    script.seed_binding_set(1, Some(store::resolve(&overrides, &defaults)));
    let layers = text.as_deref().map(store::layers_from).unwrap_or_default();
    script.seed_binding_layers(1, layers);
    script.load_binding_set(1);

    // The character's own set: its file existing makes it the active set, the reference's rule.
    // No roster identity (a rigged or capture run) leaves set 2 unseeded.
    let id = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(crate::app_core::state::identity);
    let character = id
        .as_ref()
        .and_then(|(realm, name)| crate::local_state::bindings_character_path(realm, name));
    match read_file(&character) {
        Some(text) => {
            let overrides = store::from_diff(&text);
            script.seed_binding_set(2, Some(store::resolve(&overrides, &defaults)));
            script.seed_binding_layers(2, store::layers_from(&text));
            script.load_binding_set(2);
            info!("bindings: character-specific set loaded");
        }
        None => {
            script.seed_binding_set(2, None);
            script.load_binding_set(1);
        }
    }
    if let Some(mut files) = world.get_resource_mut::<BindingFiles>() {
        files.account = account;
        files.character = character;
    }
}

/// Read one bindings file; `None` (the defaults) when absent or unreadable.
fn read_file(path: &Option<std::path::PathBuf>) -> Option<String> {
    let path = path.as_ref()?;
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("bindings: reading {}: {e}", path.display());
            None
        }
    }
}
