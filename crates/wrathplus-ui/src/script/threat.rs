//! Native Wrath threat readers over the server's threat table, its values as the wire sends them
//! (`0x741842`). The app supplies each member's aggro threshold (110% in melee, 130% outside)
//! from world geometry, where the reference tests the distance itself (`0x4f61d0`).

use super::{Model, UiScript};
use mlua::{Lua, MultiValue, Value};
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
        self.details(member).map(|d| d.status - 1)
    }

    /// `0x7374c0` over the entry: with no top target (`[mob+0xfd8]`) an entry reads status 1 and
    /// zeros (`0x7375e3`). Else a member's raw percent is `value * 100 / tank` truncated and capped
    /// at 250 (`0x71c3b0`), status 2 at 100 and up, else 1; the tank's status is 4, or 3 while
    /// another entry has status 2 (`0x737620`), so a tie is pulling. The scaled percent is 100 for
    /// the tank, else the raw byte times `0.90909` in melee or `0.76923` (`0x7375ad`, `0x7375c4`).
    fn details(&self, member: u64) -> Option<Details> {
        let value = *self.members.get(&member)?;
        let Some(tank) = self.highest else {
            return Some(Details {
                status: 1,
                ..Default::default()
            });
        };
        let tank_value = self.members.get(&tank).copied().unwrap_or(0);
        // `0x71c3b0` reads both values signed: a tank at 0 gives 100, a value <= 0 gives 0
        // (`0x71c3d2`), and a negative quotient keeps its low byte, past the cap's test.
        let raw = |v: u32| -> u8 {
            match (i64::from(v as i32), i64::from(tank_value as i32)) {
                (_, 0) => 100,
                (v, _) if v <= 0 => 0,
                (v, t) => match v * 100 / t {
                    q if q < 0 => q as u8,
                    q => q.min(250) as u8,
                },
            }
        };
        let tanking = tank == member;
        let pulling = |v: u32| raw(v) >= 100;
        let status = if tanking {
            if self
                .members
                .iter()
                .any(|(&g, &v)| g != member && pulling(v))
            {
                3
            } else {
                4
            }
        } else if pulling(value) {
            2
        } else {
            1
        };
        let raw = if tanking { 100 } else { raw(value) };
        let scaled = if tanking {
            100.0
        } else {
            // `0xa34ee4` / `0xa34ee0`, f32 products stored as f32.
            let melee = self.thresholds.get(&member).is_some_and(|&t| t <= 110.0);
            let factor = f32::from_bits(if melee { 0x3f68_ba2e } else { 0x3f44_ec4f });
            f32::from(raw) * factor
        };
        Some(Details {
            tanking,
            status,
            scaled,
            raw,
            value,
        })
    }
}

/// `0x7374c0`'s outputs: the status byte (`+0x28`, 0 none, 1-4 shown as 0-3), the raw percent
/// byte (`+0x29`), the scaled percent and the wire's threat value (`+0x2c`, read unscaled at
/// `0x741842`).
#[derive(Default)]
struct Details {
    tanking: bool,
    status: u8,
    scaled: f32,
    raw: u8,
    value: u32,
}

impl UiScript {
    pub fn set_threat(&mut self, threat: ThreatTable) {
        self.lua().app_data_mut::<Model>().expect("model").threat = threat;
    }
}

/// `0x60abf0`'s answer for an argument as `lua_tolstring` hands it over: false raises `usage`
/// (`0x613a8e`, `0x613c70`); true gives the guid, 0 for nobody.
fn resolve(lua: &Lua, v: &Value, usage: &'static str) -> mlua::Result<u64> {
    let model = lua.app_data_ref::<Model>().expect("model");
    let r = super::unit::token::token_text(lua, v)
        .map(|t| model.resolve_token(&t))
        .filter(|r| r.ok)
        .ok_or_else(|| mlua::Error::runtime(usage))?;
    Ok(r.unit().unwrap_or(0))
}

/// `0x61a4f0`'s table at `0xad2d70`, `0xAARRGGBB`: none, 0-3, and the fallback slot 1 (grey).
const STATUS_COLORS: [u32; 5] = [
    0xffff_ffff,
    0xffb0_b0b0,
    0xffff_ff77,
    0xffff_9900,
    0xffff_0000,
];

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    // UnitDetailedThreatSituation(unit, mob) (`0x613b40`): both tokens recognised, else the
    // Usage line; nothing for an unheld mob or no entry; else isTanking (1 or nil, the mob's top
    // target), status, scaled percent, raw percent and the threat value.
    globals.set(
        "UnitDetailedThreatSituation",
        lua.create_function(|lua, (unit, mob): (Value, Value)| {
            const USAGE: &str = r#"Usage: UnitDetailedThreatSituation("unit" [, "mob"])"#;
            let member = resolve(lua, &unit, USAGE)?;
            let mob = resolve(lua, &mob, USAGE)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            let details = model.threat.mobs.get(&mob).and_then(|m| m.details(member));
            Ok(match details {
                Some(d) => MultiValue::from_vec(vec![
                    if d.tanking {
                        Value::Integer(1)
                    } else {
                        Value::Nil
                    },
                    Value::Integer(i64::from(d.status - 1)),
                    Value::Number(f64::from(d.scaled)),
                    Value::Integer(i64::from(d.raw)),
                    // `fild DWORD` (`0x613c4b`): the wire's value, signed.
                    Value::Integer(i64::from(d.value as i32)),
                ]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    // UnitThreatSituation(unit [, mob]) (`0x613a60`): an unrecognised token raises; nobody, or no
    // status, answers nothing; else the status on that mob, or the worst over all (`0x52a1a0`).
    globals.set(
        "UnitThreatSituation",
        lua.create_function(|lua, (unit, mob): (Value, Value)| {
            const USAGE: &str = r#"Usage: UnitThreatSituation("unit" [, "mob"])"#;
            let member = resolve(lua, &unit, USAGE)?;
            // Nobody answers nothing before argument 2 is read (`0x613aa3`).
            if member == 0 {
                return Ok(MultiValue::new());
            }
            let mob = match super::unit::token::token_text(lua, &mob) {
                Some(_) => Some(resolve(lua, &mob, USAGE)?),
                None => None,
            };
            let model = lua.app_data_ref::<Model>().expect("model");
            let status = match mob {
                Some(mob) => model.threat.mobs.get(&mob).and_then(|m| m.details(member)),
                None => model
                    .threat
                    .mobs
                    .values()
                    .filter_map(|m| m.details(member))
                    .max_by_key(|d| d.status),
            }
            .map_or(0, |d| d.status);
            Ok(match status {
                0 => MultiValue::new(),
                s => MultiValue::from_vec(vec![Value::Integer(i64::from(s - 1))]),
            })
        })?,
    )?;
    // GetThreatStatusColor(status) (`0x511fe0`): `lua_tonumber(1) + 1` truncated indexes the
    // table, anything past it reading slot 1, so nil and junk are grey and -1 white; each channel
    // is its byte times the f32 `0x9ead78` (`0x3b80806e`, a hair under 1/255), never a raise.
    globals.set(
        "GetThreatStatusColor",
        lua.create_function(|lua, status: Value| {
            let n = lua.coerce_number(status).ok().flatten().unwrap_or(0.0) + 1.0;
            let index = usize::try_from(n.trunc() as i64)
                .ok()
                .filter(|&i| i < 5)
                .unwrap_or(1);
            let argb = STATUS_COLORS[index];
            let channel = |shift: u32| {
                f64::from((argb >> shift) & 0xff) * f64::from(f32::from_bits(0x3b80_806e))
            };
            Ok((channel(16), channel(8), channel(0)))
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
        let count = |ui: &UiScript, call: &str| {
            ui.eval::<i64>(&format!("return select('#', {call})"))
                .unwrap()
        };
        // No threat is zero values (`0x613b0a`, `0x613bd5`); a bad token raises.
        assert_eq!(count(&ui, "UnitThreatSituation('player','target')"), 0);
        assert_eq!(
            count(&ui, "UnitDetailedThreatSituation('player','target')"),
            0
        );
        assert_eq!(count(&ui, "UnitThreatSituation('party4', 'foo')"), 0);
        for call in [
            "UnitThreatSituation('foo')",
            "UnitThreatSituation()",
            "UnitThreatSituation('player', 'foo')",
            "UnitDetailedThreatSituation('player')",
        ] {
            let err = ui.eval::<()>(call).unwrap_err().to_string();
            assert!(err.contains("Usage: Unit"), "{call}: {err}");
        }
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
        // isTanking 1 or nil, the raw percent an integer, the value as the wire sent it.
        let row: (Value, u8, f64, Value, Value) = ui
            .eval("return UnitDetailedThreatSituation('player','target')")
            .unwrap();
        assert_eq!(row.0, Value::Nil);
        assert_eq!(row.1, 1);
        assert_eq!(row.2, f64::from(110.0_f32 * f32::from_bits(0x3f68_ba2e)));
        assert_eq!((row.3, row.4), (Value::Integer(110), Value::Integer(11000)));
        // The tie pulls (`0x71c3b0`: raw >= 100), so the tank reads 2, not 3.
        let mut table = ThreatTable::default();
        table.mobs.insert(
            2,
            MobThreat {
                highest: Some(1),
                members: [(1, 10000), (3, 10000)].into(),
                ..Default::default()
            },
        );
        ui.set_threat(table);
        assert_eq!(
            ui.eval::<(i64, u8)>("return UnitDetailedThreatSituation('player','target')")
                .unwrap(),
            (1, 2)
        );
        // A value past 2^31 is negative (`0x613c4b`, `0x71c3d2`): raw 0, not pulling.
        let mut table = ThreatTable::default();
        let members = [(1, 0x8000_0000), (3, 10000)].into();
        let mob = MobThreat {
            highest: Some(3),
            members,
            ..Default::default()
        };
        table.mobs.insert(2, mob);
        ui.set_threat(table);
        assert_eq!(
            ui.eval::<(u8, i64, i64)>(
                "local _, s, _, r, v = UnitDetailedThreatSituation('player','target') return s, r, v"
            )
            .unwrap(),
            (0, 0, -0x8000_0000)
        );
        let c = |b: u32| f64::from(b) * f64::from(f32::from_bits(0x3b80_806e));
        for (arg, want) in [
            ("3", (c(255), 0.0, 0.0)),
            ("1", (c(255), c(255), c(119))),
            ("0", (c(176), c(176), c(176))),
            ("-1", (c(255), c(255), c(255))),
            ("", (c(176), c(176), c(176))),
            ("9", (c(176), c(176), c(176))),
        ] {
            let got = ui
                .eval::<(f64, f64, f64)>(&format!("return GetThreatStatusColor({arg})"))
                .unwrap();
            assert_eq!(got, want, "{arg}");
        }
    }
}
