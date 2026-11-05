use super::*;
use crate::module_ui::{select_modules, Preset};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A core checkout carrying the Playerbots type's modules, LLM Chatter, and one module of the
/// player's own (mod-transmog), so selecting a type needs no import.
struct Fixture {
    root: PathBuf,
    config: Config,
    core: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "manager-profiles-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let core = root.join("core");
        for module in [
            "mod-playerbots",
            "mod-multibot-bridge",
            "mod-ah-bot",
            "mod-quest-minimap-points",
            "mod-llm-chatter",
            "mod-transmog",
        ] {
            std::fs::create_dir_all(core.join("modules").join(module).join("src")).unwrap();
        }
        std::fs::write(core.join("CMakeLists.txt"), "").unwrap();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let config = Config {
            repo: root.join("repo"),
            data: root.join("assets"),
            target: root.join("target"),
            locale: "enUS".into(),
            server: "127.0.0.1:13724".into(),
        };
        Self { root, config, core }
    }
    fn manager(&self) -> crate::Manager {
        let mut manager = crate::Manager {
            config: self.config.clone(),
            ..Default::default()
        };
        manager.profile = active(&self.config);
        let _ = manager.server_update(crate::server::Input::Source(
            self.core.to_string_lossy().into_owned(),
        ));
        manager
    }
    /// What the Server page's server type button does, synchronously.
    fn select_type(&self, manager: &mut crate::Manager, preset: Preset) {
        manager
            .server_state
            .apply_server_type(&self.config, preset.modules())
            .unwrap();
        select_modules(&self.config, &self.core, preset).unwrap();
    }
    fn enabled(&self, name: &str) -> bool {
        catalog(&self.config, &self.core)
            .entries()
            .iter()
            .find(|e| e.name == name)
            .unwrap()
            .enabled
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn catalog(config: &Config, core: &Path) -> crate::modules::Catalog {
    crate::modules::Catalog::load(config, core).unwrap()
}

fn min_bots(manager: &crate::Manager) -> Option<String> {
    manager
        .server_state
        .environment()
        .get("AC_PLAYERBOTS_MIN_RANDOM_BOTS")
        .map(str::to_owned)
}

#[test]
fn switching_keeps_each_profiles_choices_and_copies_into_a_new_one() {
    let mut store = Store {
        active: "A".into(),
        saved: BTreeMap::new(),
    };
    let mut environment = Environment::default();
    environment.set("AC_X", "1");
    let a = Selection {
        modules: Some(BTreeMap::from([("mod-x".into(), true)])),
        environment,
    };
    // B was never selected: it starts as a copy of A.
    assert_eq!(store.switch("B", a.clone()), a);
    assert_eq!(store.active, "B");
    let b = Selection::default();
    assert_eq!(store.switch("A", b.clone()), a);
    assert_eq!(store.switch("B", a.clone()), b);
    assert_eq!(store.saved.len(), 1);
    // Environment values stay out of Debug output.
    let mut secret = Environment::default();
    secret.set("AC_LLM_CHATTER_API_KEY", "do-not-print");
    let shown = format!(
        "{:?}",
        Selection {
            modules: None,
            environment: secret
        }
    );
    assert!(!shown.contains("do-not-print"));
}

#[test]
fn a_server_type_applies_to_the_active_profile_only() {
    let fixture = Fixture::new();
    let mut manager = fixture.manager();
    assert_eq!(manager.profile, NAMES[0]);
    fixture.select_type(&mut manager, Preset::Playerbots);
    assert!(fixture.enabled("mod-playerbots") && fixture.enabled("mod-transmog"));
    assert!(!fixture.enabled("mod-llm-chatter"));

    // A profile selected for the first time starts from the live choices; the type chosen there
    // stays there.
    manager.select_profile(NAMES[2].into());
    assert_eq!(manager.profile, NAMES[2]);
    assert!(fixture.enabled("mod-playerbots"));
    fixture.select_type(&mut manager, Preset::Plain);
    assert!(!fixture.enabled("mod-playerbots"));
    // The player's own module is not part of any type.
    assert!(fixture.enabled("mod-transmog"));
    assert_eq!(min_bots(&manager).as_deref(), Some("0"));
    // A module choice of the player's own is the profile's too.
    let mut live = catalog(&fixture.config, &fixture.core);
    live.set_enabled("mod-transmog", false).unwrap();
    live.save(&fixture.config).unwrap();

    manager.select_profile(NAMES[0].into());
    let entries = catalog(&fixture.config, &fixture.core);
    assert_eq!(
        crate::module_ui::current_type(entries.entries()),
        Some(Preset::Playerbots)
    );
    assert!(fixture.enabled("mod-transmog"));
    assert_eq!(min_bots(&manager).as_deref(), Some("1000"));

    manager.select_profile(NAMES[2].into());
    assert!(!fixture.enabled("mod-playerbots") && !fixture.enabled("mod-transmog"));
    assert_eq!(min_bots(&manager).as_deref(), Some("0"));
    // The settings file and the active profile are saved: a restarted Manager agrees.
    assert_eq!(active(&fixture.config), NAMES[2]);
    let restarted = fixture.manager();
    assert_eq!(restarted.profile, NAMES[2]);
    assert_eq!(min_bots(&restarted).as_deref(), Some("0"));
}

#[test]
fn no_switch_under_a_server_operation_or_to_an_unknown_profile() {
    let fixture = Fixture::new();
    let mut manager = fixture.manager();
    manager.module_state.busy = true;
    manager.select_profile(NAMES[1].into());
    assert_eq!(manager.profile, NAMES[0]);
    assert!(manager.notice.contains("Wait"));
    manager.module_state.busy = false;
    manager.select_profile("Someone else's".into());
    assert_eq!(manager.profile, NAMES[0]);
    assert!(!path(&fixture.config).exists());
}
