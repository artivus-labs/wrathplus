//! Build 12340's override bindings (`/tmp/azb-app-mig/ghidra/{secure,override}.log`): a frame
//! sets keys that dispatch reads over the normal table ([`super::UiScript::effective_binding_keys`])
//! until it clears them. Each verb takes the owner frame first and raises its usage line for a
//! non-frame; the command variants write the forms `run_command` runs: `CLICK button:mouse`
//! (`0x563f10`), `ITEM name` (`0x563cb0`), `MACRO name|id` (`0x563de0`) and `SPELL name`
//! (`0x563b80`). Labelled: the store's `0x5191c0(0xd)` gate on a priority override and its
//! key-string interning are not modelled.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// The owner frame's id, `None` for anything but a frame (the native's type-5 test and its
/// object check).
fn owner(v: &Value) -> Option<u32> {
    match v {
        Value::Table(t) => super::object::decode_id(t).ok(),
        _ => None,
    }
}

/// `isPriority` as Lua truth.
fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Boolean(false))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Store `owner`'s override of `key`; a key that does not parse binds nothing.
fn store(lua: &Lua, owner: u32, priority: bool, key: &str, command: Option<String>) {
    if let Some(key) = super::keybind::normalize_binding_key(key) {
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .keybinds
            .set_override(owner, priority, key, command);
    }
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// One `SetOverrideBinding*` verb: owner, isPriority, a string key, then the command its
/// `form` builds from the remaining arguments (`None` clears), else `usage`.
fn install_setter(
    lua: &Lua,
    name: &str,
    usage: &'static str,
    form: fn(&MultiValue) -> Option<Option<String>>,
) -> mlua::Result<()> {
    lua.globals().set(
        name,
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(owner), Some(key)) = (owner(&arg(&args, 0)), text(&arg(&args, 2))) else {
                return Err(mlua::Error::runtime(usage));
            };
            let Some(command) = form(&args) else {
                return Err(mlua::Error::runtime(usage));
            };
            store(lua, owner, truthy(&arg(&args, 1)), &key, command);
            Ok(())
        })?,
    )
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "ClearOverrideBindings",
        lua.create_function(|lua, owner_frame: Value| {
            let Some(owner) = owner(&owner_frame) else {
                return Err(mlua::Error::runtime("Usage: ClearOverrideBindings(owner)"));
            };
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .keybinds
                .clear_overrides(owner);
            Ok(())
        })?,
    )?;
    // The bare form: an optional command; none clears the key.
    install_setter(
        lua,
        "SetOverrideBinding",
        "Usage: SetOverrideBinding(owner, isPriority, \"KEY\"[, \"COMMAND\"])",
        |args| Some(text(&arg(args, 3))),
    )?;
    // A button name and an optional mouse button, `LeftButton` by default.
    install_setter(
        lua,
        "SetOverrideBindingClick",
        "Usage: SetOverrideBindingClick(owner, isPriority, \"KEY\", \"ButtonName\"[, \"mouseButton\"])",
        |args| {
            let button = text(&arg(args, 3))?;
            let mouse = text(&arg(args, 4)).unwrap_or_else(|| "LeftButton".into());
            Some(Some(format!("CLICK {button}:{mouse}")))
        },
    )?;
    for (name, usage, prefix) in [
        (
            "SetOverrideBindingItem",
            "Usage: SetOverrideBindingItem(owner, isPriority, \"KEY\", \"itemname\")",
            "ITEM",
        ),
        (
            "SetOverrideBindingMacro",
            "Usage: SetOverrideBindingMacro(owner, isPriority, \"KEY\", \"macroname\"|macroid)",
            "MACRO",
        ),
        (
            "SetOverrideBindingSpell",
            "Usage: SetOverrideBindingSpell(owner, isPriority, \"KEY\", \"spellname\")",
            "SPELL",
        ),
    ] {
        let form: fn(&MultiValue) -> Option<Option<String>> = match prefix {
            "ITEM" => |args| Some(Some(format!("ITEM {}", text(&arg(args, 3))?))),
            "MACRO" => |args| Some(Some(format!("MACRO {}", text(&arg(args, 3))?))),
            _ => |args| Some(Some(format!("SPELL {}", text(&arg(args, 3))?))),
        };
        install_setter(lua, name, usage, form)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn overrides_lay_over_the_live_keys_by_priority_and_clear_per_owner() {
        let s = UiScript::new().unwrap();
        // Dispatch keeps a declared command or a form only (`0x562140`).
        s.register_bindings(
            &crate::bindings_xml::parse(
                r#"<Bindings><Binding name="TOGGLEAUTORUN">x()</Binding></Bindings>"#,
            )
            .unwrap(),
        );
        s.run("A = CreateFrame('Frame') B = CreateFrame('Frame')")
            .unwrap();
        let before = s.keybinds_generation();
        s.run(
            "SetOverrideBinding(A, false, 'F', 'TOGGLEAUTORUN') \
             SetOverrideBindingClick(B, true, 'F', 'MyButton') \
             SetOverrideBindingSpell(A, false, 'CTRL-G', 'Fireball')",
        )
        .unwrap();
        assert!(
            s.keybinds_generation() > before,
            "a change re-derives dispatch"
        );
        let keys = s.effective_binding_keys();
        let bound = |key: &str| keys.iter().find(|(k, _)| k == key).map(|(_, c)| c.clone());
        assert_eq!(
            bound("F").as_deref(),
            Some("CLICK MyButton:LeftButton"),
            "priority wins"
        );
        assert_eq!(bound("CTRL-G").as_deref(), Some("SPELL Fireball"));
        assert!(
            s.binding_keys().iter().all(|(k, _)| k != "CTRL-G"),
            "never saved"
        );
        s.run("ClearOverrideBindings(B)").unwrap();
        let keys = s.effective_binding_keys();
        assert!(keys.contains(&("F".into(), "TOGGLEAUTORUN".into())));
        s.run("SetOverrideBinding(A, false, 'F')").unwrap();
        assert!(
            s.effective_binding_keys().iter().all(|(k, _)| k != "F"),
            "no command clears"
        );
        assert!(s.run("SetOverrideBinding({}, false, 'F', 'X')").is_err());
        assert!(s.run("ClearOverrideBindings(1)").is_err());
        assert!(s.run("SetOverrideBindingItem(A, false, 'H')").is_err());
    }

    #[test]
    fn a_click_form_command_clicks_the_named_button() {
        let s = UiScript::new().unwrap();
        s.run(
            "Clicked = nil MyButton = CreateFrame('Button', 'MyButton') \
             MyButton:SetScript('OnClick', function(self, button) Clicked = button end)",
        )
        .unwrap();
        // The click fires on the transition the button registered (`LeftButtonUp` by default).
        s.run("MyButton:RegisterForClicks('RightButtonUp')")
            .unwrap();
        assert!(s
            .execute_binding("CLICK MyButton:RightButton", false)
            .unwrap());
        assert_eq!(s.eval::<String>("return Clicked").unwrap(), "RightButton");
        assert!(!s.execute_binding("CLICK Nobody:LeftButton", true).unwrap());
        assert!(!s.execute_binding("NOT A COMMAND", true).unwrap());
    }
}
