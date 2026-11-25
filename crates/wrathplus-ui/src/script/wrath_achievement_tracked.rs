//! The tracked achievements (`[0xacfde8]`, oldest first): `AddTrackedAchievement` (`0x5b4870`)
//! appends through `0x5b23e0`, `RemoveTrackedAchievement` (`0x5b48d0`) drops through `0x5b24d0`,
//! and each fires `TRACKED_ACHIEVEMENT_UPDATE` with the id alone (`"%d"`, `0x9e289c`).

use super::wrath_achievement_progress::State;
use super::ScriptValue;
use mlua::{Lua, Value};

/// `0x5b23e0` refuses a new id once ten are tracked (`cmp edx,0xa`, `0x5b2407`).
const MAX_TRACKED: usize = 10;

fn updated(lua: &Lua, id: u32) {
    let mut model = lua.app_data_mut::<super::Model>().expect("model");
    model.pending_events.push((
        "TRACKED_ACHIEVEMENT_UPDATE".into(),
        vec![ScriptValue::Int(id.into())],
    ));
}

/// `0x5b23e0`: a tracked id fires again, a new one is appended under the cap.
fn track(lua: &Lua, id: u32) {
    let fire = lua.app_data_mut::<State>().is_some_and(|mut s| {
        if s.tracked.contains(&id) {
            return true;
        }
        let room = s.tracked.len() < MAX_TRACKED;
        if room {
            s.tracked.push(id);
        }
        room
    });
    if fire {
        updated(lua, id);
    }
}

/// `0x5b24d0`: every node with the id goes, and the event fires when one did.
pub(super) fn untrack(lua: &Lua, id: u32) {
    let removed = lua.app_data_mut::<State>().is_some_and(|mut s| {
        let before = s.tracked.len();
        s.tracked.retain(|&t| t != id);
        s.tracked.len() != before
    });
    if removed {
        updated(lua, id);
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "AddTrackedAchievement",
        lua.create_function(|lua, id: Value| {
            let id = lua.coerce_number(id)?.unwrap_or(0.0) as u32;
            let known = lua
                .app_data_ref::<super::Model>()
                .and_then(|m| {
                    m.wrath_achievements
                        .as_ref()
                        .map(|m| m.achievements.contains_key(&id))
                })
                .unwrap_or(false);
            if id == 0 || !known {
                return Err(mlua::Error::RuntimeError(
                    "Usage: AddTrackedAchievement(achievementID)".into(),
                ));
            }
            track(lua, id);
            Ok(())
        })?,
    )?;
    g.set(
        "RemoveTrackedAchievement",
        lua.create_function(|lua, id: Value| {
            let id = lua.coerce_number(id)?.unwrap_or(0.0) as u32;
            if id == 0 {
                return Err(mlua::Error::RuntimeError(
                    "Usage: RemoveTrackedAchievement(achievementID)".into(),
                ));
            }
            untrack(lua, id);
            Ok(())
        })?,
    )?;
    g.set(
        "GetTrackedAchievements",
        lua.create_function(|lua, ()| {
            let tracked = lua
                .app_data_ref::<State>()
                .map(|s| s.tracked.clone())
                .unwrap_or_default();
            Ok(mlua::MultiValue::from_vec(
                tracked
                    .into_iter()
                    .map(|id| Value::Integer(id.into()))
                    .collect(),
            ))
        })?,
    )?;
    g.set(
        "GetNumTrackedAchievements",
        lua.create_function(|lua, ()| {
            Ok(lua.app_data_ref::<State>().map_or(0, |s| s.tracked.len()))
        })?,
    )?;
    g.set(
        "IsTrackedAchievement",
        lua.create_function(|lua, id: Value| {
            let id = lua.coerce_number(id)?.unwrap_or(0.0) as u32;
            Ok(id != 0
                && lua
                    .app_data_ref::<State>()
                    .is_some_and(|s| s.tracked.contains(&id)))
        })?,
    )?;
    Ok(())
}
