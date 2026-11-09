//! Build 12340's `SetBinding*` command-form setters and the mouselook override table.
//!
//! `SetBindingClick`, `SetBindingSpell`, `SetBindingItem` and `SetBindingMacro` format a command
//! (`CLICK %s:%s`, `SPELL %s`, `ITEM %s`, `MACRO %s`) and store it as `SetBinding` does
//! (`0x562ed0`), so the key runs it through the dispatcher's form runner ([`run_form`],
//! the binding runner `0x563150`). Each stores in the trailing `mode` (1-4, default 1,
//! [`super::wrath_binding_modes`]) and on a stored key fires `UPDATE_BINDINGS` in the call
//! (`0x81b530(0x177)`) and answers 1, else nil. Labelled: the protected gate (`0x5191c0(0xd)`) has
//! no counterpart in this trusted UI.

use mlua::{Lua, MultiValue, Value};

use super::keybind::KeyBinding;
use super::Model;

/// `SetMouselookOverrideBinding`'s table (`0x5fd440`, on `InputControl`): key to command, in set
/// order.
#[derive(Default)]
struct MouselookOverrides(Vec<KeyBinding>);

/// `lua_isstring` (`0x84df60`): a string or a number.
fn is_string(v: Option<&Value>) -> bool {
    matches!(
        v,
        Some(Value::String(_) | Value::Integer(_) | Value::Number(_))
    )
}

/// `lua_tostring`: a string or a number's text, else `None`.
fn text(lua: &Lua, v: Option<&Value>) -> mlua::Result<Option<String>> {
    if !is_string(v) {
        return Ok(None);
    }
    Ok(lua
        .coerce_string(v.cloned().unwrap_or(Value::Nil))?
        .map(|s| s.to_string_lossy()))
}

/// One setter: keys and the name argument must be strings or numbers, else `usage`; `form` builds
/// the command from the arguments, and argument `mode_at` (0-based) picks the mode, default 1.
fn install_form(
    lua: &Lua,
    name: &str,
    mode_at: usize,
    usage: &'static str,
    form: fn(&Lua, &MultiValue, String) -> mlua::Result<String>,
) -> mlua::Result<()> {
    lua.globals().set(
        name,
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(key), Some(arg)) = (text(lua, args.front())?, text(lua, args.get(1))?) else {
                return Err(mlua::Error::runtime(usage));
            };
            let command = form(lua, &args, arg)?;
            let mode = super::wrath_binding_modes::mode_arg(lua, args.get(mode_at), 0);
            Ok(super::wrath_binding_modes::store_and_announce(
                lua,
                mode,
                &key,
                Some(&command),
            ))
        })?,
    )
}

/// The binding runner's command forms (`0x563150`), tried by case-insensitive prefix: `SPELL name`,
/// `ITEM name` and `MACRO name|index` act on the press and answer "ran" on the release too;
/// `CLICK button:mouse` acts on both. `Ok(false)` for a command that is no form: the reference
/// then runs it by name (`0x55f860`), which [`super::keybind::run_command`] already tried.
pub(super) fn run_form(lua: &Lua, command: &str, down: bool) -> mlua::Result<bool> {
    let form = |prefix: &str| {
        command
            .get(..prefix.len())
            .filter(|p| p.eq_ignore_ascii_case(prefix))
            .map(|_| &command[prefix.len()..])
    };
    if let Some(spell) = form("SPELL ") {
        if down {
            cast_spell(lua, spell)?;
        }
        return Ok(true);
    }
    if let Some(item) = form("ITEM ") {
        if down {
            use_item(lua, item)?;
        }
        return Ok(true);
    }
    if let Some(m) = form("MACRO ") {
        if down {
            run_macro(lua, m);
        }
        return Ok(true);
    }
    if let Some(click) = form("CLICK ") {
        return click_form(lua, click, down);
    }
    Ok(false)
}

/// The unit a `SPELL`/`ITEM` form aims at: the player under the `SELFCAST` modified click, the
/// focus under `FOCUSCAST` when there is one, else the default target (`0x563150`'s
/// `0x55f940("SELFCAST")`/`("FOCUSCAST")` legs). `Some(None)` is the player.
fn form_target(lua: &Lua) -> mlua::Result<Option<Option<&'static str>>> {
    let (selfcast, focuscast) = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let modified = |a| super::wrath_modified_click::is_modified_click(&model, Some(a));
        (modified("SELFCAST"), modified("FOCUSCAST"))
    };
    if selfcast {
        return Ok(Some(None));
    }
    let focus = focuscast
        && lua
            .globals()
            .get::<mlua::Function>("UnitExists")?
            .call::<Option<i64>>("focus")?
            .is_some();
    Ok(focus.then_some(Some("focus")))
}

/// `SPELL name` on the press: the spell by name (`0x540650`; inferred, as `SetMacroSpell`
/// resolves it: the player's book, then the pet's) cast at [`form_target`]'s unit.
fn cast_spell(lua: &Lua, name: &str) -> mlua::Result<()> {
    let target = form_target(lua)?;
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let pets = super::SpellBookState {
        tabs: Vec::new(),
        slots: model.pet_book.slots.clone(),
    };
    let on_self = target == Some(None);
    let call = if let Some(s) = super::spellbook::resolve_spell_by_name(&model.spellbook, name) {
        match target {
            Some(Some(unit)) => super::ScriptCall::CastSpellAt {
                spell_id: s.spell_id,
                unit: unit.into(),
            },
            _ => super::ScriptCall::CastSpell {
                spell_id: s.spell_id,
                on_self,
            },
        }
    } else if let Some(s) = super::spellbook::resolve_spell_by_name(&pets, name) {
        super::ScriptCall::CastPetSpell {
            spell_id: s.spell_id,
            on_self,
        }
    } else {
        return Ok(());
    };
    model.script_calls.push(call);
    Ok(())
}

/// `ITEM name` on the press, with a player (`0x4038f0`): the first carried copy by name
/// (`0x754af0`; inferred order: the equipped slots, then the backpack and bags), used as a click
/// on its slot uses it, retaining `SELFCAST`/`FOCUSCAST` intent (`0x708c20`).
fn use_item(lua: &Lua, name: &str) -> mlua::Result<()> {
    let target = form_target(lua)?.map(|unit| unit.unwrap_or("player").to_string());
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    if model.unit_guids.player == 0 {
        return Ok(());
    }
    let named = |link: Option<&str>| super::cursor::item_link_name(link).eq_ignore_ascii_case(name);
    let worn = model.inventory_slots.iter().enumerate().find_map(|(i, s)| {
        let s = s.as_ref()?;
        (s.item_id != 0 && named(s.link.as_deref())).then_some(i as u32 + 1)
    });
    let call = match worn {
        Some(slot) => Some(super::ScriptCall::UseInventoryItem(slot)),
        None => (0..=4).find_map(|bag| {
            let c = model.containers.get(&bag)?;
            let mut slots: Vec<_> = c.slots.iter().collect();
            slots.sort_by_key(|(slot, _)| **slot);
            let (slot, _) = slots
                .into_iter()
                .find(|(_, s)| s.item_id != 0 && named(s.link.as_deref()))?;
            Some(super::ScriptCall::UseContainerItem { bag, slot: *slot })
        }),
    };
    model
        .script_calls
        .extend(call.map(|call| call.at_unit(target)));
    Ok(())
}

/// `MACRO name|index` on the press: all digits is a 1-based slot (`0x566a70`), else a name
/// (`0x566330`); the macro runs with the button `"LeftButton"` (`0x564db0`).
fn run_macro(lua: &Lua, m: &str) {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let index = if !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit()) {
        m.parse::<usize>()
            .ok()
            .filter(|&i| model.macros.get(i).is_some())
    } else {
        Some(model.macros.index_by_name(m)).filter(|&i| i > 0)
    };
    if let Some(index) = index {
        model.script_calls.push(super::ScriptCall::RunMacro {
            index: index as u32,
            button: Some("LeftButton".into()),
        });
    }
}

/// `CLICK button:mouse` on the press and the release: the frame by name, its press or release
/// with that mouse button, as the pointer's would be (`0x563150` calls the frame's down or up
/// handler), so `OnClick` fires where the Button registered that transition. The colon is
/// required (`0x76e6e0(…, ':')`). `GetRunningMacroButton` answers the mouse button meanwhile
/// (`[0xbeaf48]`). `Ok(false)` when no frame has the name.
fn click_form(lua: &Lua, click: &str, down: bool) -> mlua::Result<bool> {
    let Some((name, mouse)) = click.split_once(':') else {
        return Ok(false);
    };
    let fire = {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let Some(h) = model.arena.lookup(name) else {
            return Ok(false);
        };
        let transition = format!("{mouse}{}", if down { "Down" } else { "Up" });
        super::button::wants_click(&model, h, &transition).then(|| model.frame_id(h))
    };
    if let Some(id) = fire {
        let outer = super::wrath_macro_api::swap_running_button(lua, Some(mouse.to_string()));
        super::button::click_button(lua, id, mouse, down, false);
        super::wrath_macro_api::swap_running_button(lua, outer);
    }
    Ok(true)
}

impl super::UiScript {
    /// The mouselook override keys (`SetMouselookOverrideBinding`), in set order, for dispatch to
    /// probe first while [`Self::mouselooking`].
    pub fn mouselook_binding_keys(&self) -> Vec<KeyBinding> {
        self.lua
            .app_data_ref::<MouselookOverrides>()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    /// Whether the TurnOrAction channel is held, what `IsMouselooking` answers.
    pub fn mouselooking(&self) -> bool {
        self.model_mut().input.turn_or_action
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(MouselookOverrides::default());
    // `SetBindingClick("KEY", "buttonName"[, "mouseButton"][, mode])` (`0x563940`): `CLICK
    // button:mouse`, the mouse button `LeftButton` when argument 3 is not a string.
    install_form(
        lua,
        "SetBindingClick",
        3,
        "Usage: SetBindingClick(\"KEY\", \"buttonName\"[, \"mouseButton\"][, mode])",
        |lua, args, button| {
            let mouse = text(lua, args.get(2))?.unwrap_or_else(|| "LeftButton".into());
            Ok(format!("CLICK {button}:{mouse}"))
        },
    )?;
    // `SetBindingSpell("KEY", "spellname"[, mode])` (`0x5635e0`): `SPELL name`.
    install_form(
        lua,
        "SetBindingSpell",
        2,
        "Usage: SetBindingSpell(\"KEY\", \"spellname\"[, mode])",
        |_, _, spell| Ok(format!("SPELL {spell}")),
    )?;
    // `SetBindingItem("KEY", "itemname"[, mode])` (`0x563700`): `ITEM name`.
    install_form(
        lua,
        "SetBindingItem",
        2,
        "Usage: SetBindingItem(\"KEY\", \"itemname\"[, mode])",
        |_, _, item| Ok(format!("ITEM {item}")),
    )?;
    // `SetBindingMacro("KEY", "macroname"|macroid[, mode])` (`0x563820`): `MACRO name|index`,
    // which the runner hands to `RunMacro`.
    install_form(
        lua,
        "SetBindingMacro",
        2,
        "Usage: SetBindingMacro(\"KEY\", \"macroname\"|macroid[, mode])",
        |_, _, m| Ok(format!("MACRO {m}")),
    )?;

    // `SetMouselookOverrideBinding("key", "binding" or nil)` (`0x5fd550` → `0x5fd440`): set the
    // key's command in the mouselook table, or with no command drop it; no results and no
    // `UPDATE_BINDINGS` (neither function fires one). The key is stored upper-cased, unvalidated,
    // as the reference stores the raw string; a change bumps the binding generation so dispatch
    // re-derives. Inferred: dispatch probes the table first while mouselooking. Its reader was not
    // found: the setter is the only caller of the table's find-or-insert (`0x5fd370`) and the only
    // `[0xc24954]+0x1c` lookup through `0x55f4d0`, and no read of `[0xc24954]` (in a function or
    // not) nor the mouse pre-binding handlers (`0x51fb00`, `0x8e5250`) touch it.
    lua.globals().set(
        "SetMouselookOverrideBinding",
        lua.create_function(|lua, args: MultiValue| {
            let Some(key) = text(lua, args.front())? else {
                return Err(mlua::Error::runtime(
                    "Usage: SetMouselookOverrideBinding(\"key\", \"binding\" or nil)",
                ));
            };
            let key = key.to_ascii_uppercase();
            let command = text(lua, args.get(1))?.filter(|c| !c.is_empty());
            {
                let mut table = lua
                    .app_data_mut::<MouselookOverrides>()
                    .expect("mouselook overrides");
                table.0.retain(|(k, _)| *k != key);
                if let Some(command) = command {
                    table.0.push((key, command));
                }
            }
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .keybinds
                .generation += 1;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    /// The four forms store the stock command strings on the live keys and answer 1, nil for a
    /// key the validator refuses, and raise their usage lines without a key and a name.
    #[test]
    fn the_setters_store_the_command_forms_and_answer_1_or_nil() {
        let s = UiScript::new().unwrap();
        let before = s.keybinds_generation();
        let answers = s
            .eval::<String>(
                "return table.concat({tostring(SetBindingClick('F', 'MyButton')), \
                 tostring(SetBindingClick('G', 'MyButton', 'RightButton', 2)), \
                 tostring(SetBindingSpell('CTRL-H', 'Holy Light', 1)), \
                 tostring(SetBindingItem('J', 'Hearthstone')), \
                 tostring(SetBindingMacro('K', 3)), \
                 tostring(SetBindingSpell('NOTAKEY', 'Fireball'))}, ',')",
            )
            .unwrap();
        assert_eq!(answers, "1,1,1,1,1,nil");
        assert!(s.keybinds_generation() > before, "dispatch re-derives");
        let keys = s.binding_keys();
        let bound = |key: &str| keys.iter().find(|(k, _)| k == key).map(|(_, c)| c.clone());
        assert_eq!(bound("F").as_deref(), Some("CLICK MyButton:LeftButton"));
        assert_eq!(bound("G"), None, "mode 2 is not mode 1");
        assert_eq!(
            s.eval::<String>("return GetBindingAction('G', nil, 2)")
                .unwrap(),
            "CLICK MyButton:RightButton"
        );
        assert_eq!(bound("CTRL-H").as_deref(), Some("SPELL Holy Light"));
        assert_eq!(bound("J").as_deref(), Some("ITEM Hearthstone"));
        assert_eq!(bound("K").as_deref(), Some("MACRO 3"));
        for chunk in [
            "SetBindingClick('F')",
            "SetBindingSpell(nil, 'Fireball')",
            "SetBindingItem('F', {})",
            "SetBindingMacro('F')",
        ] {
            let e = s.run(chunk).unwrap_err().to_string();
            assert!(e.contains("Usage: SetBinding"), "{chunk}: {e}");
        }
    }

    /// A `CLICK` key presses and releases the named button with the stored mouse button, so
    /// `OnClick` fires on the transition the button registered; `GetRunningMacroButton` answers
    /// the mouse button inside. No colon, or no such frame, runs nothing.
    #[test]
    fn a_click_form_presses_and_releases_the_named_button() {
        let s = UiScript::new().unwrap();
        s.run(
            "CLICKS = {} B = CreateFrame('Button', 'ProbeButton') \
             B:RegisterForClicks('MiddleButtonUp') \
             B:SetScript('OnClick', function(_, button, down) \
               CLICKS[#CLICKS + 1] = button .. tostring(down) .. GetRunningMacroButton() end) \
             SetBindingClick('F', 'ProbeButton', 'MiddleButton')",
        )
        .unwrap();
        let clicks = || {
            s.eval::<String>("return table.concat(CLICKS, ',')")
                .unwrap()
        };
        assert!(s
            .execute_binding("CLICK ProbeButton:MiddleButton", true)
            .unwrap());
        assert_eq!(clicks(), "", "registered for the release only");
        assert!(s
            .execute_binding("CLICK ProbeButton:MiddleButton", false)
            .unwrap());
        assert_eq!(clicks(), "MiddleButtonfalseMiddleButton");
        assert!(s
            .eval::<bool>("return GetRunningMacroButton() == nil")
            .unwrap());
        assert!(!s.execute_binding("CLICK ProbeButton", false).unwrap());
        assert!(!s.execute_binding("CLICK Nobody:LeftButton", false).unwrap());
    }

    /// `SPELL` casts from the book on the press, at the player under `SELFCAST`; `MACRO` runs the
    /// macro by index or name with the button `LeftButton`; `ITEM` uses the first carried copy.
    /// Each answers "ran" on the release without acting.
    #[test]
    fn the_spell_macro_and_item_forms_act_on_the_press() {
        use crate::script::{ScriptCall, SpellBookState, SpellSlotView};
        let mut s = UiScript::new().unwrap();
        s.set_spellbook(SpellBookState {
            tabs: Vec::new(),
            slots: vec![SpellSlotView {
                spell_id: 133,
                name: "Fireball".into(),
                ..Default::default()
            }],
        });
        s.run("CreateMacro('Probe', 1, '/say hi')").unwrap();
        s.take_script_calls();
        assert!(s.execute_binding("spell Fireball", true).unwrap());
        assert!(s.execute_binding("SPELL Fireball", false).unwrap());
        assert!(s.execute_binding("SPELL Nothing", true).unwrap());
        s.register_modified_clicks(
            &crate::bindings_xml::parse_modified_clicks(
                r#"<Bindings><ModifiedClick action="SELFCAST" default="ALT"/></Bindings>"#,
            )
            .unwrap(),
        );
        s.set_modifiers(false, false, true);
        assert!(s.execute_binding("SPELL Fireball", true).unwrap());
        s.set_modifiers(false, false, false);
        assert!(s.execute_binding("MACRO 1", true).unwrap());
        assert!(s.execute_binding("MACRO probe", true).unwrap());
        assert!(s.execute_binding("MACRO 9", true).unwrap());
        assert!(s.execute_binding("ITEM Hearthstone", true).unwrap());
        let macro_call = ScriptCall::RunMacro {
            index: 1,
            button: Some("LeftButton".into()),
        };
        assert_eq!(
            s.take_script_calls(),
            [
                ScriptCall::CastSpell {
                    spell_id: 133,
                    on_self: false
                },
                ScriptCall::CastSpell {
                    spell_id: 133,
                    on_self: true
                },
                macro_call.clone(),
                macro_call,
            ],
            "no player: the ITEM form uses nothing"
        );
    }

    /// `SetMouselookOverrideBinding` keeps its own table, upper-cased, a nil command dropping the
    /// key; it answers nothing and raises its usage line without a key.
    #[test]
    fn the_mouselook_table_sets_and_drops_keys() {
        let mut s = UiScript::new().unwrap();
        let before = s.keybinds_generation();
        s.run(
            "assert(select('#', SetMouselookOverrideBinding('button1', 'MOVEFORWARD')) == 0) \
             SetMouselookOverrideBinding('BUTTON2', 'MOVEBACKWARD') \
             SetMouselookOverrideBinding('BUTTON2', nil)",
        )
        .unwrap();
        assert!(s.keybinds_generation() > before);
        assert_eq!(
            s.mouselook_binding_keys(),
            vec![("BUTTON1".to_string(), "MOVEFORWARD".to_string())]
        );
        assert!(s.binding_keys().iter().all(|(k, _)| k != "BUTTON1"));
        let e = s.run("SetMouselookOverrideBinding()").unwrap_err();
        assert!(e.to_string().contains("Usage: SetMouselookOverrideBinding"));
        assert!(!s.mouselooking());
        s.set_turn_or_action_held(true);
        assert!(s.mouselooking());
    }
}
