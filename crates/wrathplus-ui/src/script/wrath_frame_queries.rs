//! Build 12340's frame and CVar registry queries: the keyboard focus, an event's frames, and a
//! CVar's record.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// `lua_isstring` (`0x84df60`): a string or a number.
fn is_string(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Integer(_) | Value::Number(_))
}

fn text(lua: &Lua, v: &Value) -> mlua::Result<String> {
    Ok(lua
        .coerce_string(v.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // `GetCurrentKeyBoardFocus()` (`0x81b820`): the focused edit box `[0xdce474]` as its frame
    // object, or nil. Inferred: that slot is the edit-box focus, as its only writers are
    // `0x965120`, `0x9651f0`, `0x9655f0` and `0x9670e0`, in the edit box's code.
    lua.globals().set(
        "GetCurrentKeyBoardFocus",
        lua.create_function(|lua, ()| {
            let id = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                model
                    .focused_editbox
                    .filter(|&h| model.arena.frame(h).is_some())
                    .map(|h| model.frame_id(h))
            };
            id.map(|id| super::object::frame_wrapper(lua, id))
                .transpose()
        })?,
    )?;

    // `GetFramesRegisteredForEvent("event")` (`0x81be70`): every frame on the event's listener
    // list (`0x81b510`'s record) as its frame object, one result each, in the list's order: the
    // order `SignalEvent` dispatches, registration order here, the `RegisterAllEvents` frames last
    // (the model's flag, `all_event_frames`). Inferred: an event no frame registered answers
    // nothing (an empty list).
    lua.globals().set(
        "GetFramesRegisteredForEvent",
        lua.create_function(|lua, event: Value| {
            if !is_string(&event) {
                return Err(mlua::Error::runtime(
                    "Usage: GetFramesRegisteredForEvent(\"event\")",
                ));
            }
            let event = text(lua, &event)?;
            let ids: Vec<u32> = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let mut handles = model
                    .event_to_frames
                    .get(&event)
                    .cloned()
                    .unwrap_or_default();
                for h in model.all_event_frames.clone() {
                    if !handles.contains(&h) {
                        handles.push(h);
                    }
                }
                handles.retain(|&h| model.arena.frame(h).is_some());
                handles.into_iter().map(|h| model.frame_id(h)).collect()
            };
            ids.into_iter()
                .map(|id| super::object::frame_wrapper(lua, id).map(Value::Table))
                .collect::<mlua::Result<MultiValue>>()
        })?,
    )?;

    // `GetCVarInfo("cvar")` (`0x50ff50`): four results, the value (`[rec+0x28]`), the default
    // (`[rec+0x40]`), then 1 or nil for flag bits 4 and 5 (`[rec+0x1c]`, [`super::wrath_cvar_flags`]);
    // four nils for a name no row answers to (`0x767460`) or a row flagged hidden (bit 6).
    lua.globals().set(
        "GetCVarInfo",
        lua.create_function(|lua, name: Value| {
            if !is_string(&name) {
                return Err(mlua::Error::runtime("Usage: GetCVarInfo(\"cvar\")"));
            }
            let name = text(lua, &name)?.to_ascii_lowercase();
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = vec![Value::Nil; 4];
            let flags = super::wrath_cvar_flags::flags(&name);
            if let Some(slot) = model.cvars.get(&name).filter(|_| flags & 0x40 == 0) {
                out[0] = Value::String(lua.create_string(&slot.value)?);
                out[1] = Value::String(lua.create_string(&slot.default)?);
                out[2] = super::binding_abi::flag(flags & 0x10 != 0);
                out[3] = super::binding_abi::flag(flags & 0x20 != 0);
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn the_keyboard_focus_is_the_focused_edit_box_or_nil() {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(1024.0, 768.0);
        s.run("assert(GetCurrentKeyBoardFocus() == nil) E = CreateFrame('EditBox', 'ProbeBox')")
            .unwrap();
        s.run("E:SetFocus() assert(GetCurrentKeyBoardFocus() == E) E:ClearFocus()")
            .unwrap();
        s.run("assert(GetCurrentKeyBoardFocus() == nil)").unwrap();
    }

    #[test]
    fn an_events_frames_come_back_in_registration_order() {
        let s = UiScript::new().unwrap();
        s.run(
            "A = CreateFrame('Frame') B = CreateFrame('Frame') C = CreateFrame('Frame') \
             B:RegisterEvent('PROBE_EVENT') A:RegisterEvent('PROBE_EVENT') \
             C:RegisterAllEvents()",
        )
        .unwrap();
        s.run(
            "local x, y, z, w = GetFramesRegisteredForEvent('PROBE_EVENT') \
             assert(x == B and y == A and z == C and w == nil) \
             assert(select('#', GetFramesRegisteredForEvent('PROBE_EVENT')) == 3) \
             A:UnregisterEvent('PROBE_EVENT') C:UnregisterAllEvents() \
             assert(GetFramesRegisteredForEvent('PROBE_EVENT') == B) \
             assert(select('#', GetFramesRegisteredForEvent('NO_SUCH_EVENT')) == 0)",
        )
        .unwrap();
        let e = s.run("GetFramesRegisteredForEvent()").unwrap_err();
        assert!(e.to_string().contains("Usage: GetFramesRegisteredForEvent"));
    }

    #[test]
    fn a_cvar_answers_its_value_default_and_two_flags() {
        let s = UiScript::new().unwrap();
        s.register_cvars([
            ("ProbeVar", "7"),
            ("autoLootDefault", "0"),
            ("autoSelfCast", "1"),
        ]);
        s.register_cvars([("accountName", "")]);
        s.run(
            "SetCVar('ProbeVar', '9') \
             local v, d, a, c = GetCVarInfo('probevar') \
             assert(v == '9' and d == '7' and a == nil and c == nil) \
             assert(select('#', GetCVarInfo('ProbeVar')) == 4) \
             local _, _, a1, c1 = GetCVarInfo('AUTOLOOTDEFAULT') assert(a1 == 1 and c1 == nil) \
             local _, _, a2, c2 = GetCVarInfo('autoSelfCast') assert(a2 == nil and c2 == 1) \
             assert(GetCVarInfo('accountName') == nil) \
             assert(select('#', GetCVarInfo('NoSuchVar')) == 4 and GetCVarInfo('NoSuchVar') == nil)",
        )
        .unwrap();
        let e = s.run("GetCVarInfo()").unwrap_err();
        assert!(e.to_string().contains("Usage: GetCVarInfo"));
    }
}
