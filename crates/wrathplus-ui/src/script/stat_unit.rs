//! The unit a character-sheet verb's token names, through build 12340's resolver (`0x60c1f0`):
//! the active player, our pet, another held unit, or nobody.

use mlua::Lua;

use super::{Model, UnitCombatStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatUnit {
    Player,
    Pet,
    /// A held unit that is neither; `is_player` is the PLAYER typemask some verbs test.
    Other {
        is_player: bool,
    },
    Nobody,
}

impl Model {
    /// `0x60c1f0`'s unit (the resolver's guid, then the held object), sorted as the verbs test it:
    /// the active player (`0x4cee50`), our pet, another held unit, or nobody.
    pub(crate) fn stat_unit(&self, token: &str) -> StatUnit {
        let Some(guid) = self.unit_guid(token) else {
            return StatUnit::Nobody;
        };
        if guid == self.local_guid() {
            StatUnit::Player
        } else if guid == self.unit_guids.pet {
            StatUnit::Pet
        } else {
            match self.unit_by_guid(guid) {
                Some(u) if u.has_object => StatUnit::Other {
                    is_player: u.is_player,
                },
                _ => StatUnit::Nobody,
            }
        }
    }
}

/// [`Model::stat_unit`] from a binding.
pub(super) fn stat_unit_of(lua: &Lua, token: &str) -> StatUnit {
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    model.stat_unit(token)
}

/// `f` over the snapshot `token` names, else over the zeros the verb pushes. The damage percent is
/// then 0 for nobody (`UnitDamage` `0x6109d6`, `UnitRangedDamage` `0x6107a4`), 1.0 for a held unit
/// that is no player (`0x610981`, `0x61075a`), and 0 for another player (`UnitRangedDamage`
/// `0x610719`; `UnitDamage` reads its percent through `0x578290`, which is not fed). Labelled:
/// another unit's own fields (its swing times, a Beast Lore resistance) are not fed, so it reads
/// zeros.
pub(super) fn with_unit_stats<T>(
    lua: &Lua,
    token: &str,
    f: impl FnOnce(&UnitCombatStats) -> T,
) -> T {
    let unit = stat_unit_of(lua, token);
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    let pushed = match unit {
        StatUnit::Player => model.player_combat_stats.as_ref(),
        StatUnit::Pet => model.pet_combat_stats.as_ref(),
        _ => None,
    };
    if let Some(stats) = pushed {
        return f(stats);
    }
    let percent = match unit {
        StatUnit::Other { is_player: false } => 1.0,
        _ => 0.0,
    };
    f(&UnitCombatStats {
        damage_percent: percent,
        ranged_damage_percent: percent,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use crate::script::{UiScript, UnitCombatStats, UnitGuids, UnitState};

    /// The resolver's tokens (`0x60c1f0`): any spelling or chain that names the player or the pet
    /// reads its snapshot; a non-string raises the verb's Usage, a number names nobody.
    #[test]
    fn the_stat_verbs_resolve_their_token_and_raise_on_a_non_string() {
        let mut s = UiScript::new().unwrap();
        let player = UnitCombatStats {
            stats: [25, 0, 0, 0, 0],
            ..Default::default()
        };
        s.set_player_combat_stats(Some(player));
        s.set_pet_combat_stats(Some(UnitCombatStats {
            stats: [63, 0, 0, 0, 0],
            ..Default::default()
        }));
        s.set_unit_guids(&UnitGuids {
            player: 7,
            pet: 8,
            target: 7,
            focus: 9,
            ..Default::default()
        });
        for (token, strength) in [
            ("Player", 25),
            ("PLAYER", 25),
            ("target", 25),
            ("playerpet", 63),
        ] {
            let got = s.eval::<i64>(&format!("return (UnitStat('{token}', 1))"));
            assert_eq!(got.unwrap(), strength, "{token}");
        }
        let damage = "return select(7, UnitDamage(12)), select(7, UnitDamage('npc')), \
            select(6, UnitRangedDamage('focus'))";
        assert_eq!(s.eval::<(f64, f64, f64)>(damage).unwrap(), (0.0, 0.0, 0.0));
        // Another held unit: percent 1.0 (`0x610981`); a player's, through `0x578290`, is not fed.
        for (is_player, percent) in [(false, 1.0), (true, 0.0)] {
            let focus = UnitState {
                exists: true,
                has_object: true,
                guid: 9,
                is_player,
                ..Default::default()
            };
            s.set_unit("focus", Some(focus));
            let got = s.eval::<f64>("return select(7, UnitDamage('focus'))");
            assert_eq!(got.unwrap(), percent, "{is_player}");
        }
        for verb in ["UnitDefense", "UnitDamage", "UnitArmor", "UnitAttackSpeed"] {
            let err = s.run(&format!("{verb}(nil)")).unwrap_err().to_string();
            assert!(err.contains(&format!("Usage: {verb}(\"unit\")")), "{err}");
        }
        let err = s.run("UnitRangedAttackPower({})").unwrap_err().to_string();
        assert!(err.contains(r#"Usage: UnitAttackPower("unit")"#), "{err}");
    }
}
