use super::*;
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
        if let Input::OpenRepository(repository) = input {
            if let Err(error) = standard::open_repository(repository) {
                self.module_state.notice = error;
            }
            return iced::Task::none();
        }
        if self.module_state.busy || !self.server_state.module_editable() {
            return iced::Task::none();
        }
        let standard_add = matches!(input, Input::AddStandard(_));
        // A standard module is the same import as a typed source, from its known repository.
        let input = match input {
            Input::AddStandard(slot) => match STANDARD.get(slot) {
                Some(module) => {
                    self.module_state.source = module.repository.into();
                    Input::Register
                }
                None => return iced::Task::none(),
            },
            input => input,
        };
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
        if standard_add {
            // The repository went in through the custom-source box; leave that box as it was.
            self.module_state.source.clear();
        }
        match result {
            Ok(task) => task,
            Err(error) => {
                self.module_state.notice = error;
                iced::Task::none()
            }
        }
    }
}
