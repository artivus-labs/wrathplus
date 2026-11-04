use super::environment::{check_name, check_value, env_name, Environment};
use super::module_settings::{Kind, MODULES, PLAYERBOTS};
use super::settings::Settings;
use super::*;
use crate::setup;

fn temp_config(tag: &str) -> (PathBuf, setup::Config) {
    let root = std::env::temp_dir().join(format!("server-env-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut config = setup::Config::defaults();
    config.repo = root.clone();
    config.data = PathBuf::new();
    (root, config)
}

#[test]
fn env_names_match_azerothcore_ini_key_conversion() {
    // The examples in Config.cpp's IniKeyToEnvVarKey comment, plus its digit rules.
    assert_eq!(env_name("SomeConfig"), "AC_SOME_CONFIG");
    assert_eq!(env_name("myNestedConfig.opt1"), "AC_MY_NESTED_CONFIG_OPT_1");
    assert_eq!(env_name("LogDB.Opt.ClearTime"), "AC_LOG_DB_OPT_CLEAR_TIME");
    assert_eq!(env_name("Rate.XP-Kill 2x"), "AC_RATE_XP_KILL_2_X");
    // Every surfaced key, spelled out.
    let expected = [
        (
            "Playerbots.RandomBotAutologin",
            "AC_PLAYERBOTS_RANDOM_BOT_AUTOLOGIN",
        ),
        ("Playerbots.MinRandomBots", "AC_PLAYERBOTS_MIN_RANDOM_BOTS"),
        ("Playerbots.MaxRandomBots", "AC_PLAYERBOTS_MAX_RANDOM_BOTS"),
        (
            "Playerbots.AddClassAccountPoolSize",
            "AC_PLAYERBOTS_ADD_CLASS_ACCOUNT_POOL_SIZE",
        ),
        (
            "Playerbots.RandomBotMinLevel",
            "AC_PLAYERBOTS_RANDOM_BOT_MIN_LEVEL",
        ),
        (
            "Playerbots.RandomBotMaxLevel",
            "AC_PLAYERBOTS_RANDOM_BOT_MAX_LEVEL",
        ),
        (
            "Playerbots.DowngradeMaxLevelBot",
            "AC_PLAYERBOTS_DOWNGRADE_MAX_LEVEL_BOT",
        ),
        (
            "Playerbots.MinRandomBotRandomizeTime",
            "AC_PLAYERBOTS_MIN_RANDOM_BOT_RANDOMIZE_TIME",
        ),
        (
            "Playerbots.MaxRandomBotRandomizeTime",
            "AC_PLAYERBOTS_MAX_RANDOM_BOT_RANDOMIZE_TIME",
        ),
        (
            "Playerbots.EnablePeriodicOnlineOffline",
            "AC_PLAYERBOTS_ENABLE_PERIODIC_ONLINE_OFFLINE",
        ),
        (
            "Playerbots.PeriodicOnlineOfflineRatio",
            "AC_PLAYERBOTS_PERIODIC_ONLINE_OFFLINE_RATIO",
        ),
    ];
    let surfaced: Vec<_> = MODULES.iter().flat_map(|m| m.settings).collect();
    assert_eq!(surfaced.len(), expected.len());
    for (setting, (key, name)) in surfaced.iter().zip(expected) {
        assert_eq!(setting.key, key);
        assert_eq!(setting.env(), name);
        check_name(name).unwrap();
    }
}

#[test]
fn names_and_values_are_checked() {
    for good in ["AC_X", "MANAGED_ADDCLASS_POOL", "_A1", "TZ"] {
        check_name(good).unwrap();
    }
    for bad in ["", "ac_x", "1A", "A-B", "A B", "A=B", "Ä"] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    for good in ["", "1000", "a b $HOME \"q\" #c", r"C:\x\y"] {
        check_value("X", good).unwrap();
    }
    for bad in ["a\nb", "a\rb", "a\0", "it's", r"ends\"] {
        assert!(check_value("X", bad).is_err(), "{bad:?}");
    }
}

#[test]
fn debug_never_shows_values() {
    let mut environment = Environment::default();
    environment.set("AC_LLM_API_KEY", "sk-not-a-real-value");
    let shown = format!("{environment:?}");
    assert!(shown.contains("AC_LLM_API_KEY"));
    assert!(!shown.contains("sk-not"));
    let settings = Settings {
        environment,
        ..Settings::default()
    };
    assert!(!format!("{settings:?}").contains("sk-not"));
    let input = environment_ui::EnvInput::Set("AC_LLM_API_KEY".into(), "sk-not".into());
    assert!(!format!("{input:?}").contains("sk-not"));
}

#[test]
fn missing_settings_mean_defaults_and_saved_values_win() {
    let mut environment = Environment::default();
    environment.set("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "1500");
    environment.set("AC_WORLD_SOMETHING", "x");
    let all = environment.effective();
    assert_eq!(all.get("AC_PLAYERBOTS_MAX_RANDOM_BOTS"), Some("1500"));
    assert_eq!(all.get("AC_PLAYERBOTS_MIN_RANDOM_BOTS"), Some("1000"));
    assert_eq!(
        all.get("AC_PLAYERBOTS_ADD_CLASS_ACCOUNT_POOL_SIZE"),
        Some("50")
    );
    assert_eq!(all.get("AC_PLAYERBOTS_DOWNGRADE_MAX_LEVEL_BOT"), Some("1"));
    assert_eq!(
        all.get("AC_PLAYERBOTS_MAX_RANDOM_BOT_RANDOMIZE_TIME"),
        Some("172800")
    );
    assert_eq!(all.get("AC_WORLD_SOMETHING"), Some("x"));
    assert!(all.render().contains("AC_WORLD_SOMETHING='x'\n"));
}

#[test]
fn typed_settings_keep_module_limits() {
    let mut environment = Environment::default();
    environment.validate().unwrap();
    Environment::default().effective().validate().unwrap();
    for (name, bad) in [
        ("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "lots"),
        ("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "-1"),
        ("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "3000000000"),
        ("AC_PLAYERBOTS_DOWNGRADE_MAX_LEVEL_BOT", "yes"),
        ("AC_PLAYERBOTS_RANDOM_BOT_MIN_LEVEL", "0"),
        ("AC_PLAYERBOTS_PERIODIC_ONLINE_OFFLINE_RATIO", "0.5"),
        ("AC_PLAYERBOTS_PERIODIC_ONLINE_OFFLINE_RATIO", "NaN"),
        // urand(min, max) asserts min <= max.
        ("AC_PLAYERBOTS_MIN_RANDOM_BOT_RANDOMIZE_TIME", "200000"),
        ("AC_PLAYERBOTS_RANDOM_BOT_MIN_LEVEL", "81"),
    ] {
        let mut wrong = environment.clone();
        wrong.set(name, bad);
        assert!(wrong.validate().is_err(), "{name}={bad}");
    }
    // No invented caps: large populations and long times are the module's business.
    environment.set("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "5000");
    environment.set("AC_PLAYERBOTS_MIN_RANDOM_BOTS", "0");
    environment.set("AC_PLAYERBOTS_MAX_RANDOM_BOT_RANDOMIZE_TIME", "1209600");
    environment.set("AC_PLAYERBOTS_PERIODIC_ONLINE_OFFLINE_RATIO", "3.5");
    environment.validate().unwrap();
    let mut unknown = Environment::default();
    unknown.set("lower", "1");
    assert!(unknown.validate().is_err());
}

#[test]
fn durations_read_as_days_or_hours() {
    let max = PLAYERBOTS
        .setting("Playerbots.MaxRandomBotRandomizeTime")
        .unwrap();
    assert_eq!(max.kind, Kind::Seconds);
    assert_eq!(max.describe("172800").as_deref(), Some("2 days"));
    assert_eq!(max.describe("7200").as_deref(), Some("2 hours"));
    assert_eq!(max.describe("1209600").as_deref(), Some("14 days"));
    assert_eq!(max.describe("x"), None);
}

#[test]
fn server_types_write_defaults_or_no_bots() {
    for module in MODULES {
        for (key, value) in module.off {
            let setting = module.setting(key).expect("off values are typed settings");
            setting.check(value).unwrap();
        }
    }
    let mut environment = Environment::default();
    environment.set("AC_PLAYERBOTS_MAX_RANDOM_BOTS", "10");
    environment.set("AC_OTHER", "kept");
    PLAYERBOTS.apply_off(&mut environment);
    assert_eq!(environment.get("AC_PLAYERBOTS_MAX_RANDOM_BOTS"), Some("0"));
    assert_eq!(
        environment.get("AC_PLAYERBOTS_RANDOM_BOT_AUTOLOGIN"),
        Some("0")
    );
    assert_eq!(
        environment.get("AC_PLAYERBOTS_ADD_CLASS_ACCOUNT_POOL_SIZE"),
        Some("0")
    );
    PLAYERBOTS.apply_defaults(&mut environment);
    assert_eq!(
        environment.get("AC_PLAYERBOTS_MAX_RANDOM_BOTS"),
        Some("2000")
    );
    assert_eq!(
        environment.get("AC_PLAYERBOTS_RANDOM_BOT_AUTOLOGIN"),
        Some("1")
    );
    assert_eq!(environment.get("AC_OTHER"), Some("kept"));
    assert_eq!(environment, environment.effective());

    let (root, config) = temp_config("types");
    let mut server = Server::default();
    server.apply_server_type(&config, &[]).unwrap();
    let saved = Settings::load(&config).unwrap();
    assert_eq!(
        saved.environment.get("AC_PLAYERBOTS_MIN_RANDOM_BOTS"),
        Some("0")
    );
    server
        .apply_server_type(&config, &["mod-playerbots", "mod-ah-bot"])
        .unwrap();
    let saved = Settings::load(&config).unwrap();
    assert_eq!(
        saved.environment.get("AC_PLAYERBOTS_MIN_RANDOM_BOTS"),
        Some("1000")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_settings_files_load_and_unchanged_ones_stay_byte_identical() {
    let old = r#"{"source":"/s","data":"/d","reuse_image":true}"#;
    let settings: Settings = serde_json::from_str(old).unwrap();
    assert!(settings.environment.is_empty());
    assert!(!serde_json::to_string(&settings)
        .unwrap()
        .contains("environment"));
    let mut settings = settings;
    settings.environment.set("AC_X", "1");
    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains(r#""environment":{"AC_X":"1"}"#));
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.environment, settings.environment);
}

#[test]
fn env_file_is_private_and_complete() {
    let (root, config) = temp_config("file");
    let path = Settings::env_file(&config);
    let mut environment = Environment::default();
    environment.set("MANAGED_EXTRA", "a $b");
    environment.write(&config, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, environment.effective().render());
    assert!(text.contains("MANAGED_EXTRA='a $b'\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    environment.set("AC_BAD", "two\nlines");
    assert!(environment.write(&config, &path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compose_gets_the_file_path_never_values() {
    let config = setup::Config::defaults();
    let mut settings = Settings::default();
    settings.environment.set("AC_SECRET_THING", "do-not-pass");
    let cmd = command::compose(&config, &settings, command::START);
    let env: Vec<_> = cmd.get_envs().collect();
    assert!(env.iter().any(|(k, v)| *k == "SERVER_ENV_FILE"
        && v.map(std::path::Path::new) == Some(Settings::env_file(&config).as_path())));
    assert!(env
        .iter()
        .any(|(k, v)| *k == "COMPOSE_DISABLE_ENV_FILE" && *v == Some(std::ffi::OsStr::new("1"))));
    for (k, v) in env {
        assert!(!k.to_string_lossy().starts_with("AC_"));
        assert!(!v
            .unwrap_or_default()
            .to_string_lossy()
            .contains("do-not-pass"));
    }
    assert!(!cmd
        .get_args()
        .any(|a| a.to_string_lossy().contains("do-not-pass")));
}

#[test]
fn compose_defaults_match_the_manager_defaults() {
    let recipe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/server");
    let defaults = std::fs::read_to_string(recipe.join("server-defaults.env")).unwrap();
    let lines: String = defaults
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(lines, Environment::default().effective().render());
    let compose = std::fs::read_to_string(recipe.join("compose.yaml")).unwrap();
    assert!(compose.contains("path: ${SERVER_ENV_FILE:-./server-defaults.env}"));
    assert!(!compose.contains("MANAGED_BOT_COUNT"));
    let runtime = std::fs::read_to_string(recipe.join("runtime.sh")).unwrap();
    for forced in [
        "MinRandomBots",
        "MaxRandomBots",
        "AddClassAccountPoolSize",
        "MANAGED_BOT_COUNT",
    ] {
        assert!(!runtime.contains(forced), "runtime.sh still sets {forced}");
    }
}
