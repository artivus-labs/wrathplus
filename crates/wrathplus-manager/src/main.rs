//! Native client setup and launcher; client addon source registration; server/profile management remain previews.
mod addons;
mod build_feedback;
mod cli;
mod client_actions;
mod pages;
mod preflight;
mod prerequisites;
mod setup;
mod style;
mod update;

use std::process::Child;

use iced::widget::{button, column, container, pick_list, row, scrollable, svg, text, Column};
use iced::{Element, Fill, Size, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Play,
    Client,
    Addons,
    Server,
    Profiles,
    Activity,
    Settings,
}

impl Page {
    const ALL: [Self; 7] = [
        Self::Play,
        Self::Client,
        Self::Server,
        Self::Addons,
        Self::Profiles,
        Self::Activity,
        Self::Settings,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Client => "Client",
            Self::Addons => "Addons",
            Self::Server => "Server",
            Self::Profiles => "Profiles",
            Self::Activity => "Activity",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddonScope {
    Client,
    Server,
    Both,
}

impl AddonScope {
    const ALL: [Self; 3] = [Self::Client, Self::Server, Self::Both];
    fn label(self) -> &'static str {
        match self {
            Self::Client => "Client only",
            Self::Server => "Server only",
            Self::Both => "Client + server",
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    Profile(String),
    Assets(String),
    Host(String),
    Repo(String),
    Search(String),
    AddonScope(AddonScope),
    Addon(usize, bool),
    RefreshAddons,
    AddonPath(String),
    AddAddonPath,
    BrowseAddons,
    AddonFolderChosen(Option<std::path::PathBuf>),
    Notice(&'static str),
    Locale(String),
    BuildDirectory(String),
    BrowseBuildDirectory,
    BuildDirectoryChosen(Option<std::path::PathBuf>),
    BrowseAssets,
    FolderChosen(Option<std::path::PathBuf>),
    Validate,
    Recheck,
    PreflightFinished(preflight::Report),
    Build,
    BuildFinished(Result<String, String>),
    ToggleBuildLog,
    CopyBuildLog,
    Launch,
    StopClient,
    Poll,
}

struct Manager {
    page: Page,
    profile: String,
    assets: String,
    build_directory: String,
    host: String,
    repo: String,
    search: String,
    addon_scope: AddonScope,
    addon_settings: addons::Settings,
    addon_entries: Vec<addons::Entry>,
    addon_path: String,
    notice: String,
    events: Vec<String>,
    config: setup::Config,
    locale: String,
    validated: bool,
    preflight: Option<preflight::Report>,
    checking: bool,
    busy: bool,
    build_feedback: build_feedback::Feedback,
    choosing: bool,
    child: Option<Child>,
    launched_config: Option<setup::Config>,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            page: Page::Play,
            profile: "Local adventures".into(),
            assets: String::new(),
            build_directory: setup::Config::defaults()
                .target
                .to_string_lossy()
                .into_owned(),
            host: "127.0.0.1:3724".into(),
            repo: String::new(),
            search: String::new(),
            addon_scope: AddonScope::Client,
            addon_settings: addons::Settings::default(),
            addon_entries: Vec::new(),
            addon_path: String::new(),
            notice: "Choose your legally obtained 3.3.5a assets to set up the client.".into(),
            events: vec!["Manager opened. Server and profiles remain previews.".into()],
            config: setup::Config::defaults(),
            locale: "enUS".into(),
            validated: false,
            preflight: None,
            checking: false,
            busy: false,
            build_feedback: build_feedback::Feedback::default(),
            choosing: false,
            child: None,
            launched_config: None,
        }
    }
}

impl Manager {
    fn record(&mut self, value: String) {
        self.notice = value.clone();
        self.events.push(value);
    }

    fn sync_config(&mut self) {
        self.config.data = self.assets.clone().into();
        self.config.locale = self.locale.clone();
        self.config.server = self.host.clone();
    }

    fn status(&self) -> &'static str {
        if self.busy {
            "Building"
        } else if self.child.is_some() {
            "Running"
        } else if setup::discover(&self.config).is_some() {
            "Available"
        } else {
            "Not built"
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let mut nav = Column::new().spacing(6);
        for page in Page::ALL {
            nav = nav.push(
                button(text(page.label()).size(15))
                    .width(Fill)
                    .padding([13, 18])
                    .style(move |theme, status| style::nav(theme, status, self.page == page))
                    .on_press(Message::Navigate(page)),
            );
            if page == Page::Addons {
                for scope in AddonScope::ALL {
                    nav = nav.push(
                        button(text(scope.label()).size(12))
                            .width(Fill)
                            .padding([7, 28])
                            .style(move |theme, status| {
                                style::nav(
                                    theme,
                                    status,
                                    self.page == Page::Addons && self.addon_scope == scope,
                                )
                            })
                            .on_press(Message::AddonScope(scope)),
                    );
                }
            }
        }
        let sidebar = container(
            column![
                svg(svg::Handle::from_memory(
                    include_bytes!("../assets/wrathplus-logo.svg").as_slice()
                ))
                .width(190)
                .height(94),
                nav,
                iced::widget::Space::new().height(Fill),
                text("CLIENT SETUP").size(10).color(style::GOLD),
                text("No live services connected")
                    .size(12)
                    .color(style::MUTED),
                text("0.1 / prototype").size(11).color(style::MUTED),
            ]
            .spacing(24)
            .height(Fill),
        )
        .padding(16)
        .width(230)
        .height(Fill)
        .style(style::sidebar);
        let header = row![
            text(self.page.label()).size(24),
            iced::widget::Space::new().width(Fill),
            column![
                text("ACTIVE PROFILE").size(10).color(style::MUTED),
                pick_list(
                    vec![
                        "Local adventures".to_owned(),
                        "Classic essentials".to_owned(),
                        "Bot workshop".to_owned()
                    ],
                    Some(self.profile.clone()),
                    Message::Profile
                )
                .width(205)
            ]
            .spacing(6),
        ]
        .align_y(iced::Center);
        let body = match self.page {
            Page::Play => self.play(),
            Page::Client => self.client(),
            Page::Addons => self.packages(),
            Page::Server => self.server(),
            Page::Profiles => self.profiles(),
            Page::Activity => self.activity(),
            Page::Settings => self.settings(),
        };
        let content = column![
            header,
            text("Client and addon setup are live · Server and profiles are previews")
                .size(11)
                .color(style::GOLD),
            scrollable(body).height(Fill),
            container(
                row![
                    text("ACTIVITY").size(10).color(style::GOLD),
                    text(&self.notice).size(12),
                    iced::widget::Space::new().width(Fill),
                    button("View").on_press(Message::Navigate(Page::Activity))
                ]
                .spacing(14)
                .align_y(iced::Center)
            )
            .padding(12)
            .width(Fill)
            .style(style::panel)
        ]
        .spacing(14)
        .height(Fill);
        row![
            sidebar,
            container(content).padding(22).width(Fill).height(Fill)
        ]
        .into()
    }
}

fn main() -> iced::Result {
    let options = cli::parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if options.help {
        println!("{}", cli::HELP);
        return Ok(());
    }
    if options.use_existing {
        if let Err(error) = setup::save_config(&options.config, &options.config.state_path()) {
            eprintln!("Existing build setup failed: {error}");
            std::process::exit(1);
        }
        println!(
            "Ready: {}",
            setup::discover(&options.config)
                .expect("validated existing build")
                .display()
        );
        return Ok(());
    }
    if options.doctor {
        let report = preflight::inspect(&options.config);
        println!("{}", report.summary());
        if !report.ready() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if options.validate_only {
        match setup::validate(&options.config) {
            Ok(validated) => println!("{}", validated.summary),
            Err(error) => {
                eprintln!("Validation failed: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if options.setup {
        match setup::build(&options.config) {
            Ok(message) => println!("{message}"),
            Err(error) => {
                eprintln!("Setup failed: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let initial = options.config;
    let icon = ::image::load_from_memory(include_bytes!(
        "../../../packaging/icon/wrathplus-icon-256.png"
    ))
    .ok()
    .and_then(|image| {
        let rgba = image.into_rgba8();
        let (width, height) = rgba.dimensions();
        iced::window::icon::from_rgba(rgba.into_raw(), width, height).ok()
    });
    iced::application(
        move || {
            let mut manager = Manager::default();
            manager.config = initial.clone();
            manager.repo = manager.config.repo.to_string_lossy().into_owned();
            manager.build_directory = manager.config.target.to_string_lossy().into_owned();
            manager.assets = manager.config.data.to_string_lossy().into_owned();
            manager.locale = manager.config.locale.clone();
            manager.host = manager.config.server.clone();
            manager.refresh_addons();
            manager
        },
        Manager::update,
        Manager::view,
    )
    .subscription(|_: &Manager| {
        iced::Subscription::run(|| {
            iced::futures::stream::unfold((), |()| async {
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let _ = sender.send(());
                });
                let _ = receiver.await;
                Some((Message::Poll, ()))
            })
        })
    })
    .title("WrathPlus Manager")
    .theme(|_: &Manager| {
        Theme::custom(
            "WrathPlus",
            iced::theme::Palette {
                background: style::BG,
                text: style::TEXT,
                primary: style::ICE,
                success: style::ICE,
                warning: style::GOLD,
                danger: iced::Color::from_rgb8(222, 114, 114),
            },
        )
    })
    .window(iced::window::Settings {
        size: Size::new(1260.0, 860.0),
        min_size: Some(Size::new(980.0, 680.0)),
        icon,
        ..Default::default()
    })
    .run()
}

#[cfg(test)]
mod tests;
