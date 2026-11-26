//! Build 12340's `GetPlayerInfoByGUID` (`0x613f90`) over its player name cache (`0x67d770` on
//! `0xc5d938`). The app pushes the players a name query has answered, with the race, class and
//! gender each answer carries, and the binding peeks them as 12340 does: a miss answers nothing and
//! asks no one (the callback `0x608370` is passed only under the player flag `0x80000`, `0x61402b`).

use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// One name-cache record as `GetPlayerInfoByGUID` reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerNameInfo {
    pub name: String,
    /// The class's display name and file name (`ChrClasses` `+0x1c`); `None` with no row.
    pub class: Option<(String, String)>,
    /// The race's display name and file name (`ChrRaces` `+0x2c`); `None` with no row.
    pub race: Option<(String, String)>,
    /// The record's gender (`+0x144`): 0 male, 1 female.
    pub gender: u8,
}

impl super::UiScript {
    /// Replace the name-cache view the app holds; shared, so a push moves one pointer.
    pub fn set_player_names(&mut self, names: Arc<HashMap<u64, PlayerNameInfo>>) {
        self.model_mut().player_names = names;
    }
}

/// A GUID string as `0x74d120` parses it: an optional `0x`, then up to 16 hex digits.
fn parse_guid(s: &str) -> u64 {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    s.chars()
        .take(16)
        .map_while(|c| c.to_digit(16))
        .fold(0u64, |acc, d| acc << 4 | u64::from(d))
}

fn text(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // GetPlayerInfoByGUID(guid) → class, englishClass, race, englishRace, sex, name, realm. A
    // non-string, or a string that parses to 0, raises (`0x613fdf`); a guid that is not a player's
    // (`0x613ff8`-`0x614010`) or a name not cached answers nothing. Sex is `[2, 3, 1]` by gender
    // (`0x9fe7ec`) and the realm always a string (`+0x34`). Labelled: a unit the table lacks, by
    // token (our own, seeded at login with no query) or by guid, answers from its snapshot.
    lua.globals().set(
        "GetPlayerInfoByGUID",
        lua.create_function(|lua, arg: Value| {
            const USAGE: &str = r#"Usage: GetPlayerInfoByGUID("playerGUID")"#;
            let guid = parse_guid(&super::binding_abi::string_arg(lua, arg, USAGE)?);
            if guid == 0 {
                return Err(mlua::Error::runtime(USAGE));
            }
            let high = (guid >> 32) as u32;
            if high & 0xf000_0000 != 0 || (guid as u32 | high & 0xf07f_ffff) == 0 {
                return Ok(MultiValue::new());
            }
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let some = |v: Option<&String>| v.map_or(Ok(Value::Nil), |v| text(lua, v));
            if let Some(r) = model.player_names.get(&guid) {
                let sex = [2, 3, 1].get(usize::from(r.gender)).copied().unwrap_or(1);
                return Ok(MultiValue::from_vec(vec![
                    some(r.class.as_ref().map(|c| &c.0))?,
                    some(r.class.as_ref().map(|c| &c.1))?,
                    some(r.race.as_ref().map(|c| &c.0))?,
                    some(r.race.as_ref().map(|c| &c.1))?,
                    Value::Integer(sex),
                    text(lua, &r.name)?,
                    text(lua, "")?,
                ]));
            }
            let tokened = model.units_by_lower.values().find(|u| u.guid == guid);
            let Some(u) = tokened.or_else(|| model.units_by_guid.get(&guid)) else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_vec(vec![
                some(u.class.as_ref())?,
                some(u.class_file.as_ref())?,
                some(u.race.as_ref())?,
                some(u.race_file.as_ref())?,
                // UnitSex's scale; the unfilled 0 answers 2, as UnitSex does.
                Value::Integer(i64::from(if u.sex == 0 { 2 } else { u.sex })),
                some(u.name.as_ref())?,
                text(lua, "")?,
            ]))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    /// A cached player never seen as a unit answers the seven, a unit the table lacks answers
    /// from its snapshot, and the zero guid raises where a non-player answers nothing.
    #[test]
    fn player_info_reads_the_name_cache_then_the_units() {
        let mut s = UiScript::new().unwrap();
        let record = PlayerNameInfo {
            name: "Grunt".into(),
            class: Some(("Warrior".into(), "WARRIOR".into())),
            race: Some(("Orc".into(), "Orc".into())),
            gender: 1,
        };
        s.set_player_names(Arc::new([(0x99, record)].into()));
        s.model_mut().units_by_guid.insert(
            0x42,
            crate::script::UnitState {
                name: Some("Ivy".into()),
                class: Some("Mage".into()),
                sex: 3,
                ..Default::default()
            },
        );
        let all = |guid: &str| {
            s.eval::<String>(&format!(
                "local t = {{GetPlayerInfoByGUID('{guid}')}} for i = 1, 7 do \
                 t[i] = tostring(t[i]) end return table.concat(t, ',')"
            ))
            .unwrap()
        };
        assert_eq!(
            all("0x0000000000000099"),
            "Warrior,WARRIOR,Orc,Orc,3,Grunt,"
        );
        assert_eq!(all("0x0000000000000042"), "Mage,nil,nil,nil,3,Ivy,");
        let me = crate::script::UnitState {
            guid: 0x43,
            name: Some("Me".into()),
            sex: 2,
            ..Default::default()
        };
        s.model_mut().units_by_lower.insert("player".into(), me);
        assert_eq!(all("0x43"), "nil,nil,nil,nil,2,Me,", "our own token");
        assert_eq!(
            s.eval::<i64>(
                "return select('#', GetPlayerInfoByGUID('0xF130000000000042')) \
                 + select('#', GetPlayerInfoByGUID('0x77'))"
            )
            .unwrap(),
            0,
            "not a player GUID; not cached"
        );
        for call in ["GetPlayerInfoByGUID('0x0')", "GetPlayerInfoByGUID()"] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(
                err.contains(r#"Usage: GetPlayerInfoByGUID("playerGUID")"#),
                "{err}"
            );
        }
    }
}
