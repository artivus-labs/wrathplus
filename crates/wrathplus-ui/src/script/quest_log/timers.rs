//! The timed-quest verbs `QuestTimerFrame.lua` and the log's detail pane read, over every row of
//! the log, folded ones included, as 12340's timer scans run to the row count `[0xc23ad0]`
//! (`0x5e6277`, `0x5e5020`).

use mlua::{Lua, MultiValue, Value};

use super::super::Model;
use super::QuestLogEntryView;

/// Signed seconds left on a row's timer, by the reference's formula at all four of its sites
/// (`0x4e0905`, `0x4e14b9`, `0x4e16c9`, `0x4de6b6`): `slot.timer + G - now - 1`, `G` folded into
/// `now`, so a quest's last second reads 0. A negative value drops the row from the lists (`js` at
/// `0x4e14c5`) and clamps to 0 in `GetQuestLogTimeLeft`. `None` when there is nothing to show:
/// - an untimed row, or a header (skipped at `0x4e1460`);
/// - a failed row: vmangos sets its timer to 1 (`Player.cpp:13430`), and the reference tests the
///   fail bit `[slot+0x7] & 0x2` first;
/// - no server clock yet, the reference's never-synced `G == 0`.
fn seconds_left(entry: &QuestLogEntryView, now: Option<f64>) -> Option<i64> {
    if entry.timer == 0 || entry.complete < 0 {
        return None;
    }
    let now = now?;
    Some((f64::from(entry.timer) - now - 1.0).floor() as i64)
}

/// [`seconds_left`] for the list verbs, which drop an expired row as the reference's `js` does.
fn live_seconds_left(entry: &QuestLogEntryView, now: Option<f64>) -> Option<i64> {
    seconds_left(entry, now).filter(|&s| s >= 0)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // One return per live timer, in log order, re-read every OnUpdate: `QuestTimerFrame_Update`
    // takes them as varargs and hides on none (`QuestTimerFrame.lua:11`, `:26`, `:37`).
    g.set(
        "GetQuestTimers",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let now = model.server_unix_time;
            Ok(MultiValue::from_vec(
                model
                    .quest_log
                    .entries
                    .iter()
                    .filter_map(|e| live_seconds_left(e, now))
                    .map(Value::Integer)
                    .collect(),
            ))
        })?,
    )?;

    // A timer's 1-based row, headers counted, for the timer buttons' click and tooltip
    // (`QuestTimerFrame.lua:43`, `QuestTimerFrame.xml:20`).
    g.set(
        "GetQuestIndexForTimer",
        lua.create_function(|lua, t: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let now = model.server_unix_time;
            let index = t.checked_sub(1).and_then(|n| {
                model
                    .quest_log
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| live_seconds_left(e, now).is_some())
                    .nth(n)
                    .map(|(i, _)| i as i64 + 1)
            });
            Ok(match index {
                Some(i) => Value::Integer(i),
                None => Value::Nil,
            })
        })?,
    )?;

    // The selection's seconds left, nil without a timer, which hides the stock "Time Remaining:"
    // row (`QuestLogFrame.lua:364`).
    g.set(
        "GetQuestLogTimeLeft",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let left = (model.quest_log_selection as usize)
                .checked_sub(1)
                .and_then(|n| model.quest_log.entries.get(n))
                .and_then(|e| seconds_left(e, model.server_unix_time));
            Ok(match left {
                // Clamped where the lists drop the row, as in the reference.
                Some(s) => Value::Integer(s.max(0)),
                None => Value::Nil,
            })
        })?,
    )?;
    Ok(())
}
