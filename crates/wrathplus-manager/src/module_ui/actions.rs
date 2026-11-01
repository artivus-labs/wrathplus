use super::*;
use iced::widget::column;
impl Manager {
    pub(crate) fn module_update(&mut self, input: Input) -> iced::Task<Message> {
        self.load_server();
        if let Input::Finished(result) = input {
            self.module_state.busy = false;
            self.module_state.notice = result.unwrap_or_else(|e| e);
            self.record(self.module_state.notice.clone());
            return iced::Task::none();
        }
        if let Input::Source(value) = input {
            self.module_state.source = value;
            return iced::Task::none();
        }
        if self.module_state.busy || !self.server_state.module_editable() {
            return iced::Task::none();
        }
        let config = self.config.clone();
        let core = self.server_state.module_source().to_owned();
        let result = (|| {
            let mut catalog = modules::Catalog::load(&config, &core)?;
            match input {
                Input::Toggle(index, enabled) => {
                    let name = catalog.entries().get(index).ok_or("Module list changed; refresh")?.name.clone();
                    catalog.set_enabled(&name, enabled)?;
                    catalog.save(&config)?;
                    Ok("Selection saved. Rebuild, then restart to apply. Existing database data is retained.".into())
                }
                Input::Register => {
                    let source = self.module_state.source.trim().to_owned();
                    if std::path::Path::new(&source).is_dir() {
                        catalog.register_local(std::path::Path::new(&source))?;
                        catalog.save(&config)?;
                        self.module_state.notice = "Module source registered. Rebuild, then restart. Only build code you trust.".into();
                        return Ok(iced::Task::none());
                    }
                    self.module_state.busy = true;
                    let (sender, receiver) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            catalog.import_github(&config, &source)?;
                            catalog.save(&config)?;
                            Ok("GitHub source imported and revision recorded. Review code before building; rebuild and restart to apply.".into())
                        })).unwrap_or_else(|_| Err("Module import worker failed".into()));
                        let _ = sender.send(result);
                    });
                    return Ok(iced::Task::perform(async { receiver.await.unwrap_or_else(|_| Err("Module import stopped".into())) }, |r| Message::Modules(Input::Finished(r))));
                }
                _ => Ok("Module catalog refreshed. Selections describe the next build, not running activation.".into()),
            }.map(|note| { self.module_state.notice = note; iced::Task::none() })
        })();
        match result {
            Ok(task) => task,
            Err(error) => {
                self.module_state.notice = error;
                iced::Task::none()
            }
        }
    }
    pub(crate) fn module_entries(&self) -> Element<'_, Message> {
        let catalog = match modules::Catalog::load(&self.config, self.server_state.module_source())
        {
            Ok(catalog) => catalog,
            Err(error) => {
                return text(format!(
                    "{error}. Choose your core checkout on Server setup first."
                ))
                .into()
            }
        };
        let mut rows = Column::new().spacing(12);
        for (index, entry) in catalog.entries().iter().enumerate() {
            let mut check = checkbox(entry.enabled).label("Include in next build");
            if !self.module_state.busy && self.server_state.module_editable() {
                check = check.on_toggle(move |v| Message::Modules(Input::Toggle(index, v)));
            }
            let configs = self.module_config_rows(entry);
            rows = rows.push(
                container(
                    column![
                        text(entry.name.clone()).size(18),
                        text(format!("{:?}", entry.source))
                            .size(12)
                            .color(style::MUTED),
                        check,
                        configs
                    ]
                    .spacing(8),
                )
                .padding(16)
                .width(Fill)
                .style(style::panel),
            );
        }
        rows.into()
    }
}
