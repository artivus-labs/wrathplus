//! Server types: a named set of standard modules for the next build, so a new player picks one
//! instead of choosing modules. A type manages only the packaged standard modules; a module the
//! player added themselves stays as it is. Server modules (the Advanced view) still allows any
//! selection.
use super::standard::{is_standard, STANDARD};
use crate::modules::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preset {
    Playerbots,
    Chatter,
    Plain,
}

impl Preset {
    pub(crate) const ALL: [Self; 3] = [Self::Playerbots, Self::Chatter, Self::Plain];
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Playerbots => "Playerbots (recommended)",
            Self::Chatter => "Playerbots + Chatter",
            Self::Plain => "Plain AzerothCore",
        }
    }
    pub(crate) fn about(self) -> &'static str {
        match self {
            Self::Playerbots => "Computer-controlled companions to group and quest with, 1,000 to 2,000 random bots that level through the world and start again when they reach the top, MultiBot buttons, a stocked auction house and minimap quest markers.",
            Self::Chatter => "Everything in Playerbots, and bots talk in character through a language model you choose. Needs a provider and API key, set under Server modules.",
            Self::Plain => "A plain realm with no bots: just you and anyone you invite.",
        }
    }
    /// The standard modules this server type builds.
    pub(crate) fn modules(self) -> &'static [&'static str] {
        const BOTS: &[&str] = &[
            "mod-playerbots",
            "mod-multibot-bridge",
            "mod-ah-bot",
            "mod-quest-minimap-points",
        ];
        const CHATTER: &[&str] = &[
            "mod-playerbots",
            "mod-multibot-bridge",
            "mod-ah-bot",
            "mod-quest-minimap-points",
            "mod-llm-chatter",
        ];
        match self {
            Self::Playerbots => BOTS,
            Self::Chatter => CHATTER,
            Self::Plain => &[],
        }
    }
    /// What to tell the player once the selection is saved.
    pub(crate) fn saved(self) -> &'static str {
        match self {
            Self::Playerbots => "Playerbots selected. Build the server to use it.",
            Self::Chatter => "Playerbots + Chatter selected. Set LLM Chatter's provider, model and key under Server modules, then build the server.",
            Self::Plain => "Plain AzerothCore selected (no bots). Build the server to use it.",
        }
    }
}

/// The server type the catalog's standard selection is exactly, if any. The player's own modules
/// do not count.
pub(crate) fn current(entries: &[Entry]) -> Option<Preset> {
    let mut enabled: Vec<&str> = entries
        .iter()
        .filter(|e| e.enabled && is_standard(&e.name))
        .map(|e| e.name.as_str())
        .collect();
    enabled.sort_unstable();
    Preset::ALL.into_iter().find(|preset| {
        let mut wanted = preset.modules().to_vec();
        wanted.sort_unstable();
        wanted == enabled
    })
}

/// The catalog changes a server type needs: standard modules to add, then which entries to switch
/// on and off. Standard modules outside the type are left out of the build, not removed; the
/// player's own modules are not touched.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Change {
    /// Standard module names, in catalog order.
    pub add: Vec<&'static str>,
    pub enable: Vec<String>,
    pub disable: Vec<String>,
}

pub(crate) fn plan(entries: &[Entry], preset: Preset) -> Change {
    let wanted = preset.modules();
    let mut change = Change::default();
    for module in STANDARD.iter().filter(|m| wanted.contains(&m.name)) {
        match entries.iter().find(|e| e.name == module.name) {
            None => change.add.push(module.name),
            Some(entry) if !entry.enabled => change.enable.push(entry.name.clone()),
            Some(_) => {}
        }
    }
    change.disable = entries
        .iter()
        .filter(|e| e.enabled && is_standard(&e.name) && !wanted.contains(&e.name.as_str()))
        .map(|e| e.name.clone())
        .collect();
    change
}

impl crate::Manager {
    /// The Server page's choice of server type; Advanced opens the full module list.
    pub(crate) fn server_types(&self) -> iced::Element<'_, crate::Message> {
        use crate::{style, Message};
        use iced::widget::{button, column, container, row, text};
        let source = self.server_state.module_source();
        let catalog = (!source.as_os_str().is_empty())
            .then(|| crate::modules::Catalog::load(&self.config, source));
        let editable = !self.module_state.busy && self.server_state.module_editable();
        let mut list = column![
            text("Server type").size(18),
            text("Decides which standard modules the next build includes; modules you added yourself stay as they are. Changing it later needs a rebuild and a restart; characters are kept.").size(13).color(style::MUTED),
        ]
        .spacing(12);
        let selected = match &catalog {
            Some(Ok(catalog)) => current(catalog.entries()),
            Some(Err(error)) => {
                list = list.push(text(error.clone()).size(13).color(style::GOLD));
                None
            }
            None => {
                list = list.push(
                    text("Get the server source first; the server type is chosen from it.")
                        .size(13)
                        .color(style::MUTED),
                );
                None
            }
        };
        let usable = editable && matches!(catalog, Some(Ok(_)));
        for preset in Preset::ALL {
            let chosen = selected == Some(preset);
            let control = if chosen {
                button("Selected")
            } else {
                button(if self.module_state.busy {
                    "Working…"
                } else {
                    "Use"
                })
                .style(style::primary)
                .on_press_maybe(usable.then_some(Message::Modules(super::Input::Preset(preset))))
            };
            list = list.push(
                container(
                    row![
                        column![
                            text(preset.title()).size(16).color(if chosen {
                                style::ICE
                            } else {
                                style::TEXT
                            }),
                            text(preset.about()).size(13).color(style::MUTED),
                        ]
                        .spacing(4)
                        .width(iced::Fill),
                        control
                    ]
                    .spacing(16)
                    .align_y(iced::Center),
                )
                .padding(12)
                .width(iced::Fill)
                .style(style::panel),
            );
        }
        let advanced = if selected.is_none() && matches!(catalog, Some(Ok(_))) {
            "Your own choice of standard modules is in use."
        } else {
            "Want other modules or settings?"
        };
        list = list.push(
            row![
                text(advanced).size(13).color(style::MUTED),
                button("Advanced: choose modules")
                    .on_press(Message::AddonScope(crate::AddonScope::Server)),
            ]
            .spacing(12)
            .align_y(iced::Center),
        );
        if !self.module_state.notice.is_empty() {
            list = list.push(text(&self.module_state.notice).size(13).color(style::GOLD));
        }
        list.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::Source;

    fn entry(name: &str, enabled: bool) -> Entry {
        Entry {
            name: name.into(),
            source: Source::Core,
            enabled,
            required: false,
        }
    }

    #[test]
    fn every_preset_module_is_a_standard_entry() {
        for preset in Preset::ALL {
            for name in preset.modules() {
                assert!(super::super::standard::is_standard(name), "{name}");
            }
        }
        assert!(Preset::Chatter.modules().contains(&"mod-llm-chatter"));
        assert!(!Preset::Playerbots.modules().contains(&"mod-llm-chatter"));
        assert!(Preset::Plain.modules().is_empty());
    }

    #[test]
    fn a_fresh_source_adds_what_the_type_lacks() {
        // A fresh checkout carries only mod-playerbots.
        let entries = [entry("mod-playerbots", true)];
        let change = plan(&entries, Preset::Playerbots);
        assert_eq!(
            change.add,
            [
                "mod-multibot-bridge",
                "mod-ah-bot",
                "mod-quest-minimap-points"
            ]
        );
        assert!(change.enable.is_empty() && change.disable.is_empty());
        let plain = plan(&entries, Preset::Plain);
        assert!(plain.add.is_empty());
        assert_eq!(plain.disable, ["mod-playerbots"]);
    }

    #[test]
    fn switching_types_toggles_without_removing() {
        let entries = [
            entry("mod-ah-bot", true),
            entry("mod-guildhouse", true),
            entry("mod-llm-chatter", false),
            entry("mod-multibot-bridge", true),
            entry("mod-playerbots", true),
            entry("mod-quest-minimap-points", true),
            entry("mod-transmog", true),
        ];
        assert_eq!(current(&entries), None);
        let change = plan(&entries, Preset::Chatter);
        assert!(change.add.is_empty());
        assert_eq!(change.enable, ["mod-llm-chatter"]);
        // mod-transmog is the player's own module: no server type switches it.
        assert_eq!(change.disable, ["mod-guildhouse"]);
        let after = apply(&entries, &change);
        assert_eq!(current(&after), Some(Preset::Chatter));
        assert_eq!(plan(&after, Preset::Chatter), Change::default());
        let none: Vec<_> = after
            .iter()
            .map(|e| entry(&e.name, e.enabled && !is_standard(&e.name)))
            .collect();
        assert_eq!(current(&none), Some(Preset::Plain));
    }

    fn apply(entries: &[Entry], change: &Change) -> Vec<Entry> {
        entries
            .iter()
            .map(|e| {
                let on = (e.enabled || change.enable.contains(&e.name))
                    && !change.disable.contains(&e.name);
                entry(&e.name, on)
            })
            .collect()
    }

    #[test]
    fn a_server_type_leaves_the_players_own_modules_as_they_were() {
        let entries = [
            entry("mod-playerbots", true),
            entry("mod-transmog", true),
            entry("mod-solo-craft", false),
        ];
        for preset in Preset::ALL {
            let change = plan(&entries, preset);
            for own in ["mod-transmog", "mod-solo-craft"] {
                assert!(!change.enable.iter().any(|n| n == own), "{own}");
                assert!(!change.disable.iter().any(|n| n == own), "{own}");
            }
            let after = apply(&entries, &change);
            assert!(after.iter().any(|e| e.name == "mod-transmog" && e.enabled));
            assert!(after
                .iter()
                .any(|e| e.name == "mod-solo-craft" && !e.enabled));
        }
        // Plain switches off the standard modules only; the type still reads as Plain.
        let plain = apply(&entries, &plan(&entries, Preset::Plain));
        assert_eq!(current(&plain), Some(Preset::Plain));
    }
}
