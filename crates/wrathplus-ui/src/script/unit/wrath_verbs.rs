//! Build 12340's group-membership, level, experience and combo-point unit verbs, each resolving
//! its token through `0x60abf0` ([`super::token`]) and reading the group globals or the unit.

use mlua::{Lua, Value};

use super::super::binding_abi::{flag, string_arg};
use super::token::token_text;
use super::Model;

/// `CREATURE_TYPEFLAGS_BOSS`, the template's type-flags bit `0x715d70` reads (`[rec+0xc] >> 2`).
const TYPE_FLAG_BOSS: u32 = 0x4;
/// The classes `GetComboPoints` lets the player read (`0x611712`): rogue and druid.
const CLASS_ROGUE: u32 = 4;
const CLASS_DRUID: u32 = 11;

/// The guid an ungated verb's argument resolves to (`lua_tolstring`, then `0x60abf0`): 0 for nil
/// or a token that does not parse.
fn guid_arg(lua: &Lua, m: &Model, v: &Value) -> u64 {
    token_text(lua, v).map_or(0, |t| m.resolve_token(&t).guid)
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // UnitInRaid(unit) (`0x60ca20`): no gate; `0x608130` walks the roster for the guid and the
    // 0-based row is pushed as a number (`0x60ca65`), so the first row answers 0; nil off it.
    g.set(
        "UnitInRaid",
        lua.create_function(|lua, token: Value| {
            let m = lua.app_data_ref::<Model>().expect("model");
            let guid = guid_arg(lua, &m, &token);
            let row = m
                .party
                .raid
                .iter()
                .position(|r| guid != 0 && r.guid == guid);
            Ok(row.map_or(Value::Nil, |i| Value::Number(i as f64)))
        })?,
    )?;

    // UnitInParty(unit) (`0x60c920`): no gate; `0x52c680` is 1 for our own guid, grouped or not,
    // else for a guid in the four party slots; raid-only members are not in them.
    g.set(
        "UnitInParty",
        lua.create_function(|lua, token: Value| {
            let m = lua.app_data_ref::<Model>().expect("model");
            let guid = guid_arg(lua, &m, &token);
            let me = m.local_guid();
            let slot = m.party.members.iter().any(|p| p.guid == guid);
            Ok(flag(guid != 0 && (guid == me || slot)))
        })?,
    )?;

    // UnitPlayerOrPetInParty(unit) (`0x60c9a0`): no gate; `0x52c8c0` scans the party slots for the
    // member or its pet (`CHARM`, else `SUMMON`, else the stored pet). The slots hold the other
    // members only, so we and our own pet answer nil.
    g.set(
        "UnitPlayerOrPetInParty",
        lua.create_function(|lua, token: Value| {
            let m = lua.app_data_ref::<Model>().expect("model");
            let guid = guid_arg(lua, &m, &token);
            let slots = m.party.members.len().min(4);
            let pet = |i: usize| m.unit_guid(&format!("partypet{}", i + 1)) == Some(guid);
            let hit = m.party.members.iter().any(|p| p.guid == guid) || (0..slots).any(pet);
            Ok(flag(guid != 0 && hit))
        })?,
    )?;

    // UnitLevel(unit) (`0x60f9e0`): the descriptor level, or -1 for a unit hostile to us
    // (`0x7251c0` at most 1, `UnitReaction` 1 or 2) at least 10 levels above us (`0x60faac`), or a
    // template flagged boss (`0x715d70`); players are not exempt. No unit is 0. Labelled: the
    // attackable-decay leg (`0x60face`) and the `"player"` record level with no object are not
    // built.
    g.set(
        "UnitLevel",
        lua.create_function(|lua, token: Value| {
            let token = string_arg(lua, token, r#"Usage: UnitLevel("unit")"#)?;
            let m = lua.app_data_ref::<Model>().expect("model");
            let Some(u) = m.unit(&token) else {
                return Ok(0i64);
            };
            let me = i64::from(m.player_req.level);
            let level = i64::from(u.level);
            let hostile = u.reaction != 0 && u.reaction <= 2 && me > 0 && me <= level - 10;
            let boss = u.creature_type_flags & TYPE_FLAG_BOSS != 0;
            Ok(if hostile || boss { -1 } else { level })
        })?,
    )?;

    // UnitXP/UnitXPMax(unit) (`0x60ea60`/`0x60eae0`): any token naming the local player reads
    // `PLAYER_XP`/`PLAYER_NEXT_LEVEL_XP` (`0x60a5d0`, `0x60a600`); every other unit is 0.
    for (name, usage, max) in [
        ("UnitXP", r#"Usage: UnitXP("unit")"#, false),
        ("UnitXPMax", r#"Usage: UnitXPMax("unit")"#, true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, token: Value| {
                let token = string_arg(lua, token, usage)?;
                let m = lua.app_data_ref::<Model>().expect("model");
                let me = m.local_guid();
                if me == 0 || m.unit_guid(&token) != Some(me) {
                    return Ok(0i64);
                }
                Ok(i64::from(if max {
                    m.player_next_level_xp
                } else {
                    m.player_xp
                }))
            })?,
        )?;
    }

    // GetComboPoints("unit"[, "target"]) (`0x611670`): `lua_isstring(1)` (its usage string lacks
    // the closing parenthesis); both tokens resolve. For our own guid: a rogue or druid (or no
    // player object) reads `PLAYER_FIELD_BYTES` byte 1 when the second unit, else the target
    // (`0x513920`, for guid 0), is the combo target. Labelled: the controlled vehicle's arm
    // (`0x5d3390`, `0x5d3520`) is not built and answers 0, as does any other unit.
    g.set(
        "GetComboPoints",
        lua.create_function(|lua, (unit, target): (Value, Value)| {
            let unit = string_arg(lua, unit, r#"Usage: GetComboPoints("unit"[, "target"]"#)?;
            let m = lua.app_data_ref::<Model>().expect("model");
            let me = m.local_guid();
            if me == 0 || m.resolve_token(&unit).guid != me {
                return Ok(0i64);
            }
            let class = m.player_req.class_id;
            if class != 0 && class != CLASS_ROGUE && class != CLASS_DRUID {
                return Ok(0);
            }
            let target = match guid_arg(lua, &m, &target) {
                0 => m.unit_guids.target,
                g => g,
            };
            Ok(if target == m.combo_target {
                i64::from(m.combo_points)
            } else {
                0
            })
        })?,
    )?;

    // UnitCreatureFamily(unit) (`0x611820`): `lua_isstring(1)`; any held unit's template family
    // word (`0x7153e0`), nil for none. The pet's snapshot carries no template, so its word is the
    // pet bar's, read off the same record.
    g.set(
        "UnitCreatureFamily",
        lua.create_function(move |lua, token: Value| {
            let usage = r#"Usage: UnitCreatureFamily("unit")"#;
            let token = string_arg(lua, token, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            let pet = model.unit_guid("pet").unwrap_or(0);
            let word = model
                .unit(&token)
                .and_then(|u| u.creature_family.as_deref());
            let family = match word {
                None if pet != 0 && model.unit_guid(&token) == Some(pet) => {
                    model.pet_bar.stats.family.as_deref()
                }
                word => word,
            };
            Ok(match family {
                Some(name) => Value::String(lua.create_string(name)?),
                None => Value::Nil,
            })
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{PlayerRecord, UiScript, UnitGuids, UnitState};

    #[test]
    fn unit_level_reads_minus_one_when_the_level_cant_be_told() {
        use crate::script::PlayerReqState;
        let mut s = UiScript::new().unwrap();
        s.set_player_req_state(PlayerReqState {
            level: 3,
            ..Default::default()
        });
        // Hostile (2) and 10 levels above the player: -1, the target frame's skull.
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 13,
                reaction: 2,
                ..Default::default()
            }),
        );
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), -1);
        // 9 above: the level.
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 12,
                reaction: 2,
                ..Default::default()
            }),
        );
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), 12);
        // Neutral (4) at 10 above: the level, as the gate is hostile-only.
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 13,
                reaction: 4,
                ..Default::default()
            }),
        );
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), 13);
        // A template flagged boss (type flags `0x4`, `0x715d70`) at any gap: -1.
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 5,
                reaction: 4,
                creature_type_flags: 0x4,
                ..Default::default()
            }),
        );
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), -1);
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 60,
                reaction: 2,
                is_player: true,
                ..Default::default()
            }),
        );
        // Players are not exempt (`0x60fa8a`): a hostile one far above us reads -1.
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), -1);
        // An unstreamed level 0 answers 0: the raw field.
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                level: 0,
                reaction: 2,
                ..Default::default()
            }),
        );
        assert_eq!(s.eval::<i64>(r#"return UnitLevel("target")"#).unwrap(), 0);
    }

    /// `0x611670`: `lua_isstring(1)` (its usage string lacks the closing parenthesis); the first
    /// unit must be us, and the second, else the target (`0x513920`), the combo target.
    #[test]
    fn get_combo_points_reads_ours_against_the_named_or_current_target() {
        let mut s = UiScript::new().unwrap();
        let (player, target, focus) = (7, 9, 8);
        s.set_unit_guids(&UnitGuids {
            player,
            target,
            focus,
            ..Default::default()
        });
        let rogue = crate::script::PlayerReqState {
            class_id: 4,
            ..Default::default()
        };
        s.set_player_req_state(rogue);
        s.set_combo_points(3, 9);
        let err = s.run("GetComboPoints()").unwrap_err().to_string();
        assert!(
            err.contains(r#"Usage: GetComboPoints("unit"[, "target"]"#),
            "{err}"
        );
        let n = |c: &str| {
            s.eval::<i64>(&format!("return GetComboPoints({c})"))
                .unwrap()
        };
        assert_eq!([n(r#""player""#), n(r#""player", "target""#)], [3, 3]);
        assert_eq!([n(r#""player", "focus""#), n(r#""target""#)], [0, 0]);
    }

    #[test]
    fn unit_xp_reads_the_player_globals_only_for_the_player_token() {
        let mut s = UiScript::new().unwrap();
        // Any token naming the local player reads them (`0x60a5d0`), its name included.
        s.set_unit_guids(&UnitGuids {
            player: 7,
            ..Default::default()
        });
        s.set_player_record(PlayerRecord {
            name: "Me".into(),
            ..Default::default()
        });
        assert_eq!(s.eval::<i64>(r#"return UnitXP("player")"#).unwrap(), 0);
        assert_eq!(s.eval::<i64>(r#"return UnitXPMax("player")"#).unwrap(), 0);
        s.set_player_xp(4200, 6000);
        assert_eq!(s.eval::<i64>(r#"return UnitXP("player")"#).unwrap(), 4200);
        assert_eq!(
            s.eval::<i64>(r#"return UnitXPMax("player")"#).unwrap(),
            6000
        );
        assert_eq!(s.eval::<i64>(r#"return UnitXP("Player")"#).unwrap(), 4200);
        assert_eq!(s.eval::<i64>(r#"return UnitXP("me")"#).unwrap(), 4200);
        assert_eq!(s.eval::<i64>(r#"return UnitXP("target")"#).unwrap(), 0);
        assert_eq!(s.eval::<i64>(r#"return UnitXPMax("target")"#).unwrap(), 0);
    }
}
