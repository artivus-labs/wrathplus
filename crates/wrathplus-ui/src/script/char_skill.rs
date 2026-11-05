//! The paper doll's skill verbs, `UnitAttackBothHands`, `UnitRangedAttack` and `UnitDefense`,
//! over the combat-stats snapshots `char_stats` reads.

use mlua::Lua;

use super::char_stats::with_unit_stats;
use super::Model;

/// The clamp `UnitDefense` (`0x610e7f`) and each hand of `UnitAttackBothHands` (`0x6104ef`) apply:
/// a modifier below `-base` becomes `-base`, so the total never reads negative.
fn skill_clamped((base, modifier): (i32, i32)) -> (i64, i64) {
    let base = i64::from(base);
    let modifier = i64::from(modifier);
    (base, if modifier + base < 0 { -base } else { modifier })
}

/// A non-player's `UnitDefense` and `UnitAttackBothHands` answer: `CGUnit_C`'s vtable bodies
/// (`0xa34d90`: `+0x140` is `0x734f70`, `+0x144` is `0x734fa0`) read `UNIT_FIELD_LEVEL`, times 5,
/// with a 0 modifier, the hand unread; a creature has no skill block.
fn cgunit_skill(model: &Model, token: &str) -> i64 {
    model.unit(token).map_or(0, |u| i64::from(u.level)) * 5
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // UnitAttackBothHands(unit) → (mainBase, mainMod, offBase, offMod): `0x518810` calls
    // `[vtbl+0xb0]` for hand 0 then hand 1. Stock reads only the first pair
    // (`PaperDollFrame.lua:398-399`). A pet's `CGUnit_C` body (`0x6136b0`) ignores the hand, so
    // both pairs are `(level * 5, 0)`.
    g.set(
        "UnitAttackBothHands",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match token.as_deref() {
                Some("player") => model
                    .player_combat_stats
                    .as_ref()
                    .map_or((0, 0, 0, 0), |s| {
                        let (mb, mm) = skill_clamped(s.main_weapon_skill);
                        let (ob, om) = skill_clamped(s.offhand_weapon_skill);
                        (mb, mm, ob, om)
                    }),
                Some("pet") => {
                    let base = cgunit_skill(&model, "pet");
                    (base, 0, base, 0)
                }
                _ => (0, 0, 0, 0),
            })
        })?,
    )?;

    // UnitRangedAttack(unit) → (base, modifier), the ranged weapon's skill pair.
    g.set(
        "UnitRangedAttack",
        lua.create_function(|lua, token: Option<String>| {
            Ok(with_unit_stats(lua, &token, |s| {
                (
                    i64::from(s.ranged_weapon_skill.0),
                    i64::from(s.ranged_weapon_skill.1),
                )
            }))
        })?,
    )?;

    // UnitDefense(unit) → (base, modifier); stock splits the modifier's sign itself
    // (`PaperDollFrame.lua:259-271`). The gate is SELF or `UNIT_FIELD_SUMMONEDBY` = us, then
    // `[vtbl+0xac]`: `CGPlayer_C` `0x5eda20` reads the Defense skill (`0x6de040`), `CGUnit_C`
    // `0x613680` answers level * 5 and 0. Any other token gets the gate's zeros.
    g.set(
        "UnitDefense",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match token.as_deref() {
                Some("player") => model
                    .player_combat_stats
                    .as_ref()
                    .map_or((0, 0), |s| skill_clamped(s.defense_skill)),
                Some("pet") => (cgunit_skill(&model, "pet"), 0),
                _ => (0, 0),
            })
        })?,
    )?;
    Ok(())
}
