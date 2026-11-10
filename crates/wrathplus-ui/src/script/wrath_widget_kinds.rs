//! Build-12340 methods of single frame kinds that the 1.12 tables lack: GameTooltip, ColorSelect,
//! ScrollingMessageFrame, Button and EditBox.

use mlua::{Lua, MultiValue, Table, Value};

use super::object::frame_handle_of;
use super::Model;
use crate::widget::KindState;

fn one(on: bool) -> Option<i64> {
    on.then_some(1)
}

/// Read `f` off the receiver's kind state; a frame of another kind raises `not a {what}`.
fn read_kind<T>(
    lua: &Lua,
    this: &Table,
    what: &str,
    f: impl FnOnce(&KindState) -> Option<T>,
) -> mlua::Result<T> {
    let h = frame_handle_of(lua, this)?;
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    model
        .arena
        .frame(h)
        .and_then(|fr| f(&fr.kind_state))
        .ok_or_else(|| mlua::Error::runtime(format!("not a {what}")))
}

/// `GameTooltip:GetUnit()` (12340 `0x61dad0`): nothing for a tooltip showing no unit (a zero guid,
/// `0x61db95`); else the unit's name and the first token naming its guid (`0x60b0b0`), nil when
/// none does. Labelled: `vehicle`, `arenaN`, `bossN` and `npc` are not searched, as
/// [`super::UnitGuids::event_tokens`] does not build them.
fn get_unit(lua: &Lua, this: Table) -> mlua::Result<MultiValue> {
    let guid = read_kind(lua, &this, "GameTooltip", |k| match k {
        KindState::Tooltip(t) => Some(t.unit_guid),
        _ => None,
    })?;
    let Some(guid) = guid.filter(|&g| g != 0) else {
        return Ok(MultiValue::new());
    };
    let (name, token) = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let unit = model
            .units_by_lower
            .values()
            .find(|u| u.guid == guid)
            .or_else(|| model.units_by_guid.get(&guid))
            .filter(|u| u.exists);
        let Some(unit) = unit else {
            return Ok(MultiValue::new());
        };
        let token = model.unit_guids.event_tokens(guid).into_iter().next();
        (unit.name.clone(), token)
    };
    let name = match name {
        Some(n) => Value::String(lua.create_string(&n)?),
        None => Value::String(super::unit::unknownobject(lua)?),
    };
    let token = match token {
        Some(t) => Value::String(lua.create_string(&t)?),
        None => Value::Nil,
    };
    Ok(MultiValue::from_vec(vec![name, token]))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let table = |key: &str| lua.named_registry_value::<Table>(key);

    table(super::tooltip::REG_TOOLTIP_METHODS)?.set("GetUnit", lua.create_function(get_unit)?)?;

    // ColorSelect SetColorHSV(h, s, v) (`0x970f50`): each argument `lua_tonumber`'d, stored raw at
    // `+0x2b0`-`+0x2b8`, then `OnColorSelect` fires with the RGB they give (`0x96e3d0`).
    // GetColorHSV() (`0x970fd0`) answers the three stored floats.
    let colors = table(super::colorselect::REG_COLORSELECT_METHODS)?;
    colors.set(
        "SetColorHSV",
        lua.create_function(|lua, (this, h, s, v): (Table, Value, Value, Value)| {
            let num = |v: Value| lua.coerce_number(v).map(|n| n.unwrap_or(0.0) as f32);
            let (h, s, v) = (num(h)?, num(s)?, num(v)?);
            let (r, g, b) = super::colorselect::with_colorselect(lua, &this, |c| {
                c.set_hsv(h, s, v);
                c.rgb_f64()
            })?;
            super::colorselect::fire_color_select(lua, &this, r, g, b)
        })?,
    )?;
    colors.set(
        "GetColorHSV",
        lua.create_function(|lua, this: Table| {
            let [h, s, v] = super::colorselect::with_colorselect(lua, &this, |c| c.hsv)?;
            Ok((f64::from(h), f64::from(s), f64::from(v)))
        })?,
    )?;

    // ScrollingMessageFrame: GetCurrentScroll() (`0x9736f0` → `0x9690c0`), lines scrolled back
    // from the newest; GetCurrentLine() (`0x973750`), the newest line's ring slot, -1 when empty;
    // GetHyperlinksEnabled() (`0x973ca0`), `+0x348` as 1 or nil.
    let smf = table(super::messageframe::REG_SCROLLINGMESSAGEFRAME_METHODS)?;
    let what = "ScrollingMessageFrame";
    smf.set(
        "GetCurrentScroll",
        lua.create_function(move |lua, this: Table| {
            read_kind(lua, &this, what, |k| match k {
                KindState::ScrollingMessage(s) => Some(s.scroll_offset as i64),
                _ => None,
            })
        })?,
    )?;
    smf.set(
        "GetCurrentLine",
        lua.create_function(move |lua, this: Table| {
            read_kind(lua, &this, what, |k| match k {
                KindState::ScrollingMessage(s) => Some(i64::from(s.head)),
                _ => None,
            })
        })?,
    )?;
    smf.set(
        "GetHyperlinksEnabled",
        lua.create_function(move |lua, this: Table| {
            read_kind(lua, &this, what, |k| match k {
                KindState::ScrollingMessage(s) => Some(one(s.hyperlinks_enabled)),
                _ => None,
            })
        })?,
    )?;

    // Button GetNormalFontObject / GetHighlightFontObject / GetDisabledFontObject (`0x977450`,
    // `0x977750`, `0x9775d0`): the state's font object, or nothing while none is set.
    let buttons = table(super::button::REG_BUTTON_METHODS)?;
    for (name, pick) in [
        ("GetNormalFontObject", 0u8),
        ("GetHighlightFontObject", 1),
        ("GetDisabledFontObject", 2),
    ] {
        buttons.set(
            name,
            lua.create_function(move |lua, this: Table| {
                let font = super::button::with_button(lua, &this, |b| match pick {
                    0 => b.normal_font.clone(),
                    1 => b.highlight_font.clone(),
                    _ => b.disabled_font.clone(),
                })?;
                match font {
                    Some(f) => Ok(MultiValue::from_vec(vec![Value::Table(
                        super::font::wrapper(lua, &f)?,
                    )])),
                    None => Ok(MultiValue::new()),
                }
            })?,
        )?;
    }

    // EditBox IsInIMECompositionMode() (`0x976940`): `+0x354`, which only an active IME
    // composition sets, and there is none here.
    table(super::editbox::REG_EDITBOX_METHODS)?.set(
        "IsInIMECompositionMode",
        lua.create_function(|lua, this: Table| frame_handle_of(lua, &this).map(|_| Value::Nil))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{UiScript, UnitGuids, UnitState};

    fn check(s: &UiScript, checks: &[&str]) {
        for c in checks {
            assert!(s.eval::<bool>(&format!("return {c}")).unwrap(), "{c}");
        }
    }

    /// `GetUnit` (`0x61dad0`): name and the guid's first token, name and nil for a unit no token
    /// names, nothing once the tooltip shows no unit.
    #[test]
    fn get_unit_answers_the_shown_units_name_and_token() {
        let mut s = UiScript::new().unwrap();
        let wolf = UnitState {
            exists: true,
            guid: 0x45,
            name: Some("Timber Wolf".into()),
            ..Default::default()
        };
        s.set_unit("target", Some(wolf));
        s.set_unit_guids(&UnitGuids {
            target: 0x45,
            ..Default::default()
        });
        s.run("TT = CreateFrame('GameTooltip', 'TT') TT:SetOwner(CreateFrame('Frame'), 'ANCHOR_NONE')")
            .unwrap();
        check(&s, &["select('#', TT:GetUnit()) == 0"]);
        s.run("TT:SetUnit('target')").unwrap();
        check(
            &s,
            &["select('#', TT:GetUnit()) == 2 and TT:GetUnit() == 'Timber Wolf'"],
        );
        check(&s, &["select(2, TT:GetUnit()) == 'target'"]);
        s.set_unit_guids(&UnitGuids::default());
        check(
            &s,
            &["select('#', TT:GetUnit()) == 2 and select(2, TT:GetUnit()) == nil"],
        );
        s.run("TT:ClearLines()").unwrap();
        check(&s, &["select('#', TT:GetUnit()) == 0"]);
    }

    /// `SetColorHSV` stores the floats raw and fires `OnColorSelect` with their RGB (`0x970f50`).
    #[test]
    fn color_hsv_round_trips_and_fires() {
        let s = UiScript::new().unwrap();
        s.run(
            r#"C = CreateFrame("ColorSelect") C:SetScript("OnColorSelect", function(_, r, g, b) RGB = { r, g, b } end)
            C:SetColorHSV(120, 1, 1)"#,
        )
        .unwrap();
        check(
            &s,
            &[
                "table.concat({ C:GetColorHSV() }, ' ') == '120 1 1'",
                "table.concat(RGB, ' ') == '0 1 0' and select(2, C:GetColorRGB()) == 1",
            ],
        );
    }

    /// The scrolling frame's head slot (`0x973750`): -1 empty, one step round the ring per line
    /// (`0x96aa5c`), kept by a back-fill (`0x96aa2f`), stepped back by a removal (`0x96a693`).
    #[test]
    fn scrolling_frame_scroll_line_and_link_queries() {
        let s = UiScript::new().unwrap();
        s.run("M = CreateFrame('ScrollingMessageFrame') M:SetMaxLines(3)")
            .unwrap();
        check(
            &s,
            &["M:GetCurrentLine() == -1 and M:GetCurrentScroll() == 0 and M:GetHyperlinksEnabled() == 1"],
        );
        s.run("for i = 1, 5 do M:AddMessage(i) end").unwrap();
        check(&s, &["M:GetCurrentLine() == 1"]);
        s.run("M:AddMessage('old', 1, 1, 1, 0, true) M:SetHyperlinksEnabled(false)")
            .unwrap();
        check(
            &s,
            &["M:GetCurrentLine() == 1 and M:GetHyperlinksEnabled() == nil"],
        );
        s.run("M:AddMessage('tagged', 1, 1, 1, 0, false, 7) M:RemoveMessagesByAccessID(7)")
            .unwrap();
        check(&s, &["M:GetCurrentLine() == 1"]);
        s.run("M:Clear()").unwrap();
        check(&s, &["M:GetCurrentLine() == -1"]);
    }

    /// The per-state font getters (`0x977450`, `0x977750`, `0x9775d0`): the object, or nothing.
    #[test]
    fn button_font_getters_answer_the_object_or_nothing() {
        let s = UiScript::new().unwrap();
        s.run(
            "B = CreateFrame('Button') F = CreateFont('WidgetKindsFont') B:SetNormalFontObject(F)",
        )
        .unwrap();
        check(
            &s,
            &[
                "B:GetNormalFontObject() == F and select('#', B:GetHighlightFontObject()) == 0",
                "select('#', B:GetDisabledFontObject()) == 0",
            ],
        );
    }
}
