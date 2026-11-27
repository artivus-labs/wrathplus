//! Lua globals whose whole body pushes one catalog id at `CGGameUI::DisplayError`'s dispatcher.
//! Engine-side refusals share the [`Model::ui_errors`] queue, which the app resolves against
//! GlobalStrings and fires as `UI_ERROR_MESSAGE` (event `0xe0`), the reference's route; system
//! lines with no catalog row share the unkeyed [`SystemLines`] queue.
//!
//! `NotWhileDeadError` (`0x48d340`, registered at `0x83e398`) is `push 0x7e; call 0x496720`: no
//! argument, no dead check, no return, and a silent catalog row (`0xb4be70`). FrameXML decides
//! when to call it (`UIParent.lua:663-666`, `ContainerFrame.lua:147` and `:190`).

use mlua::Lua;

use super::model::Model;

/// Catalog id `0x7e`'s GlobalStrings key ("You can't do that when you're dead.").
const NOT_WHILE_DEAD_KEY: &str = "ERR_PLAYER_DEAD";

/// The system lines a binding prints with no catalog row, `0x509dd0(text, 0)`, for the app to
/// raise as full `CHAT_MSG_SYSTEM` events: the stock handler reads more than `arg1`.
#[derive(Default)]
struct SystemLines(Vec<String>);

/// Queue one already-resolved system line.
pub(super) fn system_line(lua: &Lua, text: String) {
    if let Some(mut q) = lua.app_data_mut::<SystemLines>() {
        q.0.push(text);
    }
}

impl super::UiScript {
    /// Drain the system lines bindings printed since the last call.
    pub fn take_system_lines(&mut self) -> Vec<String> {
        self.lua
            .app_data_mut::<SystemLines>()
            .map_or_else(Vec::new, |mut q| std::mem::take(&mut q.0))
    }
}

/// Register the error-display globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(SystemLines::default());
    lua.globals().set(
        "NotWhileDeadError",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.ui_errors.push(NOT_WHILE_DEAD_KEY);
            Ok(())
        })?,
    )
}
