//! Wrath player descriptor values and separately supplied GameTables conversions.
//! Missing conversions remain zero until the app has the corresponding table rows.

use super::stat_unit::StatUnit;
use super::{binding_abi, Model, UiScript};
use mlua::{Lua, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathStatConversions {
    pub rating_bonus: [f32; 25],
    /// The four spirit and crit conversions, the active player's (`0x71bae0`, `0x71bb70`,
    /// `0x71ba60`, `0x71b9f0` with the mana verb's `0.001f`), as the doubles they push.
    pub crit_from_agility: f64,
    pub spell_crit_from_intellect: f64,
    pub health_from_spirit: f64,
    pub mana_from_spirit: f64,
}

/// Percentages and mana/second are already resolved by the server. School arrays use
/// descriptor order (physical=0); Lua's school indexes start at 1. Ratings start at 1 too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathPaperDollStats {
    pub ratings: [u32; 25],
    pub dodge: f32,
    pub parry: f32,
    pub block: f32,
    pub melee_crit: f32,
    pub ranged_crit: f32,
    pub spell_crit: [f32; 7],
    pub spell_damage: [i32; 7],
    pub spell_healing: u32,
    pub spell_penetration: i32,
    pub shield_block: u32,
    pub expertise: [u32; 2],
    /// `PLAYER_PET_SPELL_POWER`, `GetPetSpellBonusDamage` (`0x60e410` reads player word 1177).
    pub pet_spell_power: i32,
    pub conversions: WrathStatConversions,
}

impl UiScript {
    /// Replace the entire player snapshot, including zeroing it on logout/no player.
    pub fn set_wrath_paperdoll_stats(&mut self, stats: WrathPaperDollStats) {
        self.model_mut().wrath_paperdoll_stats = stats;
    }
}

/// The rating verbs' index (`0x60ddb0`, `0x60de70`, `0x6082c0`): `isnumber` or the Usage, `fistp`
/// under a chop control word, less one, unsigned below 25 or `0xa1f780`'s range error.
fn rating_index(lua: &Lua, v: Value, usage: &'static str) -> mlua::Result<usize> {
    let i = binding_abi::number_arg(lua, v, usage)?.wrapping_sub(1) as u32;
    if i < 25 {
        Ok(i as usize)
    } else {
        Err(mlua::Error::RuntimeError(
            "ratingIndex is in the range 1 .. 25".into(),
        ))
    }
}

/// The school verbs' index (`0x60e290`, `0x60e310`): a bare `tonumber`, so nil is 0, truncated
/// (`0x88b9c0`), less one, unsigned at most 6 or the Usage.
fn school_index(lua: &Lua, v: Value, usage: &'static str) -> mlua::Result<usize> {
    let i = (binding_abi::coerced_number(lua, Some(v)) as i64 as i32).wrapping_sub(1) as u32;
    if i <= 6 {
        Ok(i as usize)
    } else {
        Err(mlua::Error::RuntimeError(usage.into()))
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    for (name, pick) in [
        (
            "GetDodgeChance",
            (|s: &WrathPaperDollStats| s.dodge as f64) as fn(&WrathPaperDollStats) -> f64,
        ),
        ("GetParryChance", |s| s.parry as f64),
        ("GetBlockChance", |s| s.block as f64),
        ("GetCritChance", |s| s.melee_crit as f64),
        ("GetRangedCritChance", |s| s.ranged_crit as f64),
        ("GetShieldBlock", |s| s.shield_block as f64),
        // `0x60e3b0`: signed.
        ("GetSpellBonusHealing", |s| s.spell_healing as i32 as f64),
        ("GetSpellPenetration", |s| s.spell_penetration as f64),
        // `0x60e4e0`: native CR_ARMOR_PENETRATION (24), at most 100.0 (`0x9e8cc8`, `fcom`).
        ("GetArmorPenetration", |s| {
            let bonus = s.conversions.rating_bonus[24];
            f64::from(if bonus > 100.0 { 100.0 } else { bonus })
        }),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, ()| {
                let m = lua.app_data_ref::<Model>().expect("model");
                Ok(pick(&m.wrath_paperdoll_stats))
            })?,
        )?;
    }
    // `0x60e130`, `0x60e1b0`, `0x612980`, `0x612a00`: `isstring` or the Usage, then the active
    // player or a unit with a pet number (`0x4cee50`, `UNIT_FIELD_PETNUMBER`); anything else 0.
    // Labelled: a pet's own conversions, which read its class, level and stats, are not fed.
    for (name, usage, pick) in [
        (
            "GetCritChanceFromAgility",
            r#"Usage: GetCritChanceFromAgility("unit")"#,
            (|s: &WrathStatConversions| s.crit_from_agility) as fn(&WrathStatConversions) -> f64,
        ),
        (
            "GetSpellCritChanceFromIntellect",
            r#"Usage: GetSpellCritChanceFromIntellect("unit")"#,
            |s| s.spell_crit_from_intellect,
        ),
        (
            "GetUnitHealthRegenRateFromSpirit",
            r#"Usage: GetUnitHealthRegenRateFromSpirit("unit")"#,
            |s| s.health_from_spirit,
        ),
        (
            "GetUnitManaRegenRateFromSpirit",
            r#"Usage: GetUnitManaRegenRateFromSpirit("unit")"#,
            |s| s.mana_from_spirit,
        ),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, unit: Value| {
                let unit = binding_abi::string_arg(lua, unit, usage)?;
                let m = lua.app_data_ref::<Model>().expect("model");
                Ok(if m.stat_unit(&unit) == StatUnit::Player {
                    pick(&m.wrath_paperdoll_stats.conversions)
                } else {
                    0.0
                })
            })?,
        )?;
    }
    // One number each; the player's values, 0 without one. GetMaxCombatRatingBonus is a client
    // constant (`0x608333`): `0xa1f778`'s 33.000001311302185 for native ratings 14 to 16, the
    // crit-taken trio, and `0x9ec208`'s -1 for the rest.
    for (name, usage, rating) in [
        (
            "GetCombatRating",
            "Usage: GetCombatRating(ratingIndex)",
            true,
        ),
        (
            "GetCombatRatingBonus",
            "Usage: GetCombatRatingBonus(ratingIndex)",
            true,
        ),
        (
            "GetMaxCombatRatingBonus",
            "Usage: GetMaxCombatRatingBonus(ratingIndex)",
            true,
        ),
        (
            "GetSpellCritChance",
            "Usage: GetSpellCritChance(school)",
            false,
        ),
        (
            "GetSpellBonusDamage",
            "Usage: GetSpellBonusDamage(school)",
            false,
        ),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, value: Value| {
                let i = if rating {
                    rating_index(lua, value, usage)?
                } else {
                    school_index(lua, value, usage)?
                };
                let m = lua.app_data_ref::<Model>().expect("model");
                let s = &m.wrath_paperdoll_stats;
                Ok(match name {
                    // `0x60de4d`: signed.
                    "GetCombatRating" => f64::from(s.ratings[i] as i32),
                    "GetCombatRatingBonus" => f64::from(s.conversions.rating_bonus[i]),
                    "GetMaxCombatRatingBonus" if (14..=16).contains(&i) => 33.000001311302185,
                    "GetMaxCombatRatingBonus" => -1.0,
                    "GetSpellCritChance" => f64::from(s.spell_crit[i]),
                    _ => f64::from(s.spell_damage[i]),
                })
            })?,
        )?;
    }
    for name in ["GetExpertise", "GetExpertisePercent"] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, ()| {
                let m = lua.app_data_ref::<Model>().expect("model");
                let s = &m.wrath_paperdoll_stats;
                Ok(match name {
                    // `0x612cb0`: signed, times 0.25.
                    "GetExpertisePercent" => (
                        f64::from(s.expertise[0] as i32) * 0.25,
                        f64::from(s.expertise[1] as i32) * 0.25,
                    ),
                    _ => (s.expertise[0] as f64, s.expertise[1] as f64),
                })
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_preserves_school_rating_indexes_and_multi_return_shapes() {
        let mut s = UiScript::new().unwrap();
        // 12340 has no such global (the name is not in the exe).
        assert!(s
            .eval::<bool>("return GetDodgeBlockParryChanceFromDefense == nil")
            .unwrap());
        let mut stats = WrathPaperDollStats {
            dodge: 5.5,
            parry: 6.5,
            block: 7.5,
            melee_crit: 8.5,
            ranged_crit: 9.5,
            shield_block: 123,
            spell_healing: 456,
            spell_penetration: 34,
            expertise: [12, 8],
            ..Default::default()
        };
        for i in 0..25 {
            stats.ratings[i] = (i + 100) as u32;
            stats.conversions.rating_bonus[i] = i as f32 + 0.5;
        }
        for i in 0..7 {
            stats.spell_crit[i] = i as f32 + 4.5;
            stats.spell_damage[i] = i as i32 + 50;
        }
        stats.conversions.crit_from_agility = 1.5;
        stats.conversions.spell_crit_from_intellect = 2.5;
        stats.conversions.health_from_spirit = 3.5;
        stats.conversions.mana_from_spirit = 4.5;
        s.set_wrath_paperdoll_stats(stats);
        // The conversions answer the unit `0x60c1f0` resolves when it is the active player
        // (`0x4cee50`), whatever the spelling.
        s.set_unit_guids(&crate::script::UnitGuids {
            player: 1,
            ..Default::default()
        });
        s.run("assert(GetCritChanceFromAgility('PLAYER')==1.5)")
            .unwrap();
        s.run("assert(GetDodgeChance()==5.5 and GetParryChance()==6.5 and GetBlockChance()==7.5) assert(GetCritChance()==8.5 and GetRangedCritChance()==9.5) assert(GetShieldBlock()==123 and GetSpellBonusHealing()==456 and GetSpellPenetration()==34) for i=1,25 do assert(GetCombatRating(i)==99+i) assert(GetCombatRatingBonus(i)==i-.5) assert(GetMaxCombatRatingBonus(i)==(i>=15 and i<=17 and 33.000001311302185 or -1)) end for i=1,7 do assert(GetSpellCritChance(i)==i+3.5) assert(GetSpellBonusDamage(i)==49+i) end local a,b=GetExpertise() assert(a==12 and b==8) a,b=GetExpertisePercent() assert(a==3 and b==2) assert(GetCritChanceFromAgility('player')==1.5 and GetSpellCritChanceFromIntellect('player')==2.5) assert(GetUnitHealthRegenRateFromSpirit('player')==3.5 and GetUnitManaRegenRateFromSpirit('player')==4.5) assert(GetUnitHealthRegenRateFromSpirit('target')==0)").unwrap();
        let err = s.run("GetCritChanceFromAgility()").unwrap_err().to_string();
        assert!(
            err.contains(r#"Usage: GetCritChanceFromAgility("unit")"#),
            "{err}"
        );
        for (call, raise) in [
            ("GetCombatRating(0)", "ratingIndex is in the range 1 .. 25"),
            (
                "GetMaxCombatRatingBonus(26)",
                "ratingIndex is in the range 1 .. 25",
            ),
            (
                "GetCombatRatingBonus('x')",
                "Usage: GetCombatRatingBonus(ratingIndex)",
            ),
            ("GetSpellCritChance()", "Usage: GetSpellCritChance(school)"),
            (
                "GetSpellBonusDamage(8)",
                "Usage: GetSpellBonusDamage(school)",
            ),
        ] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(err.contains(raise), "{call}: {err}");
        }
        // The signed words and the armor penetration cap; the unclamped school sum is the
        // snapshot's (`ui_paperdoll`).
        let mut signed = WrathPaperDollStats {
            spell_healing: -40i32 as u32,
            expertise: [-8i32 as u32, 4],
            ..Default::default()
        };
        signed.ratings[0] = -3i32 as u32;
        signed.conversions.rating_bonus[24] = 140.0;
        s.set_wrath_paperdoll_stats(signed);
        s.run("assert(GetSpellBonusHealing()==-40 and GetCombatRating(1)==-3) assert(GetArmorPenetration()==100 and GetExpertisePercent()==-2)").unwrap();
        s.set_wrath_paperdoll_stats(Default::default());
        s.run("assert(GetCombatRating(1)==0 and GetCritChance()==0 and GetUnitHealthRegenRateFromSpirit('player')==0) local a,b=GetExpertise() assert(a==0 and b==0)").unwrap();
    }
}
