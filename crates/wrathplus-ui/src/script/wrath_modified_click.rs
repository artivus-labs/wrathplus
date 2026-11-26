//! Build 12340's modified clicks, binding set 3 of the key-binding system: `IsModifiedClick`
//! (`0x55fcc0` → `0x55f940`), `GetModifiedClick` (`0x55fc20`), `SetModifiedClick` (`0x55fb90`),
//! `GetModifiedClickAction` (`0x55ea70` → `0x55e870`) and `GetNumModifiedClickActions`
//! (`0x55dc60`). The actions and their default chords are `Bindings.xml`'s `<ModifiedClick>` rows;
//! a name finds its action case-folded (`0x55f4d0`). Evidence:
//! `/tmp/azb-app-mig/ghidra/shim{1,2}.log`.
//!
//! A chord (`0x55e180`) is a mask of the six modifier keys (`0x512d20` 0-5: left and right shift,
//! ctrl and alt) and an optional mouse button: modifier words (`0x55d860`), then `BUTTONn`, then
//! the end. `GetModifiedClick` prints it `ALT-CTRL-SHIFT-BUTTONn`, `NONE` when empty (`0x55e230`).
//!
//! `IsModifiedClick(arg)`: with no argument, 1 if any modifier key is down, else if any action's
//! chord holds. An argument led by modifier words needs each named pair held (`SHIFT` either key,
//! `LSHIFT` the left one) and what follows is not read; any other names an action, whose chord
//! holds (`0x55d6b0`) when one of its keys is down and its button, read only while a click
//! dispatches, is the click's.
//!
//! Each action keeps a chord per binding set (account, character, live) over its default;
//! `LoadBindings` and `SaveBindings` copy them with the keys (`0x562b80`), and a set's own chords
//! are saved with it as `modifiedclick <chord> <action>` lines (written at `0x55f32d`, read at
//! `0x5642ae`).

use std::collections::HashMap;

use mlua::{Lua, Value};

use super::binding_abi::{flag, ftol_arg, optional_string, string_arg};
use super::wrath_input_sides::button_name;
use super::{Model, UiScript};

/// A chord as `0x55e180` stores it: the six-key mask and the `BUTTONn` digit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Chord {
    mask: u8,
    button: Option<u8>,
}

/// The live set's index in [`State::sets`]; 0 and 1 are the account and character sets.
const LIVE: usize = 2;

#[derive(Default)]
pub(crate) struct State {
    /// `(action, default chord)` in registration order (`0x108`'s list).
    actions: Vec<(String, Chord)>,
    /// The chords the account, character and live sets hold of their own, by upper-cased action;
    /// an action absent from one takes its default (the slot's flag bit 0, cleared at `0x55e20a`).
    sets: [HashMap<String, Chord>; 3],
}

impl State {
    fn find(&self, action: &str) -> Option<usize> {
        self.actions
            .iter()
            .position(|(a, _)| a.eq_ignore_ascii_case(action))
    }

    /// Action `i`'s live chord.
    fn chord(&self, i: usize) -> Chord {
        let (name, default) = &self.actions[i];
        let own = self.sets[LIVE].get(&name.to_ascii_uppercase());
        own.copied().unwrap_or(*default)
    }

    /// `LoadBindings`' copy into the live set (`0x562b80(from, 3)`), `from` 0 the defaults.
    pub(super) fn load(&mut self, from: usize) {
        self.sets[LIVE] = match from {
            1 | 2 => self.sets[from - 1].clone(),
            _ => HashMap::new(),
        };
    }

    /// `SaveBindings(which)`' copy out of the live set (`0x562b80(3, which)`); saving the account
    /// set drops the character's, as the keys' save does.
    pub(super) fn save(&mut self, which: u32) {
        if let 1 | 2 = which {
            self.sets[which as usize - 1] = self.sets[LIVE].clone();
        }
        if which == 1 {
            self.sets[1].clear();
        }
    }
}

impl UiScript {
    /// Register `Bindings.xml`'s `<ModifiedClick>` rows; a repeated action keeps its place and
    /// takes the new default. Labelled: a default `0x55e180` refuses reads as no chord.
    pub fn register_modified_clicks(&self, rows: &[crate::bindings_xml::ModifiedClick]) {
        let mut model = self.model_mut();
        let state = &mut model.wrath_modified_clicks;
        for row in rows {
            let chord = parse(Some(&row.default)).unwrap_or_default();
            match state.find(&row.action) {
                Some(i) => state.actions[i].1 = chord,
                None => state.actions.push((row.action.clone(), chord)),
            }
        }
    }

    /// Seed set 1 or 2 with its bindings file's `modifiedclick` lines as `(chord, action)`; a chord
    /// `0x55e180` refuses is dropped, as the reader (`0x5642ae`) drops it.
    pub fn seed_modified_clicks(&mut self, set: u32, lines: &[(String, String)]) {
        let Some(slot) = (set as usize).checked_sub(1).filter(|&s| s < LIVE) else {
            return;
        };
        let own = lines
            .iter()
            .filter_map(|(chord, action)| Some((action.to_ascii_uppercase(), parse(Some(chord))?)));
        self.model_mut().wrath_modified_clicks.sets[slot] = own.collect();
    }

    /// Set 1's or 2's own chords as `(chord, action)` in action order, its bindings file's
    /// `modifiedclick %s %s` lines (`0x55f32d`); an action the set inherits has none.
    pub fn modified_click_lines(&self, set: u32) -> Vec<(String, String)> {
        let model = self.model_mut();
        let state = &model.wrath_modified_clicks;
        let Some(own) = (set as usize).checked_sub(1).filter(|&s| s < LIVE) else {
            return Vec::new();
        };
        let line = |(a, _): &(String, Chord)| {
            let chord = state.sets[own].get(&a.to_ascii_uppercase())?;
            Some((format(*chord), a.clone()))
        };
        state.actions.iter().filter_map(line).collect()
    }
}

/// `0x55d860`: the modifier words leading `s`, case-folded and each with an optional `-`, as key
/// bits, and what follows them.
fn modifier_prefix(mut s: &str) -> (u8, &str) {
    const WORDS: [(&str, u8); 9] = [
        ("LSHIFT", 0x01),
        ("RSHIFT", 0x02),
        ("SHIFT", 0x03),
        ("LCTRL", 0x04),
        ("RCTRL", 0x08),
        ("CTRL", 0x0c),
        ("LALT", 0x10),
        ("RALT", 0x20),
        ("ALT", 0x30),
    ];
    let mut mask = 0;
    let leads = |s: &str, w: &str| {
        s.len() >= w.len() && s.as_bytes()[..w.len()].eq_ignore_ascii_case(w.as_bytes())
    };
    while let Some(&(word, bits)) = WORDS.iter().find(|(w, _)| leads(s, w)) {
        mask |= bits;
        s = &s[word.len()..];
        s = s.strip_prefix('-').unwrap_or(s);
    }
    (mask, s)
}

/// `0x55f940`'s modifier test: each pair the mask names has one of its named keys down.
fn held(mask: u8, keys: u8) -> bool {
    (0..3).all(|pair| {
        let want = mask >> (2 * pair) & 3;
        want == 0 || keys >> (2 * pair) & want != 0
    })
}

/// `0x55e180`: nil, empty or `NONE` is the empty chord; else modifier words, then optionally
/// `BUTTON` and one character (`atoi` reads its digit, else 0), then the end. `None` refuses.
fn parse(binding: Option<&str>) -> Option<Chord> {
    let binding = binding.filter(|b| !b.is_empty() && !b.eq_ignore_ascii_case("NONE"));
    let Some(binding) = binding else {
        return Some(Chord::default());
    };
    let (mask, rest) = modifier_prefix(binding);
    let button = match rest.as_bytes() {
        [] => None,
        [head @ .., n] if head.eq_ignore_ascii_case(b"BUTTON") => {
            Some(if n.is_ascii_digit() { n - b'0' } else { 0 })
        }
        _ => return None,
    };
    Some(Chord { mask, button })
}

/// `0x55e230` with `0x55d990`: `ALT-`, `CTRL-`, `SHIFT-` (or one side's `LALT-`, `RCTRL-`, ...),
/// then `BUTTONn`; `NONE` for the empty chord.
fn format(chord: Chord) -> String {
    let mut s = String::new();
    for (shift, [both, left, right]) in [
        (4, ["ALT-", "LALT-", "RALT-"]),
        (2, ["CTRL-", "LCTRL-", "RCTRL-"]),
        (0, ["SHIFT-", "LSHIFT-", "RSHIFT-"]),
    ] {
        s.push_str(match chord.mask >> shift & 3 {
            3 => both,
            1 => left,
            2 => right,
            _ => "",
        });
    }
    match chord.button {
        Some(n) => s + &format!("BUTTON{n}"),
        None if s.is_empty() => "NONE".into(),
        None => {
            s.pop();
            s
        }
    }
}

/// `0x55d6b0`: an empty chord never holds; one of its keys must be down, and its button, while a
/// click dispatches, must be the click's, compared exactly.
fn chord_holds(chord: Chord, keys: u8, click: Option<&str>) -> bool {
    (chord.mask != 0 || chord.button.is_some())
        && (chord.mask == 0 || chord.mask & keys != 0)
        && match (chord.button, click) {
            (Some(n), Some(click)) => click == button_name(i64::from(n)),
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

/// `0x55f940`: `arg` is `IsModifiedClick`'s `lua_tolstring`, `None` for no string.
pub(super) fn is_modified_click(lua: &Lua, arg: Option<&str>) -> bool {
    let keys = super::wrath_input_sides::sides(lua);
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    let click = model.last_mouse_button.as_deref();
    let state = &model.wrath_modified_clicks;
    let holds = |i: usize| chord_holds(state.chord(i), keys, click);
    match arg {
        None => keys != 0 || (0..state.actions.len()).any(holds),
        Some(arg) => match modifier_prefix(arg).0 {
            0 => state.find(arg).is_some_and(holds),
            mask => held(mask, keys),
        },
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "IsModifiedClick",
        lua.create_function(|lua, arg: Value| {
            let arg = optional_string(lua, &arg);
            Ok(flag(is_modified_click(lua, arg.as_deref())))
        })?,
    )?;
    g.set(
        "GetModifiedClick",
        lua.create_function(|lua, action: Value| {
            let action = string_arg(lua, action, "Usage: GetModifiedClick(\"action\")")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_modified_clicks;
            Ok(state.find(&action).map(|i| format(state.chord(i))))
        })?,
    )?;
    g.set(
        "SetModifiedClick",
        lua.create_function(|lua, (action, binding): (Value, Value)| {
            let usage = "Usage: SetModifiedClick(\"action\", \"binding\")";
            let action = string_arg(lua, action, usage)?;
            let binding = optional_string(lua, &binding);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let state = &mut model.wrath_modified_clicks;
            match (state.find(&action), parse(binding.as_deref())) {
                (Some(i), Some(chord)) => {
                    let own = state.actions[i].0.to_ascii_uppercase();
                    state.sets[LIVE].insert(own, chord);
                    Ok(())
                }
                // `luaL_error`'s `%s` of a nil binding prints `(null)`.
                _ => Err(mlua::Error::runtime(format!(
                    "SetModifiedClick(): Unknown action ({action}) or binding ({})",
                    binding.as_deref().unwrap_or("(null)")
                ))),
            }
        })?,
    )?;
    g.set(
        "GetModifiedClickAction",
        lua.create_function(|lua, index: Value| {
            let index = ftol_arg(lua, index, "Usage: GetModifiedClickAction(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let actions = &model.wrath_modified_clicks.actions;
            let action = usize::try_from(index.wrapping_sub(1))
                .ok()
                .and_then(|i| actions.get(i));
            Ok(action.map(|(a, _)| a.clone()))
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
        // `0x55ea70` truncates its index (`0x88b9c0`); `0x55f4d0` finds an action case-folded.
        assert_eq!(
            s.eval::<String>("return GetModifiedClickAction(3.7)")
                .unwrap(),
            "CHATLINK"
        );
        assert_eq!(
            s.eval::<String>("return GetModifiedClick('chatlink')")
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
        // `0x55f940` reads the six keys: `SHIFT` is either side, `LSHIFT` the left alone, and a
        // string led by modifier words is decided by them; a number names an action.
        s.set_modifier_sides(crate::script::ModifierKeys {
            right_shift: true,
            ..Default::default()
        });
        let probe =
            "local t = {} for _, a in ipairs({'LSHIFT', 'RSHIFT', 'SHIFT', 'SHIFT-BUTTON1', \
                     'shiftfoo', 'chatlink', 1}) do t[#t + 1] = tostring(IsModifiedClick(a)) end \
                     return table.concat(t, ',')";
        assert_eq!(s.eval::<String>(probe).unwrap(), "nil,1,1,1,1,1,nil");
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
        // `0x55e180`: nil or `NONE` clears; only modifier words and one `BUTTONn` parse; the
        // chord prints in `0x55d990`'s order.
        let set = |binding: &str| {
            s.eval::<String>(&format!(
                "local ok, e = pcall(SetModifiedClick, 'SELFCAST', {binding}) \
                 return ok and GetModifiedClick('SELFCAST') or e"
            ))
            .unwrap()
        };
        assert_eq!(set("nil"), "NONE");
        assert_eq!(set("'shift-ctrl-button1'"), "CTRL-SHIFT-BUTTON1");
        assert_eq!(set("'lalt-rshift'"), "LALT-RSHIFT");
        assert_eq!(set("'BUTTON2'"), "BUTTON2");
        assert!(set("'A'").ends_with("Unknown action (SELFCAST) or binding (A)"));
        assert!(set("'SHIFT-BUTTON12'").contains("or binding (SHIFT-BUTTON12)"));
        assert!(s
            .eval::<String>("return select(2, pcall(SetModifiedClick, 'NOPE'))")
            .unwrap()
            .ends_with("Unknown action (NOPE) or binding ((null))"));
    }

    /// `0x562b80` copies the chords with the keys; a set's own chords are its file's lines, which
    /// seed it back.
    #[test]
    fn the_chords_are_saved_and_loaded_with_the_binding_sets() {
        let s = vm();
        let selfcast = |s: &UiScript| {
            s.eval::<String>("return GetModifiedClick('SELFCAST')")
                .unwrap()
        };
        s.run("SetModifiedClick('selfcast', 'CTRL') SaveBindings(2)")
            .unwrap();
        let own = vec![("CTRL".to_string(), "SELFCAST".to_string())];
        assert_eq!(s.modified_click_lines(2), own);
        assert!(
            s.modified_click_lines(1).is_empty(),
            "inherited, not its own"
        );
        s.run("LoadBindings(1)").unwrap();
        assert_eq!(selfcast(&s), "ALT");
        s.run("LoadBindings(2)").unwrap();
        assert_eq!(selfcast(&s), "CTRL");

        let mut s = vm();
        s.seed_binding_set(1, Some(Vec::new()));
        let lines = [("SHIFT-BUTTON2", "chatlink"), ("A", "SELFCAST")];
        s.seed_modified_clicks(1, &lines.map(|(c, a)| (c.to_string(), a.to_string())));
        s.load_binding_set(1);
        let chatlink = s.eval::<String>("return GetModifiedClick('CHATLINK')");
        assert_eq!(chatlink.unwrap(), "SHIFT-BUTTON2");
        assert_eq!(selfcast(&s), "ALT", "a refused chord is dropped");
    }
}
