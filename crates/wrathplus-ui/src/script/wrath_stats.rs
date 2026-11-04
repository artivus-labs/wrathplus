//! Wrath's derived stat readers used by PaperDollFrame.lua.

use mlua::Lua;

use super::super::Model;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // PaperDollFrame.lua:280 uses this as a display multiplier. The server already
    // folds max-health auras into the descriptor's maximum health; this client has
    // no separate local health modifier to apply a second time (for any unit).
    lua.globals().set(
        "GetUnitMaxHealthModifier",
        lua.create_function(|_, _unit: Option<String>| Ok(1.0))?,
    )?;
    lua.globals().set(
        "GetAttackPowerForStat",
        lua.create_function(|lua, (stat, amount): (u32, f64)| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let class = model
                .player_record
                .class
                .as_ref()
                .map(|(_, token)| token.as_str());
            // StatSystem.cpp: Player::UpdateAttackPowerAndDamage's melee formulas. The
            // level/base offsets are not contributions from the supplied stat itself.
            let coefficient = match (stat, class) {
                (1, Some("WARRIOR" | "PALADIN" | "DEATHKNIGHT" | "DRUID")) => 2.0,
                (1, Some("HUNTER" | "ROGUE" | "SHAMAN" | "MAGE" | "PRIEST" | "WARLOCK")) => 1.0,
                (2, Some("HUNTER" | "ROGUE" | "SHAMAN")) => 1.0,
                (2, Some("DRUID"))
                    if model.shapeshift.forms.iter().any(|form| {
                        // Cat Form has one spell rank; never infer form from a localized label
                        // or its position in the stance bar.
                        form.view.active && form.view.spell_id == 768
                    }) =>
                {
                    1.0
                }
                _ => 0.0,
            };
            Ok(amount * coefficient)
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::super::super::{ShapeshiftFormView, UiScript};

    #[test]
    fn wrath_attack_power_stat_contributions_follow_class_not_level_or_label() {
        let s = UiScript::new().unwrap();
        for (class, strength, agility) in [
            ("WARRIOR", 2., 0.),
            ("PALADIN", 2., 0.),
            ("DEATHKNIGHT", 2., 0.),
            ("HUNTER", 1., 1.),
            ("ROGUE", 1., 1.),
            ("SHAMAN", 1., 1.),
            ("MAGE", 1., 0.),
            ("PRIEST", 1., 0.),
            ("WARLOCK", 1., 0.),
            ("DRUID", 2., 0.),
        ] {
            s.model_mut().player_record.class = Some(("Localized class".into(), class.into()));
            assert_eq!(
                s.eval::<f64>("return GetAttackPowerForStat(1, 21.5)")
                    .unwrap(),
                21.5 * strength,
                "{class}"
            );
            assert_eq!(
                s.eval::<f64>("return GetAttackPowerForStat(2, 21.5)")
                    .unwrap(),
                21.5 * agility,
                "{class}"
            );
            s.run("for stat=3,5 do assert(GetAttackPowerForStat(stat,100)==0) end assert(GetAttackPowerForStat(1,0)==0)").unwrap();
        }
    }

    #[test]
    fn wrath_max_health_display_multiplier_is_neutral_for_every_unit() {
        let s = UiScript::new().unwrap();
        s.run("for _, unit in ipairs({'player', 'target', 'pet', 'party1', 'unknown'}) do assert(GetUnitMaxHealthModifier(unit) == 1) end assert(GetUnitMaxHealthModifier() == 1)").unwrap();
    }

    #[test]
    fn wrath_druid_agility_only_contributes_in_active_cat_form() {
        let mut s = UiScript::new().unwrap();
        s.model_mut().player_record.class = Some(("Druide".into(), "DRUID".into()));
        for (spell_id, active, expected) in [
            (768, true, 40.),
            (768, false, 0.),
            (5487, true, 0.),
            (9634, true, 0.),
            (24858, true, 0.),
        ] {
            s.set_shapeshift_forms(vec![ShapeshiftFormView {
                spell_id,
                active,
                name: "not a form identifier".into(),
                ..Default::default()
            }]);
            assert_eq!(
                s.eval::<f64>("return GetAttackPowerForStat(2,40)").unwrap(),
                expected
            );
            assert_eq!(
                s.eval::<f64>("return GetAttackPowerForStat(1,40)").unwrap(),
                80.
            );
        }
    }
}
