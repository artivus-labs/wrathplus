//! The paper doll's skill verbs, `UnitAttackBothHands`, `UnitRangedAttack` and `UnitDefense`,
//! over the combat-stats snapshots `char_stats` reads.

use mlua::Lua;

use super::Model;

/// The clamp `UnitDefense` (`0x610e7f`) and each hand of `UnitAttackBothHands` (`0x6104ef`) apply:
/// a modifier below `-base` becomes `-base`, so the total never reads negative.
fn skill_clamped((base, modifier): (i32, i32)) -> (i64, i64) {
    let base = i64::from(base);
    let modifier = i64::from(modifier);
    (base, if modifier + base < 0 { -base } else { modifier })
}

/// A player skill pair with its combat ratings' points added to the modifier, each the rating
/// bonus (`0x6de410`) stored as a float, less 0.5 (`0xad9f58`), `fistp` to nearest, ties to even.
fn with_ratings(model: &Model, (base, temp): (i32, i32), native: &[usize]) -> (i32, i32) {
    let bonus = &model.wrath_paperdoll_stats.conversions.rating_bonus;
    let points: i32 = native
        .iter()
        .map(|&r| (f64::from(bonus[r]) - 0.5).round_ties_even() as i32)
        .sum();
    (base, temp.wrapping_add(points))
}

/// A non-player's `UnitDefense` and `UnitAttackBothHands` answer: `CGUnit_C`'s vtable bodies
/// (`0xa34d90`: `+0x140` is `0x734f70`, `+0x144` is `0x734fa0`) read `UNIT_FIELD_LEVEL`, times 5,
/// with a 0 modifier, the hand unread; a creature has no skill block.
fn cgunit_skill(model: &Model, token: &str) -> i64 {
    model.unit(token).map_or(0, |u| i64::from(u.level)) * 5
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // UnitAttackBothHands(unit) → (mainBase, mainMod, offBase, offMod): `0x610450` calls
    // `[vtbl+0x144]` for hand 0 then hand 1; a pet's `CGUnit_C` body (`0x734fa0`) ignores the
    // hand, so both pairs are `(level * 5, 0)`.
    g.set(
        "UnitAttackBothHands",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match token.as_deref() {
                Some("player") => model
                    .player_combat_stats
                    .as_ref()
                    .map_or((0, 0, 0, 0), |s| {
                        // `0x6e0c30`: CR_WEAPON_SKILL (0), then the hand's own, 20 or 21.
                        let main = with_ratings(&model, s.main_weapon_skill, &[0, 20]);
                        let off = with_ratings(&model, s.offhand_weapon_skill, &[0, 21]);
                        let ((mb, mm), (ob, om)) = (skill_clamped(main), skill_clamped(off));
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

    // UnitRangedAttack(unit) → (base, modifier) (`0x6107d0` → `0x71b740` → `0x6e0d30`): the ranged
    // weapon's skill pair plus CR_WEAPON_SKILL (0) and CR_WEAPON_SKILL_RANGED (22), no clamp;
    // `0x71b740` answers 0, 0 for a non-player.
    g.set(
        "UnitRangedAttack",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let pair = match (token.as_deref(), &model.player_combat_stats) {
                (Some("player"), Some(s)) => with_ratings(&model, s.ranged_weapon_skill, &[0, 22]),
                _ => (0, 0),
            };
            Ok((i64::from(pair.0), i64::from(pair.1)))
        })?,
    )?;

    // UnitDefense(unit) → (base, modifier) (`0x610de0`); stock splits the modifier's sign itself.
    // The gate is SELF or `UNIT_FIELD_SUMMONEDBY` = us, then `[vtbl+0x140]`: the player's
    // `0x6de6d0` reads the Defense skill, a pet's `CGUnit_C` body (`0x734f70`) answers level * 5
    // and 0. Any other token gets the gate's zeros.
    g.set(
        "UnitDefense",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match token.as_deref() {
                Some("player") => model.player_combat_stats.as_ref().map_or((0, 0), |s| {
                    // `0x6de6d0`: CR_DEFENSE_SKILL (1).
                    skill_clamped(with_ratings(&model, s.defense_skill, &[1]))
                }),
                Some("pet") => (cgunit_skill(&model, "pet"), 0),
                _ => (0, 0),
            })
        })?,
    )?;
    Ok(())
}
