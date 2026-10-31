use super::{style, AddonScope, Manager, Message, Page};
use iced::widget::{button, checkbox, column, container, row, text, text_input, Column};
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
fn metric(
    label: &'static str,
    value: &'static str,
    detail: &'static str,
) -> Element<'static, Message> {
    card(
        column![
            text(label).size(11).color(style::MUTED),
            text(value).size(24),
            text(detail).size(11).color(style::MUTED)
        ]
        .spacing(8),
    )
}
fn launch_row(
    name: &'static str,
    status: &'static str,
    detail: &'static str,
    launch: &'static str,
    page: Page,
) -> Element<'static, Message> {
    row![
        text(name).size(17).width(90),
        text(status).size(13).color(style::GOLD).width(95),
        text(detail).size(12).color(style::MUTED).width(Fill),
        button(text(launch))
            .style(style::primary)
            .padding([8, 16])
            .on_press(Message::Notice(
                "launch/start is a stub; no process was started."
            )),
        button("Setup")
            .padding([8, 12])
            .on_press(Message::Navigate(page))
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
                        "Stopped",
                        "Local AzerothCore · sample instance",
                        "Start",
                        Page::Server
                    ),
                    text("States: Uninstalled / Stopped / Running · sample values, not detected.")
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
        .style(style::primary)
        .padding([10, 18]);
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
                    text("You’ll need your own World of Warcraft 3.3.5a installation. Retail and current Classic installations won’t work.").size(13),
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
                    text("Already have a client? Save your settings and launch it. Otherwise, build one below. The first build may take a while.").size(13),
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

    pub(super) fn packages(&self) -> Element<'_, Message> {
        let mut tabs = iced::widget::Row::new().spacing(8);
        for scope in AddonScope::ALL {
            tabs = tabs.push(
                button(text(scope.label()))
                    .padding([8, 12])
                    .style(move |theme, status| {
                        style::nav(theme, status, self.addon_scope == scope)
                    })
                    .on_press(Message::AddonScope(scope)),
            );
        }
        let samples = [
            (
                "Readable UI",
                AddonScope::Client,
                "Client addon · no server module required",
            ),
            (
                "Playerbots",
                AddonScope::Server,
                "AzerothCore module · compatible core build required",
            ),
            (
                "MultiBot UI + Playerbots",
                AddonScope::Both,
                "Requires MultiBot client addon AND Playerbots server module",
            ),
        ];
        let mut list = Column::new().spacing(10);
        for (index, (name, scope, detail)) in samples.into_iter().enumerate() {
            if scope != self.addon_scope
                || !name.to_lowercase().contains(&self.search.to_lowercase())
            {
                continue;
            }
            list = list.push(card(
                row![
                    checkbox(self.addons[index]).on_toggle(move |v| Message::Addon(index, v)),
                    column![
                        text(name).size(17),
                        text(detail).size(12).color(style::MUTED),
                        text("Sample package · compatibility not checked")
                            .size(11)
                            .color(style::GOLD)
                    ]
                    .spacing(6)
                    .width(Fill),
                    action(
                        "Details",
                        "package requirements and provenance are not connected."
                    )
                ]
                .spacing(14)
                .align_y(iced::Center),
            ));
        }
        column![
            tabs,
            text(match self.addon_scope {
                AddonScope::Client => "Client addons",
                AddonScope::Server => "AzerothCore modules · build / migration workflow",
                AddonScope::Both => "Coupled packages · both sides required",
            })
            .size(13)
            .color(style::MUTED),
            row![
                text_input("Search sample packages…", &self.search)
                    .on_input(Message::Search)
                    .padding(10),
                action("Import…", "package import is not connected.")
            ]
            .spacing(10),
            list,
            card(
                column![
                    text("Repository").size(17),
                    row![
                        text_input("https://github.com/owner/repository", &self.repo)
                            .on_input(Message::Repo)
                            .padding(10),
                        action(
                            "Inspect",
                            "repository inspection is not connected; no network request sent."
                        )
                    ]
                    .spacing(10)
                ]
                .spacing(12)
            ),
        ]
        .spacing(14)
        .into()
    }

    pub(super) fn server(&self) -> Element<'_, Message> {
        column![
            card(column![launch_row("Server", "Stopped", "Local AzerothCore · sample instance", "Start", Page::Server),
                row![action("Stop", "server stop is not connected."), action("Restart", "server restart is not connected."), action("Logs", "server logs are not connected."), action("Attach existing…", "server adoption is not connected."), action("Create…", "realm setup is not connected.")].spacing(8),
            ].spacing(14)),
            row![metric("USERS ONLINE", "—", "Human players · not queried"), metric("PLAYERBOTS ACTIVE", "—", "Available when module installed"), metric("LLM AGENTS", "—", "Explicit headless-client flag · proposed")].spacing(10),
            card(column![text("Players & agents").size(18),
                row![text_input("Find player or agent…", &self.search).on_input(Message::Search).padding(10), action("Find players", "player search is not connected."), action("Find agents", "agent search is not connected.")].spacing(8),
                row![text("IDENTITY").size(11).width(Fill), text("TYPE").size(11).width(130), text("STATUS").size(11).width(100)].spacing(12),
                population_row("Example player", "Human", "Offline"),
                population_row("Example bot", "Playerbot", "Unknown"),
                population_row("Example agent", "LLM agent", "Unknown"),
                text("Sample identities only. Agents use a proposed explicit flag, never inferred from names.").size(11).color(style::MUTED),
                row![action("Set GM…", "GM changes require selected identity and confirmation; stub only."), action("Inspect", "identity inspection is not connected."), action("Kick…", "kick is not connected."), action("Playerbot controls", "module-specific bot controls are not connected.")].spacing(8),
            ].spacing(12)),
            card(column![text("Maintenance").size(18),
                row![action("Check updates", "update inspection is not connected."), action("Build", "server build is not connected."), action("Migrations…", "migration planning is not connected."), action("Backup", "database backup is not connected."), action("Restore…", "restore requires backup and confirmation; stub only.")].spacing(8),
                button("AzerothCore modules →").on_press(Message::AddonScope(AddonScope::Server)),
                text("Pinned sources · migration plan · backup · downtime review before applying.").size(11).color(style::MUTED),
            ].spacing(12)),
        ].spacing(14).into()
    }

    pub(super) fn profiles(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(10);
        for name in ["Local adventures", "Classic essentials", "Bot workshop"] {
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
            text("Sample profiles · in-memory selection only")
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
        card(events)
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
fn population_row(
    name: &'static str,
    kind: &'static str,
    status: &'static str,
) -> Element<'static, Message> {
    row![
        text(name).size(13).width(Fill),
        text(kind).size(12).color(style::ICE).width(130),
        text(status).size(12).color(style::MUTED).width(100)
    ]
    .spacing(12)
    .into()
}
