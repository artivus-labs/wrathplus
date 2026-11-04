//! Build 12340's modified clicks, binding set 3 of the key-binding system: `IsModifiedClick`
//! (`0x55fcc0` → `0x55f940`), `GetModifiedClick` (`0x55fc20`), `SetModifiedClick` (`0x55fb90`),
//! `GetModifiedClickAction` (`0x55ea70` → `0x55e870`) and `GetNumModifiedClickActions`
//! (`0x55dc60`). The actions and their default chords are `Bindings.xml`'s `<ModifiedClick>` rows.
//! Evidence: `/tmp/azb-app-mig/ghidra/shim{1,2}.log`.
//!
//! `IsModifiedClick(arg)`: with no argument, 1 if any modifier key is down, else if any action's
//! chord matches. An argument that parses as modifiers alone (`"SHIFT"`, `"CTRL-ALT"`) needs those
//! held, others free. Anything else is an action, and its chord must match.
//!
//! Labelled: the chord matcher (`0x55d6b0`) is not decompiled; a chord matches when its modifiers
//! are exactly the ones held and its `BUTTONn` part, if any, is the last mouse button pressed.
//! The client tells left from right modifiers (`0x512d20` 0-5); the app feeds one bit per pair,
//! so `LSHIFT` and `RSHIFT` read as `SHIFT`. A `SetModifiedClick` is not saved with the bindings.

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};

#[derive(Default)]
pub(crate) struct State {
    /// `(action, chord)` in registration order (`0x108`'s list).
    actions: Vec<(String, String)>,
}

impl UiScript {
    /// Register `Bindings.xml`'s `<ModifiedClick>` rows; a repeated action keeps its place and
    /// takes the new default.
    pub fn register_modified_clicks(&self, rows: &[crate::bindings_xml::ModifiedClick]) {
        let mut model = self.model_mut();
        let actions = &mut model.wrath_modified_clicks.actions;
        for row in rows {
            let chord = row.default.to_ascii_uppercase();
            match actions.iter_mut().find(|(a, _)| a == &row.action) {
                Some(slot) => slot.1 = chord,
                None => actions.push((row.action.clone(), chord)),
            }
        }
    }
}

/// The `(shift, ctrl, alt)` a modifier-only string names, or `None` when a part is not one.
fn modifier_mask(s: &str) -> Option<(bool, bool, bool)> {
    let mut mask = (false, false, false);
    for part in s.split('-') {
        match part.to_ascii_uppercase().as_str() {
            "SHIFT" | "LSHIFT" | "RSHIFT" => mask.0 = true,
            "CTRL" | "LCTRL" | "RCTRL" => mask.1 = true,
            "ALT" | "LALT" | "RALT" => mask.2 = true,
            _ => return None,
        }
    }
    Some(mask)
}

/// `LeftButton` → `BUTTON1`, as the binding names spell mouse buttons.
fn button_key(button: &str) -> Option<&'static str> {
    Some(match button {
        "LeftButton" => "BUTTON1",
        "RightButton" => "BUTTON2",
        "MiddleButton" => "BUTTON3",
        "Button4" => "BUTTON4",
        "Button5" => "BUTTON5",
        _ => return None,
    })
}

fn chord_matches(chord: &str, held: (bool, bool, bool), last_button: Option<&str>) -> bool {
    if chord == "NONE" || chord.is_empty() {
        return false;
    }
    let mut want = (false, false, false);
    let mut key = None;
    for part in chord.split('-') {
        match modifier_mask(part) {
            Some(m) => {
                want.0 |= m.0;
                want.1 |= m.1;
                want.2 |= m.2;
            }
            None => key = Some(part),
        }
    }
    // 12340's `0x55d6b0`: any one of the chord's modifiers held passes, and its button is checked
    // only while a click dispatches (the click's own button, `[0xb499a8+0x1234]`), so a hovered
    // `CTRL-BUTTON1` matches on Ctrl alone.
    let any = (want.0 && held.0) || (want.1 && held.1) || (want.2 && held.2);
    (any || (want == (false, false, false) && key.is_some()))
        && match (key, last_button) {
            (Some(k), Some(b)) => button_key(b) == Some(k),
            _ => true,
        }
}

impl super::UiScript {
    /// A mouse button press or release at `(x, y)`; `button` is the WoW name (`"LeftButton"`, …).
    /// The button is the click's own inside its `OnMouseDown`/`OnMouseUp`, `OnClick` and
    /// `OnDoubleClick` handlers, what `IsModifiedClick` and the macro `[button]` option read
    /// ([`super::button::swap_clicked`]).
    pub fn mouse_button(&mut self, x: f32, y: f32, button: &str, down: bool) -> bool {
        self.mouse_dispatch(x, y, button, down)
    }
}

pub(super) fn is_modified_click(model: &Model, arg: Option<&str>) -> bool {
    let held = model.modifiers;
    let last = model.last_mouse_button.as_deref();
    let actions = &model.wrath_modified_clicks.actions;
    match arg {
        None => {
            held.0
                || held.1
                || held.2
                || actions
                    .iter()
                    .any(|(_, chord)| chord_matches(chord, held, last))
        }
        Some(arg) => match modifier_mask(arg) {
            Some(want) => (!want.0 || held.0) && (!want.1 || held.1) && (!want.2 || held.2),
            None => actions
                .iter()
                .find(|(a, _)| a == arg)
                .is_some_and(|(_, chord)| chord_matches(chord, held, last)),
        },
    }
}

fn string_arg(args: &MultiValue, i: usize) -> Option<String> {
    match args.get(i)? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "IsModifiedClick",
        lua.create_function(|lua, args: MultiValue| {
            let arg = string_arg(&args, 0);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(is_modified_click(&model, arg.as_deref()).then_some(1.0))
        })?,
    )?;
    g.set(
        "GetModifiedClick",
        lua.create_function(|lua, args: MultiValue| {
            let action = string_arg(&args, 0)
                .ok_or_else(|| mlua::Error::runtime("Usage: GetModifiedClick(\"action\")"))?;
            let chord = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_modified_clicks
                .actions
                .iter()
                .find(|(a, _)| *a == action)
                .map(|(_, c)| c.clone());
            Ok(chord)
        })?,
    )?;
    g.set(
        "SetModifiedClick",
        lua.create_function(|lua, args: MultiValue| {
            let action = string_arg(&args, 0).ok_or_else(|| {
                mlua::Error::runtime("Usage: SetModifiedClick(\"action\", \"binding\")")
            })?;
            let binding = string_arg(&args, 1);
            let chord = binding.as_deref().and_then(|b| {
                if b.eq_ignore_ascii_case("NONE") || modifier_mask(b).is_some() {
                    Some(b.to_ascii_uppercase())
                } else {
                    super::keybind::normalize_binding_key(b)
                }
            });
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let slot = model
                .wrath_modified_clicks
                .actions
                .iter_mut()
                .find(|(a, _)| *a == action);
            match (slot, chord) {
                (Some(slot), Some(chord)) => {
                    slot.1 = chord;
                    Ok(())
                }
                _ => Err(mlua::Error::runtime(format!(
                    "SetModifiedClick(): Unknown action ({action}) or binding ({})",
                    binding.unwrap_or_default()
                ))),
            }
        })?,
    )?;
    g.set(
        "GetModifiedClickAction",
        lua.create_function(|lua, args: MultiValue| {
            let index = args
                .front()
                .and_then(|v| lua.coerce_integer(v.clone()).ok().flatten())
                .ok_or_else(|| mlua::Error::runtime("Usage: GetModifiedClickAction(index)"))?;
            let action = usize::try_from(index - 1).ok().and_then(|i| {
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .wrath_modified_clicks
                    .actions
                    .get(i)
                    .map(|(a, _)| a.clone())
            });
            Ok(action)
        })?,
    )?;
    g.set(
        "GetNumModifiedClickActions",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_modified_clicks
                .actions
                .len())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings_xml::{parse_modified_clicks, ModifiedClick};

    fn vm() -> UiScript {
        let s = UiScript::new().unwrap();
        s.register_modified_clicks(
            &parse_modified_clicks(
                r#"<Bindings>
    <ModifiedClick action="SELFCAST" default="ALT"/>
    <ModifiedClick action="FOCUSCAST" default="NONE"/>
    <ModifiedClick action="CHATLINK" default="SHIFT-BUTTON1"/>
    <ModifiedClick action="SOCKETITEM" default="SHIFT-BUTTON2"/>
</Bindings>"#,
            )
            .unwrap(),
        );
        s
    }

    #[test]
    fn the_rows_parse_in_document_order() {
        assert_eq!(
            parse_modified_clicks(r#"<Bindings><ModifiedClick action="SPLITSTACK"/></Bindings>"#)
                .unwrap(),
            [ModifiedClick {
                action: "SPLITSTACK".into(),
                default: "NONE".into(),
            }]
        );
        let s = vm();
        assert_eq!(
            s.eval::<i64>("return GetNumModifiedClickActions()")
                .unwrap(),
            4
        );
        assert_eq!(
            s.eval::<String>("return GetModifiedClickAction(3)")
                .unwrap(),
            "CHATLINK"
        );
        assert_eq!(
            s.eval::<String>("return GetModifiedClick('CHATLINK')")
                .unwrap(),
            "SHIFT-BUTTON1"
        );
    }

    #[test]
    fn an_action_matches_its_chord_and_its_button() {
        let mut s = vm();
        // 12340's `0x55d6b0`: outside a click the button part is not read, so a held SHIFT
        // matches both SHIFT-BUTTON1 and SHIFT-BUTTON2.
        s.set_modifiers(true, false, false);
        assert!(s
            .eval::<bool>(
                "return IsModifiedClick('SOCKETITEM') == 1 and IsModifiedClick('CHATLINK') == 1 \
                 and IsModifiedClick('SHIFT') == 1 and IsModifiedClick() == 1"
            )
            .unwrap());
        // Inside a right click the button decides.
        s.run(
            "local b = CreateFrame('Button', 'ChordProbe', UIParent) b:SetWidth(40) b:SetHeight(40) \
             b:SetPoint('CENTER') b:RegisterForClicks('AnyUp') b:SetScript('OnClick', function() \
             SEEN = tostring(IsModifiedClick('SOCKETITEM')) .. tostring(IsModifiedClick('CHATLINK')) end)",
        )
        .unwrap();
        s.resolve();
        let (x, y) = s
            .eval::<(f32, f32)>("return ChordProbe:GetCenter()")
            .unwrap();
        s.mouse_button(x, y, "RightButton", true);
        s.mouse_button(x, y, "RightButton", false);
        assert_eq!(s.eval::<String>("return SEEN").unwrap(), "1nil");
        // Any one of the chord's modifiers held passes; an extra one does not reject.
        s.set_modifiers(true, true, false);
        assert!(s
            .eval::<bool>("return IsModifiedClick('SOCKETITEM') == 1")
            .unwrap());
        s.set_modifiers(false, false, false);
        assert!(s
            .eval::<bool>("return IsModifiedClick('FOCUSCAST') == nil")
            .unwrap());
        assert!(s.eval::<bool>("return IsModifiedClick() == nil").unwrap());
    }

    #[test]
    fn set_modified_click_rebinds_a_known_action_only() {
        let mut s = vm();
        s.run("SetModifiedClick('SELFCAST', 'ctrl')").unwrap();
        assert_eq!(
            s.eval::<String>("return GetModifiedClick('SELFCAST')")
                .unwrap(),
            "CTRL"
        );
        s.set_modifiers(false, true, false);
        assert!(s
            .eval::<bool>("return IsModifiedClick('SELFCAST') == 1")
            .unwrap());
        assert!(s.run("SetModifiedClick('NOPE', 'ALT')").is_err());
        assert!(s.run("SetModifiedClick('SELFCAST', 'NOTAKEY')").is_err());
        assert!(s.run("GetModifiedClick()").is_err());
    }
}
