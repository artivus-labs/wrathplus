//! The quest watch set behind the tracker: `AddQuestWatch`, `RemoveQuestWatch`, the queries and
//! `ShiftQuestWatches`, keyed by quest id so a log shuffle keeps its watches.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use mlua::{Lua, Value};

use super::super::binding_abi::flag;
use super::super::Model;

/// The watched quest ids in watch order, and a timed watch's expiry in `GetTime` seconds (a
/// permanent watch has none); `set_quest_log` prunes a quest that left the log.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuestWatches {
    pub(crate) ids: Vec<u32>,
    pub(crate) expiry: HashMap<u32, f64>,
}

impl QuestWatches {
    /// Drops the expiry of a quest no longer watched.
    pub(crate) fn prune_expiry(&mut self) {
        let ids = &self.ids;
        self.expiry.retain(|id, _| ids.contains(id));
    }
}

/// The watch set's cap, `0x19` in `0x5e64d0`; stock FrameXML's `MAX_WATCHABLE_QUESTS` agrees.
const MAX_WATCHES: usize = 25;

/// The expiry sweep (`0x5dfef0`): drops every timed watch whose expiry has passed and answers how
/// many, as the reference fires `QUEST_LOG_UPDATE` (`0x615020(0x125)`) once per dropped watch. The
/// verbs never expire a watch themselves: one lapsed between sweeps still reads as watched.
pub(crate) fn sweep(lua: &Lua) -> usize {
    let now = super::super::clock::now(lua);
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let QuestWatches { ids, expiry } = &mut model.quest_watches;
    let before = ids.len();
    expiry.retain(|id, at| {
        let live = *at > now;
        if !live {
            ids.retain(|w| w != id);
        }
        live
    });
    before - ids.len()
}

/// `0x5e64d0(id, 0)`, the `trackedQuests` loader's add (`0x5e65b0`): a permanent watch on a quest
/// in the log, under the cap; an id already watched keeps its watch.
pub(crate) fn restore(model: &mut Model, ids: &[u32]) {
    for &id in ids {
        let in_log = model
            .quest_log
            .entries
            .iter()
            .any(|e| !e.is_header && e.quest_id == id)
            || model.quest_log.hidden_quest_ids.contains(&id);
        let watches = &mut model.quest_watches;
        if in_log && !watches.ids.contains(&id) && watches.ids.len() < MAX_WATCHES {
            watches.ids.push(id);
        }
    }
}

// A quest row's id; a header has none (`0x5e67c0` reads `[0xc237b8]`, the header flag).
fn watch_id_at(model: &Model, index: u32) -> Option<u32> {
    (index as usize)
        .checked_sub(1)
        .and_then(|n| model.quest_log.entries.get(n))
        .filter(|e| !e.is_header)
        .map(|e| e.quest_id)
}

impl crate::script::UiScript {
    /// The permanent watches in watch order, what the reference saves to `trackedQuests` at
    /// logout (`0x5e0240`): a timed watch (`AddQuestWatch(i, seconds)`) is not saved.
    pub fn quest_watches_permanent(&self) -> Vec<u32> {
        let model = self.model_ref();
        let w = &model.quest_watches;
        w.ids
            .iter()
            .copied()
            .filter(|id| !w.expiry.contains_key(id))
            .collect()
    }

    /// Re-adds saved watches as permanent ones, each on a quest in the current log, as the
    /// reference's `trackedQuests` loader (`0x5e65b0`) does on the session's first log build.
    pub fn restore_quest_watches(&mut self, ids: &[u32]) {
        restore(&mut self.model_mut(), ids);
    }
}

/// A greeting row's `isComplete` (`0x58bcb0`): the quest's log row, done by `0x5e0ea0` (its
/// objectives met) or `0x5e10b0` (nothing to kill or collect). Labelled: read off the log's state
/// (complete, or every objective line finished), not the template's counters; a quest not in the
/// log is not complete.
pub(crate) fn quest_done(model: &Model, quest_id: u32) -> bool {
    model
        .quest_log
        .entries
        .iter()
        .find(|e| !e.is_header && e.quest_id == quest_id)
        .is_some_and(|e| {
            e.complete == 1 || (e.complete == 0 && e.objectives.iter().all(|o| o.finished))
        })
}

/// `0x58aba0`, the grey-quest test: the player is above the quest's level by more than
/// `[0xa12fd0][player level / 5]` (4, 4, 5, 5, 6, 6, 7, 7, then 8), compared signed. A level -1
/// quest scales with the player and is never trivial (`0x58aba7`). No player, not trivial.
pub(crate) fn quest_trivial(model: &Model, quest_level: i32) -> bool {
    const MARGIN: [i64; 8] = [4, 4, 5, 5, 6, 6, 7, 7];
    let Some(player) = model.unit("player").map(|u| i64::from(u.level)) else {
        return false;
    };
    let margin = MARGIN.get(player as usize / 5).copied().unwrap_or(8);
    quest_level != -1 && i64::from(quest_level) + margin < player
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // WatchFrame.lua:1468 passes watch positions, not quest-log indices.
    g.set(
        "ShiftQuestWatches",
        lua.create_function(|lua, (from, to): (usize, usize)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let watches = &mut model.quest_watches.ids;
            if from > 0 && to > 0 && from <= watches.len() && to <= watches.len() && from != to {
                let id = watches.remove(from - 1);
                watches.insert(to - 1, id);
            }
            Ok(())
        })?,
    )?;
    g.set(
        "GetQuestWatchIndex",
        lua.create_function(|lua, index: u32| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(watch_id_at(&model, index).and_then(|id| {
                model
                    .quest_watches
                    .ids
                    .iter()
                    .position(|w| *w == id)
                    .map(|i| i + 1)
            }))
        })?,
    )?;

    // The watch set, keyed by quest id (`[0xc23680]`, 12-byte rows, count `[0xc23ae0]`).
    // AddQuestWatch(index[, seconds]) (`0x5e67c0` → `0x5e64d0`): at most 25 (`0x19`), a header or
    // out-of-range row adds nothing. `seconds` arms a timed watch; re-adding a permanent watch
    // changes nothing, and re-adding a timed one re-arms it, or makes it permanent with no
    // seconds. The native stamps `time()`; here the expiry is a `GetTime` stamp, dropped by
    // [`sweep`].
    g.set(
        "GetNumQuestWatches",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.quest_watches.ids.len() as i64)
        })?,
    )?;
    g.set(
        "IsQuestWatched",
        lua.create_function(|lua, i: u32| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(
                watch_id_at(&model, i).is_some_and(|id| model.quest_watches.ids.contains(&id)),
            ))
        })?,
    )?;
    g.set(
        "AddQuestWatch",
        lua.create_function(|lua, (i, seconds): (u32, Option<f64>)| {
            let now = super::super::clock::now(lua);
            let seconds = seconds.map_or(0, |s| s.round() as i64);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(id) = watch_id_at(&model, i) else {
                return Ok(());
            };
            let arm = (seconds != 0).then_some(now + seconds as f64);
            if model.quest_watches.ids.contains(&id) {
                if let Entry::Occupied(mut timed) = model.quest_watches.expiry.entry(id) {
                    match arm {
                        Some(at) => {
                            timed.insert(at);
                        }
                        None => {
                            timed.remove();
                        }
                    }
                }
            } else if model.quest_watches.ids.len() < MAX_WATCHES {
                model.quest_watches.ids.push(id);
                if let Some(at) = arm {
                    model.quest_watches.expiry.insert(id, at);
                }
            }
            Ok(())
        })?,
    )?;
    g.set(
        "RemoveQuestWatch",
        lua.create_function(|lua, i: u32| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some(id) = watch_id_at(&model, i) {
                model.quest_watches.ids.retain(|w| *w != id);
                model.quest_watches.expiry.remove(&id);
            }
            Ok(())
        })?,
    )?;
    // The watched quest's current 1-based row, nil when it is not a visible row.
    g.set(
        "GetQuestIndexForWatch",
        lua.create_function(|lua, w: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let index = w
                .checked_sub(1)
                .and_then(|n| model.quest_watches.ids.get(n))
                .and_then(|id| {
                    model
                        .quest_log
                        .entries
                        .iter()
                        .position(|e| e.quest_id == *id)
                })
                .map(|p| p as i64 + 1);
            Ok(match index {
                Some(i) => Value::Integer(i),
                None => Value::Nil,
            })
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{QuestLogEntryView, QuestLogState};
    use crate::script::UiScript;

    #[test]
    fn watch_cap_is_twenty_five_and_dupes_are_ignored() {
        let mut s = UiScript::new().unwrap();
        let mut state = QuestLogState::default();
        for q in 1..=26u32 {
            state.entries.push(QuestLogEntryView {
                quest_id: q,
                title: format!("Quest {q}"),
                level: 1,
                ..Default::default()
            });
        }
        s.set_quest_log(state);
        for i in 1..=26 {
            s.run(&format!("AddQuestWatch({i})")).unwrap();
        }
        // `0x5e64d0` stops at `0x19`.
        assert_eq!(s.eval::<i64>("return GetNumQuestWatches()").unwrap(), 25);
        s.run("AddQuestWatch(1)").unwrap();
        assert_eq!(s.eval::<i64>("return GetNumQuestWatches()").unwrap(), 25);
        assert_eq!(s.quest_log_watched(), (1..=25).collect::<Vec<u32>>());
    }

    /// `AddQuestWatch(index, seconds)` arms a timed watch that re-arms on a re-add and expires;
    /// a re-add without seconds makes it permanent, and a permanent watch ignores seconds.
    #[test]
    fn a_timed_watch_re_arms_expires_and_a_permanent_one_stays() {
        let mut s = UiScript::new().unwrap();
        let mut state = QuestLogState::default();
        for q in 1..=3u32 {
            state.entries.push(QuestLogEntryView {
                quest_id: q,
                title: format!("Quest {q}"),
                level: 1,
                ..Default::default()
            });
        }
        state.entries[2].is_header = true;
        s.set_quest_log(state);
        let watched = |s: &UiScript, i: u32| {
            s.eval::<bool>(&format!("return IsQuestWatched({i}) == 1"))
                .unwrap()
        };
        s.tick(10.0);
        s.run("AddQuestWatch(1, 300) AddQuestWatch(2) AddQuestWatch(2, 300) AddQuestWatch(3)")
            .unwrap();
        assert!(!watched(&s, 3), "a header adds nothing");
        s.tick(299.0);
        assert!(watched(&s, 1));
        s.run("AddQuestWatch(1, 300)").unwrap();
        s.tick(299.0);
        assert!(watched(&s, 1), "re-armed");
        s.tick(2.0);
        assert!(!watched(&s, 1), "expired 300 s after the last arm");
        assert!(
            watched(&s, 2),
            "a permanent watch ignores a later timed add"
        );
        s.run("AddQuestWatch(1, 300) AddQuestWatch(1)").unwrap();
        s.tick(400.0);
        assert!(watched(&s, 1), "a re-add without seconds made it permanent");
    }
}
