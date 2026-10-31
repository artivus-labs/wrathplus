use super::*;

impl Manager {
    pub(super) fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::ToggleBuildLog => self.build_feedback.expanded = !self.build_feedback.expanded,
            Message::CopyBuildLog => {
                return iced::clipboard::write(self.build_feedback.copy_text())
            }
            Message::BrowseBuildDirectory
                if !self.choosing && !self.busy && !self.checking && self.child.is_none() =>
            {
                return self.choose_build_directory()
            }
            Message::BrowseBuildDirectory => {}
            Message::BuildDirectoryChosen(path) => {
                self.choosing = false;
                if let Some(path) = path {
                    return self
                        .update(Message::BuildDirectory(path.to_string_lossy().into_owned()));
                }
            }
            Message::BuildDirectory(value)
                if !self.busy && !self.checking && self.child.is_none() =>
            {
                self.build_directory = value;
                self.config.target = self.build_directory.clone().into();
                self.preflight = None;
            }
            Message::BuildDirectory(_) => {}

            Message::BrowseAssets if !self.choosing && !self.busy => return self.choose_assets(),
            Message::BrowseAssets => {}
            Message::FolderChosen(path) => {
                self.choosing = false;
                if let Some(path) = path {
                    self.assets = path.to_string_lossy().into_owned();
                    self.preflight = None;
                    self.validated = false;
                    self.sync_config();
                    return self.update(Message::Validate);
                }
            }
            Message::Recheck if !self.busy && !self.checking => return self.check_prerequisites(),
            Message::Recheck => {}
            Message::PreflightFinished(report) => {
                self.checking = false;
                self.record(report.summary());
                self.preflight = Some(report);
            }
            Message::Validate => {
                self.sync_config();
                self.validated = false;
                match setup::validate(&self.config) {
                    Ok(_) => {
                        self.validated = true;
                        if setup::discover(&self.config).is_some() {
                            if let Err(error) =
                                setup::save_config(&self.config, &self.config.state_path())
                            {
                                self.record(error);
                                return iced::Task::none();
                            }
                        }
                        self.record(
                            if setup::discover(&self.config).is_some() {
                                "Game files checked and settings saved."
                            } else {
                                "Game files ready. Build your client to continue."
                            }
                            .into(),
                        );
                    }
                    Err(error) => self.record(error),
                }
            }
            Message::Build if self.build_ready() => {
                self.sync_config();
                self.busy = true;
                let (log_sender, log_receiver) = std::sync::mpsc::channel();
                self.build_feedback = build_feedback::Feedback::default();
                self.build_feedback.started = Some(std::time::Instant::now());
                self.build_feedback.receiver = Some(log_receiver);
                self.validated = false;
                self.record("Building your client. This may take a while.".into());
                let config = self.config.clone();
                // Cargo runs on a blocking worker, never on the UI event thread.
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(|| {
                        setup::build_with_log(&config, |path| {
                            let _ = log_sender.send(path);
                        })
                    })
                    .unwrap_or_else(|_| {
                        Err("Client build worker panicked; inspect build log.".into())
                    });
                    let _ = sender.send(result);
                });
                return iced::Task::perform(
                    async move {
                        receiver
                            .await
                            .unwrap_or_else(|_| Err("Build worker disconnected.".into()))
                    },
                    Message::BuildFinished,
                );
            }
            Message::BuildFinished(result) => {
                self.build_feedback.refresh();
                self.build_feedback.finished = Some(std::time::Instant::now());
                self.build_feedback.error = result.as_ref().err().cloned();
                self.build_feedback.expanded |= result.is_err();
                self.busy = false;
                self.validated = result.is_ok();
                self.record(match result {
                    Ok(value) => value,
                    Err(error) => format!("Setup failed: {error}"),
                });
            }
            Message::Launch if !self.busy && self.child.is_none() => {
                self.sync_config();
                match setup::launch(&self.config) {
                    Ok(child) => {
                        self.child = Some(child);
                        self.launched_config = Some(self.config.clone());
                        self.record("Client started. Log in through the game.".into());
                    }
                    Err(error) => self.record(error),
                }
            }
            Message::StopClient => {
                if let Some(child) = self.child.as_mut() {
                    match setup::stop(child) {
                        Ok(()) => {
                            self.record("Stopping client…".into());
                            return self.update(Message::Poll);
                        }
                        Err(error) => self.record(format!("Couldn’t stop client: {error}")),
                    }
                }
            }
            Message::Poll => {
                crate::branding::set_dock_icon();
                if self.busy {
                    self.build_feedback.refresh();
                }
                if let (Some(config), Some(child)) = (&self.launched_config, self.child.as_mut()) {
                    match setup::reap(config, child) {
                        Ok(true) => {
                            self.child = None;
                            self.launched_config = None;
                            self.record("Client exited; ready to launch again.".into());
                        }
                        Ok(false) => {}
                        Err(error) => {
                            if self.notice != error {
                                self.record(error);
                            }
                        }
                    }
                }
            }
            Message::Build | Message::Launch => {}
            Message::Locale(value) => {
                self.locale = value;
                self.validated = false;
                self.sync_config();
            }
            Message::Navigate(page) => self.page = page,
            Message::Profile(value) => {
                self.profile = value;
                self.notice =
                    "Preview profile selected. No configuration or world state was changed.".into();
            }
            Message::Assets(value) => {
                self.assets = value;
                self.preflight = None;
                self.validated = false;
                self.sync_config();
                self.refresh_addons();
            }
            Message::Host(value) => {
                self.host = value;
                self.sync_config();
            }
            Message::Repo(value) => {
                self.repo = value;
                self.config.repo = self.repo.clone().into();
                self.addon_settings = addons::Settings::default();
                self.addon_entries.clear();
                self.refresh_addons();
                self.preflight = None;
                self.validated = false;
            }
            Message::Search(value) => self.search = value,
            Message::AddonScope(value) => {
                self.page = Page::Addons;
                self.addon_scope = value;
            }
            Message::Addon(index, value) => self.toggle_addon(index, value),
            Message::RefreshAddons => self.refresh_addons(),
            Message::AddonPath(value) => self.addon_path = value,
            Message::AddAddonPath => self.add_addon_folder(),
            Message::BrowseAddons if !self.choosing && self.addon_editable() => {
                self.choosing = true;
                return iced::Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Select addon or AddOns folder")
                            .pick_folder()
                            .await
                            .map(|h| h.path().to_owned())
                    },
                    Message::AddonFolderChosen,
                );
            }
            Message::BrowseAddons => {}
            Message::AddonFolderChosen(path) => {
                self.choosing = false;
                if let Some(path) = path {
                    self.addon_path = path.to_string_lossy().into_owned();
                    self.add_addon_folder();
                }
            }
            Message::Notice(value) => {
                self.notice = format!("Preview only — {value}");
                self.events.push(self.notice.clone());
            }
        }
        iced::Task::none()
    }
}
