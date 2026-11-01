//! Native server-module catalog UI. Selection changes the next build, never a running realm.
mod actions;
use crate::{modules, style, Manager, Message};
use iced::widget::{button, checkbox, column, container, row, text, text_input, Column};
use iced::{Element, Fill};

#[derive(Default)]
pub(crate) struct State {
    pub source: String,
    pub busy: bool,
    pub notice: String,
}
#[derive(Debug, Clone)]
pub(crate) enum Input {
    Source(String),
    Refresh,
    Register,
    Toggle(usize, bool),
    Finished(Result<String, String>),
}
impl Manager {
    pub(crate) fn module_page(&self) -> Element<'_, Message> {
        let state = &self.module_state;
        column![
            text("Server modules").size(22),
            text("Choose the modules included in your next server image. Changes do not affect a running realm: rebuild, then restart. Database migrations may run on startup; disabling a module does not remove its database tables.").size(13).color(style::MUTED),
            text("Default single-player foundation").size(18),
            button("MultiBot bridge source").on_press(Message::Modules(Input::Source("Wishmaster117/mod-multibot-bridge".into()))),
            row![button("LLM chatter source").on_press(Message::Modules(Input::Source("Hokken/mod-llm-chatter".into()))), button("Guildhouse source").on_press(Message::Modules(Input::Source("azerothcore/mod-guildhouse".into()))), button("Playerbot guildhouse source").on_press(Message::Modules(Input::Source("DustinHendrickson/mod-player-bot-guildhouse".into())))].spacing(8),
            text("Standard foundation: Playerbots and mod-multibot-bridge for MultiBot controls. Playerbots is selected by default; import the bridge source explicitly if absent. All modules remain optional. Source registration is not proof of inclusion in the running image.").size(13),
            row![text_input("GitHub owner/repository, HTTPS URL or local module folder", &state.source).on_input(|v| Message::Modules(Input::Source(v))).padding(10), button(if state.busy { "Importing…" } else { "Add source" }).on_press_maybe((!state.busy).then_some(Message::Modules(Input::Register))), button("Refresh").on_press_maybe((!state.busy).then_some(Message::Modules(Input::Refresh)))].spacing(10),
            text(&state.notice).size(13).color(style::GOLD),
            self.module_entries(),
            button(self.server_state.module_build_label()).on_press_maybe(self.server_state.module_build_input(!state.busy).map(Message::Server)),
            self.server_state.build_feedback_view(),
            text("LLM Chatter includes a bundled generation service. Save its provider config, then use Start / check LLM bridge on Server setup to provision it; no realm rebuild is needed for the bridge. Status and Activity include bridge health/logs. Chatter's Known bots list uses stored memories, not nearby bots.").size(13).color(style::MUTED),
            text("Builds can take over an hour. Rebuilding does not restart a running server.").size(13).color(style::MUTED),
        ].spacing(14).into()
    }
}
