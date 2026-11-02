//! The standard module set: the modules a local Playerbots realm usually wants, each one click
//! from being in the next build. Anything else is a custom source.
use crate::modules::Source;

pub(crate) struct Standard {
    /// The module directory name, which is also its catalog name.
    pub name: &'static str,
    pub title: &'static str,
    /// GitHub owner/repository, imported on Add unless the core checkout already carries it.
    pub repository: &'static str,
    pub about: &'static str,
}

pub(crate) const STANDARD: &[Standard] = &[
    Standard {
        name: "mod-playerbots",
        title: "Playerbots",
        repository: "mod-playerbots/mod-playerbots",
        about: "Computer-controlled players. Invite them to your party and they quest, fight and level with you; random bots fill the world.",
    },
    Standard {
        name: "mod-multibot-bridge",
        title: "MultiBot bridge",
        repository: "Wishmaster117/mod-multibot-bridge",
        about: "Server side of the MultiBot addon: its buttons command your bots directly instead of through chat commands.",
    },
    Standard {
        name: "mod-llm-chatter",
        title: "LLM Chatter",
        repository: "Hokken/mod-llm-chatter",
        about: "Bots talk in character, using a language model you configure. Comes with a bridge service; save its provider settings, then start it from Server setup.",
    },
    Standard {
        name: "mod-guildhouse",
        title: "Guild house",
        repository: "azerothcore/mod-guildhouse",
        about: "Guilds can buy a private, phased guild house and furnish it with trainers, vendors and portals.",
    },
    Standard {
        name: "mod-player-bot-guildhouse",
        title: "Playerbot guild house",
        repository: "DustinHendrickson/mod-player-bot-guildhouse",
        about: "Bots in your guild visit and leave the guild house over time. Needs Playerbots and Guild house.",
    },
    Standard {
        name: "mod-ah-bot",
        title: "Auction house bot",
        repository: "azerothcore/mod-ah-bot",
        about: "Keeps the auction houses stocked, so a small realm still has a market. The server sets up its seller character on first start.",
    },
];

pub(crate) fn is_standard(name: &str) -> bool {
    STANDARD.iter().any(|s| s.name == name)
}

/// Where a catalog entry's code comes from, in words.
pub(crate) fn source_label(source: &Source) -> String {
    match source {
        Source::Core => "Included in the server source folder".into(),
        Source::Local { path } => format!("Local folder · {}", path.display()),
        Source::Github {
            repository,
            revision,
            ..
        } => format!(
            "github.com/{repository} @ {}",
            revision.get(..7).unwrap_or(revision)
        ),
    }
}

/// Open a module's GitHub page in the player's browser. No shell.
pub(crate) fn open_repository(repository: &str) -> Result<(), String> {
    let url = format!("https://github.com/{repository}");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_standard_module_imports_under_its_own_name() {
        let mut names = std::collections::HashSet::new();
        for module in STANDARD {
            assert!(names.insert(module.name), "duplicate {}", module.name);
            // Import names a module after its repository; the card finds it by that name.
            assert_eq!(module.repository.rsplit('/').next(), Some(module.name));
            assert_eq!(
                crate::modules::normalize_repository(module.repository).as_deref(),
                Ok(module.repository)
            );
        }
        assert!(is_standard("mod-ah-bot") && !is_standard("mod-transmog"));
    }

    #[test]
    fn sources_read_as_words_not_debug_output() {
        assert_eq!(
            source_label(&Source::Github {
                repository: "azerothcore/mod-ah-bot".into(),
                revision: "c11d8310000000000000000000000000000000ff".into(),
                path: "/x".into(),
            }),
            "github.com/azerothcore/mod-ah-bot @ c11d831"
        );
        assert_eq!(
            source_label(&Source::Core),
            "Included in the server source folder"
        );
    }
}
