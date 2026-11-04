//! Build 12340's binding verbs that 1.12 lacks: the flying-era Start/Stop names, `MoveAndSteer`,
//! the vehicle camera's zoom and `InteractUnit`, on the movement and camera paths of
//! [`super::input_verbs`] and the call queue's interact.

use mlua::{Lua, MultiValue, Value};

use super::input_verbs::{in_binding, push, push_call, zoom_amount};
use super::{BindingInput, FiredInput, HeldInput, Model, ScriptCall};

/// Register the build-12340 names.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // The 3.x flying-era names the stock `JUMP` and `SITORSTAND` bodies call. Each Start fires the
    // ground verb and raises the held key; the engine picks one by the mover's state: on foot the
    // jump or the sit, flying the ascend or descend (the jump `0x9883f0` refuses a flier). Each
    // Stop releases the key. The same gate as the movement table.
    let flying_era: [(&str, &[BindingInput]); 4] = [
        (
            "JumpOrAscendStart",
            &[
                BindingInput::Fire(FiredInput::Jump),
                BindingInput::Start(HeldInput::Ascend),
            ],
        ),
        ("AscendStop", &[BindingInput::Stop(HeldInput::Ascend)]),
        (
            "SitStandOrDescendStart",
            &[
                BindingInput::Fire(FiredInput::SitOrStand),
                BindingInput::Start(HeldInput::Descend),
            ],
        ),
        ("DescendStop", &[BindingInput::Stop(HeldInput::Descend)]),
    ];
    for (name, inputs) in flying_era {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                if in_binding(lua) {
                    for &input in inputs {
                        push(lua, input);
                    }
                }
                Ok(())
            })?,
        )?;
    }
    // `MoveAndSteerStart` (`0x5fc780`, table `0xad19d0`) is `CameraOrSelectOrMoveStart`'s leg
    // (`0x5fa170(2)`, the left button's bit) then `TurnOrActionStart`'s (`0x5fa170(1)`), behind
    // the same gate `0x5191c0(0)`; `MoveAndSteerStop` (`0x5fc830`) clears 2 (sticky 0) then 1. So
    // the key is the both-button run on the held pair's own channels. Not modelled: the
    // first-button memo each Start leg writes (`[InputControl+0x18]`, `0x5fc7a6`/`0x5fc7e6`).
    for (name, start) in [("MoveAndSteerStart", true), ("MoveAndSteerStop", false)] {
        let edge = if start {
            BindingInput::Start
        } else {
            BindingInput::Stop
        };
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                if in_binding(lua) {
                    push(lua, edge(HeldInput::CameraOrSelectOrMove));
                    push(lua, edge(HeldInput::TurnOrAction));
                }
                Ok(())
            })?,
        )?;
    }
    // `VehicleCameraZoomIn`/`Out` (`0x6018a0`/`0x6018b0`, table `0xad2100`) tail-call
    // `CameraZoomIn`/`Out` (`0x6017e0`/`0x601840`) on the same Lua state, so they are the camera
    // zoom with its argument.
    for (name, zoom_in) in [
        ("VehicleCameraZoomIn", true),
        ("VehicleCameraZoomOut", false),
    ] {
        let fire = if zoom_in {
            FiredInput::CameraZoomIn
        } else {
            FiredInput::CameraZoomOut
        };
        g.set(
            name,
            lua.create_function(move |lua, v: Option<Value>| {
                if let Some(yards) = zoom_amount(lua, v) {
                    push(lua, BindingInput::Fire(fire(yards)));
                }
                Ok(())
            })?,
        )?;
    }
    g.set("InteractUnit", lua.create_function(interact_unit)?)?;
    Ok(())
}

/// `InteractUnit(unitToken[, exactMatch])` (`0x527f00`, table `0xac8350`): a string (or number)
/// first argument, else the usage error. `"mouseover"` reads the mouseover global; another token
/// resolves through `0x520190`. Then `0x5277b0` finds the object and runs its interact (the
/// object's vtable `+0xb0`), answering 1, or nil with no object. WrathPlus answers 1 for a token
/// naming a unit and queues [`ScriptCall::InteractUnit`], whose applier runs the world's interact
/// on the object; nil for a token naming nobody.
///
/// Labelled: `0x520190`'s by-name fallback (`exactMatch`) is not built, as for `StartAttack`; a
/// guid the app has not streamed answers 1 here where `0x5277b0` answers nil; `0x5277b0`'s
/// no-player 1 and its `0x524bf0(guid)` before the call are not modelled; and the taint branch
/// (`[0xd4139c]`, `0x513530`'s action-blocked event) has no counterpart in this trusted UI.
fn interact_unit(lua: &Lua, args: MultiValue) -> mlua::Result<Value> {
    let token = match args.into_iter().next() {
        Some(v @ (Value::String(_) | Value::Integer(_) | Value::Number(_))) => {
            lua.coerce_string(v)?.map(|s| s.to_string_lossy())
        }
        _ => None,
    };
    let Some(token) = token else {
        return Err(mlua::Error::runtime(
            "Usage: InteractUnit(unitToken[, exactMatch])",
        ));
    };
    let guid = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        model.unit_guids.guid_of(&token).ok().flatten()
    };
    let Some(guid) = guid.filter(|&g| g != 0) else {
        return Ok(Value::Nil);
    };
    push_call(lua, ScriptCall::InteractUnit { guid });
    Ok(Value::Number(1.0))
}

#[cfg(test)]
#[path = "wrath_input_verbs_tests.rs"]
mod tests;
