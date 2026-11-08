//! Wrath secure-template and bar globals (`SecureTemplates.lua`, `TotemFrame.lua`,
//! `MainMenuBar.lua`), from build 12340's bindings: `CastSpellByID` (`0x53e060`), `RunMacro`
//! (`0x566e80`), `RunMacroText` (`0x566400`), `UnitCanAssist` (`0x60d690`),
//! `AllowAttributeChanges` (`0x49f210`), `DestroyTotem` (`0x51d520`) and the addon memory pair
//! (`0x511530`/`0x511510`). The calls that act ride [`ScriptCall`] in call order.

use mlua::{Lua, MultiValue, Value};

use super::{Model, ScriptCall};

fn push(lua: &Lua, call: ScriptCall) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .script_calls
        .push(call);
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Number(f) => Some(*f),
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

/// A run's button argument as `lua_tostring` reads it: a string or a number, else none.
fn button_text(lua: &Lua, v: Value) -> mlua::Result<Option<String>> {
    Ok(match v {
        Value::String(_) | Value::Integer(_) | Value::Number(_) => {
            lua.coerce_string(v)?.map(|s| s.to_string_lossy())
        }
        _ => None,
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // CastSpellByID(spellID[, target]): a spell in the player's book casts there, at the target
    // token's unit; else the pet book's casts through the pet. Unknown ids cast nothing.
    g.set(
        "CastSpellByID",
        lua.create_function(|lua, (id, target): (Value, Option<String>)| {
            let Some(id) = number(&id).and_then(|n| u32::try_from(n as i64).ok()) else {
                return Err(mlua::Error::runtime(
                    "Usage: CastSpellByID(spellID[, target])",
                ));
            };
            let (player, pet) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                (
                    model.spellbook.slots.iter().any(|s| s.spell_id == id),
                    model.pet_book.slots.iter().any(|s| s.spell_id == id),
                )
            };
            let target = target.filter(|t| !t.is_empty());
            let on_self = target
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("player"));
            let call = Some((on_self, target));
            match (call, player, pet) {
                (Some((on_self, None)), true, _) => push(
                    lua,
                    ScriptCall::CastSpell {
                        spell_id: id,
                        on_self,
                    },
                ),
                (Some((_, Some(unit))), true, _) => {
                    push(lua, ScriptCall::CastSpellAt { spell_id: id, unit })
                }
                (Some((on_self, _)), false, true) => push(
                    lua,
                    ScriptCall::CastPetSpell {
                        spell_id: id,
                        on_self,
                    },
                ),
                _ => {}
            }
            Ok(())
        })?,
    )?;
    // RunMacro(index or name[, button]): the macro's lines, as `UseAction` runs a macro slot; the
    // button, argument 2 as a string (`0x566e80`), is what `GetRunningMacroButton` answers.
    // Unresolved, labelled: the `button` argument's conditional is not modelled.
    g.set(
        "RunMacro",
        lua.create_function(|lua, (m, button): (Value, Value)| {
            let index = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                match &m {
                    Value::String(s) if number(&m).is_none() => {
                        let name = s.to_str()?.to_string();
                        Some(model.macros.index_by_name(&name)).filter(|&i| i > 0)
                    }
                    _ => number(&m)
                        .and_then(|n| usize::try_from(n as i64).ok())
                        .filter(|&i| model.macros.get(i).is_some()),
                }
            };
            if let Some(index) = index {
                push(
                    lua,
                    ScriptCall::RunMacro {
                        index: index as u32,
                        button: button_text(lua, button)?,
                    },
                );
            }
            Ok(())
        })?,
    )?;
    g.set(
        "RunMacroText",
        lua.create_function(|lua, (text, button): (Value, Value)| {
            let Value::String(text) = text else {
                return Err(mlua::Error::runtime(
                    r#"Usage: RunMacroText("text" [, "button"])"#,
                ));
            };
            // The text is copied into a 1024-byte buffer (`0x566451`).
            let mut text = text.to_str()?.to_string();
            while text.len() > 1023 {
                text.pop();
            }
            let button = button_text(lua, button)?;
            push(lua, ScriptCall::RunMacroText { text, button });
            Ok(())
        })?,
    )?;
    // UnitCanAssist(unit, otherUnit): 1 when both resolve and the other unit takes help
    // (`0x7293d0`). Unresolved, labelled: the snapshot's friendly reaction with no attack stands
    // in for the reference's assist test.
    g.set(
        "UnitCanAssist",
        lua.create_function(|lua, (a, b): (Value, Value)| {
            let (Value::String(_), Value::String(b)) = (&a, &b) else {
                return Err(mlua::Error::runtime(
                    r#"Usage: UnitCanAssist("unit", "otherUnit")"#,
                ));
            };
            let other = b.to_str()?.to_string();
            let friend: Option<i64> = lua
                .globals()
                .get::<mlua::Function>("UnitIsFriend")?
                .call((a, other.clone()))?;
            let attack: Option<i64> = lua
                .globals()
                .get::<mlua::Function>("UnitCanAttack")?
                .call(("player", other))?;
            Ok((friend.is_some() && attack.is_none()).then_some(1))
        })?,
    )?;
    // Frame:AllowAttributeChanges(), the secure-attribute unlock; this trusted UI protects
    // nothing, so it changes nothing.
    let frames: mlua::Table = lua.named_registry_value(super::REG_FRAME_METHODS)?;
    frames.set(
        "AllowAttributeChanges",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    g.set(
        "DestroyTotem",
        lua.create_function(|lua, v: Value| {
            let Some(slot) = number(&v) else {
                return Err(mlua::Error::runtime("Usage: DestroyTotem(slot)"));
            };
            let slot = slot.round() as i64;
            if !(1..=4).contains(&slot) {
                return Err(mlua::Error::runtime(
                    "Totem slot must be in range of 1 to 4",
                ));
            }
            push(
                lua,
                ScriptCall::DestroyTotem {
                    slot: (slot - 1) as u8,
                },
            );
            Ok(())
        })?,
    )?;
    // Per-addon allocation accounting (`0x511530`) is not kept by this VM: every addon reads 0 KB.
    g.set(
        "GetAddOnMemoryUsage",
        lua.create_function(|_, _: MultiValue| Ok(0))?,
    )?;
    g.set(
        "UpdateAddOnMemoryUsage",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{ScriptCall, SpellBookState, SpellSlotView, UiScript};

    #[test]
    fn calls_ride_the_queue_in_native_shapes() {
        let mut s = UiScript::new().unwrap();
        s.set_spellbook(SpellBookState {
            tabs: Vec::new(),
            slots: vec![SpellSlotView {
                spell_id: 133,
                ..Default::default()
            }],
        });
        s.run(
            r#"CastSpellByID(133) CastSpellByID(133, "player") CastSpellByID(133, "focus")
            CastSpellByID(999) RunMacroText("/say hi") DestroyTotem(2)
            assert(not pcall(DestroyTotem, 5) and not pcall(CastSpellByID))
            assert(GetAddOnMemoryUsage("Blizzard_TimeManager") == 0)"#,
        )
        .unwrap();
        assert_eq!(
            s.take_script_calls(),
            [
                ScriptCall::CastSpell {
                    spell_id: 133,
                    on_self: false
                },
                ScriptCall::CastSpellAt {
                    spell_id: 133,
                    unit: "player".into()
                },
                ScriptCall::CastSpellAt {
                    spell_id: 133,
                    unit: "focus".into()
                },
                ScriptCall::RunMacroText {
                    text: "/say hi".into(),
                    button: None,
                },
                ScriptCall::DestroyTotem { slot: 1 },
            ]
        );
    }
}
