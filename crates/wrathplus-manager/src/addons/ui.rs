//! Native addon catalog and registration controls. Checkboxes register sources, not per-character enable bits.
use super::*;
use crate::{style, AddonScope, Manager, Message};
use iced::widget::{button, checkbox, column, container, row, text, text_input, Column};
use iced::{Element, Fill};

impl Manager {
    pub(crate) fn addon_editable(&self) -> bool {
        !self.busy && !self.checking && self.child.is_none()
    }
    pub(crate) fn refresh_addons(&mut self) {
        match load(&self.config) {
            Ok(settings) => {
                self.addon_entries = discover(&self.config, &settings);
                self.addon_settings = settings;
            }
            Err(error) => {
                self.addon_settings = Settings::default();
                self.addon_entries.clear();
                self.record(error);
            }
        }
    }
    pub(crate) fn add_addon_folder(&mut self) {
        if !self.addon_editable() {
            return;
        }
        let path = match std::fs::canonicalize(self.addon_path.trim()) {
            Ok(path) => path,
            Err(e) => {
                self.record(format!("Choose an existing addon folder: {e}"));
                return;
            }
        };
        if scan_folder(&path).is_empty() {
            self.record(
                "No addon TOCs found. Choose an addon folder or a folder containing addons.".into(),
            );
            return;
        }
        let mut settings = self.addon_settings.clone();
        if !settings.folders.contains(&path) {
            settings.folders.push(path);
        }
        match save_and_apply(&self.config, &settings) {
            Ok(()) => {
                self.refresh_addons();
                self.record("Addon source added. Select Add to client to register an addon for the next launch.".into());
            }
            Err(e) => self.record(e),
        }
    }
    pub(crate) fn toggle_addon(&mut self, index: usize, on: bool) {
        if !self.addon_editable() {
            return;
        }
        let Some(entry) = self.addon_entries.get(index) else {
            return;
        };
        let mut settings = self.addon_settings.clone();
        if on {
            if entry.interface != 30300 || entry.missing > 0 {
                self.record(
                    "Addon has version or manifest-file blockers. Resolve them before adding it."
                        .into(),
                );
                return;
            }
            if settings
                .selected
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&entry.name))
            {
                self.record(
                    "Another source already provides this addon name. Remove it first.".into(),
                );
                return;
            }
            settings.selected.push(entry.selection());
        } else {
            settings.selected.retain(|s| s != &entry.selection());
        }
        match save_and_apply(&self.config, &settings) {
            Ok(()) => {
                self.refresh_addons();
                self.record(if on { "Addon registered for the next launch. Use the client's AddOns screen for per-character enable state; runtime compatibility remains unverified." } else { "Managed addon link removed. Source files and saved variables were preserved." }.into());
            }
            Err(error) => self.record(error),
        }
    }
    pub(crate) fn packages(&self) -> Element<'_, Message> {
        let tabs = row(AddonScope::ALL.into_iter().map(|scope| {
            button(text(scope.label()))
                .padding([8, 12])
                .on_press(Message::AddonScope(scope))
                .into()
        }))
        .spacing(8);
        if self.addon_scope != AddonScope::Client {
            return column![tabs, text("Server modules and coupled-package installation are not implemented. Client addons are managed in Client only.")].spacing(14).into();
        }
        let mut entries = Column::new().spacing(12);
        for (index, entry) in self.addon_entries.iter().enumerate() {
            if !format!("{} {}", entry.name, entry.title)
                .to_lowercase()
                .contains(&self.search.to_lowercase())
            {
                continue;
            }
            let added = entry.selected(&self.addon_settings);
            let local = entry
                .source
                .starts_with(self.config.state_root().join("AddOns"));
            let mut check = checkbox(added || local).label(if local {
                "Local addon · use in-game AddOns to disable"
            } else {
                "Add to client (next launch)"
            });
            if self.addon_editable() && !local {
                check = check.on_toggle(move |v| Message::Addon(index, v));
            }
            entries = entries.push(
                container(
                    column![
                        row![
                            text(&entry.title).size(18),
                            text(format!("{} · {}", entry.name, entry.version))
                                .size(12)
                                .color(style::MUTED)
                        ]
                        .spacing(12),
                        text(entry.source.to_string_lossy().into_owned())
                            .size(12)
                            .color(style::MUTED),
                        text(entry.requirements(&self.addon_entries)).size(12),
                        text(if added {
                            "Registered · execution not verified"
                        } else {
                            "Discovered · not registered"
                        })
                        .size(12)
                        .color(style::GOLD),
                        check,
                    ]
                    .spacing(8),
                )
                .padding(16)
                .width(Fill)
                .style(style::panel),
            );
        }
        if self.addon_entries.is_empty() {
            entries = entries.push(text(
                "No addons discovered. Add an addon directory or an AddOns collection folder.",
            ));
        }
        column![tabs,
            text("Installed addon sources").size(22),
            text("Scans local AddOns, the selected asset install's Interface/AddOns, repository addons and your custom folders. Sources remain read-only; selected addons are linked into the client's local state. Only add code you trust.").size(13).color(style::MUTED),
            row![text_input("Filter addons…", &self.search).on_input(Message::Search).padding(10), button("Refresh").on_press(Message::RefreshAddons)].spacing(10),
            row![text_input("Custom addon / AddOns folder", &self.addon_path).on_input(Message::AddonPath).padding(10), button("Browse…").on_press_maybe((self.addon_editable() && !self.choosing).then_some(Message::BrowseAddons)), button("Add source").on_press_maybe(self.addon_editable().then_some(Message::AddAddonPath))].spacing(10),
            text("Apply while stopped. Windows links require Developer Mode or symlink permission. Existing unmanaged addons are never overwritten. Disable per character in the game's AddOns screen; removing a managed registration preserves source and saved data.").size(12).color(style::MUTED),
            entries,
        ].spacing(14).into()
    }
}
