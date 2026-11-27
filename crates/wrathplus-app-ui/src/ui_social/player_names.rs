//! The name cache as `GetPlayerInfoByGUID` reads it: every player a name query answered, with the
//! race, class and gender the answer carried, pushed whole when the cache's generation moves.
//! Cost: one map of the session's answered names (the cache is cleared at world entry), rebuilt
//! once in a frame where an answer lands; the VM takes it behind an `Arc`.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use wrathplus_ui::script::{PlayerNameInfo, UiScript};

use crate::app_core::catalogs::{class_names, race_names};
use crate::names::NameCache;

/// Push the answered names when the cache moved, or into a fresh VM.
pub fn feed_player_names(
    script: Option<NonSendMut<UiScript>>,
    names: Res<NameCache>,
    mut pushed: Local<crate::app_core::vm_memo::VmMemo<Option<u64>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let pushed = pushed.get(&script);
    if *pushed == Some(names.generation()) {
        return;
    }
    *pushed = Some(names.generation());
    let pair = |n: Option<(&str, &str)>| n.map(|(a, b)| (a.to_string(), b.to_string()));
    let table: HashMap<u64, PlayerNameInfo> = names
        .answered_players()
        .map(|(guid, name, (race, class, gender))| {
            let info = PlayerNameInfo {
                name: name.to_string(),
                class: pair(class_names(class)),
                race: pair(race_names(race)),
                gender,
            };
            (guid, info)
        })
        .collect();
    script.set_player_names(Arc::new(table));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name answered with its traits reaches the binding; a name learned without them does not.
    #[test]
    fn answered_names_reach_get_player_info_by_guid() {
        let mut app = App::new();
        app.init_resource::<NameCache>()
            .insert_non_send_resource(UiScript::new().unwrap())
            .add_systems(Update, feed_player_names);
        let mut names = app.world_mut().resource_mut::<NameCache>();
        names.insert_player(0x99, "Grunt".into(), Some((2, 1, 1)));
        names.insert_player(0x98, "Seeded".into(), None);
        app.update();
        let script = app.world().non_send_resource::<UiScript>();
        assert_eq!(
            script
                .eval::<(String, String, i64, String, i64)>(
                    "local c, _, r, _, sex, n = GetPlayerInfoByGUID('0x99') \
                     return c, r, sex, n, select('#', GetPlayerInfoByGUID('0x98'))"
                )
                .unwrap(),
            ("Warrior".into(), "Orc".into(), 3, "Grunt".into(), 0)
        );
    }
}
