//! Build-12340 resource readers over profile-native raw snapshots.

use mlua::{Lua, Value};

use super::Model;

const TOKENS: [&str; 7] = [
    "MANA",
    "RAGE",
    "FOCUS",
    "ENERGY",
    "HAPPINESS",
    "RUNES",
    "RUNIC_POWER",
];

/// `lua_isnumber`, then `lua_tointeger` (`0x84e070`, `fistp`: ties to even, NaN or past i32
/// `i32::MIN`); `default` for a non-number.
fn numeric_index(lua: &Lua, value: Option<Value>, default: i64) -> i64 {
    let n = value.and_then(|v| lua.coerce_number(v).ok().flatten());
    n.map_or(default, |n| {
        i64::from(crate::script::item_arg::tointeger(n) as i32)
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // UnitPowerType(unit[, index]) (`0x60f100`): `lua_isstring(1)`; the index is `lua_isnumber`
    // then `n - 1` (`0x60f151`), 0 without one. Index 0 answers the unit's type and its token
    // (`0x7fe4b0`), the roster's type for a member with no object, else `0, ""`; any other index
    // answers `0, ""` (`0x60f327`). Labelled: the vehicle arm (`[unit+0xf5c]` flagged `0x100000`,
    // index below 3, a `PowerDisplay` row of the unit's type) that answers five values (type,
    // token, r, g, b / 255) is not built: the app pushes no vehicle or power-display rows.
    lua.globals().set(
        "UnitPowerType",
        lua.create_function(|lua, (token, index): (Value, Option<Value>)| {
            let usage = r#"Usage: UnitPowerType("unit"[, index])"#;
            let token = super::super::binding_abi::string_arg(lua, token, usage)?;
            if numeric_index(lua, index, 1) != 1 {
                return Ok((0, ""));
            }
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(unit) = model.unit(&token).filter(|u| u.exists) else {
                return Ok((0, ""));
            };
            let ty = unit.power_type;
            Ok((ty, TOKENS.get(usize::from(ty)).copied().unwrap_or("")))
        })?,
    )?;
    for (name, usage, maximum) in [
        ("UnitPower", r#"Usage: UnitPower("unit"[, type])"#, false),
        (
            "UnitPowerMax",
            r#"Usage: UnitPowerMax("unit"[, type])"#,
            true,
        ),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, (token, requested): (Value, Option<Value>)| {
                let token = super::super::binding_abi::string_arg(lua, token, usage)?;
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let Some(unit) = model.unit(&token).filter(|u| u.exists) else {
                    return Ok(0u32);
                };
                // Native 0x60ed40/0x60ef40: 7 selects the active type; invalid indices return 0.
                let requested = numeric_index(lua, requested, 7);
                if !(0..=7).contains(&requested) {
                    return Ok(0);
                }
                let ty = if requested == 7 {
                    unit.power_type
                } else {
                    requested as u8
                };
                let is_self = unit.guid != 0
                    && model
                        .unit("player")
                        .is_some_and(|player| player.guid == unit.guid);
                if !maximum && ty != unit.power_type && !is_self {
                    // Native 0x4cee50 compares identity, not token spelling.
                    return Ok(0);
                }
                // No object (an out-of-range member or pet): the roster record's power, for the
                // unit's own type only (party `+0x14`, raid `+0x64`), as the snapshot carries it.
                // A held unit whose power fields have not arrived reads 0.
                let Some(powers) = unit.powers.as_ref() else {
                    let own = ty == unit.power_type && !unit.has_object;
                    return Ok(match (own, maximum) {
                        (false, _) => 0,
                        (true, false) => unit.power,
                        (true, true) => unit.max_power,
                    });
                };
                let Some(value) = powers.get(usize::from(ty)) else {
                    return Ok(0);
                };
                // Build-12340 helper 0x7fde00/table0xaf5220; third arguments are ignored.
                let divisor = match ty {
                    1 | 6 => 10,
                    4 => 1000,
                    _ => 1,
                };
                Ok((if maximum {
                    value.maximum
                } else {
                    value.current
                }) / divisor)
            })?,
        )?;
    }
    // `UnitMana` and `UnitManaMax` are the same functions as `UnitPower` and `UnitPowerMax` in
    // 12340's table (both at `0x60ed40`, both at `0x60ef40`).
    let g = lua.globals();
    g.set("UnitMana", g.get::<mlua::Function>("UnitPower")?)?;
    g.set("UnitManaMax", g.get::<mlua::Function>("UnitPowerMax")?)?;
    Ok(())
}
