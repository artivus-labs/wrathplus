//! The app half of `wrathplus_ui::script::macros`: the icon chooser's catalog, the files under
//! `wrathplus-config/macros/`, the runner, and the bound spell the action bar reads. 1.12 macros are
//! client state only: no opcode carries them, and the reference saves them to `macros-cache.txt`.

use bevy::prelude::*;

use wrathplus_ui::script::{MacroState, RunningMacro, ScriptValue, UiScript};

use crate::app_core::schedule::InWorldGated;
use wrathplus_assets::{LockRecover, WorldAssets};

pub(crate) mod run;
mod store;
#[cfg(test)]
mod tests;

/// The files this session's macros live in, the per-character one keyed by realm and name as the
/// reference's folders are. `None` is session-only (a capture, or no install): nothing is written.
#[derive(Resource, Default)]
pub(crate) struct MacroFiles {
    account: Option<std::path::PathBuf>,
    character: Option<std::path::PathBuf>,
    /// The `(realm, character)` the files were loaded for, per VM: a relog meets a fresh VM with an
    /// empty macro table, even as the same character.
    identity: crate::app_core::vm_memo::VmMemo<Option<(String, String)>>,
}

/// Macro id to its bound spell, the reference's `[rec+0x564]` (read by `0x4e5ba0`), which an
/// action-bar macro slot's state reads through. A missing entry names no macro, which `0x4e5030`
/// greys, unlike a macro that casts nothing.
#[derive(Resource, Default)]
pub(crate) struct MacroBoundSpells(pub(crate) std::collections::HashMap<u32, BoundSpell>);

pub(crate) use run::BoundSpell;

pub struct UiMacroPlugin;

impl Plugin for UiMacroPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MacroFiles>()
            .init_resource::<MacroBoundSpells>()
            .add_systems(
                Update,
                (
                    // Once per VM, in `Update`: it needs both the patch chain and the VM, and each
                    // login builds a fresh VM.
                    load_icon_catalog.in_set(crate::app_core::schedule::UiFeed),
                    // Before the action feeds read it (they run in `UnitFeed`), so a macro edited
                    // this frame reports its new spell's cooldown the same frame.
                    rebind_macro_spells
                        .in_set(crate::app_core::schedule::UiFeed)
                        .after(crate::ui_chat::commands::build_slash_commands)
                        .before(crate::app_core::schedule::UnitFeed),
                    // In-world only: the per-character file needs the character.
                    load_macros
                        .in_set(crate::app_core::schedule::UiFeed)
                        .in_set(InWorldGated),
                    // Every frame, in or out of world, so a `/logout` never strands an edit; after
                    // the script tick that dirtied the table.
                    save_dirty_macros.after(crate::app_core::schedule::UiInput),
                ),
            );
    }
}

/// An immutable archive catalog belongs to its asset source, not the short-lived UI VM.
/// Keep a weak identity so replacing the source invalidates both success and failure without
/// pinning the old archives. Failed loads are attempted once per source, as successful ones are.
struct IconCatalogCache<T, V = Vec<String>> {
    source: std::sync::Weak<T>,
    icons: Option<Result<V, String>>,
}

impl<T, V> Default for IconCatalogCache<T, V> {
    fn default() -> Self {
        Self {
            source: std::sync::Weak::new(),
            icons: None,
        }
    }
}

impl<T, V> IconCatalogCache<T, V> {
    fn get_or_load(
        &mut self,
        source: &std::sync::Arc<T>,
        load: impl FnOnce() -> Result<V, String>,
    ) -> (&Result<V, String>, bool) {
        let same_source = self
            .source
            .upgrade()
            .is_some_and(|old| std::sync::Arc::ptr_eq(&old, source));
        let hit = same_source && self.icons.is_some();
        if !hit {
            self.source = std::sync::Arc::downgrade(source);
            self.icons = Some(load());
        }
        (
            self.icons.as_ref().expect("catalog load stored a result"),
            hit,
        )
    }
}

type CatalogSource = std::sync::Mutex<wrathplus_formats::Chain>;
type IconSource = Option<std::sync::Weak<CatalogSource>>;

/// Seed each VM from a source-owned immutable catalog. A failed load leaves the chooser empty
/// (`MacroPopupOkayButton_Update` keeps OKAY disabled). Build 12340's two lists: the spell icons and
/// the item icons the guild-bank tab chooser also reads.
fn load_icon_catalog(
    script: Option<NonSendMut<UiScript>>,
    assets: Option<Res<WorldAssets>>,
    mut seeded: Local<crate::app_core::vm_memo::VmMemo<IconSource>>,
    mut wrath: Local<IconCatalogCache<CatalogSource, wrathplus_formats::WrathMacroIcons>>,
) {
    let (Some(mut script), Some(assets)) = (script, assets) else {
        return;
    };
    seed_wrath_icon_catalog(&mut script, &assets.chain, &mut seeded, &mut wrath, || {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_wrath_macro_icons(&mut chain).map_err(|e| format!("{e:#}"))
    });
}

/// The feed policy, isolated from GPU resources so the replacement tests exercise it: once per
/// VM and source, both lists.
fn seed_wrath_icon_catalog<T>(
    script: &mut UiScript,
    chain: &std::sync::Arc<T>,
    seeded: &mut crate::app_core::vm_memo::VmMemo<Option<std::sync::Weak<T>>>,
    catalog: &mut IconCatalogCache<T, wrathplus_formats::WrathMacroIcons>,
    load: impl FnOnce() -> Result<wrathplus_formats::WrathMacroIcons, String>,
) {
    let source = seeded.get(script);
    if source
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        .is_some_and(|old| std::sync::Arc::ptr_eq(&old, chain))
    {
        return;
    }
    *source = Some(std::sync::Arc::downgrade(chain));
    let (loaded, hit) = catalog.get_or_load(chain, load);
    match loaded {
        Ok(icons) => {
            script.set_macro_spell_icons(icons.spell.clone());
            script.set_macro_item_icons(icons.item.clone());
            info!(
                "ui_macro: {} spell and {} item icons in the choosers (cached={hit})",
                icons.spell.len(),
                icons.item.len()
            );
        }
        Err(e) if !hit => {
            error!("ui_macro: no Interface\\Icons\\ listing — the choosers are empty: {e}")
        }
        Err(_) => {}
    }
}

pub(crate) use crate::app_core::state::identity;

/// Seed the engine's macro table from disk, once per character per VM.
fn load_macros(
    script: Option<NonSendMut<UiScript>>,
    roster: Res<crate::app_core::state::Roster>,
    mut files: ResMut<MacroFiles>,
) {
    let Some(mut script) = script else { return };
    let Some(id) = identity(&roster) else { return };
    if files.identity.get(&script).as_ref() == Some(&id) {
        return; // already loaded into this VM
    }
    let (realm, character) = (&id.0, &id.1);
    files.account = crate::local_state::macros_account_path();
    files.character = crate::local_state::macros_character_path(realm, character);
    *files.identity.get(&script) = Some(id.clone());

    let read = |path: &Option<std::path::PathBuf>, base| -> Vec<wrathplus_ui::script::MacroView> {
        let Some(path) = path else { return Vec::new() };
        match std::fs::read_to_string(path) {
            Ok(text) => store::parse(&text, base),
            // Absent is the first run, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                warn!("ui_macro: reading {}: {e}", path.display());
                Vec::new()
            }
        }
    };
    let state = MacroState {
        account: read(&files.account, 0),
        character: read(&files.character, wrathplus_ui::script::CHARACTER_MACRO_BASE),
    };
    info!(
        "ui_macro: {} account + {} character macros for {character} on {realm}",
        state.account.len(),
        state.character.len()
    );
    script.set_macros(state);
    // `UPDATE_MACROS` (string at `0x852460`): the macro frame redraws on it, as after an edit.
    script.fire_event("UPDATE_MACROS", vec![]);
}

/// Persist on the engine's dirty edge, and fire `UPDATE_MACROS`.
fn save_dirty_macros(script: Option<NonSendMut<UiScript>>, files: Res<MacroFiles>) {
    let Some(mut script) = script else { return };
    if !script.take_macros_dirty() {
        return;
    }
    let state = script.macros();
    // Fire first: the redraw must not depend on the write landing.
    script.fire_event("UPDATE_MACROS", vec![]);
    for (path, macros) in [
        (&files.account, &state.account),
        (&files.character, &state.character),
    ] {
        let Some(path) = path else { continue };
        if let Err(e) = crate::local_state::write_atomic(path, &store::write(macros)) {
            warn!("ui_macro: saving {}: {e}", path.display());
        }
    }
}

/// Recompute every macro's bound spell when the macro table or the spell book changes: gated on
/// the engine's macro generation and on change detection over the action store.
fn rebind_macro_spells(
    script: Option<NonSendMut<UiScript>>,
    actions: Res<crate::app_core::state::PlayerActions>,
    table: Option<Res<crate::ui_chat::commands::SlashCommands>>,
    mut bound: ResMut<MacroBoundSpells>,
    mut last_generation: Local<crate::app_core::vm_memo::VmMemo<Option<u64>>>,
) {
    let (Some(mut script), Some(table)) = (script, table) else {
        return;
    };
    // Per VM: a fresh VM restarts its generation at 0, which a bare memo would gate off for good.
    let last_generation = last_generation.get(&script);
    let generation = script.macros_generation();
    if *last_generation == Some(generation) && !actions.is_changed() {
        return;
    }
    *last_generation = Some(generation);

    let (macros, book) = (script.macros(), script.spellbook());
    let mut fresh = std::collections::HashMap::new();
    // By macro id, which the action words carry.
    for m in macros.account.iter().chain(&macros.character) {
        fresh.insert(m.id, run::bound_spell(&table, &m.body, &book));
    }
    // A `SetMacroSpell`/`SetMacroItem` pair replaces the body's (the record leaves the re-derive
    // list, `0x565ad0`), and both setters refresh the bar's slots for the macro (`0x5aa920`).
    let explicit = script.macro_explicit_binds();
    for (id, e) in &explicit {
        if let Some(b) = fresh.get_mut(id) {
            *b = run::explicit_bound(e);
        }
    }
    // The engine's copy, for `GetActionAutocast`, re-pushed on every recompute so a fresh VM has
    // it. The book flag is the player's unless a `SetMacroSpell` found the spell in the pet's.
    script.set_macro_bindings(
        fresh
            .iter()
            .map(|(&id, b)| {
                let spell = match *b {
                    BoundSpell::None | BoundSpell::Item(_) => 0,
                    BoundSpell::Unresolved => -1,
                    BoundSpell::Spell(id) => id as i32,
                };
                (
                    id,
                    wrathplus_ui::script::MacroBinding {
                        spell,
                        pet_book: explicit.get(&id).is_some_and(|e| e.pet_book),
                    },
                )
            })
            .collect(),
    );
    if fresh != bound.0 {
        let spells = fresh
            .values()
            .filter(|b| matches!(b, BoundSpell::Spell(_)))
            .count();
        debug!(
            "ui_macro: {} macro(s), {spells} bound to a spell",
            fresh.len()
        );
        bound.0 = fresh;
    }
}

/// The event each macro line is fired as: id `0x188`, whose registry slot `0xbe17b8` is written
/// once, at `0x51b4ff`, with the string at `0x852470`.
const EXECUTE_CHAT_LINE: &str = "EXECUTE_CHAT_LINE";

/// Run free macro text, `RunMacroText`'s body (`0x566400` → `0x564db0`, the macro runner), as a
/// macro body's lines run: a scratch record with no id, so `GetRunningMacro` answers nil and
/// `GetRunningMacroButton` the button.
pub(crate) fn run_macro_text(script: &mut UiScript, text: &str, button: Option<String>) {
    let lines: Vec<String> = run::macro_lines(text).map(str::to_string).collect();
    let previous = script.begin_running_macro(RunningMacro { id: None, button });
    for line in lines {
        script.fire_event(EXECUTE_CHAT_LINE, vec![ScriptValue::Str(line)]);
    }
    script.end_running_macro(previous);
}

/// Run a macro by its 1-based index; returns whether anything ran. Like the reference's runner
/// (`0x4f14e0`, reached only from `UseAction` at `0x4e6098`), it fires `EXECUTE_CHAT_LINE` per
/// non-empty line and nothing else: the stock `ChatFrame1` sends each through its edit box
/// (`ChatFrame.lua:1343`), and an addon registered for the event sees every macro line.
/// `GetRunningMacro` and `GetRunningMacroButton` answer the macro and `button` while its lines
/// run, and the caller's run after (`0x564db0` saves and restores both).
pub(crate) fn run_macro(script: &mut UiScript, index: u32, button: Option<String>) -> bool {
    let Some((id, body)) = script
        .macros()
        .get(index as usize)
        .map(|m| (m.id, m.body.clone()))
    else {
        return false;
    };
    let lines: Vec<String> = run::macro_lines(&body).map(str::to_string).collect();
    if lines.is_empty() {
        return false;
    }
    debug!("ui_macro: running macro {index} ({} line(s))", lines.len());
    let previous = script.begin_running_macro(RunningMacro {
        id: Some(id),
        button,
    });
    for line in lines {
        script.fire_event(EXECUTE_CHAT_LINE, vec![ScriptValue::Str(line)]);
    }
    script.end_running_macro(previous);
    true
}
