//! Build-12340 spell predicates. Full learned-spell arrays are not the filtered visible book.
use std::collections::HashSet;

use mlua::{Lua, Value};

use super::super::{binding_abi::number_arg, Model, UiScript};

#[derive(Clone, Debug, Default)]
pub(crate) struct KnownSpells {
    player: HashSet<u32>,
    pet: HashSet<u32>,
    pet_actions: Vec<u32>,
}

impl UiScript {
    /// Replace the native known-spell arrays and ten raw pet action words together. The app
    /// must pass all learned IDs, including spells hidden from the visible spellbook; clear on
    /// session reset and replace the pet data on pet replacement/removal.
    pub fn set_wrath_known_spells(
        &mut self,
        player: impl IntoIterator<Item = u32>,
        pet: impl IntoIterator<Item = u32>,
        pet_actions: impl IntoIterator<Item = u32>,
    ) {
        self.model_mut().wrath_known_spells = KnownSpells {
            player: player.into_iter().collect(),
            pet: pet.into_iter().collect(),
            pet_actions: pet_actions.into_iter().take(10).collect(),
        };
    }
}

// Native optional flag helper 0x815500: boolean, number, or numeric string; other types keep
// default zero. This is not ordinary Lua truthiness (0 and "0" both select the player).
pub(crate) fn pet_flag(lua: &Lua, value: Value) -> mlua::Result<bool> {
    if let Value::Boolean(b) = value {
        return Ok(b);
    }
    Ok(lua
        .coerce_number(value)?
        .is_some_and(|n| n as i64 as i32 != 0))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "IsSpellKnown",
        lua.create_function(|lua, (id, pet): (Value, Value)| {
            // 0x53c3a0: signed truncated positive spell ID; success always pushes a boolean.
            let id = number_arg(lua, id, "Usage: IsSpellKnown(spellID[, isPet])")?;
            if id <= 0 {
                return Err(mlua::Error::runtime(
                    "Usage: IsSpellKnown(spellID[, isPet])",
                ));
            }
            let pet = pet_flag(lua, pet)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_known_spells;
            let id = id as u32;
            // 0x53b4e0 scans the selected array. Pet-only fallback 0x5d3560 checks ten
            // action words: spell types 1 and 8..17, six-bit type and 24-bit spell ID.
            Ok(if pet {
                state.pet.contains(&id)
                    || state.pet_actions.iter().any(|&word| {
                        let kind = (word >> 24) & 0x3f;
                        (kind == 1 || (8..18).contains(&kind)) && word & 0xffffff == id
                    })
            } else {
                state.player.contains(&id)
            })
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_spell_uses_full_separate_arrays_and_boolean_abi() {
        let mut script = UiScript::new().unwrap();
        script.set_wrath_known_spells([133, 668], [172], []);
        for args in [
            "133",
            "'668.9'",
            "133,false",
            "133,0",
            "133,'0'",
            "133,{}",
            "133,0.5",
        ] {
            assert!(
                script
                    .eval::<bool>(&format!("return IsSpellKnown({args})"))
                    .unwrap(),
                "{args}"
            );
        }
        for args in ["172,true", "172,1", "172,'1'", "172,-1"] {
            assert!(
                script
                    .eval::<bool>(&format!("return IsSpellKnown({args})"))
                    .unwrap(),
                "{args}"
            );
        }
        for args in ["172", "133,true", "999"] {
            assert!(script
                .eval::<bool>(&format!("return IsSpellKnown({args}) == false"))
                .unwrap());
        }
        assert_eq!(
            script
                .eval::<String>("return type(IsSpellKnown(133))")
                .unwrap(),
            "boolean"
        );
        assert_eq!(
            script
                .eval::<i64>("return select('#', IsSpellKnown(999))")
                .unwrap(),
            1
        );
        for args in ["", "nil", "false", "{}", "'bad'", "0", "0.9", "-1"] {
            assert!(script
                .eval::<bool>(&format!("return IsSpellKnown({args})"))
                .unwrap_err()
                .to_string()
                .contains("Usage: IsSpellKnown"));
        }
        script.set_wrath_known_spells([], [], []);
        assert!(!script.eval::<bool>("return IsSpellKnown(133)").unwrap());
    }

    #[test]
    fn pet_action_fallback_keeps_native_type_mask_and_ten_slot_bound() {
        let mut script = UiScript::new().unwrap();
        for kind in 0..64 {
            script.set_wrath_known_spells([], [], [(kind << 24) | 172]);
            assert_eq!(
                script
                    .eval::<bool>("return IsSpellKnown(172,true)")
                    .unwrap(),
                kind == 1 || (8..18).contains(&kind),
                "{kind}"
            );
            assert!(!script.eval::<bool>("return IsSpellKnown(172)").unwrap());
        }
        script.set_wrath_known_spells([], [], [0x810000ac]);
        assert!(script
            .eval::<bool>("return IsSpellKnown(172,true)")
            .unwrap());
        script.set_wrath_known_spells([], [], [0; 10].into_iter().chain([0x010000ac]));
        assert!(!script
            .eval::<bool>("return IsSpellKnown(172,true)")
            .unwrap());
    }
}
