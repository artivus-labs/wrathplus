//! The VM's hooks for an external agent (the client's control port, `wrathplus-agent`): an event
//! tap that copies every event the UI fires, and an input switch that lets movement verbs run from
//! any Lua, not only from inside a key binding. Both are off unless an agent launch turns them on,
//! and both live in their own `app_data`, so a VM without an agent pays one lookup per event.

use mlua::Lua;

use super::{ScriptValue, UiScript};

/// The most events held between drains. A drain runs every frame, so this only bounds a stall.
const TAP_LIMIT: usize = 20_000;

#[derive(Default)]
struct AgentHooks {
    /// `Some` while the tap is on: events fired since the last drain, in firing order.
    tap: Option<Vec<(String, Vec<ScriptValue>)>>,
    /// Events dropped because the tap was full, since the last drain.
    dropped: u64,
    /// Movement verbs pass their hardware-event gate from any Lua.
    input_open: bool,
}

/// Copy a fired event into the tap, when one is on.
pub(super) fn tap_event(lua: &Lua, event: &str, args: &[ScriptValue]) {
    let Some(mut hooks) = lua.app_data_mut::<AgentHooks>() else {
        return;
    };
    let hooks = &mut *hooks;
    if let Some(tap) = hooks.tap.as_mut() {
        if tap.len() < TAP_LIMIT {
            tap.push((event.to_string(), args.to_vec()));
        } else {
            hooks.dropped += 1;
        }
    }
}

/// Whether an agent opened the movement gate (see [`UiScript::set_agent_input`]).
pub(super) fn input_open(lua: &Lua) -> bool {
    lua.app_data_ref::<AgentHooks>()
        .is_some_and(|hooks| hooks.input_open)
}

/// Events taken from the tap, and how many were dropped since the last take.
pub struct TappedEvents {
    pub events: Vec<(String, Vec<ScriptValue>)>,
    pub dropped: u64,
}

impl UiScript {
    fn agent_hooks(&self) -> mlua::AppDataRefMut<'_, AgentHooks> {
        if self.lua().app_data_ref::<AgentHooks>().is_none() {
            self.lua().set_app_data(AgentHooks::default());
        }
        self.lua()
            .app_data_mut::<AgentHooks>()
            .expect("agent hooks just set")
    }

    /// Turn the event tap on or off. Off discards anything not yet taken.
    pub fn set_event_tap(&self, on: bool) {
        let mut hooks = self.agent_hooks();
        match (on, hooks.tap.is_some()) {
            (true, false) => hooks.tap = Some(Vec::new()),
            (false, true) => hooks.tap = None,
            _ => {}
        }
    }

    /// Take the events fired since the last take (empty while the tap is off).
    pub fn take_tapped_events(&self) -> TappedEvents {
        let mut hooks = self.agent_hooks();
        let events = hooks.tap.as_mut().map(std::mem::take).unwrap_or_default();
        TappedEvents {
            events,
            dropped: std::mem::take(&mut hooks.dropped),
        }
    }

    /// Let movement verbs (`MoveForwardStart`, `JumpOrAscendStart`, …) run from any Lua, as an
    /// agent's scripts need; stock gates them on a hardware event (a key binding body).
    pub fn set_agent_input(&self, open: bool) {
        self.agent_hooks().input_open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_copies_fired_events_only_while_on() {
        let mut script = UiScript::new().unwrap();
        script.fire_event("PLAYER_LEVEL_UP", vec![ScriptValue::Int(2)]);
        assert!(script.take_tapped_events().events.is_empty());
        script.set_event_tap(true);
        script.fire_event("PLAYER_LEVEL_UP", vec![ScriptValue::Int(3)]);
        script.fire_event("CHAT_MSG_SAY", vec![ScriptValue::Str("hi".into())]);
        let taken = script.take_tapped_events();
        assert_eq!(taken.dropped, 0);
        assert_eq!(
            taken.events,
            vec![
                ("PLAYER_LEVEL_UP".to_string(), vec![ScriptValue::Int(3)]),
                (
                    "CHAT_MSG_SAY".to_string(),
                    vec![ScriptValue::Str("hi".into())]
                ),
            ]
        );
        assert!(script.take_tapped_events().events.is_empty());
        script.set_event_tap(false);
        script.fire_event("PLAYER_LEVEL_UP", vec![]);
        assert!(script.take_tapped_events().events.is_empty());
    }

    #[test]
    fn agent_input_opens_the_movement_gate() {
        let mut script = UiScript::new().unwrap();
        script.run("MoveForwardStart()").unwrap();
        assert!(
            script.take_binding_input().is_empty(),
            "stock: refused outside a binding"
        );
        script.set_agent_input(true);
        script.run("MoveForwardStart()").unwrap();
        assert!(!script.take_binding_input().is_empty());
    }
}
