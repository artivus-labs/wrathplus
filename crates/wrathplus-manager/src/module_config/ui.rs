use super::*;
use crate::{style, Manager, Message};
use iced::{
    widget::{button, column, container, mouse_area, opaque, row, space, stack, text, text_editor},
    Element, Fill, Task,
};

impl Manager {
    pub(crate) fn config_update(&mut self, input: Input) -> Task<Message> {
        self.load_server();
        let core = self.server_state.module_source().to_owned();
        let config = &self.config;
        let state = &mut self.config_editor;
        let result = (|| -> Result<()> {
            match input {
                Input::Cancel => {
                    if state.dirty {
                        state.notice =
                            "Unsaved changes. Save, or choose Discard changes to close.".into();
                    } else {
                        *state = State::default();
                    }
                }
                Input::Discard => {
                    *state = State::default();
                }
                Input::Open { module, name } => {
                    if state.open {
                        return Err("Close the current editor first".into());
                    }
                    state.open = true;
                    state.module = module.clone();
                    state.name = name.clone();
                    let file = discovery::resolve(config, &core, &module, &name)?;
                    let existing = match std::fs::symlink_metadata(&file.target) {
                        Ok(_) => Some(files::read(&file.target)?),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                        Err(_) => return Err("Cannot inspect local config".into()),
                    };
                    let content = if let Some(snapshot) = &existing {
                        snapshot.text.clone()
                    } else {
                        files::read(&file.template)?.text
                    };
                    state.content = text_editor::Content::with_text(&content);
                    state.original = existing;
                    state.selected = Some(file.target);
                    state.notice.clear();
                }
                Input::Edit(action) => {
                    if state.open && state.selected.is_some() {
                        if let text_editor::Action::Edit(text_editor::Edit::Paste(value)) = &action
                        {
                            if value.len().saturating_add(state.content.text().len()) > LIMIT {
                                return Err("Paste would exceed 256 KiB; edits retained".into());
                            }
                        }
                        if action.is_edit()
                            && state.content.text().len() >= LIMIT
                            && matches!(
                                action,
                                text_editor::Action::Edit(
                                    text_editor::Edit::Insert(_)
                                        | text_editor::Edit::Enter
                                        | text_editor::Edit::Indent
                                )
                            )
                        {
                            return Err("Editor size limit reached".into());
                        }
                        let edited = action.is_edit();
                        state.content.perform(action);
                        if edited {
                            state.dirty = true;
                        }
                    }
                }
                Input::Save => {
                    let target = state.selected.as_ref().ok_or("No config loaded")?;
                    // Revalidate ownership, selection and path before committing a pending local copy.
                    let file = discovery::resolve(config, &core, &state.module, &state.name)?;
                    if file.target != *target {
                        return Err("Config location changed; reopen the editor".into());
                    }
                    files::write(
                        config,
                        &core,
                        target,
                        &state.content.text(),
                        state.original.as_ref(),
                    )?;
                    *state = State::default();
                    self.module_state.notice = "Config saved. Restart the server to apply. LLM bridge remains a separate service.".into();
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            state.notice = error;
        }
        Task::none()
    }

    pub(crate) fn config_overlay<'a>(&'a self, base: Element<'a, Message>) -> Element<'a, Message> {
        let state = &self.config_editor;
        if !state.open {
            return base;
        }
        let mut editor = text_editor(&state.content).height(Fill);
        if state.selected.is_some() {
            editor = editor.on_action(|a| Message::ModuleConfig(Input::Edit(a)));
        }
        let mut actions = row![space().width(Fill)];
        if state.dirty {
            actions = actions
                .push(button("Discard changes").on_press(Message::ModuleConfig(Input::Discard)));
        }
        actions = actions
            .push(button("Cancel").on_press(Message::ModuleConfig(Input::Cancel)))
            .push(
                button("Save").style(style::primary).on_press_maybe(
                    state
                        .selected
                        .as_ref()
                        .map(|_| Message::ModuleConfig(Input::Save)),
                ),
            );
        let dialog = container(column![
            text(format!("Edit {}", state.name)).size(22),
            text(&state.module).size(13).color(style::MUTED),
            text("Saved to a local copy; the original module template stays unchanged. Restart the server to apply.").size(13).color(style::MUTED),
            container(editor).clip(true).height(Fill),
            text(&state.notice).size(13).color(style::GOLD),
            actions.spacing(10),
        ].spacing(12)).padding(24).width(1000).height(650).style(style::panel);
        let shade =
            mouse_area(
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(|_| container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.72).into()),
                        ..Default::default()
                    }),
            )
            .on_press(Message::ModuleConfig(Input::Cancel));
        stack![
            base,
            opaque(shade),
            container(opaque(dialog)).center_x(Fill).center_y(Fill)
        ]
        .into()
    }
}
