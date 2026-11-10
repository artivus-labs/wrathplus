//! The tracked quests across sessions. 12340 keeps them in the per-character CVar
//! `trackedQuests` (registered at `0x51e006`, flags `0x120`, default empty): the quest-log build
//! reads it once per session, on the first build with rows (`0x5e6c18` → `0x5e65b0`, latch
//! `[0xc23af0]`), adding each id as a permanent watch (`0x5e64d0(id, 0)`), and the world-leave
//! sequence (`0x52906c` → `0x5e0240`) writes the permanent watches back; a timed watch is never
//! saved.
//!
//! Ours is `wrathplus-config/quests/<realm>-<character>.txt`, the ids in watch order on one line,
//! written at logout, disconnect and quit; the reference keeps the CVar in the character's
//! config cache. The watch table is the client's, so a `ReloadUI` keeps it: the old VM's
//! permanent watches are seated into the new one on its first log push.

use std::path::PathBuf;

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

use crate::app_core::state::ClientState;
use crate::app_core::vm_memo::VmMemo;

/// The character whose list is live, its file, and the permanent watches as last seen: the save
/// runs after the VM may be gone, and a new VM is seated from them.
#[derive(Resource, Default)]
pub(crate) struct TrackedQuestsFile {
    identity: Option<(String, String)>,
    path: Option<PathBuf>,
    watched: Vec<u32>,
}

/// `wrathplus-config/quests/<realm>-<character>.txt`, named as the layout cache's file is.
fn path_for(realm: &str, character: &str) -> Option<PathBuf> {
    let layout = crate::local_state::layout_character_path(realm, character)?;
    Some(
        layout
            .parent()?
            .parent()?
            .join("quests")
            .join(layout.file_name()?),
    )
}

/// The ids in file order; anything that is not a quest id is skipped.
fn parse(text: &str) -> Vec<u32> {
    text.split_whitespace()
        .filter_map(|t| t.parse().ok())
        .filter(|&id| id != 0)
        .collect()
}

fn render(ids: &[u32]) -> String {
    let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
    format!("{}\n", ids.join(" "))
}

/// Reads the file when the roster names a new character, seats the list into each VM on its first
/// push with a quest row, then follows the VM's permanent watches. The seat fires
/// `QUEST_LOG_UPDATE` so the tracker draws the restored watches; the reference seats them inside
/// the log build, ahead of that build's own update.
pub(crate) fn sync_tracked_quests(
    script: Option<NonSendMut<UiScript>>,
    roster: Res<crate::app_core::state::Roster>,
    mut file: ResMut<TrackedQuestsFile>,
    mut seated: Local<VmMemo<bool>>,
) {
    let Some(mut script) = script else { return };
    let Some(id) = crate::app_core::state::identity(&roster) else {
        return;
    };
    if file.identity.as_ref() != Some(&id) {
        file.path = path_for(&id.0, &id.1);
        file.identity = Some(id);
        file.watched = match file.path.as_ref().map(std::fs::read_to_string) {
            Some(Ok(text)) => parse(&text),
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => {
                warn!("tracked quests: cannot read: {e}");
                Vec::new()
            }
            _ => Vec::new(),
        };
    }
    let seated = seated.get(&script);
    follow(&mut script, &mut file.watched, seated);
}

/// Seats `watched` into a VM not yet `seated` once its log has a quest row, then follows the VM's
/// permanent watches into it.
fn follow(script: &mut UiScript, watched: &mut Vec<u32>, seated: &mut bool) {
    if !*seated {
        let has_rows = script.eval::<bool>(
            "for i = 1, GetNumQuestLogEntries() do \
             if not select(5, GetQuestLogTitle(i)) then return true end end return false",
        );
        if !has_rows.unwrap_or(false) {
            return;
        }
        script.restore_quest_watches(watched);
        script.fire_event("QUEST_LOG_UPDATE", vec![]);
        *seated = true;
    }
    *watched = script.quest_watches_permanent();
}

fn save(file: &TrackedQuestsFile) {
    let Some(path) = &file.path else {
        return; // never loaded: a glue-only run or a hermetic capture
    };
    if let Err(e) = crate::local_state::write_atomic(path, &render(&file.watched)) {
        warn!("tracked quests: cannot write {}: {e}", path.display());
    }
}

/// `OnExit(InWorld)`: `/logout` back to the glue, or a disconnect. The next login re-reads.
fn save_on_session_end(mut file: ResMut<TrackedQuestsFile>) {
    save(&file);
    *file = TrackedQuestsFile::default();
}

/// `AppExit`, as a quit from in-world never leaves `InWorld`.
fn save_on_exit(file: Res<TrackedQuestsFile>, mut exits: MessageReader<AppExit>) {
    if exits.read().next().is_some() {
        save(&file);
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<TrackedQuestsFile>()
        .add_systems(
            Update,
            sync_tracked_quests
                .in_set(crate::app_core::schedule::UnitFeed)
                .after(crate::ui_quest_log::feed_quest_log),
        )
        .add_systems(OnExit(ClientState::InWorld), save_on_session_end);
    crate::shutdown::on_app_exit(app, save_on_exit.into_configs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_round_trips_and_junk_is_skipped() {
        assert_eq!(render(&[808, 818, 789]), "808 818 789\n");
        assert_eq!(parse("808 818 789\n"), vec![808, 818, 789]);
        assert_eq!(parse("808\r\n x 0 -3 818"), vec![808, 818]);
        assert_eq!(render(&[]), "\n");
    }

    /// A VM is seated once its log has a quest row, with the saved ids still in the log, as
    /// permanent watches after any it already has; a timed watch is followed but not saved; a new
    /// VM (a `ReloadUI`) is seated with the permanent watches the old one ended with.
    #[test]
    fn a_vm_is_seated_once_from_the_list_and_follows_its_permanent_watches() {
        use wrathplus_ui::script::{QuestLogEntryView, QuestLogState};
        let log = || QuestLogState {
            entries: [808, 818, 789]
                .map(|id| QuestLogEntryView {
                    quest_id: id,
                    title: format!("Quest {id}"),
                    level: 5,
                    ..Default::default()
                })
                .to_vec(),
            num_quests: 3,
        };
        let mut watched = vec![818, 4242, 808];
        let mut seated = false;
        let mut s = UiScript::new().unwrap();
        follow(&mut s, &mut watched, &mut seated);
        assert!(!seated, "no quest row yet: the list waits");
        assert_eq!(watched, [818, 4242, 808]);

        s.set_quest_log(log());
        follow(&mut s, &mut watched, &mut seated);
        assert!(seated);
        assert_eq!(
            s.quest_log_watched(),
            [818, 808],
            "a quest not in the log is dropped"
        );
        s.run("AddQuestWatch(3, 300)").unwrap();
        follow(&mut s, &mut watched, &mut seated);
        assert_eq!(s.quest_log_watched(), [818, 808, 789]);
        assert_eq!(watched, [818, 808], "the timed watch is not saved");

        let mut reloaded = UiScript::new().unwrap();
        reloaded.set_quest_log(log());
        let mut seated = false;
        follow(&mut reloaded, &mut watched, &mut seated);
        assert_eq!(reloaded.quest_log_watched(), [818, 808]);
    }

    #[test]
    fn the_file_sits_beside_the_layout_cache() {
        let quests = path_for("Realm", "Char").unwrap();
        let layout = crate::local_state::layout_character_path("Realm", "Char").unwrap();
        assert_eq!(quests.file_name(), layout.file_name());
        assert!(quests.parent().unwrap().ends_with("quests"));
    }
}
