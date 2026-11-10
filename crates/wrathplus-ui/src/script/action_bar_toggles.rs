//! `GetActionBarToggles` and `SetActionBarToggles`: the four extra action bars' visibility, the
//! low nibble of the server's `PLAYER_FIELD_BYTES` byte 2.
//!
//! The setter (`0x5a8290`) only posts `CMSG_SET_ACTIONBAR_TOGGLES` (`0x2bf`); the getter
//! (`0x5a8790`) reads the live descriptor byte, `[[player+0x1008]+0x1066]` (`0x5a87bd`), which only
//! the `SMSG_UPDATE_OBJECT` apply writes. A `Get` right after a `Set` returns the old value for a
//! round trip, as `UIParent.lua:649` says where it leaves the `PLAYER_ENTERING_WORLD` read
//! commented out; stock reads it through the Interface Options checkboxes' `GetValue`
//! (`InterfaceOptionsPanels.xml:1385`, `:1404`, `:1423`, `:1442`).
//!
//! The setter builds the byte from zero out of four arguments (`0x5a829c`, `0x5a82bb`): a fifth,
//! which `InterfaceOptionsPanels.lua:1193` passes, is dropped, and a high nibble the server held is
//! cleared.

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::bool_or_default;
use super::Model;

/// Toggle bits the bindings read and write, four both ways (setter `0x5a82c0`, getter `0x5a8802`).
const ACTION_BAR_TOGGLE_BITS: u32 = 4;

impl super::UiScript {
    /// Push the server's `PLAYER_FIELD_BYTES` byte 2, the only way this value moves.
    pub fn set_action_bar_toggles(&mut self, toggles: u8) {
        self.model_mut().action_bar_toggles = Some(toggles);
    }

    /// The last pushed byte. Lua reads `None` as a zero byte, four `nil`s, as the reference does
    /// with no local player (`0x5a8798`-`0x5a87c3`).
    pub fn action_bar_toggles(&self) -> Option<u8> {
        self.model_ref().action_bar_toggles
    }

    /// Drain the queued `CMSG_SET_ACTIONBAR_TOGGLES` payloads, one per `SetActionBarToggles` call:
    /// the binding has no did-it-change test and no connection check, so two calls are two packets.
    pub fn take_action_bar_toggle_sends(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.model_mut().action_bar_toggle_sends)
    }
}

/// Register the two action-bar-toggle globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetActionBarToggles() -> four values, each the number 1 (`fld1`, `0x5a87ec`) or nil (`0x5a87f7`),
    // never a boolean; return N tests bit N-1.
    g.set(
        "GetActionBarToggles",
        lua.create_function(|lua, ()| {
            let byte = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model.action_bar_toggles.unwrap_or(0)
            };
            let bit = |i: u32| {
                if byte & (1 << i) != 0 {
                    Value::Number(1.0)
                } else {
                    Value::Nil
                }
            };
            Ok((bit(0), bit(1), bit(2), bit(3)))
        })?,
    )?;

    // SetActionBarToggles(a, b, c, d) -> nothing. Arguments coerce through `0x815500`, not Lua
    // truthiness, so a "0" string is off; the default is false (`push 0`, `0x5a82a2`). Nothing
    // local is written: only the server moves the byte.
    g.set(
        "SetActionBarToggles",
        lua.create_function(|lua, args: MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let mut packed = 0u8;
            for i in 0..ACTION_BAR_TOGGLE_BITS {
                if bool_or_default(args.get(i as usize), false) {
                    packed |= 1 << i;
                }
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.action_bar_toggle_sends.push(packed);
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    /// Argument N sets bit N-1 (`0x5a82a4`, `0x5a82b9`).
    #[test]
    fn the_four_arguments_pack_into_bits_0_through_3() {
        let mut s = UiScript::new().unwrap();
        for (call, want) in [
            ("SetActionBarToggles(1)", 0x01),
            ("SetActionBarToggles(nil, 1)", 0x02),
            ("SetActionBarToggles(nil, nil, 1)", 0x04),
            ("SetActionBarToggles(nil, nil, nil, 1)", 0x08),
            ("SetActionBarToggles(1, 1, 1, 1)", 0x0f),
            ("SetActionBarToggles()", 0x00),
        ] {
            s.run(call).unwrap();
            assert_eq!(
                s.take_action_bar_toggle_sends(),
                vec![want],
                "{call} packs {want:#04x}"
            );
        }
    }

    /// The coercion is `0x815500`: the string `"0"`, truthy in Lua, is off.
    #[test]
    fn the_arguments_are_coerced_the_binarys_way_not_luas() {
        let mut s = UiScript::new().unwrap();

        s.run(r#"SetActionBarToggles("1", "0", "1", "0")"#).unwrap();
        assert_eq!(
            s.take_action_bar_toggle_sends(),
            vec![0x05],
            r#"the strings the panel passes: "0" is FALSE, where Lua truthiness says true"#
        );

        // A number truncates toward zero (`__ftol2_sse`, `0x88b9c0`).
        s.run("SetActionBarToggles(0.5, -0.9, 1.5, 0)").unwrap();
        assert_eq!(
            s.take_action_bar_toggle_sends(),
            vec![0x04],
            "fractions inside (-1, 1) truncate to 0 and read false; 1.5 truncates to 1"
        );

        s.run(r#"SetActionBarToggles("on", "off", "enabled", "disabled")"#)
            .unwrap();
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x05]);
        s.run(r#"SetActionBarToggles("true", "false", "yes", "no")"#)
            .unwrap();
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x05]);

        s.run("SetActionBarToggles(true, false, true, false)")
            .unwrap();
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x05]);

        s.run(r#"SetActionBarToggles({}, "maybe", 1, 1)"#).unwrap();
        assert_eq!(
            s.take_action_bar_toggle_sends(),
            vec![0x0c],
            "both take the default, which is FALSE here"
        );
    }

    #[test]
    fn a_fifth_argument_is_accepted_and_dropped() {
        let mut s = UiScript::new().unwrap();
        s.run("SetActionBarToggles(nil, nil, nil, nil, 1)").unwrap();
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x00]);
        s.run("SetActionBarToggles(1, 1, 1, 1, 1, 1, 1)").unwrap();
        assert_eq!(
            s.take_action_bar_toggle_sends(),
            vec![0x0f],
            "still four bits — the accumulator can only ever hold 0x00..0x0f"
        );
    }

    #[test]
    fn a_set_destroys_whatever_the_server_held_above_the_low_nibble() {
        let mut s = UiScript::new().unwrap();
        s.set_action_bar_toggles(0xf5);
        s.run("SetActionBarToggles(1, 0, 0, 0)").unwrap();
        assert_eq!(
            s.take_action_bar_toggle_sends(),
            vec![0x01],
            "0xf5's high nibble is gone — the accumulator never saw it"
        );
    }

    #[test]
    fn the_getter_returns_four_ones_or_nils_from_the_pushed_byte() {
        let mut s = UiScript::new().unwrap();

        // Nothing pushed reads as a zero byte, as no local player does (`0x5a87c3`).
        assert!(s
            .eval::<bool>(
                "local a,b,c,d = GetActionBarToggles() \
                 return a == nil and b == nil and c == nil and d == nil"
            )
            .unwrap());
        assert_eq!(s.arity("GetActionBarToggles()").unwrap(), 4);

        s.set_action_bar_toggles(0x0a);
        assert!(s
            .eval::<bool>(
                "local a,b,c,d = GetActionBarToggles() \
                 return a == nil and b == 1 and c == nil and d == 1"
            )
            .unwrap());
        assert!(
            s.eval::<bool>("local a,b = GetActionBarToggles() return type(b) == 'number'")
                .unwrap(),
            "the NUMBER 1 (0x84e2a0 pushes tag 3 / double 1.0), never the boolean true"
        );

        // The high nibble is never tested.
        s.set_action_bar_toggles(0xf0);
        assert!(s
            .eval::<bool>(
                "local a,b,c,d = GetActionBarToggles() \
                 return a == nil and b == nil and c == nil and d == nil"
            )
            .unwrap());
    }

    #[test]
    fn the_setter_does_not_touch_the_local_copy_so_the_read_lags_a_round_trip() {
        let mut s = UiScript::new().unwrap();
        s.set_action_bar_toggles(0x00);
        s.run("SetActionBarToggles(1, 1, 0, 0)").unwrap();
        assert_eq!(
            s.action_bar_toggles(),
            Some(0x00),
            "still the server's value — the setter wrote nothing local"
        );
        assert!(s
            .eval::<bool>("local a = GetActionBarToggles() return a == nil")
            .unwrap());

        let sent = s.take_action_bar_toggle_sends();
        assert_eq!(sent, vec![0x03]);
        // The server's `SMSG_UPDATE_OBJECT` echo.
        s.set_action_bar_toggles(sent[0]);
        assert!(s
            .eval::<bool>(
                "local a,b,c,d = GetActionBarToggles() \
                 return a == 1 and b == 1 and c == nil and d == nil"
            )
            .unwrap());
    }

    #[test]
    fn every_call_queues_a_packet_and_returns_nothing() {
        let mut s = UiScript::new().unwrap();
        s.run("SetActionBarToggles(1) SetActionBarToggles(1) SetActionBarToggles(nil)")
            .unwrap();
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x01, 0x01, 0x00]);
        assert_eq!(s.arity("SetActionBarToggles(1)").unwrap(), 0);
        assert_eq!(s.take_action_bar_toggle_sends(), vec![0x01]);
    }
}
