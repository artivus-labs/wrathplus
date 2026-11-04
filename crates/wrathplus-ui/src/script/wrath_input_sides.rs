//! Build 12340's per-side modifier queries and the mouse-button names.
//!
//! The six modifier keys are bits of `[[0xbeadd8]+0x130]`: 0 left shift, 1 right shift, 2 left
//! ctrl, 3 right ctrl, 4 left alt, 5 right alt; `IsShiftKeyDown` (`0x514580`) and
//! `IsAltKeyDown` (`0x514820`) test the two bits of their pair. The app feeds the sides
//! ([`super::UiScript::set_modifier_sides`]); a feed of the pairs alone
//! ([`super::UiScript::set_modifiers`]) keeps a held side and reads a newly held pair as its
//! left key. Not modelled: the OS key-state fallback (`0x47d230`) the reference takes when
//! `[[0xbeadd8]+300]` is 0.

use mlua::{Lua, Value};

use super::Model;

/// The six modifier keys, each held or not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierKeys {
    pub left_shift: bool,
    pub right_shift: bool,
    pub left_ctrl: bool,
    pub right_ctrl: bool,
    pub left_alt: bool,
    pub right_alt: bool,
}

impl ModifierKeys {
    /// The reference's bit word.
    fn bits(self) -> u8 {
        [
            self.left_shift,
            self.right_shift,
            self.left_ctrl,
            self.right_ctrl,
            self.left_alt,
            self.right_alt,
        ]
        .iter()
        .enumerate()
        .fold(0, |w, (i, &b)| w | (u8::from(b) << i))
    }
}

/// The side bits, as the last feed left them.
#[derive(Default)]
struct Sides(u8);

/// Fold a pairs-only feed into the side bits: a released pair clears both sides, a held pair
/// with neither side set reads as its left key.
pub(super) fn fold(lua: &Lua, (shift, ctrl, alt): (bool, bool, bool)) {
    let Some(mut sides) = lua.app_data_mut::<Sides>() else {
        return;
    };
    for (pair, held) in [shift, ctrl, alt].into_iter().enumerate() {
        let mask = 0b11 << (2 * pair);
        if !held {
            sides.0 &= !mask;
        } else if sides.0 & mask == 0 {
            sides.0 |= 1 << (2 * pair);
        }
    }
}

impl super::UiScript {
    /// Push the six modifier keys: the sides behind `IsLeftShiftKeyDown` and its kin, and the
    /// pairs behind `IsShiftKeyDown`/`IsControlKeyDown`/`IsAltKeyDown`. Call it where
    /// [`Self::set_modifiers`] would be called, before the frame's mouse events.
    pub fn set_modifier_sides(&mut self, keys: ModifierKeys) {
        self.model_mut().modifiers = (
            keys.left_shift || keys.right_shift,
            keys.left_ctrl || keys.right_ctrl,
            keys.left_alt || keys.right_alt,
        );
        if let Some(mut sides) = self.lua.app_data_mut::<Sides>() {
            sides.0 = keys.bits();
        }
    }
}

/// `GetMouseButtonName(n)`'s table: the index to the button's bit (`0x5eec20`: 1 left, 2 right,
/// 3 middle, then 4..=31 one bit each), the bit to its name (`0x96f530`), `"UNKNOWN"` for an
/// index outside 1..=31 (bit 0).
fn button_name(n: i64) -> String {
    match n {
        1 => "LeftButton".into(),
        2 => "RightButton".into(),
        3 => "MiddleButton".into(),
        4..=31 => format!("Button{n}"),
        _ => "UNKNOWN".into(),
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(Sides::default());
    // `IsLeftShiftKeyDown` (`0x5144c0`), `IsRightShiftKeyDown` (`0x514520`),
    // `IsLeftControlKeyDown` (`0x514610`), `IsRightControlKeyDown` (`0x514670`),
    // `IsLeftAltKeyDown` (`0x514760`), `IsRightAltKeyDown` (`0x5147c0`): the side's bit, 1 or nil.
    for (name, bit) in [
        ("IsLeftShiftKeyDown", 0),
        ("IsRightShiftKeyDown", 1),
        ("IsLeftControlKeyDown", 2),
        ("IsRightControlKeyDown", 3),
        ("IsLeftAltKeyDown", 4),
        ("IsRightAltKeyDown", 5),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, ()| {
                let sides = lua.app_data_ref::<Sides>().map_or(0, |s| s.0);
                Ok(super::binding_abi::flag(sides >> bit & 1 != 0))
            })?,
        )?;
    }

    // `GetMouseButtonName(n)` (`0x514ba0`): a number (or numeric string) truncated to an index and
    // named through [`button_name`]; any other string answers itself; anything else nil.
    lua.globals().set(
        "GetMouseButtonName",
        lua.create_function(|lua, v: Value| {
            if let Some(n) = lua.coerce_number(v.clone())? {
                return Ok(Value::String(
                    lua.create_string(button_name(n.trunc() as i64))?,
                ));
            }
            Ok(match v {
                Value::String(s) => Value::String(s),
                _ => Value::Nil,
            })
        })?,
    )?;

    // `GetMouseButtonClicked()` (`0x50f950`): `[[0xb499a8]+0x1234]`, the button of the click being
    // handled, nil outside one ([`super::button::swap_clicked`] says where it is set).
    lua.globals().set(
        "GetMouseButtonClicked",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            model
                .last_mouse_button
                .as_deref()
                .map(|b| lua.create_string(b))
                .transpose()
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ModifierKeys;
    use crate::script::UiScript;

    const PROBE: &str = "local t = {} for _, f in ipairs({IsLeftShiftKeyDown, \
        IsRightShiftKeyDown, IsLeftControlKeyDown, IsRightControlKeyDown, IsLeftAltKeyDown, \
        IsRightAltKeyDown, IsShiftKeyDown, IsControlKeyDown, IsAltKeyDown}) do \
        t[#t + 1] = tostring(f()) end return table.concat(t, ',')";

    #[test]
    fn each_side_answers_its_own_bit_and_the_pairs_either() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.eval::<String>(PROBE).unwrap(),
            "nil,nil,nil,nil,nil,nil,nil,nil,nil"
        );
        s.set_modifier_sides(ModifierKeys {
            right_shift: true,
            left_ctrl: true,
            right_alt: true,
            ..Default::default()
        });
        assert_eq!(s.eval::<String>(PROBE).unwrap(), "nil,1,1,nil,nil,1,1,1,1");
        // A pairs-only feed keeps a still-held side and reads a new pair as its left key.
        s.set_modifiers(true, false, true);
        assert_eq!(
            s.eval::<String>(PROBE).unwrap(),
            "nil,1,nil,nil,nil,1,1,nil,1"
        );
        s.set_modifiers(false, true, false);
        assert_eq!(
            s.eval::<String>(PROBE).unwrap(),
            "nil,nil,1,nil,nil,nil,nil,1,nil"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', IsRightAltKeyDown())")
                .unwrap(),
            1
        );
    }

    #[test]
    fn mouse_button_names_follow_the_index_table() {
        let s = UiScript::new().unwrap();
        let names = s
            .eval::<String>(
                "local t = {} for _, v in ipairs({1, 2, 3, 4, 31, 0, 32, -1, 2.9, '3', \
                 'Wheel'}) do t[#t + 1] = GetMouseButtonName(v) end \
                 return table.concat(t, ',') .. ',' .. tostring(GetMouseButtonName({}))",
            )
            .unwrap();
        assert_eq!(
            names,
            "LeftButton,RightButton,MiddleButton,Button4,Button31,UNKNOWN,UNKNOWN,UNKNOWN,\
             RightButton,MiddleButton,Wheel,nil"
        );
    }

    #[test]
    fn the_clicked_button_is_answered_inside_a_click_only() {
        let s = UiScript::new().unwrap();
        s.run(
            "B = CreateFrame('Button', 'ProbeClick') B:RegisterForClicks('AnyUp') \
             B:SetScript('OnClick', function() SEEN = GetMouseButtonClicked() end) \
             B:Click('RightButton')",
        )
        .unwrap();
        assert_eq!(s.eval::<String>("return SEEN").unwrap(), "RightButton");
        assert!(s
            .eval::<bool>("return GetMouseButtonClicked() == nil")
            .unwrap());
    }

    /// A pointer press answers its button in `OnMouseDown` and `OnClick`, nil in a handler the
    /// reference does not wrap (`OnDragStart`) and after the dispatch.
    #[test]
    fn the_pointers_button_is_answered_in_the_wrapped_handlers_only() {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(1024.0, 768.0);
        s.run(
            "SEEN = {} local b = CreateFrame('Button', 'MouseProbe', UIParent) b:SetWidth(40) \
             b:SetHeight(40) b:SetPoint('CENTER') b:RegisterForClicks('AnyDown') \
             b:RegisterForDrag('RightButton') \
             local function note(tag) return function() \
               SEEN[#SEEN + 1] = tag .. '=' .. tostring(GetMouseButtonClicked()) end end \
             b:SetScript('OnMouseDown', note('down')) b:SetScript('OnClick', note('click')) \
             b:SetScript('OnDragStart', note('drag'))",
        )
        .unwrap();
        s.resolve();
        let (x, y) = s
            .eval::<(f32, f32)>("return MouseProbe:GetCenter()")
            .unwrap();
        s.mouse_button(x, y, "RightButton", true);
        let seen = s.eval::<String>("return table.concat(SEEN, ',')").unwrap();
        assert!(
            seen.contains("down=RightButton") && seen.contains("click=RightButton"),
            "{seen}"
        );
        assert!(!seen.contains("drag=RightButton"), "{seen}");
        s.mouse_button(x, y, "RightButton", false);
        assert!(s
            .eval::<bool>("return GetMouseButtonClicked() == nil")
            .unwrap());
    }
}
