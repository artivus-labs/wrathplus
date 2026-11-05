//! Create your account (issue #50): the guided setup's step that makes the player's own game
//! account on the managed realm, over the realm's admin command port as agents' accounts are made.
//! The password is prefilled with a default for this local realm and editable. It travels only in
//! the command's request body (never argv, logs or `Debug` output) and is kept in a private file
//! beside the Manager's admin secret, so a client playing on this realm is prefilled with it.
use super::settings::Settings;
use super::{Input, Server};
use crate::{setup, Manager, Message};

type Result<T> = std::result::Result<T, String>;

/// The managed realm's auth address (`packaging/server/runtime.sh`); the client's login is
/// prefilled with the created account only for this server.
pub(crate) const LOCAL_REALM: &str = "127.0.0.1:13724";
/// The prefilled password: a local realm only this computer reaches.
pub(super) const DEFAULT_PASSWORD: &str = "wrathplus";
/// AzerothCore's longest account password (`MAX_PASS_STR`).
const PASSWORD_LIMIT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    New(String),
    /// The realm already had the account; its password was left as it was.
    Existing(String),
}

#[derive(Clone)]
pub enum AccountInput {
    Name(String),
    Password(String),
    Create,
    Finished(Result<Created>),
}
impl std::fmt::Debug for AccountInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Password(redacted)"),
            Self::Name(name) => f.debug_tuple("Name").field(name).finish(),
            Self::Create => f.write_str("Create"),
            Self::Finished(result) => f.debug_tuple("Finished").field(result).finish(),
        }
    }
}

/// The step's fields and outcome.
pub(super) struct AccountForm {
    pub name: String,
    pub password: String,
    pub busy: bool,
    /// The account the client is prefilled with, once made (or found).
    pub created: Option<String>,
    pub notice: Option<Result<String>>,
}
impl Default for AccountForm {
    fn default() -> Self {
        Self {
            name: default_name(
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .ok(),
            ),
            password: DEFAULT_PASSWORD.into(),
            busy: false,
            created: None,
            notice: None,
        }
    }
}

/// The computer's user name as an account name (letters and digits, at most 16), or PLAYER.
pub(super) fn default_name(user: Option<String>) -> String {
    let name: String = user
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect::<String>()
        .to_ascii_uppercase();
    if name.is_empty() {
        "PLAYER".into()
    } else {
        name
    }
}

/// Letters and digits, at most 16 (the realm upper-cases names itself).
pub(super) fn check_name(name: &str) -> Result<String> {
    let name = name.trim().to_ascii_uppercase();
    super::characters::check_account(&name)
        .map(|_| name)
        .map_err(|_| "Account name: use 1 to 16 letters and digits.".into())
}

/// Printable ASCII without spaces (the command splits on them), at most 16 characters. The
/// message never repeats the password.
pub(super) fn check_password(password: &str) -> Result<()> {
    if (1..=PASSWORD_LIMIT).contains(&password.len())
        && password.bytes().all(|b| b.is_ascii_graphic())
    {
        Ok(())
    } else {
        Err(format!(
            "Password: use 1 to {PASSWORD_LIMIT} letters, digits or symbols, without spaces."
        ))
    }
}

/// Make the account with `run` (one worldserver command, over SOAP in the Manager) and give it
/// Wrath content. An account that already exists is kept as it is. `wait` paces the retry while
/// the realm finishes creating the account. Errors never contain the password.
pub(crate) fn create_with(
    run: &mut dyn FnMut(&str) -> Result<String>,
    wait: &dyn Fn(),
    account: &str,
    password: &str,
) -> Result<Created> {
    check_password(password)?;
    let created = match run(&format!(".account create {account} {password}")) {
        Ok(_) => Created::New(account.into()),
        Err(e) if e.to_ascii_lowercase().contains("already exist") => {
            Created::Existing(account.into())
        }
        Err(e) => return Err(format!("Create {account}: {}", e.replace(password, "…"))),
    };
    // Wrath content needs the expansion flag on the account. The realm creates accounts
    // asynchronously, so the flag can briefly find no account yet.
    let addon = format!(".account set addon {account} 2");
    let mut attempt = 1;
    while let Err(e) = run(&addon) {
        if attempt >= 20 || !e.contains("not exist") {
            return Err(e);
        }
        attempt += 1;
        wait();
    }
    Ok(created)
}

/// [`create_with`] against the managed realm. Blocking; the realm must be running.
pub(crate) fn create(config: &setup::Config, account: &str, password: &str) -> Result<Created> {
    create_with(
        &mut |command| super::admin_command(config, command),
        &|| std::thread::sleep(std::time::Duration::from_millis(250)),
        account,
        password,
    )
}

fn local_path(config: &setup::Config) -> std::path::PathBuf {
    Settings::state(config).join("local-account")
}

/// Keep the player's account privately (0600) for the client's login prefill.
pub(super) fn remember(config: &setup::Config, account: &str, password: &str) -> Result<()> {
    let mut guard = config.clone();
    guard.data = if config.data.as_os_str().is_empty() {
        config.repo.join(".unselected-client-assets")
    } else {
        setup::resolved(&config.data)?
    };
    setup::write_state(
        &guard,
        &local_path(config),
        format!("{account}\n{password}\n").as_bytes(),
    )
    .map_err(|_| "Could not save the account for the client's login".to_owned())
}

/// The player's account on the managed realm and its password, if the guided setup made one.
pub(crate) fn local(config: &setup::Config) -> Option<(String, String)> {
    let contents = std::fs::read_to_string(local_path(config)).ok()?;
    let mut lines = contents.lines();
    let account = lines.next()?.trim().to_owned();
    let password = lines.next()?.to_owned();
    (super::characters::check_account(&account).is_ok() && check_password(&password).is_ok())
        .then_some((account, password))
}

impl Server {
    /// The guided step's state, for `guide`.
    pub(super) fn account(&self) -> &AccountForm {
        &self.account
    }
    pub(super) fn load_account(&mut self, config: &setup::Config) {
        self.account.created = local(config).map(|(account, _)| account);
    }
    /// The step's name and (masked) password fields, and why the last try failed.
    pub(super) fn account_fields(&self) -> iced::Element<'_, Message> {
        use iced::widget::{column, row, text, text_input};
        let form = &self.account;
        let message = |input| Message::Server(Input::Account(input));
        let mut fields = column![row![
            text_input("Account name", &form.name)
                .on_input(move |v| message(AccountInput::Name(v)))
                .padding(8)
                .width(200),
            text_input("Password", &form.password)
                .secure(true)
                .on_input(move |v| message(AccountInput::Password(v)))
                .padding(8)
                .width(200),
        ]
        .spacing(10)]
        .spacing(6);
        if let Some(Err(error)) = &form.notice {
            fields = fields.push(text(error.clone()).size(13).color(crate::style::GOLD));
        }
        fields.into()
    }
    /// The Create button's message, while the fields are usable.
    pub(super) fn account_input(&self) -> Option<Message> {
        let form = &self.account;
        (self.running()
            && !form.busy
            && check_name(&form.name).is_ok()
            && check_password(&form.password).is_ok())
        .then_some(Message::Server(Input::Account(AccountInput::Create)))
    }
}

impl Manager {
    pub(super) fn account_update(&mut self, input: AccountInput) -> iced::Task<Message> {
        let running = self.server_state.running();
        let form = &mut self.server_state.account;
        match input {
            AccountInput::Name(name) if !form.busy => form.name = name,
            AccountInput::Password(password) if !form.busy => form.password = password,
            AccountInput::Create if !form.busy && running => {
                let account = match check_name(&form.name)
                    .and_then(|name| check_password(&form.password).map(|_| name))
                {
                    Ok(account) => account,
                    Err(error) => {
                        form.notice = Some(Err(error));
                        return iced::Task::none();
                    }
                };
                form.busy = true;
                form.notice = None;
                let password = form.password.clone();
                let config = self.config.clone();
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(|| {
                        let created = create(&config, &account, &password)?;
                        remember(&config, &account, &password)?;
                        Ok(created)
                    })
                    .unwrap_or_else(|_| Err("Creating the account stopped unexpectedly".into()));
                    let _ = sender.send(result);
                });
                return iced::Task::perform(
                    async {
                        receiver
                            .await
                            .unwrap_or_else(|_| Err("Creating the account stopped".into()))
                    },
                    |r| Message::Server(Input::Account(AccountInput::Finished(r))),
                );
            }
            AccountInput::Finished(result) => {
                form.busy = false;
                let message = match &result {
                    Ok(Created::New(account)) => format!("Account {account} created. Play fills in its name and password when the client uses this server."),
                    Ok(Created::Existing(account)) => format!("{account} already exists on this realm and was kept as it was; its password was not changed. Play fills in the password entered here."),
                    Err(error) => format!("Account not created: {error}"),
                };
                if let Ok(Created::New(account) | Created::Existing(account)) = result {
                    form.created = Some(account);
                    form.notice = Some(Ok(message.clone()));
                } else {
                    form.notice = Some(Err(message.clone()));
                }
                self.events.push(format!("Server: {message}"));
            }
            _ => {}
        }
        iced::Task::none()
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
