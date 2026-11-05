//! The Server page's settings card: typed settings of the selected standard modules, and the
//! full environment list under Advanced. Edits change the saved settings on Save or Start and
//! reach the realm the next time it starts.
use super::environment::{check_name, check_value};
use super::module_settings::{Kind, Module, Setting, MODULES};
use super::{Action, Input, Server};
use crate::{style, Manager, Message};
use iced::widget::{button, checkbox, column, row, text, text_input, Column};
use iced::{Element, Fill};

#[derive(Clone)]
pub enum EnvInput {
    /// Set a variable (a typed setting or an Advanced row) to a value.
    Set(String, String),
    /// Drop a saved variable: a typed one goes back to its default.
    Remove(String),
    NewName(String),
    NewValue(String),
    Add,
    ToggleAdvanced,
}
impl std::fmt::Debug for EnvInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Values may be keys or passwords.
        f.write_str("Server environment input (values redacted)")
    }
}

#[derive(Default)]
pub(super) struct EnvForm {
    name: String,
    value: String,
    advanced: bool,
    notice: Option<String>,
}

impl Server {
    pub(super) fn environment_update(&mut self, input: EnvInput) {
        let form = &mut self.env_form;
        let environment = &mut self.settings.environment;
        form.notice = None;
        match input {
            EnvInput::Set(name, value) => environment.set(name, value),
            EnvInput::Remove(name) => environment.remove(&name),
            EnvInput::NewName(name) => form.name = name.trim().to_ascii_uppercase(),
            EnvInput::NewValue(value) => form.value = value,
            EnvInput::Add => {
                match check_name(&form.name).and_then(|_| check_value(&form.name, &form.value)) {
                    Ok(()) => {
                        environment.set(
                            std::mem::take(&mut form.name),
                            std::mem::take(&mut form.value),
                        );
                    }
                    Err(error) => form.notice = Some(error),
                }
            }
            EnvInput::ToggleAdvanced => form.advanced = !form.advanced,
        }
    }

    /// A server type's settings, saved at once: types that build a module reset its typed
    /// settings to the defaults; a type without it writes the module's "off" values.
    pub(crate) fn apply_server_type(
        &mut self,
        config: &crate::setup::Config,
        modules: &[&str],
    ) -> Result<(), String> {
        for module in MODULES {
            if modules.contains(&module.module) {
                module.apply_defaults(&mut self.settings.environment);
            } else {
                module.apply_off(&mut self.settings.environment);
            }
        }
        self.settings.save(config)
    }
}

fn message(input: EnvInput) -> Message {
    Message::Server(Input::Environment(input))
}

/// Values of names like these are shown masked in the Advanced list.
fn secret(name: &str) -> bool {
    ["KEY", "TOKEN", "SECRET", "PASSWORD"]
        .iter()
        .any(|word| name.contains(word))
}

fn typed_row<'a>(setting: &'static Setting, value: &str, editable: bool) -> Element<'a, Message> {
    let name = setting.env();
    let control: Element<'a, Message> = if setting.kind == Kind::Switch {
        let mut check = checkbox(value.trim() == "1");
        if editable {
            check = check.on_toggle(move |on| {
                message(EnvInput::Set(
                    name.clone(),
                    if on { "1" } else { "0" }.into(),
                ))
            });
        }
        check.into()
    } else {
        let mut input = text_input(setting.default, value).padding(8).width(140);
        if editable {
            input = input.on_input(move |v| message(EnvInput::Set(name.clone(), v)));
        }
        input.into()
    };
    let mut about = column![text(setting.label).size(14)].spacing(2);
    if !setting.help.is_empty() {
        about = about.push(text(setting.help).size(12).color(style::MUTED));
    }
    let mut side = column![control].spacing(2);
    match setting.check(value) {
        Err(error) => side = side.push(text(error).size(12).color(style::GOLD)),
        Ok(()) => {
            if let Some(reading) = setting.describe(value) {
                side = side.push(text(reading).size(12).color(style::MUTED));
            }
        }
    }
    row![about.width(Fill), side]
        .spacing(16)
        .align_y(iced::Center)
        .into()
}

impl Manager {
    fn module_settings(&self, module: &'static Module, editable: bool) -> Element<'_, Message> {
        let environment = &self.server_state.settings.environment;
        let mut list = Column::new()
            .spacing(10)
            .push(text(module.title).size(16).color(style::ICE));
        for setting in module.settings {
            list = list.push(typed_row(setting, setting.value(environment), editable));
        }
        list.into()
    }

    fn environment_list(&self, editable: bool) -> Element<'_, Message> {
        let state = &self.server_state;
        let saved = &state.settings.environment;
        let mut list = column![text("Every variable the server starts with. AzerothCore reads AC_ plus a config key in upper snake case (Playerbots.MinRandomBots is AC_PLAYERBOTS_MIN_RANDOM_BOTS) as that key, for any module. Values are kept in the Manager's state folder and never logged.").size(12).color(style::MUTED)].spacing(8);
        for (name, value) in state.settings.environment.effective().iter() {
            let name = name.to_owned();
            let set = name.clone();
            let mut input = text_input("", value)
                .padding(6)
                .width(Fill)
                .secure(secret(&name));
            if editable {
                input = input.on_input(move |v| message(EnvInput::Set(set.clone(), v)));
            }
            let typed = MODULES
                .iter()
                .flat_map(|m| m.settings)
                .any(|s| s.env() == name);
            let remove = saved.get(&name).is_some().then(|| {
                button(if typed { "Default" } else { "Remove" })
                    .on_press_maybe(editable.then(|| message(EnvInput::Remove(name.clone()))))
            });
            let mut line = row![
                text(name.clone())
                    .font(iced::Font::MONOSPACE)
                    .size(12)
                    .width(Fill),
                input
            ]
            .spacing(10)
            .align_y(iced::Center);
            if let Some(remove) = remove {
                line = line.push(remove);
            }
            list = list.push(line);
        }
        let form = &state.env_form;
        let mut name = text_input("NEW_VARIABLE", &form.name)
            .padding(6)
            .width(Fill);
        let mut value = text_input("value", &form.value).padding(6).width(Fill);
        if editable {
            name = name.on_input(|v| message(EnvInput::NewName(v)));
            value = value.on_input(|v| message(EnvInput::NewValue(v)));
        }
        list = list.push(
            row![
                name,
                value,
                button("Add").on_press_maybe(editable.then(|| message(EnvInput::Add)))
            ]
            .spacing(10)
            .align_y(iced::Center),
        );
        if let Some(notice) = &form.notice {
            list = list.push(text(notice).size(12).color(style::GOLD));
        }
        list.into()
    }

    /// Settings card: typed settings for each selected standard module, then Advanced.
    pub(crate) fn server_settings(&self) -> Element<'_, Message> {
        let state = &self.server_state;
        let editable = !state.locked() && !state.choosing && !self.module_state.busy;
        let source = state.module_source();
        let selected: Vec<String> = if source.as_os_str().is_empty() {
            Vec::new()
        } else {
            crate::modules::Catalog::load(&self.config, source)
                .map(|c| {
                    c.entries()
                        .iter()
                        .filter(|e| e.enabled)
                        .map(|e| e.name.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut card = column![
            text("Server settings").size(18),
            text("Used the next time the server starts: Stop, then Start. They take the place of the same settings in a module's config file.").size(13).color(style::MUTED),
        ]
        .spacing(12);
        for module in MODULES
            .iter()
            .filter(|m| selected.iter().any(|s| s == m.module))
        {
            card = card.push(self.module_settings(module, editable));
        }
        card = card.push(
            button(if state.env_form.advanced {
                "Hide environment variables"
            } else {
                "Advanced: environment variables"
            })
            .on_press(message(EnvInput::ToggleAdvanced)),
        );
        if state.env_form.advanced {
            card = card.push(self.environment_list(editable));
        }
        card.push(
            button("Save settings")
                .style(style::primary)
                .on_press_maybe(editable.then_some(Message::Server(Input::Run(Action::Save)))),
        )
        .into()
    }
}
