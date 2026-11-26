//! The tracked achievements across sessions and reloads. 12340 keeps them in the per-character
//! CVar `trackedAchievements` (registered at `0x51e027`, flags `0x120`, default empty), packed in
//! the character's config cache: read once when the character's settings arrive (`0x518c66` →
//! `0x5128d0` → `0x5b3840`), each id re-added through `0x5b23e0`, and written on leaving the
//! world (`0x529071` → `0x5b1fa0`). The list is client state, so a `ReloadUI` keeps it.
//!
//! Ours is `wrathplus-config/achievements/<realm>-<character>.txt`, the ids in order on one line,
//! written at logout, disconnect and quit, beside the tracked quests' file. The CVar keeps its
//! registered default: 12340 writes it only at that save, never as the list changes.

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

use crate::app_core::state::ClientState;
use crate::app_core::vm_memo::VmMemo;
use crate::ui_quest::lines::watch_saved::CharacterIds;

/// The character's saved list, and whether it was just read: the first VM to take it gets the
/// load's events, a reloaded one is seated silently.
#[derive(Resource, Default)]
pub(crate) struct TrackedAchievementsFile {
    file: CharacterIds,
    loaded: bool,
}

/// Reads the file when the roster names a new character, seats the list into each in-game VM
/// once, then follows the VM's list.
fn sync_tracked_achievements(
    script: Option<NonSendMut<UiScript>>,
    roster: Res<crate::app_core::state::Roster>,
    mut saved: ResMut<TrackedAchievementsFile>,
    mut seated: Local<VmMemo<bool>>,
) {
    let Some(mut script) = script else { return };
    let Some(id) = crate::app_core::state::identity(&roster) else {
        return;
    };
    if saved.file.select("achievements", id) {
        saved.loaded = true;
    }
    let seated = seated.get(&script);
    follow(&mut script, &mut saved, seated);
}

fn follow(script: &mut UiScript, saved: &mut TrackedAchievementsFile, seated: &mut bool) {
    if !*seated {
        let fire = std::mem::take(&mut saved.loaded);
        script.restore_wrath_tracked_achievements(&saved.file.ids, fire);
        *seated = true;
    }
    saved.file.ids = script.wrath_tracked_achievements();
}

/// `OnExit(InWorld)`: `/logout` back to the glue, or a disconnect. The next login re-reads.
fn save_on_session_end(mut saved: ResMut<TrackedAchievementsFile>) {
    saved.file.save();
    *saved = TrackedAchievementsFile::default();
}

/// `AppExit`, as a quit from in-world never leaves `InWorld`.
fn save_on_exit(saved: Res<TrackedAchievementsFile>, mut exits: MessageReader<AppExit>) {
    if exits.read().next().is_some() {
        saved.file.save();
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<TrackedAchievementsFile>()
        .add_systems(
            Update,
            // After the feed, so an earn's untrack is in the list the save writes.
            sync_tracked_achievements
                .in_set(crate::app_core::schedule::UiFeed)
                .after(super::wrath_achievements::feed_achievements)
                .run_if(crate::app_core::ui_state::ingame_ui_up),
        )
        .add_systems(OnExit(ClientState::InWorld), save_on_session_end);
    crate::shutdown::on_app_exit(app, save_on_exit.into_configs());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first VM is seated from the file with one event per id under the cap of ten; the list
    /// then follows the VM; a reloaded VM is seated with it, without events.
    #[test]
    fn a_vm_is_seated_once_from_the_list_and_a_reload_keeps_it_silently() {
        let mut saved = TrackedAchievementsFile {
            loaded: true,
            ..Default::default()
        };
        saved.file.ids = (1..=11).collect();
        let events = "n = 0 local f = CreateFrame('Frame') \
            f:RegisterEvent('TRACKED_ACHIEVEMENT_UPDATE') f:SetScript('OnEvent', function() n = n + 1 end)";
        let (mut s, mut seated) = (UiScript::new().unwrap(), false);
        s.run(events).unwrap();
        follow(&mut s, &mut saved, &mut seated);
        s.run("RemoveTrackedAchievement(1)").unwrap();
        follow(&mut s, &mut saved, &mut seated);
        s.tick(0.0);
        assert_eq!(
            s.eval::<i64>("return n").unwrap(),
            11,
            "ten loaded and one removed"
        );
        assert_eq!(saved.file.ids, (2..=10).collect::<Vec<_>>());

        let (mut reloaded, mut seated) = (UiScript::new().unwrap(), false);
        reloaded.run(events).unwrap();
        follow(&mut reloaded, &mut saved, &mut seated);
        reloaded.tick(0.0);
        assert_eq!(reloaded.wrath_tracked_achievements(), saved.file.ids);
        assert_eq!(reloaded.eval::<i64>("return n").unwrap(), 0);
    }
}
