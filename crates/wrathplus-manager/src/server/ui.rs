use super::{worker, Action, Input};
use crate::{style, Manager, Message};
use iced::widget::{
    button, column, container, progress_bar, row, scrollable, space, text, text_input,
};
use iced::{Element, Fill, Length};

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(18)
        .width(Fill)
        .style(style::panel)
        .into()
}

impl Manager {
    pub(crate) fn server_logs(&self) -> Element<'_, Message> {
        let state = &self.server_state;
        let mut panel = column![
            text("Server logs").size(18),
            text(state.detail()).size(12),
            button(if state.show_logs {
                "Hide output"
            } else {
                "Show output"
            })
            .on_press(Message::Server(Input::ToggleLogs)),
        ]
        .spacing(10);
        if state.show_logs {
            if let Some(path) = &state.log_path {
                panel = panel.push(text(format!("Saved log: {}", path.display())).size(12));
            }
            panel = panel.push(
                scrollable(
                    text(if state.log.is_empty() {
                        "No server output yet."
                    } else {
                        &state.log
                    })
                    .font(iced::Font::MONOSPACE)
                    .size(12),
                )
                .height(250),
            );
        }
        card(panel)
    }
    pub(crate) fn server(&self) -> Element<'_, Message> {
        let state = &self.server_state;
        let editable = !state.locked() && !state.choosing;
        let action = |label, action| {
            button(label).on_press_maybe(editable.then_some(Message::Server(Input::Run(action))))
        };
        let mut status = column![
            row![
                column![text("Local Playerbots server").size(20), text(state.label()).color(style::GOLD)].spacing(6),
                space().width(Fill),
                button(state.control_label()).style(style::primary)
                    .on_press_maybe(state.control_input().map(Message::Server)),
                action("Refresh", Action::Status),
                button("View logs").on_press(Message::Navigate(crate::Page::Activity)),
            ].spacing(12).align_y(iced::Center),
            text("Play locally with computer-controlled companions. Stopping the server keeps your characters and progress.").size(13).color(style::MUTED),
            text(state.detail()).size(12).color(style::MUTED),
            action("Start / check LLM bridge", Action::Start),
            button("Use this server for the client").on_press(Message::Server(Input::Connect)),
        ].spacing(12);
        if state.busy && !matches!(state.active, Some(Action::Status)) {
            status = status.push(text(&state.stage).size(14));
            if let Some(started) = state.started {
                let elapsed = started.elapsed().as_secs();
                status = status.push(
                    text(format!(
                        "Elapsed {}h {:02}m {:02}s",
                        elapsed / 3600,
                        elapsed / 60 % 60,
                        elapsed % 60
                    ))
                    .size(12),
                );
            }
            let left = state.pulse.min(20 - state.pulse) + 1;
            status = status.push(
                row![
                    space().width(Length::FillPortion(left)),
                    container(space().height(7))
                        .width(Length::FillPortion(3))
                        .style(|_| container::Style::default().background(style::ICE)),
                    space().width(Length::FillPortion(12 - left))
                ]
                .width(Fill),
            );
            if matches!(state.active, Some(Action::Build | Action::ForceBuild)) {
                if let Some((done, total)) = worker::count(&state.log) {
                    status = status
                        .push(text(format!("Build tasks: {done} / {total}")).size(12))
                        .push(progress_bar(0.0..=total as f32, done as f32).girth(5));
                }
            }
        }
        if let Some(error) = &state.error {
            status = status.push(text(error).color(style::GOLD));
        }
        let settings = column![
            text("Server files (advanced)").size(18),
            text("Set up by the steps above. Already have a clean AzerothCore Playerbot checkout or extracted server data? Enter their folders here instead.").size(13).color(style::MUTED),
            text("Source folder").size(13),
            text_input("AzerothCore source folder", &state.source)
                .on_input_maybe(editable.then_some(|v| Message::Server(Input::Source(v))))
                .padding(10),
            text("Server data folder").size(13),
            row![
                text_input("Select server data", &state.data)
                    .on_input_maybe(editable.then_some(|v| Message::Server(Input::Data(v))))
                    .padding(10),
                button("Browse…")
                    .on_press_maybe(editable.then_some(Message::Server(Input::Browse)))
            ]
            .spacing(10),
            action("Save settings", Action::Save),
        ]
        .spacing(12);
        let mut build = column![
            text("Server build").size(18),
            text("The first build can take over an hour. Rebuilding does not restart a running server.").size(13).color(style::MUTED),
        ].spacing(12);
        if let Some(warning) = state.build.warning() {
            build = build.push(text(warning).size(13).color(style::GOLD));
        }
        build = build.push(
            button(state.build.label())
                .on_press_maybe(state.build.input(editable).map(Message::Server)),
        );
        let mut page = iced::widget::Column::new().spacing(14);
        if let Some(guide) = self.server_guide() {
            page = page.push(guide);
        }
        page.push(card(status))
            .push(self.transfer_card())
            .push(card(self.server_types()))
            .push(card(build))
            .push(state.build_feedback_view())
            .push(card(settings))
            .into()
    }
}
