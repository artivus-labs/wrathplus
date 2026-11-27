//! Build-12340 saved instances (`RaidInfoFrame`): `RequestRaidInfo` (`0x500be0`, CMSG `0x2CD`),
//! `GetNumSavedInstances` (`0x4fca50`), `GetSavedInstanceInfo` (`0x4ff2c0`) and
//! `SetSavedInstanceExtend` (`0x50dc10`), over the list App publishes from each
//! `SMSG_RAID_INSTANCE_INFO` (receiver `0x501030`, which then fires `UPDATE_INSTANCE_INFO`).
//! Evidence: one headless Ghidra round (`/tmp/azb-app-mig/ghidra/saved.log`).
//!
//! - A row keeps the receipt's words: the reset is the seconds the server sent, never counted
//!   down here, as natively.
//! - A row whose map `Map.dbc` lacks answers the ten defaults, as natively.
//! - `SetSavedInstanceExtend(index, extend)` takes a boolean, sets the row's extended bit at once,
//!   prints `RAID_INSTANCE_LOCK_EXTENDED`/`_NOT_EXTENDED` with the instance's name as a system
//!   line, and sends CMSG `0x292` with the map, difficulty and bit.
//! - Both index verbs truncate (`fistp` under a chopping control word, `0x4ff2f8`, `0x50dc4c`).

use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::{Model, ScriptValue, UiScript};

/// One `SMSG_RAID_INSTANCE_INFO` row with what App resolves from `Map.dbc` and `MapDifficulty`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathSavedInstance {
    pub map: u32,
    /// `Map.dbc`'s name; `None` when the map is missing, which answers the defaults.
    pub name: Option<String>,
    pub difficulty: u32,
    pub instance: u64,
    pub locked: bool,
    pub extended: bool,
    /// Seconds to the reset, as received.
    pub reset: i32,
    /// `Map.dbc`'s instance type is 2.
    pub is_raid: bool,
    /// `0x634ab0`: the `MapDifficulty` player cap.
    pub max_players: i32,
    /// `MapDifficulty`'s difficulty string, a global-string key; empty without a row.
    pub difficulty_key: String,
}

/// A gesture App sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathSavedInstanceRequest {
    RequestRaidInfo,
    Extend {
        map: u32,
        difficulty: u32,
        extend: bool,
    },
}

#[derive(Default)]
pub(crate) struct State {
    list: Vec<WrathSavedInstance>,
    requests: Vec<WrathSavedInstanceRequest>,
}

impl UiScript {
    /// Replace the list (`0x501030`); App fires `UPDATE_INSTANCE_INFO` after it.
    pub fn set_wrath_saved_instances(&mut self, list: Arc<Vec<WrathSavedInstance>>) {
        self.model_mut().wrath_saved_instances.list = list.as_ref().clone();
    }

    /// The gestures since the last drain, in call order.
    pub fn take_wrath_saved_instance_requests(&mut self) -> Vec<WrathSavedInstanceRequest> {
        std::mem::take(&mut self.model_mut().wrath_saved_instances.requests)
    }
}

fn global_string(lua: &Lua, key: &str) -> String {
    lua.globals()
        .get::<Option<String>>(key)
        .ok()
        .flatten()
        .unwrap_or_else(|| key.to_string())
}

/// `0x4fe070`: the Map.dbc name, inside `DUNGEON_NAME_WITH_DIFFICULTY` with the MapDifficulty
/// string when that key is non-empty (`0x4fe0ad`-`0x4fe0df`); empty for a missing map.
fn dungeon_name(lua: &Lua, row: &WrathSavedInstance) -> String {
    use crate::strings::{fill, Arg};
    let Some(name) = row.name.as_deref() else {
        return String::new();
    };
    if row.difficulty_key.is_empty() {
        return name.to_string();
    }
    let label = global_string(lua, &row.difficulty_key);
    let template = global_string(lua, "DUNGEON_NAME_WITH_DIFFICULTY");
    fill(&template, &[Arg::S(name), Arg::S(&label)])
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "RequestRaidInfo",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .wrath_saved_instances
                .requests
                .push(WrathSavedInstanceRequest::RequestRaidInfo);
            Ok(())
        })?,
    )?;
    g.set(
        "GetNumSavedInstances",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.wrath_saved_instances.list.len() as i64)
        })?,
    )?;
    // GetSavedInstanceInfo(index): name, id, reset, difficulty, locked, extended,
    // instanceIDMostSig, isRaid, maxPlayers, difficultyName.
    g.set(
        "GetSavedInstanceInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = args
                .front()
                .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
            else {
                return Err(mlua::Error::runtime("Usage: GetSavedInstanceInfo(index)"));
            };
            let row = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                usize::try_from(index as i64 - 1)
                    .ok()
                    .and_then(|i| model.wrath_saved_instances.list.get(i))
                    .filter(|r| r.name.is_some())
                    .cloned()
            };
            let Some(r) = row else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Integer(0),
                    Value::Nil,
                    Value::Integer(0),
                    Value::Boolean(false),
                    Value::Boolean(false),
                    Value::Integer(0),
                    Value::Boolean(false),
                    Value::Integer(0),
                    Value::String(lua.create_string("")?),
                ]));
            };
            let difficulty_name = if r.difficulty_key.is_empty() {
                String::new()
            } else {
                global_string(lua, &r.difficulty_key)
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(r.name.as_deref().unwrap_or(""))?),
                Value::Number(f64::from(r.instance as u32)),
                Value::Integer(i64::from(r.reset)),
                Value::Integer(i64::from(r.difficulty) + 1),
                Value::Boolean(r.locked),
                Value::Boolean(r.extended),
                Value::Number(f64::from((r.instance >> 32) as u32)),
                Value::Boolean(r.is_raid),
                Value::Integer(i64::from(r.max_players)),
                Value::String(lua.create_string(&difficulty_name)?),
            ]))
        })?,
    )?;
    // SetSavedInstanceExtend(index, extend): the second argument must be a boolean.
    g.set(
        "SetSavedInstanceExtend",
        lua.create_function(|lua, args: MultiValue| {
            let index = args
                .front()
                .and_then(|v| lua.coerce_number(v.clone()).ok().flatten());
            let (Some(index), Some(Value::Boolean(extend))) = (index, args.get(1).cloned()) else {
                return Err(mlua::Error::runtime(
                    "Usage: SetSavedInstanceExtend(index, extend)",
                ));
            };
            let row = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let state = &mut model.wrath_saved_instances;
                let row = usize::try_from(index as i64 - 1)
                    .ok()
                    .and_then(|i| state.list.get_mut(i));
                row.map(|r| {
                    r.extended = extend;
                    let row = r.clone();
                    state.requests.push(WrathSavedInstanceRequest::Extend {
                        map: row.map,
                        difficulty: row.difficulty,
                        extend,
                    });
                    row
                })
            };
            if let Some(row) = row {
                let key = if extend {
                    "RAID_INSTANCE_LOCK_EXTENDED"
                } else {
                    "RAID_INSTANCE_LOCK_NOT_EXTENDED"
                };
                let line = global_string(lua, key).replacen("%s", &dungeon_name(lua, &row), 1);
                super::tick::fire_event_into(lua, "CHAT_MSG_SYSTEM", vec![ScriptValue::Str(line)]);
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: Option<&str>) -> WrathSavedInstance {
        WrathSavedInstance {
            map: 533,
            name: name.map(str::to_string),
            difficulty: 1,
            instance: 0x0000_0002_0000_0007,
            locked: true,
            extended: false,
            reset: 86_400,
            is_raid: true,
            max_players: 25,
            difficulty_key: "RAID_DIFFICULTY_25PLAYER".into(),
        }
    }

    #[test]
    fn rows_answer_ten_values_and_an_unknown_map_the_defaults() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_saved_instances(Arc::new(vec![row(Some("Naxxramas")), row(None)]));
        assert_eq!(s.eval::<i64>("return GetNumSavedInstances()").unwrap(), 2);
        let (name, id, reset, difficulty, locked, extended, high, raid, max): (
            String,
            i64,
            i64,
            i64,
            bool,
            bool,
            i64,
            bool,
            i64,
        ) = s.eval("return GetSavedInstanceInfo(1)").unwrap();
        assert_eq!(
            (
                name.as_str(),
                id,
                reset,
                difficulty,
                locked,
                extended,
                high,
                raid,
                max
            ),
            ("Naxxramas", 7, 86_400, 2, true, false, 2, true, 25)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetSavedInstanceInfo(1))")
                .unwrap(),
            10
        );
        assert!(s
            .eval::<bool>(
                "return GetSavedInstanceInfo(2) == nil and select(2, GetSavedInstanceInfo(2)) == 0"
            )
            .unwrap());
        assert!(s
            .eval::<bool>("return GetSavedInstanceInfo(3) == nil")
            .unwrap());
        assert!(s
            .eval::<bool>("return GetSavedInstanceInfo(1.9) == 'Naxxramas'")
            .unwrap());
        assert!(s.run("GetSavedInstanceInfo()").is_err());
    }

    #[test]
    fn extending_sets_the_row_prints_and_sends() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_saved_instances(Arc::new(vec![row(Some("Naxxramas"))]));
        s.run(
            "SEEN = nil local f = CreateFrame('Frame') f:RegisterEvent('CHAT_MSG_SYSTEM')
             f:SetScript('OnEvent', function(_, _, msg) SEEN = msg end)
             RAID_INSTANCE_LOCK_EXTENDED = 'Kept: %s.'
             DUNGEON_NAME_WITH_DIFFICULTY = '%1$s [%2$s]'
             RAID_DIFFICULTY_25PLAYER = 'Big'
             SetSavedInstanceExtend(1.9, true) RequestRaidInfo()",
        )
        .unwrap();
        assert!(s
            .eval::<bool>("return select(6, GetSavedInstanceInfo(1))")
            .unwrap());
        assert_eq!(
            s.eval::<String>("return SEEN").unwrap(),
            "Kept: Naxxramas [Big]."
        );
        assert_eq!(
            s.take_wrath_saved_instance_requests(),
            [
                WrathSavedInstanceRequest::Extend {
                    map: 533,
                    difficulty: 1,
                    extend: true
                },
                WrathSavedInstanceRequest::RequestRaidInfo
            ]
        );
        assert!(s.run("SetSavedInstanceExtend(1, 1)").is_err());
    }
}
