//! Build 12340's raid-frame unit queries (`Blizzard_RaidUI.lua`): `UnitClassBase` (`0x610040`),
//! `UnitInRange` (`0x612f10`), `UnitTargetsVehicleInRaidUI` (`0x613780`), `GetMuteStatus`
//! (`0x4fcdf0`) and `AddOrDelMute` (`0x6b6bd0`). Evidence: one headless Ghidra round
//! (`/tmp/azb-wire-ghidra/census2`).
//!
//! - `UnitClassBase` answers `UnitClass`'s pair: the names carried here have no sex-specific
//!   forms.
//! - `UnitInRange` is 1 for a party or raid member (`0x512a30`) within 40 yards of the player,
//!   inclusive (`0xa0ff3c`, 3D), and nil otherwise.
//! - `UnitTargetsVehicleInRaidUI` is a boolean: the unit's vehicle seat carries `VehicleSeat.dbc`
//!   `flagsB & 0x8` (`+0xb4`). The app pushes the guids that qualify.
//! - `GetMuteStatus(unitOrName[, channel])` is 1 when the unit is on the ignore list (`0x6b52e0`)
//!   or the voice mute list (`0x6b5350`), else the voice channel's mute. The voice mute list and
//!   voice channels are never filled here: AzerothCore handles neither `CMSG_ADD_VOICE_IGNORE`
//!   (`0x3DB`) nor `CMSG_DEL_VOICE_IGNORE` (`0x3DC`) (`STATUS_NEVER`), and voice is out of scope.
//!   Labelled: the native compares guids, and the ignore list here holds names, so a unit is
//!   compared by its name.
//! - `AddOrDelMute(name)` sends `0x3DC` for a name on the mute list and `0x3DB` otherwise. Since
//!   AzerothCore drops both, it changes nothing here (labelled).

use std::collections::HashSet;

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// `UnitInRange`'s 40 yards, squared.
const IN_RANGE_SQ: f64 = 40.0 * 40.0;

impl super::UiScript {
    /// The units whose vehicle seat shows its vehicle in the raid UI.
    pub fn set_raid_vehicle_units(&mut self, guids: HashSet<u64>) {
        let mut model = self.model_mut();
        if model.raid_vehicle_units != guids {
            model.raid_vehicle_units = guids;
        }
    }
}

fn string_only(args: &MultiValue, i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "UnitClassBase",
        lua.create_function(|lua, token: Value| {
            super::unit::class_pair(lua, token, r#"Usage: UnitClassBase("unit")"#)
        })?,
    )?;

    g.set(
        "UnitInRange",
        lua.create_function(|lua, args: MultiValue| {
            let Some(token) = string_only(&args, 0) else {
                return Err(mlua::Error::runtime(r#"Usage: UnitInRange("unit")"#));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let hit = model.unit_guid(&token).is_some_and(|guid| {
                // Rosters zero absent slots; the guid is non-zero.
                let member = model.party.members.iter().any(|m| m.guid == guid)
                    || model.party.raid.iter().any(|m| m.guid == guid);
                member
                    && model
                        .unit_reach
                        .get(&guid)
                        .is_some_and(|r| r.dist_sq <= IN_RANGE_SQ)
            });
            Ok(if hit { Value::Integer(1) } else { Value::Nil })
        })?,
    )?;

    g.set(
        "UnitTargetsVehicleInRaidUI",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let hit = string_only(&args, 0)
                .and_then(|t| model.unit_guid(&t))
                .is_some_and(|g| model.raid_vehicle_units.contains(&g));
            Ok(hit)
        })?,
    )?;

    g.set(
        "GetMuteStatus",
        lua.create_function(|lua, args: MultiValue| {
            let Some(who) = string_only(&args, 0) else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // A unit token reads its unit's name; anything else is a name.
            let name = model
                .unit(&who)
                .filter(|u| u.exists)
                .and_then(|u| u.name.clone())
                .unwrap_or(who);
            let ignored = model
                .social
                .ignores
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&name));
            Ok(MultiValue::from_vec(vec![if ignored {
                Value::Integer(1)
            } else {
                Value::Nil
            }]))
        })?,
    )?;

    g.set(
        "AddOrDelMute",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn mute_status_reads_the_ignore_list_and_mutes_change_nothing() {
        let mut s = UiScript::new().unwrap();
        s.set_social(crate::script::SocialState {
            ignores: vec!["Grief".into()],
            ..Default::default()
        });
        assert_eq!(
            s.eval::<i64>("return GetMuteStatus('grief', 'raid')")
                .unwrap(),
            1
        );
        assert!(s
            .eval::<bool>("return GetMuteStatus('Bob') == nil")
            .unwrap());
        assert_eq!(
            s.eval::<i64>("return select('#', GetMuteStatus())")
                .unwrap(),
            0
        );
        s.run("AddOrDelMute('Bob')").unwrap();
        assert!(s
            .eval::<bool>("return GetMuteStatus('Bob') == nil")
            .unwrap());
    }

    #[test]
    fn range_and_vehicle_answers_miss_without_a_unit() {
        let s = UiScript::new().unwrap();
        assert!(s
            .eval::<bool>("return UnitInRange('raid1') == nil")
            .unwrap());
        assert!(!s
            .eval::<bool>("return UnitTargetsVehicleInRaidUI('raid1')")
            .unwrap());
        assert!(s.run("UnitInRange()").is_err());
        assert!(s
            .eval::<bool>("return UnitClassBase('raid1') == nil")
            .unwrap());
        assert!(s.run("UnitClassBase()").is_err());
    }
}
