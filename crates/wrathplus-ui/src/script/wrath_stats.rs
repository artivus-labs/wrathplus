//! Wrath's derived stat readers used by PaperDollFrame.lua.

use mlua::{Lua, Value};

use super::super::{binding_abi, Model};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // PaperDollFrame.lua:280 uses this as a display multiplier. The server already
    // folds max-health auras into the descriptor's maximum health; this client has
    // no separate local health modifier to apply a second time (for any unit).
    lua.globals().set(
        "GetUnitMaxHealthModifier",
        lua.create_function(|_, _unit: Option<String>| Ok(1.0))?,
    )?;
    // GetAttackPowerForStat(stat, value) (`0x60e560`): both arguments `isnumber` (one Usage,
    // `0xa230e4`) and truncated (`0x88b9c0`); no player answers 0.
    lua.globals().set(
        "GetAttackPowerForStat",
        lua.create_function(|lua, (stat, value): (Value, Value)| {
            const USAGE: &str = "Usage: Script_GetAttackPowerForStat(stat, value)";
            let stat = binding_abi::number_arg(lua, stat, USAGE)?;
            let value = binding_abi::number_arg(lua, value, USAGE)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let class = model.unit("player").and_then(|u| u.damage_bonus_stat);
            Ok(attack_power_for_stat(
                class,
                model.shapeshift.form_byte,
                stat,
                value,
            ))
        })?,
    )
}

/// `0x6d7070`: the class's `ChrClasses` row (field 1, `== 1` for the Agility classes, `0x6d70aa`)
/// and `a = max(value - 10, 0)`. Strength gives `a`, or `2a` outside those classes; Agility gives
/// `a` inside them, plus the whole `value` in a form whose flags carry `0x20` (`0x6d7108`), which
/// `SpellShapeshiftForm.dbc` sets on form 1, Cat Form, alone. No class row, or any other stat, is 0.
fn attack_power_for_stat(class: Option<u32>, form: u8, stat: i32, value: i32) -> i32 {
    const CAT_FORM: u8 = 1;
    let Some(bonus_stat) = class else { return 0 };
    let agility_class = bonus_stat == 1;
    let a = value.wrapping_sub(10).max(0);
    match stat.wrapping_sub(1) {
        0 if agility_class => a,
        0 => a.wrapping_mul(2),
        1 => {
            let own = if agility_class { a } else { 0 };
            own.wrapping_add(if form == CAT_FORM { value } else { 0 })
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{UiScript, UnitState};

    /// `0x6d7070` on 12340's `ChrClasses` field 1: Hunter, Rogue and Shaman are 1, the rest 0.
    #[test]
    fn attack_power_for_stat_takes_ten_off_and_doubles_strength_outside_agility_classes() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.eval::<f64>("return GetAttackPowerForStat(1, 23)")
                .unwrap(),
            0.0
        );
        for (bonus_stat, strength, agility) in [(0, 26., 0.), (1, 13., 13.)] {
            s.set_unit(
                "player",
                Some(UnitState {
                    exists: true,
                    damage_bonus_stat: Some(bonus_stat),
                    ..Default::default()
                }),
            );
            for (call, expected) in [
                ("1, 23", strength),
                ("1, 23.9", strength),
                ("2, 23", agility),
                ("1, 10", 0.),
                ("1, 5", 0.),
                ("3, 23", 0.),
                ("'1', '23'", strength),
            ] {
                let got = s.eval::<f64>(&format!("return GetAttackPowerForStat({call})"));
                assert_eq!(got.unwrap(), expected, "{bonus_stat}: {call}");
            }
        }
        let err = s
            .run("GetAttackPowerForStat(1, 'x')")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Usage: Script_GetAttackPowerForStat(stat, value)"));
    }

    #[test]
    fn wrath_max_health_display_multiplier_is_neutral_for_every_unit() {
        let s = UiScript::new().unwrap();
        s.run("for _, unit in ipairs({'player', 'target', 'pet', 'party1', 'unknown'}) do assert(GetUnitMaxHealthModifier(unit) == 1) end assert(GetUnitMaxHealthModifier() == 1)").unwrap();
    }

    #[test]
    fn wrath_druid_agility_only_contributes_in_cat_form() {
        let mut s = UiScript::new().unwrap();
        s.set_unit(
            "player",
            Some(UnitState {
                exists: true,
                damage_bonus_stat: Some(0),
                ..Default::default()
            }),
        );
        // Cat Form (1) adds the whole value; Bear (5), Dire Bear (8) and Moonkin (31) nothing.
        for (form, expected) in [(1, 40.), (0, 0.), (5, 0.), (8, 0.), (31, 0.)] {
            s.set_shapeshift_form_byte(form);
            let agility = s.eval::<f64>("return GetAttackPowerForStat(2,40)").unwrap();
            assert_eq!(agility, expected, "form {form}");
            let strength = s.eval::<f64>("return GetAttackPowerForStat(1,40)").unwrap();
            assert_eq!(strength, 60.);
        }
    }
}
