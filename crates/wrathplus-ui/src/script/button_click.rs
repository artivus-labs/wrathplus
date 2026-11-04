//! The Button click path: which transitions fire `OnClick` and the pressed look, and the
//! scripted or pointer click itself.

use mlua::{Lua, Value};

use super::*;

/// Whether `h` fires `OnClick` for the transition `name` (e.g. `"RightButtonDown"`): a Button
/// checks its `RegisterForClicks` set, ignoring case, where `AnyUp`/`AnyDown` (2.0+) cover every
/// button; any other kind fires on release only.
pub(crate) fn wants_click(model: &Model, h: FrameHandle, name: &str) -> bool {
    match model.arena.frame(h).map(|f| &f.kind_state) {
        Some(KindState::Button(bs)) => bs.registered_clicks.iter().any(|s| {
            s.eq_ignore_ascii_case(name)
                || (s.eq_ignore_ascii_case("AnyUp") && name.ends_with("Up"))
                || (s.eq_ignore_ascii_case("AnyDown") && name.ends_with("Down"))
        }),
        _ => name.ends_with("Up"),
    }
}

/// Whether `h` is registered for `button` (`"LeftButton"`, …) as Up or Down, the press-art gate.
/// `OnMouseDown` (`0x779210`) tests `m | m << 8` against `[this+0x330]` (Down in byte 0, Up in
/// byte 1, `0x77924b`) before the Down-only `OnClick` test (`0x77926b`), so any registration for
/// that button pushes the art, click or not.
pub(crate) fn wants_press_visual(model: &Model, h: FrameHandle, button: &str) -> bool {
    match model.arena.frame(h).map(|f| &f.kind_state) {
        // `AnyUp`/`AnyDown` (2.0+, `ActionButton.lua:90`) register every button.
        Some(KindState::Button(bs)) => bs.registered_clicks.iter().any(|s| {
            s.strip_suffix("Up")
                .or_else(|| s.strip_suffix("Down"))
                .is_some_and(|b| b.eq_ignore_ascii_case(button) || b.eq_ignore_ascii_case("Any"))
        }),
        _ => false,
    }
}

/// The click shared by the input path and `Click()`: a disabled Button fires nothing, a CheckButton
/// toggles before `OnClick` runs (`0x785550`), then `OnClick` gets the button name alone as `arg1`
/// (`0x779540` fires it with the format `"%s"`); `down`, true only for a click a `…ButtonDown`
/// registration fired, reaches the nameplate's select alone. `scripted` marks a Lua `Click()`
/// (`0x7826c0` passes 1, the mouse `0x779280`/`0x7793a4` pass 0): a LootButton then does nothing at
/// all, not even its `OnClick` (`0x4c182b`); every other kind ignores it.
pub(crate) fn click_button(lua: &Lua, id: u32, button: &str, down: bool, scripted: bool) {
    let mut take_loot = None;
    let fire = {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let Some(&h) = model.id_to_frame.get(&id) else {
            return;
        };
        let is_loot = model
            .arena
            .frame(h)
            .is_some_and(|f| f.kind == crate::widget::FrameKind::LootButton);
        if is_loot {
            if scripted {
                return;
            }
            // Deviation: the slot is read before the handler, where the reference reads it after
            // (`[esi+0x4dc]`), because reading first keeps the borrow simple and no stock handler
            // re-slots its row. Shift, ctrl or alt suppresses the take (`0x4c183a`/`0x4c1848`/
            // `0x4c1856`).
            let (shift, ctrl, alt) = model.modifiers;
            if !shift && !ctrl && !alt {
                take_loot = model.arena.frame(h).and_then(|f| match &f.kind_state {
                    KindState::Button(bs) => bs.loot_slot,
                    _ => None,
                });
            }
        }
        let Some(frame) = model.arena.frame_mut(h) else {
            return;
        };
        let is_check = frame.kind == FrameKind::CheckButton;
        match &mut frame.kind_state {
            // A Button fires only while enabled;
            KindState::Button(bs) => {
                if bs.enabled() {
                    if is_check {
                        bs.checked = !bs.checked;
                    }
                    true
                } else {
                    false
                }
            }
            // any other kind with an `OnClick` just fires.
            _ => true,
        }
    };
    if !fire {
        return;
    }
    let btn = match lua.create_string(button) {
        Ok(s) => Value::String(s),
        Err(_) => return,
    };
    // The click's button for its handlers, as the Button's click wrapper swaps it (`0x96fd70`).
    let outer = swap_clicked(lua, Some(button.to_string()));
    for script in ["PreClick", "OnClick", "PostClick"] {
        let args = vec![btn.clone(), Value::Boolean(down)];
        if let Err(e) = event::fire_widget_handler(lua, id, script, args) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .record_script_error(e.to_string());
        }
    }
    swap_clicked(lua, outer);
    // A nameplate click selects its unit, from the pointer or `Click()` alike (the plate's
    // override `0x7cb910` chains the base), after the handler so an erroring hook cannot eat it. A
    // press selects nothing: the plate registers Up clicks only (`0x7cb637`).
    if !down {
        crate::script::nameplate::note_click(lua, id, button);
    }
    // The take runs whatever the handler did (`0x4c1867`): a row whose `OnClick` errored still
    // loots.
    if let Some(slot) = take_loot {
        // The slot is 0-based; the take queue speaks the 1-based row.
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .loot_picks
            .push(slot + 1);
    }
}

/// Set `[[0xb499a8]+0x1234]`, the button of the click being dispatched, answering the old one:
/// what `GetMouseButtonClicked` (`0x50f950`), `IsModifiedClick` and the macro `[button]` option
/// read. The reference sets it around `OnMouseDown`/`OnMouseUp` (`0x48fc30`, `0x48fce0`) and a
/// Button's click and double-click (`0x96fd70`, `0x96fdd0`, inferred the double-click), and puts
/// the outer value back after, so it is nil outside those handlers.
pub(crate) fn swap_clicked(lua: &Lua, button: Option<String>) -> Option<String> {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    std::mem::replace(&mut model.last_mouse_button, button)
}

/// Fire `script` on frame `id` with the button name `btn` as its argument, `button` the clicked
/// button meanwhile ([`swap_clicked`]): the pointer's `OnMouseDown`, `OnMouseUp` and
/// `OnDoubleClick`.
pub(crate) fn fire_clicked(
    lua: &Lua,
    id: u32,
    script: &str,
    button: &str,
    btn: &Value,
) -> mlua::Result<()> {
    let outer = swap_clicked(lua, Some(button.to_string()));
    let fired = event::fire_widget_handler(lua, id, script, vec![btn.clone()]);
    swap_clicked(lua, outer);
    fired
}
