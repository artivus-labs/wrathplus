use super::{style, Manager, Message, Page};
use iced::widget::{button, column, container, row, text, text_input, Column};
use iced::{Element, Fill};

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(16)
        .width(Fill)
        .style(style::panel)
        .into()
}
fn action(label: &'static str, note: &'static str) -> Element<'static, Message> {
    button(text(label).size(13))
        .padding([8, 12])
        .on_press(Message::Notice(note))
        .into()
}
fn launch_row<'a>(
    name: &'static str,
    status: &'static str,
    detail: &'a str,
    launch: &'static str,
    start: Option<Message>,
    page: Page,
) -> Element<'a, Message> {
    row![
        text(name).size(17).width(90),
        text(status).size(13).color(style::GOLD).width(95),
        text(detail).size(12).color(style::MUTED).width(Fill),
        button(text(launch))
            .style(style::primary)
            .on_press_maybe(start),
        button("Setup").on_press(Message::Navigate(page))
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}

impl Manager {
    pub(super) fn play(&self) -> Element<'_, Message> {
        column![
            card(
                column![
                    row![
                        text("QUICK LAUNCH").size(11).color(style::MUTED),
                        iced::widget::Space::new().width(Fill),
                        action("Start server + client", "combined launch is not connected.")
                    ]
                    .align_y(iced::Center),
                    self.client_launch_row(),
                    text(
                        super::setup::discover(&self.config)
                            .map(|p| format!("Detected build: {}", p.display()))
                            .unwrap_or_else(|| "No playable build detected".into())
                    )
                    .size(11)
                    .color(style::MUTED),
                    launch_row(
                        "Server",
                        self.server_state.label(),
                        self.server_state.detail(),
                        self.server_state.control_label(),
                        self.server_state.control_input().map(Message::Server),
                        Page::Server
                    ),
                    text("Manage server files, builds and logs in Setup.")
                        .size(11)
                        .color(style::MUTED),
                ]
                .spacing(16)
            ),
            card(
                column![
                    text("CONNECTION").size(11).color(style::MUTED),
                    row![
                        text("Realm").width(90),
                        text(&self.host).width(Fill),
                        button("Client settings").on_press(Message::Navigate(Page::Client)),
                        button("Server controls").on_press(Message::Navigate(Page::Server))
                    ]
                    .spacing(12)
                    .align_y(iced::Center),
                    row![
                        text("Addons").width(90),
                        text("Client / server / dependencies")
                            .color(style::MUTED)
                            .width(Fill),
                        button("Manage").on_press(Message::Navigate(Page::Addons))
                    ]
                    .spacing(12)
                    .align_y(iced::Center),
                ]
                .spacing(14)
            ),
        ]
        .spacing(14)
        .into()
    }

    fn client_launch_row(&self) -> Element<'_, Message> {
        row![
            text("Client").size(17).width(90),
            text(self.status()).color(style::GOLD).width(95),
            text("3.3.5a · Build 12340").size(12).width(Fill),
            self.client_control(),
            button("Setup").on_press(Message::Navigate(Page::Client))
        ]
        .spacing(12)
        .align_y(iced::Center)
        .into()
    }

    pub(super) fn client(&self) -> Element<'_, Message> {
        let build = button(if self.busy {
            "Building…"
        } else {
            "Build client"
        })
        .style(style::primary);
        let build = if self.build_ready() {
            build.on_press(Message::Build)
        } else {
            build
        };
        column![
            card(
                column![
                    text("Set up your client").size(18),
                    self.client_launch_row(),
                    text("1 · Your game installation").size(16),
                    text("Needs your own World of Warcraft 3.3.5a install.").size(13),
                    text("2 · Select your game files").size(16),
                    row![
                        text_input("3.3.5a Data folder", &self.assets)
                            .on_input(Message::Assets)
                            .padding(10),
                        button(if self.choosing { "Choosing…" } else { "Browse…" }).on_press_maybe((!self.choosing && !self.busy).then_some(Message::BrowseAssets)),
                        button("Check files").on_press(Message::Validate)
                    ]
                    .spacing(10),
                    row![text("Locale").width(90), text_input("enUS", &self.locale).on_input(Message::Locale).padding(10).width(120),
                        text(if self.validated { "Files ready" } else { "Select and check your game files" }).size(12).color(style::MUTED)].spacing(12).align_y(iced::Center),
                    text("3 · Build / client directory").size(16),
                    row![
                        text_input("Build or existing client folder", &self.build_directory).padding(10)
                            .on_input_maybe((!self.busy && !self.checking && self.child.is_none()).then_some(Message::BuildDirectory)),
                        button("Browse…").on_press_maybe((!self.choosing && !self.busy && !self.checking && self.child.is_none()).then_some(Message::BrowseBuildDirectory))
                    ].spacing(10),
                    text(super::setup::discover(&self.config).map(|p| format!("Detected client: {}", p.display())).unwrap_or_else(|| "No playable client found at this location".into())).size(11).color(style::MUTED),
                    button("Save settings").on_press_maybe((!self.busy && !self.checking && self.child.is_none()).then_some(Message::Validate)),
                    self.prerequisite_list(),

                    text("4 · Build your client").size(16),
                    text("Already have a client? Launch it. Otherwise build one below.").size(13),
                    build,
                    self.build_feedback(),
                ]
                .spacing(14)
            ),
            card(
                column![
                    text("Configuration").size(18),
                    text("Auth server").size(12).color(style::MUTED),
                    text_input("host:port", &self.host)
                        .on_input(Message::Host)
                        .padding(10),
                    text("Classic is the default. Experience and graphics settings are configured in-game.").size(12).color(style::MUTED),
                    row![
                        action("Graphics", "graphics settings are a stub."),
                        action("Audio & input", "audio/input settings are a stub."),
                        action("Logs", "client logs are not connected."),
                        self.client_control()
                    ]
                    .spacing(8),
                ]
                .spacing(14)
            ),
        ]
        .spacing(14)
        .into()
    }

    pub(super) fn profiles(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(10);
        for name in crate::profiles::NAMES {
            list = list.push(card(
                row![
                    text(name).size(17).width(Fill),
                    button(if self.profile == name {
                        "Selected"
                    } else {
                        "Select"
                    })
                    .on_press(Message::Profile(name.into()))
                ]
                .align_y(iced::Center),
            ));
        }
        column![
            text("Each profile has its own server modules and settings.")
                .size(12)
                .color(style::MUTED),
            list,
            row![
                action("New", "profile creation is not connected."),
                action("Duplicate", "duplication is not connected."),
                action("Import / export", "profile transfer is not connected.")
            ]
            .spacing(8)
        ]
        .spacing(14)
        .into()
    }
    pub(super) fn activity(&self) -> Element<'_, Message> {
        let mut events = Column::new().spacing(10);
        for (index, event) in self.events.iter().enumerate().rev() {
            events = events.push(
                row![
                    text(format!("{:02}", index + 1))
                        .size(12)
                        .color(style::GOLD),
                    text(event).size(13)
                ]
                .spacing(14),
            );
        }
        column![
            text("Operation history and logs").size(18),
            card(column![text("Manager history").size(18), events].spacing(12)),
            card(column![text("Client build").size(18), self.build_feedback()].spacing(12)),
            self.server_logs(),
        ]
        .spacing(14)
        .into()
    }
    pub(super) fn settings(&self) -> Element<'_, Message> {
        card(
            column![
                text("Manager settings").size(18),
                text("Dark appearance · system text scaling").size(13),
                text("No persistence or credentials connected.")
                    .size(12)
                    .color(style::MUTED),
                action("Diagnostics", "diagnostics are not connected.")
            ]
            .spacing(14),
        )
    }
}
