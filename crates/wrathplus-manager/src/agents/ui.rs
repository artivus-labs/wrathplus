//! The Agents page: make an agent, start or stop its headless client, watch it, hand it to a model.
use super::Agent;
use crate::{style, Manager, Message};
use iced::widget::{button, column, container, row, text, text_input, Column};
use iced::{Element, Fill};

#[derive(Debug, Clone)]
pub(crate) enum Input {
    Draft(String),
    Create,
    Created(Result<Agent, String>),
    Start(String),
    Stop(String),
    View(String),
    CopyMcp(String),
    Forget(String),
}

fn send(input: Input) -> Message {
    Message::Agents(input)
}

impl Manager {
    fn agent(&self, account: &str) -> Option<&Agent> {
        self.agents.list.iter().find(|a| a.account == account)
    }

    /// Read the saved agents once, and reap clients that exited (called from the poll).
    pub(crate) fn agents_poll(&mut self) {
        if !self.agents.loaded {
            self.agents.loaded = true;
            match super::load(&self.config) {
                Ok(list) => self.agents.list = list,
                Err(error) => self.agents.notice = error,
            }
        }
        let mut exited = Vec::new();
        for (account, child) in &mut self.agents.running {
            if let Ok(Some(status)) = child.try_wait() {
                exited.push((account.clone(), status));
            }
        }
        for (account, status) in exited {
            self.agents.running.remove(&account);
            let note = format!(
                "{account}'s client exited ({status}); its log is client.log in the agent folder."
            );
            self.agents.exits.insert(account, note.clone());
            self.events.push(format!("Agents: {note}"));
        }
    }

    pub(crate) fn agents_update(&mut self, input: Input) -> iced::Task<Message> {
        let config = self.config.clone();
        match input {
            Input::Draft(name) => self.agents.draft = name.chars().take(24).collect(),
            Input::Create if !self.agents.busy => {
                let name = self.agents.draft.trim().to_owned();
                self.agents.busy = true;
                self.agents.notice = format!("Creating {name}'s account on the realm…");
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(|| super::create(&config, &name))
                        .unwrap_or_else(|_| Err("Agent creation stopped unexpectedly".into()));
                    let _ = sender.send(result);
                });
                return iced::Task::perform(
                    async move {
                        receiver
                            .await
                            .unwrap_or_else(|_| Err("Agent creation stopped".into()))
                    },
                    |r| send(Input::Created(r)),
                );
            }
            Input::Created(result) => {
                self.agents.busy = false;
                match result {
                    Ok(agent) => {
                        self.agents.notice = format!(
                            "{} is ready: account {}, control port {}. Start its client when you want it to play.",
                            agent.name, agent.account, agent.port
                        );
                        self.agents.draft.clear();
                        self.agents.list.push(agent);
                    }
                    Err(error) => self.agents.notice = error,
                }
                self.events.push(format!("Agents: {}", self.agents.notice));
            }
            Input::Start(account) => {
                if let Some(agent) = self.agent(&account).cloned() {
                    self.agents.exits.remove(&account);
                    match super::start(&config, &agent) {
                        Ok(child) => {
                            self.agents.running.insert(account, child);
                            self.agents.notice = format!(
                                "{}'s client is starting with no window; it logs in and waits at character select.",
                                agent.name
                            );
                        }
                        Err(error) => self.agents.notice = error,
                    }
                }
            }
            Input::Stop(account) => {
                if let Some(mut child) = self.agents.running.remove(&account) {
                    let _ = child.kill();
                    let _ = child.wait();
                    self.agents.notice = format!("{account}'s client stopped.");
                }
            }
            Input::View(account) => {
                if let Some(agent) = self.agent(&account) {
                    if let Err(error) =
                        super::view_url(&config, agent).and_then(|url| super::open(&url))
                    {
                        self.agents.notice = error;
                    }
                }
            }
            Input::CopyMcp(account) => {
                if let Some(agent) = self.agent(&account) {
                    match super::mcp_command(&config, agent) {
                        Ok(line) => {
                            self.agents.notice = format!(
                                "Copied the MCP setup for {}. Paste it in a terminal; it holds the agent's port token.",
                                agent.name
                            );
                            return iced::clipboard::write(line);
                        }
                        Err(error) => self.agents.notice = error,
                    }
                }
            }
            Input::Forget(account) if !self.agents.running.contains_key(&account) => {
                match super::forget(&config, &account) {
                    Ok(list) => {
                        self.agents.list = list;
                        self.agents.notice = format!(
                            "Forgot {account} here. Its realm account and characters are kept."
                        );
                    }
                    Err(error) => self.agents.notice = error,
                }
            }
            _ => {}
        }
        iced::Task::none()
    }

    pub(crate) fn agents_page(&self) -> Element<'_, Message> {
        let agents = &self.agents;
        let realm_up = self.server_state.running();
        let mut page =
            column![
            text("Agents").size(22),
            text("Each agent has its own account and a windowless client that an AI model plays.")
                .size(13)
                .color(style::MUTED),
            row![
                text_input("Agent name, e.g. Scout", &agents.draft)
                    .on_input(|v| send(Input::Draft(v)))
                    .padding(10),
                button(if agents.busy { "Creating…" } else { "Create agent" })
                    .style(style::primary)
                    .on_press_maybe(
                        (realm_up && !agents.busy && !agents.draft.trim().is_empty())
                            .then(|| send(Input::Create))
                    ),
            ]
            .spacing(10),
        ]
            .spacing(14);
        if !realm_up {
            page = page.push(
                text("Start the server to create agents or run their clients.")
                    .size(13)
                    .color(style::GOLD),
            );
        }
        if !agents.notice.is_empty() {
            page = page.push(text(&agents.notice).size(13).color(style::GOLD));
        }
        let mut cards = Column::new().spacing(12);
        for agent in &agents.list {
            cards = cards.push(self.agent_card(agent, realm_up));
        }
        if agents.list.is_empty() {
            cards = cards.push(text("No agents yet.").size(13).color(style::MUTED));
        }
        page.push(cards).into()
    }

    fn agent_card<'a>(&'a self, agent: &'a Agent, realm_up: bool) -> Element<'a, Message> {
        let running = self.agents.running.contains_key(&agent.account);
        let account = agent.account.clone();
        let state = if running {
            format!("Running · control port {} · no window", agent.port)
        } else {
            self.agents
                .exits
                .get(&agent.account)
                .cloned()
                .unwrap_or_else(|| "Stopped".into())
        };
        let control = if running {
            button("Stop").on_press(send(Input::Stop(account.clone())))
        } else {
            button("Start")
                .style(style::primary)
                .on_press_maybe(realm_up.then(|| send(Input::Start(account.clone()))))
        };
        container(
            column![
                row![
                    column![
                        text(&agent.name).size(18),
                        text(format!("Account {}", agent.account))
                            .size(12)
                            .color(style::MUTED),
                    ]
                    .spacing(4)
                    .width(Fill),
                    control,
                    button("Watch")
                        .on_press_maybe(running.then(|| send(Input::View(account.clone())))),
                    button("Copy MCP setup").on_press(send(Input::CopyMcp(account.clone()))),
                    button("Forget")
                        .on_press_maybe((!running).then(|| send(Input::Forget(account.clone())))),
                ]
                .spacing(10)
                .align_y(iced::Center),
                text(state)
                    .size(12)
                    .color(if running { style::ICE } else { style::MUTED }),
            ]
            .spacing(8),
        )
        .padding(16)
        .width(Fill)
        .style(style::panel)
        .into()
    }
}
