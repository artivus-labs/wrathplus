//! Build-12340 chat-window bindings FloatingChatFrame.lua calls that the 1.12 surface lacks, and
//! the Wrath shape of `GetChatWindowInfo`.
//!
//! Wow.exe 12340 evidence (`/tmp/azb-ui-fcf-native.log`, `/tmp/azb-ui-fcf-helpers.log`): the
//! window records sit at `0xbcf0b0`, stride 0x100, ten slots: name `+0x00`, message-group flags
//! `+0x80`, font size `+0xd0`, colour bytes `+0xd4..+0xd7` (B, G, R, A), locked `+0xd8`, docked
//! `+0xdc`, shown `+0xe0`, uninteractable `+0xe4`. `GetChatWindowInfo` `0x4fbd90`,
//! `SetChatWindowUninteractable` `0x4fc6e0`, `ResetChatWindows` `0x501f00`,
//! `SetChatColorNameByClass` `0x4fedc0`, `JoinPermanentChannel` `0x50dac0` (the shared join
//! `0x50d900` with its permanent flag set), `BNLeaveConversation` `0x536030`.
//!
//! The registries are 12340's: 62 chat types (`chat_types`) and 46 message groups
//! (`chat_window::MESSAGE_GROUPS`). Per record ResetChatWindows frees the channels (`0x5019d0`)
//! and writes the default record (`0x501800`: name "", size 0, locked, docked 0, hidden, colour
//! 0, 0, 0 with alpha `0x28`). The host persists `uninteractable` and colorNameByClass in
//! `chat-cache.txt`.

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, is_number};
use super::chat_types;
use super::chat_window::{
    checked_window, edit_look, window_index, ChatWindowLook, ENGINE_CHAT_WINDOW_SLOTS,
};
use super::{Model, ScriptValue};

/// `byte · (1/255)` in f32 (`[0xa45564]`), as the colour getters push it.
fn unit(b: u8) -> f64 {
    f64::from(f32::from(b) * (1.0f32 / 255.0f32))
}

/// The ten windows' saved geometry (records `0xbcf0b0`, stride 0x100): width and height
/// `+0xf8/+0xfc`, the anchor point, x and y `+0xec/+0xf0`, and the saved-position flag `+0xf4`.
#[derive(Clone, Default)]
struct SavedWindow {
    dimensions: Option<(f32, f32)>,
    position: Option<(String, f32, f32)>,
}

#[derive(Default)]
struct SavedGeometry([SavedWindow; ENGINE_CHAT_WINDOW_SLOTS]);

impl super::UiScript {
    /// Seeds the `ChatChannels.dbc` auto-join rows (`flags & 1`) as `(Shortcut, id)`, which
    /// `ResetChatWindows` registers in window 1. No event and no window change.
    pub fn set_chat_channel_seed(&mut self, rows: Vec<(String, u32)>) {
        self.model_mut().chat_channel_seed = rows;
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    lua.set_app_data(SavedGeometry::default());

    // GetChatWindowSavedDimensions(index) (`0x4fc020`) → width, height for a window 1..10;
    // nothing outside. GetChatWindowSavedPosition(index) (`0x4fbf60`) → point, x, y once saved,
    // else nothing. A non-number answers nothing. Deviation, labelled: the native record always
    // holds dimensions, its defaults written by ResetChatWindows (`0x501f00`, unread for them);
    // an unsaved window answers nothing here, as sizing to 0x0 would. The setters' own checks
    // and the chat-cache persistence are unread; the setters store.
    g.set(
        "GetChatWindowSavedDimensions",
        lua.create_function(|lua, id: Value| {
            let Some(slot) = window_index(lua, Some(&id)) else {
                return Ok(MultiValue::new());
            };
            let saved = lua.app_data_ref::<SavedGeometry>().expect("geometry");
            let Some((w, h)) = saved.0[slot].dimensions else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_vec(vec![
                Value::Number(f64::from(w)),
                Value::Number(f64::from(h)),
            ]))
        })?,
    )?;
    g.set(
        "SetChatWindowSavedDimensions",
        lua.create_function(|lua, (id, w, h): (Value, Value, Value)| {
            if let Some(slot) = window_index(lua, Some(&id)) {
                let (w, h) = (
                    lua.coerce_number(w)?.unwrap_or(0.0),
                    lua.coerce_number(h)?.unwrap_or(0.0),
                );
                let mut saved = lua.app_data_mut::<SavedGeometry>().expect("geometry");
                saved.0[slot].dimensions = Some((w as f32, h as f32));
            }
            Ok(())
        })?,
    )?;
    g.set(
        "GetChatWindowSavedPosition",
        lua.create_function(|lua, id: Value| {
            let Some(slot) = window_index(lua, Some(&id)) else {
                return Ok(MultiValue::new());
            };
            let saved = lua.app_data_ref::<SavedGeometry>().expect("geometry");
            let Some((point, x, y)) = saved.0[slot].position.clone() else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&point)?),
                Value::Number(f64::from(x)),
                Value::Number(f64::from(y)),
            ]))
        })?,
    )?;
    g.set(
        "SetChatWindowSavedPosition",
        lua.create_function(|lua, (id, point, x, y): (Value, Value, Value, Value)| {
            if let Some(slot) = window_index(lua, Some(&id)) {
                let point = lua
                    .coerce_string(point)?
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default();
                let (x, y) = (
                    lua.coerce_number(x)?.unwrap_or(0.0),
                    lua.coerce_number(y)?.unwrap_or(0.0),
                );
                let mut saved = lua.app_data_mut::<SavedGeometry>().expect("geometry");
                saved.0[slot].position = Some((point, x as f32, y as f32));
            }
            Ok(())
        })?,
    )?;

    // GetChatWindowInfo(index) (`0x4fbd90`) → name, fontSize, r, g, b, alpha, shown, locked,
    // docked, uninteractable. shown and locked are 1 or nil, docked the dock position read unsigned
    // or nil, uninteractable the stored number or nil (`0x4fbe9a`); FrameXML tests them bare.
    g.set(
        "GetChatWindowInfo",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: GetChatWindowInfo(index)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(MultiValue::new());
            };
            let look = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .chat_window_looks[i]
                .clone();
            let flag = |on: bool| if on { Value::Integer(1) } else { Value::Nil };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&look.name)?),
                Value::Integer(i64::from(look.font_size)),
                Value::Number(unit(look.r)),
                Value::Number(unit(look.g)),
                Value::Number(unit(look.b)),
                Value::Number(unit(look.a)),
                flag(look.shown),
                flag(look.locked),
                look.docked
                    .map_or(Value::Nil, |d| Value::Integer(i64::from(d))),
                flag(look.uninteractable),
            ]))
        })?,
    )?;

    // SetChatWindowUninteractable(index, uninteractable) (`0x4fc6e0`): the flag coercion with
    // default 0 into `+0xe4`; no event. FCF_SetUninteractable (FloatingChatFrame.lua:982) pairs it
    // with the frame's own SetHyperlinksEnabled.
    g.set(
        "SetChatWindowUninteractable",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: SetChatWindowUninteractable(index, uninteractable)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let on = bool_or_default(args.get(1), false);
            edit_look(lua, i, |l| l.uninteractable = on);
            Ok(())
        })?,
    )?;

    // ResetChatWindows() (`0x501f00`), FCF_ResetChatWindows' engine half
    // (FloatingChatFrame.lua:1775): the chat-type colours and colorNameByClass back to their
    // defaults (the CHANNELn extras from CHANNEL), every window record back to stock, the
    // auto-join channels registered in window 1, then UPDATE_CHAT_WINDOWS and, per chat type in
    // registry order, UPDATE_CHAT_COLOR(name, r, g, b) and
    // UPDATE_CHAT_COLOR_NAME_BY_CLASS(name, colorNameByClass).
    g.set(
        "ResetChatWindows",
        lua.create_function(|lua, _ignored: MultiValue| {
            let rows: Vec<(String, [u8; 3], bool)> = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                chat_types::reset(&mut model.chat_colors);
                // Every default row's `+0x48` is 0, the extras' too (copied from CHANNEL's).
                model
                    .chat_colors
                    .iter_mut()
                    .for_each(|c| c.name_by_class = false);
                model.chat_colors_changed = true;
                let seed = model.chat_channel_seed.clone();
                for i in 0..ENGINE_CHAT_WINDOW_SLOTS {
                    model.chat_window_looks[i] = ChatWindowLook::stock(i);
                    model.chat_window_changes.insert(i);
                }
                model.chat_window_looks[0].channels = seed;
                model
                    .chat_colors
                    .iter()
                    .map(|c| (c.name.clone(), c.rgb, c.name_by_class))
                    .collect()
            };
            super::tick::fire_event_into(lua, "UPDATE_CHAT_WINDOWS", Vec::new());
            for (name, rgb, by_class) in rows {
                super::tick::fire_event_into(
                    lua,
                    "UPDATE_CHAT_COLOR",
                    vec![
                        ScriptValue::Str(name.clone()),
                        ScriptValue::Number(chat_types::from_byte(rgb[0])),
                        ScriptValue::Number(chat_types::from_byte(rgb[1])),
                        ScriptValue::Number(chat_types::from_byte(rgb[2])),
                    ],
                );
                super::tick::fire_event_into(
                    lua,
                    "UPDATE_CHAT_COLOR_NAME_BY_CLASS",
                    vec![ScriptValue::Str(name), ScriptValue::Bool(by_class)],
                );
            }
            Ok(())
        })?,
    )?;

    // SetChatColorNameByClass(chatType, colorNameByClass) (`0x4fedc0`): a non-string raises; the
    // flag coercion with default 0. A chat type matched case-folded stores the flag and fires
    // UPDATE_CHAT_COLOR_NAME_BY_CLASS with the caller's spelling, changed or not; an unknown type
    // stores and fires nothing.
    g.set(
        "SetChatColorNameByClass",
        lua.create_function(|lua, args: MultiValue| {
            let Some(name) = args
                .front()
                .and_then(|v| super::binding_abi::optional_string(lua, v))
            else {
                return Err(mlua::Error::runtime(
                    "Usage: SetChatColorNameByClass(chatType, colorNameByClass)",
                ));
            };
            let on = bool_or_default(args.get(1), false);
            {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let Some(i) = chat_types::position(&model.chat_colors, &name) else {
                    return Ok(());
                };
                model.chat_colors[i].name_by_class = on;
                model.chat_colors_changed = true;
            }
            super::tick::fire_event_into(
                lua,
                "UPDATE_CHAT_COLOR_NAME_BY_CLASS",
                vec![ScriptValue::Str(name), ScriptValue::Bool(on)],
            );
            Ok(())
        })?,
    )?;

    // JoinPermanentChannel(name, password, frameID, hasVoice) (`0x50dac0`): the shared join with
    // the permanent flag, so the same answers, send and window registration as JoinChannelByName.
    // The host persists joined custom channels either way; voice is not supported.
    let join: mlua::Function = g.get("JoinChannelByName")?;
    g.set(
        "JoinPermanentChannel",
        lua.create_function(move |_, args: MultiValue| {
            let first_three: MultiValue = args.into_iter().take(3).collect();
            join.call::<MultiValue>(first_three)
        })?,
    )?;

    // BNLeaveConversation(channel) (`0x536030`): every check before the usage line asks for a
    // connected Battle.net session, which a 3.3.5a realm never opens, so it does nothing, raising
    // nothing even without an argument.
    g.set(
        "BNLeaveConversation",
        lua.create_function(|_, _ignored: MultiValue| Ok(()))?,
    )?;

    super::editbox::install_wrath(lua)?;
    super::messageframe::install_wrath(lua)
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    fn wrath() -> UiScript {
        UiScript::new().unwrap()
    }

    #[test]
    fn get_chat_window_info_has_the_tenth_uninteractable_value() {
        let mut s = wrath();
        assert_eq!(
            s.eval::<i64>("return select('#', GetChatWindowInfo(1))")
                .unwrap(),
            10
        );
        assert!(s
            .eval::<bool>("return select(10, GetChatWindowInfo(3)) == nil")
            .unwrap());
        s.run("SetChatWindowUninteractable(3, true)").unwrap();
        assert_eq!(
            s.eval::<i64>("return select(10, GetChatWindowInfo(3))")
                .unwrap(),
            1
        );
        assert_eq!(s.take_chat_window_changes(), vec![2]);
        s.run("SetChatWindowUninteractable(3, nil) SetChatWindowUninteractable(11, 1)")
            .unwrap();
        assert!(s
            .eval::<bool>("return select(10, GetChatWindowInfo(3)) == nil")
            .unwrap());
        assert_eq!(
            s.eval::<i64>("return select('#', GetChatWindowInfo(11))")
                .unwrap(),
            0,
            "an index outside the ten records answers nothing"
        );
        assert!(s.run("SetChatWindowUninteractable('x')").is_err());
    }

    #[test]
    fn reset_chat_windows_restores_records_colours_and_fires_in_order() {
        let mut s = wrath();
        s.set_chat_channel_seed(vec![("General".into(), 1), ("Trade".into(), 2)]);
        s.run(
            "SetChatWindowName(1, 'Mine') SetChatWindowDocked(1, nil) SetChatWindowShown(4, 1) \
             SetChatWindowUninteractable(2, 1) ChangeChatColor('SAY', 0, 0, 0) \
             SetChatColorNameByClass('SAY', true) \
             SEEN = {} local f = CreateFrame('Frame') \
             for _, e in ipairs({'UPDATE_CHAT_WINDOWS', 'UPDATE_CHAT_COLOR', \
                                 'UPDATE_CHAT_COLOR_NAME_BY_CLASS'}) do f:RegisterEvent(e) end \
             f:SetScript('OnEvent', function(_, e, a, b) SEEN[#SEEN + 1] = {e, a, b} end) \
             ResetChatWindows()",
        )
        .unwrap();
        let looks = s.chat_window_looks();
        assert_eq!(looks[0].name, "");
        assert_eq!(looks[0].docked, Some(1));
        assert_eq!(looks[1].docked, Some(2));
        assert!(!looks[1].uninteractable);
        assert!(!looks[3].shown);
        assert_eq!(
            looks[0].channels,
            vec![("General".to_string(), 1), ("Trade".to_string(), 2)]
        );
        assert_eq!(s.chat_colors()[1].rgb, [255, 255, 255]);
        assert!(!s.chat_colors()[1].name_by_class);
        assert_eq!(
            s.eval::<String>("return SEEN[1][1]").unwrap(),
            "UPDATE_CHAT_WINDOWS"
        );
        assert_eq!(
            s.eval::<String>("return SEEN[2][1]").unwrap(),
            "UPDATE_CHAT_COLOR"
        );
        assert_eq!(s.eval::<String>("return SEEN[2][2]").unwrap(), "SYSTEM");
        assert_eq!(
            s.eval::<String>("return SEEN[3][1]").unwrap(),
            "UPDATE_CHAT_COLOR_NAME_BY_CLASS"
        );
        assert!(!s.eval::<bool>("return SEEN[3][3]").unwrap());
        assert_eq!(
            s.eval::<i64>("return #SEEN").unwrap() as usize,
            1 + 2 * s.chat_colors().len()
        );
        assert_eq!(s.take_chat_window_changes(), (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn set_chat_color_name_by_class_fires_the_callers_spelling() {
        let s = wrath();
        s.run(
            "local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_CHAT_COLOR_NAME_BY_CLASS') \
             f:SetScript('OnEvent', function(_, _, a, b) N = (N or 0) + 1 A, B = a, b end) \
             SetChatColorNameByClass('guild', 1) SetChatColorNameByClass('NOPE', 1)",
        )
        .unwrap();
        assert_eq!(s.eval::<i64>("return N").unwrap(), 1);
        assert_eq!(s.eval::<String>("return A").unwrap(), "guild");
        assert!(s.eval::<bool>("return B").unwrap());
        assert!(s
            .chat_colors()
            .iter()
            .any(|c| c.name == "GUILD" && c.name_by_class));
        assert!(s.run("SetChatColorNameByClass()").is_err());
    }

    #[test]
    fn join_permanent_channel_joins_like_join_channel_by_name() {
        let mut s = wrath();
        s.run("A = {JoinPermanentChannel('MyChan', nil, 2, 1)}")
            .unwrap();
        assert_eq!(s.eval::<i64>("return A[1]").unwrap(), 0);
        assert_eq!(
            s.take_channel_commands(),
            vec![crate::script::ChannelCommand::Join {
                name: "MyChan".into(),
                password: String::new(),
            }]
        );
        assert_eq!(
            s.chat_window_looks()[1].channels,
            vec![("MyChan".to_string(), 0)]
        );
    }

    #[test]
    fn bn_leave_conversation_is_inert_without_a_battle_net_session() {
        let s = wrath();
        assert_eq!(
            s.eval::<i64>("return select('#', BNLeaveConversation(1))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<i64>("return select('#', BNLeaveConversation())")
                .unwrap(),
            0,
            "no usage error without a session"
        );
    }

    #[test]
    fn edit_box_clear_history_drops_lines_and_keeps_the_cap() {
        let s = wrath();
        s.run(
            "E = CreateFrame('EditBox') E:SetHistoryLines(4) \
             E:AddHistoryLine('/s one') E:AddHistoryLine('/s two') E:ClearHistory()",
        )
        .unwrap();
        assert_eq!(s.eval::<i64>("return E:GetHistoryLines()").unwrap(), 4);
        let e: mlua::Table = s.eval("return E").unwrap();
        let h = crate::script::object::frame_handle_of(s.lua(), &e).unwrap();
        let n = super::super::editbox::history_len(s.lua(), h);
        assert_eq!(n, Some(0));
    }
}
