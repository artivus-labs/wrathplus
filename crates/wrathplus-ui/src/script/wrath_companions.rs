//! Wrath companions (`PetPaperDollFrame.lua`'s companion tab): the CRITTER and MOUNT lists
//! (`0xbe8df8`/`0xbe8dfc`, one learn-ordered spell array per type) and their bindings
//! `GetNumCompanions` (`0x53c2a0`), `GetCompanionInfo` (`0x53e2c0`), `CallCompanion`
//! (`0x53e5e0`), `DismissCompanion` (`0x53e6b0`), `GetCompanionCooldown` (`0x53e490`),
//! `PickupCompanion` (`0x53c310`), `SummonRandomCritter` (`0x53e7e0`), and `PlayerModel:
//! SetCreature` (`0x597960`). The app builds the lists; the type token is matched by
//! `0x53b600` against `"CRITTER"`/`"MOUNT"` (`0xacccc4`).

use mlua::{Lua, MultiValue, Table, Value};

use super::cursor::{queue_cursor_update, CursorPayload, CursorSpell};
use super::modelframe::{with_model, REG_PLAYERMODEL_METHODS};
use super::{Model, ScriptCall, UiScript};

/// The two companion types, in `0x53b600`'s table order.
pub const COMPANION_TYPES: [&str; 2] = ["CRITTER", "MOUNT"];

/// One companion, as `GetCompanionInfo` answers it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompanionView {
    /// The spell's `EffectMiscValue[0]` (`0x53e339` reads record `+0x1b8`).
    pub creature_id: u32,
    /// The creature cache's name; `None` (nil) until the record lands.
    pub creature_name: Option<String>,
    pub spell_id: u32,
    /// The spell's `SpellIcon.dbc` path; `None` answers nil.
    pub icon: Option<String>,
    /// `0x802cb0`: the critter out is this creature, or this mount aura is on the player.
    pub active: bool,
    /// The spell's cooldown `(start_ms, duration_ms, enabled)` on the `GetTime` clock.
    pub cooldown: Option<(i64, u32, bool)>,
}

/// The pushed companion state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCompanions {
    /// Indexed by [`COMPANION_TYPES`].
    pub lists: [Vec<CompanionView>; 2],
    /// The player's `UNIT_FIELD_CRITTER`, which `DismissCompanion("CRITTER")` dismisses.
    pub critter_guid: u64,
}

impl UiScript {
    /// Replace the companion lists and critter guid.
    pub fn set_wrath_companions(&mut self, companions: WrathCompanions) {
        self.model_mut().wrath_companions = companions;
    }

    /// The creature a named `PlayerModel` pane was pointed at by `SetCreature`.
    pub fn model_pane_creature(&self, name: &str) -> Option<u32> {
        self.model_pane(name).and_then(|m| m.creature)
    }
}

/// `0x53b600`: the type token, case-insensitively as `_strnicmp` (`0x76e780`) compares it.
fn companion_type(v: &Value) -> Option<usize> {
    let Value::String(s) = v else { return None };
    let s = s.as_bytes();
    COMPANION_TYPES
        .iter()
        .position(|t| t.as_bytes().eq_ignore_ascii_case(&s))
}

/// The 1-based index argument (`0x84e030`, rounded), as a 0-based list index.
fn index_arg(v: &Value) -> Option<usize> {
    let n = match v {
        Value::Integer(i) => *i as f64,
        Value::Number(f) => *f,
        Value::String(s) => s.to_str().ok()?.trim().parse().ok()?,
        _ => return None,
    };
    usize::try_from(n.round() as i64 - 1).ok()
}

fn usage(text: &str) -> mlua::Error {
    mlua::Error::runtime(format!("Usage: {text}"))
}

/// The companion at `(type, index)`, or the binding's usage error.
fn entry(
    lua: &Lua,
    ty: &Value,
    index: &Value,
    text: &str,
) -> mlua::Result<(usize, Option<CompanionView>)> {
    let (Some(ty), Some(i)) = (companion_type(ty), index_arg(index)) else {
        return Err(usage(text));
    };
    let model = lua.app_data_ref::<Model>().expect("model");
    Ok((ty, model.wrath_companions.lists[ty].get(i).cloned()))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "GetNumCompanions",
        lua.create_function(|lua, ty: Value| {
            let ty = companion_type(&ty).ok_or_else(|| usage("GetNumCompanions(type)"))?;
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(model.wrath_companions.lists[ty].len() as i64)
        })?,
    )?;
    // creatureID, creatureName, creatureSpellID, icon, issummoned (1 or nil); nothing past the end.
    g.set(
        "GetCompanionInfo",
        lua.create_function(|lua, (ty, index): (Value, Value)| {
            let (_, c) = entry(lua, &ty, &index, "GetCompanionInfo(type, index)")?;
            let Some(c) = c else {
                return Ok(MultiValue::new());
            };
            let text = |s: Option<String>| -> mlua::Result<Value> {
                s.map_or(Ok(Value::Nil), |s| lua.create_string(s).map(Value::String))
            };
            Ok(MultiValue::from_vec(vec![
                Value::Integer(i64::from(c.creature_id)),
                text(c.creature_name)?,
                Value::Integer(i64::from(c.spell_id)),
                text(c.icon)?,
                if c.active {
                    Value::Integer(1)
                } else {
                    Value::Nil
                },
            ]))
        })?,
    )?;
    // `0x80da40(spell, 0, 0, 0, 0)`: cast with no explicit target.
    g.set(
        "CallCompanion",
        lua.create_function(|lua, (ty, index): (Value, Value)| {
            let (_, c) = entry(lua, &ty, &index, "CallCompanion(type, index)")?;
            if let Some(c) = c {
                let mut model = lua.app_data_mut::<Model>().expect("model");
                model.script_calls.push(ScriptCall::CastSpell {
                    spell_id: c.spell_id,
                    on_self: false,
                });
            }
            Ok(())
        })?,
    )?;
    // Every active companion of the type: a critter by its guid, a mount through the dismount.
    g.set(
        "DismissCompanion",
        lua.create_function(|lua, ty: Value| {
            let ty = companion_type(&ty).ok_or_else(|| usage("DismissCompanion(type)"))?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let active = model.wrath_companions.lists[ty].iter().any(|c| c.active);
            let critter = model.wrath_companions.critter_guid;
            if active {
                let call = if ty == 0 {
                    (critter != 0).then_some(ScriptCall::DismissCritter { critter })
                } else {
                    Some(ScriptCall::Dismount)
                };
                model.script_calls.extend(call);
            }
            Ok(())
        })?,
    )?;
    // start, duration, enable: the spell's cooldown, as `GetSpellCooldown` reports one.
    g.set(
        "GetCompanionCooldown",
        lua.create_function(|lua, (ty, index): (Value, Value)| {
            let (_, c) = entry(lua, &ty, &index, "GetCompanionCooldown(type, index)")?;
            let Some(c) = c else {
                return Ok((0.0, 0.0, 0));
            };
            let now = super::clock::now(lua);
            Ok(match c.cooldown {
                Some((start_ms, duration_ms, enabled)) => {
                    let (start, duration) =
                        (start_ms as f64 / 1000.0, f64::from(duration_ms) / 1000.0);
                    if start + duration > now || !enabled {
                        (start, duration, i32::from(enabled))
                    } else {
                        (0.0, 0.0, 1)
                    }
                }
                None => (0.0, 0.0, 1),
            })
        })?,
    )?;
    // `0x53bbc0`: the cursor clears; the same spell already held is the drop, an empty cursor
    // takes the companion's spell. The `0x5191c0(0xe)` gate is untraced.
    g.set(
        "PickupCompanion",
        lua.create_function(|lua, (ty, index): (Value, Value)| {
            let (kind, c) = entry(lua, &ty, &index, "PickupCompanion(type, index)")?;
            let Some(c) = c else { return Ok(()) };
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let held = match &model.cursor {
                Some(CursorPayload::Spell(s)) => Some(s.spell_id),
                Some(_) => Some(0),
                None => None,
            };
            model.cursor = None;
            if held.is_none() {
                // `GetCursorInfo` names a companion by its list and 1-based index (`0x8009b0`).
                model.cursor = Some(CursorPayload::Spell(CursorSpell {
                    book_slot: index_arg(&index).map_or(0, |i| i as u32 + 1),
                    book_type: COMPANION_TYPES[kind].into(),
                    spell_id: c.spell_id,
                    texture: c.icon,
                    passive: false,
                }));
            }
            queue_cursor_update(&mut model);
            Ok(())
        })?,
    )?;
    // `0x6d7b30`: a `rand() %` pick over the player's critter array (`+0x18d0`), taken here to
    // be the CRITTER list.
    g.set(
        "SummonRandomCritter",
        lua.create_function(|lua, ()| {
            let pick: f64 = lua
                .globals()
                .get::<Table>("math")?
                .get::<mlua::Function>("random")?
                .call(())?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let list = &model.wrath_companions.lists[0];
            if !list.is_empty() {
                let i = ((pick * list.len() as f64) as usize).min(list.len() - 1);
                let spell_id = list[i].spell_id;
                model.script_calls.push(ScriptCall::CastSpell {
                    spell_id,
                    on_self: false,
                });
            }
            Ok(())
        })?,
    )?;
    let playermodel: Table = lua.named_registry_value(REG_PLAYERMODEL_METHODS)?;
    playermodel.set(
        "SetCreature",
        lua.create_function(|lua, (this, creature): (Table, Value)| {
            let id = match creature {
                Value::Integer(i) => u32::try_from(i).ok(),
                Value::Number(f) => u32::try_from(f as i64).ok(),
                _ => None,
            }
            .ok_or_else(|| usage("SetCreature(creatureID)"))?;
            with_model(lua, &this, |m| {
                m.creature = Some(id);
                m.unit = None;
                m.path = None;
            })
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn companion(spell_id: u32, active: bool) -> CompanionView {
        CompanionView {
            creature_id: spell_id + 1000,
            creature_name: Some(format!("Pet{spell_id}")),
            spell_id,
            icon: Some("Interface\\Icons\\Pet".into()),
            active,
            cooldown: None,
        }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_companions(WrathCompanions {
            lists: [
                vec![companion(10, false), companion(11, true)],
                vec![companion(20, false)],
            ],
            critter_guid: 0xF130_0000_0000_0042,
        });
        s
    }

    #[test]
    fn counts_and_info_follow_the_pushed_lists() {
        let s = vm();
        s.run(
            r##"
            assert(GetNumCompanions("CRITTER") == 2 and GetNumCompanions("MOUNT") == 1)
            assert(GetNumCompanions("critter") == 2 and GetNumCompanions("Mount") == 1)
            assert(GetCompanionInfo("mount", 1) == 1020 and not pcall(GetNumCompanions))
            local id, name, spell, icon, out = GetCompanionInfo("CRITTER", 2)
            assert(id == 1011 and name == "Pet11" and spell == 11 and icon and out == 1)
            assert(select(5, GetCompanionInfo("CRITTER", 1)) == nil)
            assert(select("#", GetCompanionInfo("MOUNT", 2)) == 0)
            assert(not pcall(GetCompanionInfo, "MOUNT"))
            local start, duration, enable = GetCompanionCooldown("MOUNT", 1)
            assert(start == 0 and duration == 0 and enable == 1)
            "##,
        )
        .unwrap();
    }

    #[test]
    fn call_dismiss_and_random_queue_their_native_calls() {
        let mut s = vm();
        s.run(r#"CallCompanion("MOUNT", 1) DismissCompanion("CRITTER") DismissCompanion("MOUNT") SummonRandomCritter() CallCompanion("MOUNT", 9)"#)
            .unwrap();
        let calls = s.take_script_calls();
        assert_eq!(
            calls[..2],
            [
                ScriptCall::CastSpell {
                    spell_id: 20,
                    on_self: false
                },
                ScriptCall::DismissCritter {
                    critter: 0xF130_0000_0000_0042
                },
            ]
        );
        // No mount is out, so the mount dismiss queues nothing; the random pick is a critter.
        assert_eq!(calls.len(), 3);
        assert!(matches!(
            calls[2],
            ScriptCall::CastSpell {
                spell_id: 10 | 11,
                ..
            }
        ));
    }

    #[test]
    fn pickup_takes_the_spell_and_a_second_pickup_drops_it() {
        let mut s = vm();
        s.run(r#"PickupCompanion("CRITTER", 1)"#).unwrap();
        assert!(matches!(
            s.cursor_payload(),
            Some(CursorPayload::Spell(CursorSpell { spell_id: 10, .. }))
        ));
        let info = s
            .eval::<(String, i64, String)>("return GetCursorInfo()")
            .unwrap();
        assert_eq!(
            info,
            ("companion".into(), 1, "CRITTER".into()),
            "`0x515200` mode 3"
        );
        s.run(r#"PickupCompanion("CRITTER", 1)"#).unwrap();
        assert!(s.cursor_payload().is_none());
        let _ = s.take_script_calls();
    }

    #[test]
    fn set_creature_points_a_player_model_at_the_template() {
        let s = vm();
        s.run(r#"local m = CreateFrame("PlayerModel", "Pane") m:SetCreature(1011) assert(not pcall(m.SetCreature, m))"#)
            .unwrap();
        assert_eq!(s.model_pane_creature("Pane"), Some(1011));
    }
}
