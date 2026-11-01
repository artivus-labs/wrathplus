use super::*;
use crate::{modules::Entry, style, Manager, Message};
use iced::{
    widget::{button, column, row, space, text, Column},
    Element, Fill,
};
impl Manager {
    pub(crate) fn module_config_rows(&self, entry: &Entry) -> Element<'_, Message> {
        let mut rows = Column::new().spacing(8);
        match discover(&self.config, self.server_state.module_source(), entry) {
            Ok(files) => {
                for file in files {
                    rows = rows.push(
                        row![
                            column![
                                text(file.name.clone()).size(14),
                                text("Edits use a local copy. Original template is kept.")
                                    .size(12)
                                    .color(style::MUTED)
                            ]
                            .spacing(4),
                            space().width(Fill),
                            button("Edit config").on_press_maybe(
                                (!self.module_state.busy).then_some(Message::ModuleConfig(
                                    Input::Open {
                                        module: file.module,
                                        name: file.name
                                    }
                                ))
                            )
                        ]
                        .spacing(12)
                        .align_y(iced::Center),
                    );
                }
            }
            Err(error) => {
                rows = rows.push(
                    text(format!("Config unavailable: {error}"))
                        .size(12)
                        .color(style::GOLD),
                );
            }
        }
        rows.into()
    }
}
