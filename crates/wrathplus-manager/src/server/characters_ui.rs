//! The Server page's Characters card: list the realm's characters, export one to a `.pdump` file,
//! import a dump into an account. Work runs on a thread; the card never blocks the page.
use super::characters::{self, Roster};
use super::{status::Health, Input};
use crate::{style, Manager, Message};
use iced::widget::{button, column, container, pick_list, row, text, text_input, Column};
use iced::{Element, Fill};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct Transfer {
    roster: Roster,
    loaded: bool,
    busy: bool,
    notice: String,
    file: Option<PathBuf>,
    account: Option<String>,
    rename: String,
}

#[derive(Debug, Clone)]
pub enum TransferInput {
    Refresh,
    Loaded(Result<Roster, String>),
    Export(String),
    ExportTo(String, Option<PathBuf>),
    Choose,
    Chosen(Option<PathBuf>),
    Account(String),
    Rename(String),
    Import,
    Done(Result<String, String>),
}

fn send(input: TransferInput) -> Message {
    Message::Server(Input::Transfer(input))
}

/// Run blocking work on a thread and deliver its result as a message.
fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: fn(Result<T, String>) -> TransferInput,
) -> iced::Task<Message> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err("Character transfer stopped unexpectedly".into()));
        let _ = sender.send(result);
    });
    iced::Task::perform(
        async move {
            receiver
                .await
                .unwrap_or_else(|_| Err("Character transfer stopped".into()))
        },
        move |r| send(done(r)),
    )
}

pub(super) fn race(id: u8) -> &'static str {
    match id {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        10 => "Blood Elf",
        11 => "Draenei",
        _ => "Unknown race",
    }
}

pub(super) fn class(id: u8) -> &'static str {
    match id {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        6 => "Death Knight",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => "Unknown class",
    }
}

impl Manager {
    /// Load the roster once the realm is up, so the card is filled without a click.
    pub(super) fn transfer_poll(&mut self) -> iced::Task<Message> {
        let state = &self.server_state;
        if state.health == Health::Running && !state.transfer.loaded && !state.transfer.busy {
            return self.transfer_update(TransferInput::Refresh);
        }
        iced::Task::none()
    }

    pub(super) fn transfer_update(&mut self, input: TransferInput) -> iced::Task<Message> {
        let config = self.config.clone();
        let ready = self.server_state.health == Health::Running && !self.server_state.locked();
        let transfer = &mut self.server_state.transfer;
        match input {
            TransferInput::Refresh if !transfer.busy => {
                transfer.busy = true;
                transfer.loaded = true;
                return background(characters::roster, TransferInput::Loaded);
            }
            TransferInput::Loaded(result) => {
                transfer.busy = false;
                match result {
                    Ok(roster) => {
                        if transfer
                            .account
                            .as_ref()
                            .is_none_or(|a| !roster.accounts.contains(a))
                        {
                            transfer.account = roster.accounts.first().cloned();
                        }
                        transfer.roster = roster;
                    }
                    Err(error) => transfer.notice = error,
                }
            }
            TransferInput::Export(name) if ready && !transfer.busy => {
                return iced::Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Export character")
                            .set_file_name(format!("{name}.pdump"))
                            .add_filter("Character dump", &["pdump"])
                            .save_file()
                            .await
                            .map(|f| f.path().to_owned());
                        (name, file)
                    },
                    |(name, file)| send(TransferInput::ExportTo(name, file)),
                );
            }
            TransferInput::ExportTo(name, Some(path)) if ready && !transfer.busy => {
                transfer.busy = true;
                transfer.notice = format!("Exporting {name}…");
                return background(
                    move || characters::export(&config, &name, &path),
                    TransferInput::Done,
                );
            }
            TransferInput::Choose if !transfer.busy => {
                return iced::Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import character")
                            .add_filter("Character dump", &["pdump", "sql", "txt"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_owned())
                    },
                    |file| send(TransferInput::Chosen(file)),
                );
            }
            TransferInput::Chosen(Some(path)) => transfer.file = Some(path),
            TransferInput::Account(account) => transfer.account = Some(account),
            TransferInput::Rename(name) => {
                transfer.rename = name
                    .chars()
                    .filter(char::is_ascii_alphabetic)
                    .take(12)
                    .collect();
            }
            TransferInput::Import if ready && !transfer.busy => {
                let (Some(file), Some(account)) = (transfer.file.clone(), transfer.account.clone())
                else {
                    transfer.notice = "Choose a dump file and an account first.".into();
                    return iced::Task::none();
                };
                let rename = (!transfer.rename.is_empty()).then(|| transfer.rename.clone());
                transfer.busy = true;
                transfer.notice = "Importing…".into();
                return background(
                    move || characters::import(&config, &file, &account, rename.as_deref()),
                    TransferInput::Done,
                );
            }
            TransferInput::Done(result) => {
                transfer.busy = false;
                let succeeded = result.is_ok();
                transfer.notice = result.unwrap_or_else(|e| e);
                self.events
                    .push(format!("Characters: {}", self.server_state.transfer.notice));
                if succeeded {
                    self.server_state.transfer.file = None;
                    self.server_state.transfer.rename.clear();
                    return self.transfer_update(TransferInput::Refresh);
                }
            }
            _ => {}
        }
        iced::Task::none()
    }

    pub(super) fn transfer_card(&self) -> Element<'_, Message> {
        let state = &self.server_state;
        let transfer = &state.transfer;
        let ready = state.health == Health::Running && !state.locked() && !transfer.busy;
        let mut card = column![
            row![
                text("Characters").size(18).width(Fill),
                button("Refresh").on_press_maybe(
                    (state.health == Health::Running && !transfer.busy)
                        .then(|| send(TransferInput::Refresh))
                ),
            ]
            .align_y(iced::Center),
            text("Move characters to and from any AzerothCore server as character dump files (.pdump). Log a character out before exporting it.")
                .size(13)
                .color(style::MUTED),
        ]
        .spacing(12);
        if state.health != Health::Running {
            card = card.push(
                text("Start the server to see, export or import characters.")
                    .size(13)
                    .color(style::GOLD),
            );
        }
        let mut list = Column::new().spacing(8);
        for c in &transfer.roster.characters {
            let export = button(if c.online { "Online" } else { "Export…" }).on_press_maybe(
                (ready && !c.online).then(|| send(TransferInput::Export(c.name.clone()))),
            );
            list = list.push(
                row![
                    text(&c.name).size(15).width(160),
                    text(format!(
                        "Level {} {} {}",
                        c.level,
                        race(c.race),
                        class(c.class)
                    ))
                    .size(13)
                    .color(style::MUTED)
                    .width(Fill),
                    text(&c.account).size(12).color(style::MUTED).width(120),
                    export,
                ]
                .spacing(12)
                .align_y(iced::Center),
            );
        }
        if transfer.loaded && transfer.roster.characters.is_empty() && !transfer.busy {
            list = list.push(
                text("No player characters yet.")
                    .size(13)
                    .color(style::MUTED),
            );
        }
        let file = transfer
            .file
            .as_ref()
            .map_or_else(|| "No file chosen".to_owned(), |p| p.display().to_string());
        card = card
            .push(list)
            .push(text("Import a character").size(15))
            .push(
                row![
                    button("Choose file…")
                        .on_press_maybe((!transfer.busy).then(|| send(TransferInput::Choose))),
                    text(file).size(12).color(style::MUTED).width(Fill),
                ]
                .spacing(12)
                .align_y(iced::Center),
            )
            .push(
                row![
                    pick_list(
                        transfer.roster.accounts.clone(),
                        transfer.account.clone(),
                        |a| send(TransferInput::Account(a))
                    )
                    .placeholder("Account"),
                    text_input("New name (optional)", &transfer.rename)
                        .on_input(|v| send(TransferInput::Rename(v)))
                        .padding(8)
                        .width(220),
                    button(if transfer.busy {
                        "Working…"
                    } else {
                        "Import"
                    })
                    .style(style::primary)
                    .on_press_maybe(
                        (ready && transfer.file.is_some() && transfer.account.is_some())
                            .then(|| send(TransferInput::Import))
                    ),
                ]
                .spacing(12)
                .align_y(iced::Center),
            );
        if !transfer.notice.is_empty() {
            card = card.push(text(&transfer.notice).size(13).color(style::GOLD));
        }
        container(card)
            .padding(18)
            .width(Fill)
            .style(style::panel)
            .into()
    }
}
