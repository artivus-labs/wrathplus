//! Native server-module catalog UI. Selection changes the next build, never a running realm.
mod actions;
mod presets;
mod standard;
use crate::{modules, style, Manager, Message};
#[cfg(test)]
pub(crate) use actions::select_modules;
use iced::widget::{button, checkbox, column, container, row, text, text_input, Column};
use iced::{Element, Fill};
#[cfg(test)]
pub(crate) use presets::current as current_type;
pub(crate) use presets::Preset;
pub(crate) use standard::STANDARD;

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
    /// Add one of the standard modules (index into `STANDARD`) from its GitHub repository or
    /// its folder in this checkout.
    AddStandard(usize),
    OpenRepository(&'static str),
    Toggle(usize, bool),
    /// Select a server type's modules for the next build, adding standard ones it lacks.
    Preset(Preset),
    Finished(Result<String, String>),
}
impl Manager {
    pub(crate) fn module_page(&self) -> Element<'_, Message> {
        let state = &self.module_state;
        column![
            text("Server modules").size(22),
            text("Choose the modules included in your next server image. Changes do not affect a running realm: rebuild, then restart. Database migrations may run on startup; disabling a module does not remove its database tables.").size(13).color(style::MUTED),
            text(&state.notice).size(13).color(style::GOLD),
            text("STANDARD MODULES").size(11).color(style::MUTED),
            self.module_cards(true),
            text("CUSTOM MODULES").size(11).color(style::MUTED),
            text("Any other AzerothCore module: a GitHub repository or a module folder on this computer. Only build code you trust.").size(13).color(style::MUTED),
            row![text_input("GitHub owner/repository, HTTPS URL or local module folder", &state.source).on_input(|v| Message::Modules(Input::Source(v))).padding(10), button(if state.busy { "Importing…" } else { "Add source" }).on_press_maybe((!state.busy).then_some(Message::Modules(Input::Register))), button("Refresh").on_press_maybe((!state.busy).then_some(Message::Modules(Input::Refresh)))].spacing(10),
            self.module_cards(false),
            button(self.server_state.module_build_label()).on_press_maybe(self.server_state.module_build_input(!state.busy).map(Message::Server)),
            self.server_state.build_feedback_view(),
            text("Builds can take over an hour. Rebuilding does not restart a running server.").size(13).color(style::MUTED),
        ].spacing(14).into()
    }

    /// One card per module: the standard set (added or not), or the custom entries.
    fn module_cards(&self, standard: bool) -> Element<'_, Message> {
        let catalog = match modules::Catalog::load(&self.config, self.server_state.module_source())
        {
            Ok(catalog) => catalog,
            Err(error) => {
                return text(format!(
                    "{error}. Choose your core checkout on Server setup first."
                ))
                .into()
            }
        };
        let editable = !self.module_state.busy && self.server_state.module_editable();
        let entries = catalog.entries();
        let mut cards = Column::new().spacing(12);
        if standard {
            for (slot, module) in STANDARD.iter().enumerate() {
                let found = entries
                    .iter()
                    .enumerate()
                    .find(|(_, e)| e.name == module.name);
                let control: Element<'_, Message> = match found {
                    Some((index, entry)) => self.include_box(index, entry.enabled, editable),
                    None => button(if self.module_state.busy {
                        "Importing…"
                    } else {
                        "Add"
                    })
                    .style(style::primary)
                    .on_press_maybe(editable.then_some(Message::Modules(Input::AddStandard(slot))))
                    .into(),
                };
                let (link, unadded): (Element<'_, Message>, _) = match module.origin {
                    standard::Origin::Github(repository) => (
                        button(text(format!("github.com/{repository}")).size(12))
                            .style(link)
                            .padding(0)
                            .on_press(Message::Modules(Input::OpenRepository(repository)))
                            .into(),
                        "Not added yet · imported from GitHub when you press Add",
                    ),
                    standard::Origin::Bundled(_) => (
                        text("Made for WrathPlus").size(12).into(),
                        "Not added yet · ships with WrathPlus",
                    ),
                };
                let source = found.map_or_else(
                    || unadded.to_owned(),
                    |(_, e)| standard::source_label(&e.source),
                );
                let mut body = column![row![
                    column![
                        text(module.title).size(18),
                        text(module.about).size(13).color(style::MUTED),
                        row![link, text(source).size(12).color(style::MUTED)].spacing(14),
                    ]
                    .spacing(6)
                    .width(Fill),
                    control
                ]
                .spacing(16)
                .align_y(iced::Center),]
                .spacing(10);
                if let Some((_, entry)) = found {
                    body = body.push(self.module_config_rows(entry));
                }
                cards = cards.push(card(body));
            }
        } else {
            for (index, entry) in entries.iter().enumerate() {
                if standard::is_standard(&entry.name) {
                    continue;
                }
                cards = cards.push(card(
                    column![
                        row![
                            column![
                                text(entry.name.clone()).size(18),
                                text(standard::source_label(&entry.source))
                                    .size(12)
                                    .color(style::MUTED),
                            ]
                            .spacing(6)
                            .width(Fill),
                            self.include_box(index, entry.enabled, editable)
                        ]
                        .spacing(16)
                        .align_y(iced::Center),
                        self.module_config_rows(entry)
                    ]
                    .spacing(10),
                ));
            }
        }
        cards.into()
    }

    fn include_box(&self, index: usize, enabled: bool, editable: bool) -> Element<'_, Message> {
        let mut check = checkbox(enabled).label("Include in next build");
        if editable {
            check = check.on_toggle(move |v| Message::Modules(Input::Toggle(index, v)));
        }
        check.into()
    }
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(16)
        .width(Fill)
        .style(style::panel)
        .into()
}

fn link(theme: &iced::Theme, status: button::Status) -> button::Style {
    let mut style = button::text(theme, status);
    style.text_color = if matches!(status, button::Status::Hovered) {
        style::TEXT
    } else {
        style::ICE
    };
    style
}
