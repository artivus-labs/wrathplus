//! Native Wrath threat readers over the server's hundredths-of-threat table. The app
//! supplies each member's aggro threshold (110% in melee, 130% outside) from world geometry.

use super::{Model, UiScript};
use mlua::{Lua, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreatTable {
    pub mobs: HashMap<u64, MobThreat>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobThreat {
    pub highest: Option<u64>,
    pub members: HashMap<u64, u32>,
    /// Aggro threshold percentages, keyed by member GUID. Missing geometry uses ranged 130%.
    pub thresholds: HashMap<u64, f64>,
}

impl MobThreat {
    /// `member`'s threat situation on this mob, `UnitThreatSituation`'s 0-3, or `None` when not
    /// on its list.
    pub fn situation(&self, member: u64) -> Option<u8> {
        self.details(member).map(|d| d.1)
    }

    fn details(&self, member: u64) -> Option<(bool, u8, f64, f64, f64)> {
        let value = f64::from(*self.members.get(&member)?);
        let tanking = self.highest == Some(member);
        let tank_value = self
            .highest
            .and_then(|tank| self.members.get(&tank))
            .copied()
            .or_else(|| self.members.values().copied().max())
            .unwrap_or(0);
        let raw = if tank_value == 0 {
            0.0
        } else {
            value / f64::from(tank_value) * 100.0
        };
        let threshold = self.thresholds.get(&member).copied().unwrap_or(130.0);
        let scaled = if tanking {
            100.0
        } else {
            raw / threshold * 100.0
        };
        let status = if tanking {
            if self
                .members
                .iter()
                .any(|(&guid, &threat)| guid != member && f64::from(threat) > value)
            {
                2
            } else {
                3
            }
        } else if raw >= 100.0 {
            1
        } else {
            0
        };
        Some((tanking, status, scaled, raw, value / 100.0))
    }
}

impl UiScript {
    pub fn set_threat(&mut self, threat: ThreatTable) {
        self.lua().app_data_mut::<Model>().expect("model").threat = threat;
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    globals.set(
        "UnitDetailedThreatSituation",
        lua.create_function(|lua, (unit, mob): (String, String)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let member = model.unit_guid(&unit);
            let mob = model.unit_guid(&mob);
            let details = member
                .zip(mob)
                .and_then(|(member, mob)| model.threat.mobs.get(&mob)?.details(member));
            Ok(match details {
                Some((tanking, status, scaled, raw, value)) => (
                    Value::Boolean(tanking),
                    Value::Integer(i64::from(status)),
                    Value::Number(scaled),
                    Value::Number(raw),
                    Value::Number(value),
                ),
                None => (Value::Nil, Value::Nil, Value::Nil, Value::Nil, Value::Nil),
            })
        })?,
    )?;
    globals.set(
        "UnitThreatSituation",
        lua.create_function(|lua, (unit, mob): (String, Option<String>)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let Some(member) = model.unit_guid(&unit) else {
                return Ok(None);
            };
            if let Some(mob) = mob {
                let mob = model.unit_guid(&mob);
                Ok(mob.and_then(|mob| model.threat.mobs.get(&mob)?.details(member).map(|d| d.1)))
            } else {
                Ok(model
                    .threat
                    .mobs
                    .values()
                    .filter_map(|mob| mob.details(member).map(|d| d.1))
                    .max())
            }
        })?,
    )?;
    globals.set(
        "GetThreatStatusColor",
        lua.create_function(|_, status: u8| {
            Ok(match status {
                1 => (1.0, 1.0, 0.47),
                2 => (1.0, 0.6, 0.0),
                3 => (1.0, 0.0, 0.0),
                _ => (0.69, 0.69, 0.69),
            })
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readers_distinguish_missing_zero_tanking_and_thresholds() {
        let mut ui = UiScript::new().unwrap();
        ui.set_unit_guids(&super::super::UnitGuids {
            player: 1,
            target: 2,
            ..Default::default()
        });
        assert_eq!(
            ui.eval::<Option<u8>>("return UnitThreatSituation('player','target')")
                .unwrap(),
            None
        );
        let mob = MobThreat {
            highest: Some(3),
            members: [(1, 11000), (3, 10000)].into(),
            thresholds: [(1, 110.0)].into(),
        };
        ui.set_threat(ThreatTable {
            mobs: [(2, mob)].into(),
        });
        assert_eq!(
            ui.eval::<u8>("return UnitThreatSituation('player')")
                .unwrap(),
            1
        );
        let (tank, status, scaled, raw, value): (bool, u8, f64, f64, f64) = ui
            .eval("return UnitDetailedThreatSituation('player','target')")
            .unwrap();
        assert!(!tank);
        assert_eq!(status, 1);
        assert!((scaled - 100.0).abs() < 1e-8);
        assert!((raw - 110.0).abs() < 1e-8);
        assert_eq!(value, 110.0);
        let mut table = ThreatTable::default();
        table.mobs.insert(
            2,
            MobThreat {
                highest: Some(1),
                members: [(1, 10000), (3, 11000)].into(),
                ..Default::default()
            },
        );
        ui.set_threat(table);
        assert_eq!(
            ui.eval::<u8>("return UnitThreatSituation('player','target')")
                .unwrap(),
            2
        );
        assert_eq!(
            ui.eval::<(f64, f64, f64)>("return GetThreatStatusColor(3)")
                .unwrap(),
            (1.0, 0.0, 0.0)
        );
    }
}
