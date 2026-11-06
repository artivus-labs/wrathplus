//! Wrath's derived stat readers used by PaperDollFrame.lua.

use mlua::{Lua, Value};

use super::super::{binding_abi, Model};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // GetUnitMaxHealthModifier(unit) (`0x612870`): `isstring` or the Usage, then for the active
    // player or a unit with a pet number `UNIT_FIELD_MAXHEALTHMODIFIER + 1.0` (`0x6128d3`), else 0.
    // Labelled: the unit is the exact "player" or "pet" snapshot, not `0x60c1f0`'s resolve.
    lua.globals().set(
        "GetUnitMaxHealthModifier",
        lua.create_function(|lua, unit: Value| {
            let usage = r#"Usage: GetUnitMaxHealthModifier("unit")"#;
            let unit = binding_abi::string_arg(lua, unit, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let stats = match unit.as_str() {
                "player" => model.player_combat_stats.as_ref(),
                "pet" => model.pet_combat_stats.as_ref(),
                _ => None,
            };
            Ok(stats.map_or(0.0, |s| f64::from(s.max_health_modifier) + 1.0))
        })?,
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
                model.shapeshift.form_flags,
                stat,
                value,
            ))
        })?,
    )
}

/// `0x6d7070`: the class's `ChrClasses` row (field 1, `== 1` for the Agility classes, `0x6d70aa`)
/// and `a = max(value - 10, 0)`. Strength gives `a`, or `2a` outside those classes; Agility gives
/// `a` inside them, plus the whole `value` in a form whose flags carry `0x20` (`0x6d7108`), which
/// the current `SpellShapeshiftForm.dbc` row supplies. No class row, or any other stat, is 0.
fn attack_power_for_stat(class: Option<u32>, form_flags: u32, stat: i32, value: i32) -> i32 {
    let Some(bonus_stat) = class else { return 0 };
    let agility_class = bonus_stat == 1;
    let a = value.wrapping_sub(10).max(0);
    match stat.wrapping_sub(1) {
        0 if agility_class => a,
        0 => a.wrapping_mul(2),
        1 => {
            let own = if agility_class { a } else { 0 };
            own.wrapping_add(if form_flags & 0x20 != 0 { value } else { 0 })
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{UiScript, UnitCombatStats, UnitState};

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
    fn max_health_modifier_is_the_field_plus_one_for_the_player_or_pet_only() {
        let mut s = UiScript::new().unwrap();
        s.set_player_combat_stats(Some(UnitCombatStats {
            max_health_modifier: 0.25,
            ..Default::default()
        }));
        s.set_pet_combat_stats(Some(UnitCombatStats::default()));
        s.run("assert(GetUnitMaxHealthModifier('player') == 1.25 and GetUnitMaxHealthModifier('pet') == 1) assert(GetUnitMaxHealthModifier('target') == 0)").unwrap();
        let err = s.run("GetUnitMaxHealthModifier()").unwrap_err().to_string();
        assert!(
            err.contains(r#"Usage: GetUnitMaxHealthModifier("unit")"#),
            "{err}"
        );
    }

    #[test]
    fn agility_contribution_uses_form_row_flags_not_form_identity() {
        let mut s = UiScript::new().unwrap();
        for bonus_stat in [0, 1] {
            s.set_unit(
                "player",
                Some(UnitState {
                    exists: true,
                    damage_bonus_stat: Some(bonus_stat),
                    ..Default::default()
                }),
            );
            // Cat without the flag, a different flagged form, and an absent row.
            for (form, flags) in [(1, 0), (5, 0x20), (31, 0x21), (0, 0)] {
                s.set_shapeshift_form_byte(form);
                s.set_shapeshift_form_flags(flags);
                let expected =
                    if bonus_stat == 1 { 30 } else { 0 } + if flags & 0x20 != 0 { 40 } else { 0 };
                assert_eq!(
                    s.eval::<i32>("return GetAttackPowerForStat(2,40)").unwrap(),
                    expected
                );
                assert_eq!(
                    s.eval::<i32>("return GetAttackPowerForStat(1,40)").unwrap(),
                    if bonus_stat == 1 { 30 } else { 60 }
                );
            }
        }
    }
}
