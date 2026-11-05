//! Typed server settings for standard modules: a small declarative table per module, written as
//! AzerothCore environment variables (see `environment`). Bounds are only the module's own hard
//! limits (its value types, and pairs it hands to `urand(min, max)`, which asserts `min <= max`).
//! Another standard module adds its own `Module` entry to `MODULES`.
use super::environment::{env_name, Environment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A whole number the module reads as `int32`.
    Whole,
    /// A duration in seconds the module reads as `int32`.
    Seconds,
    /// 0 or 1.
    Switch,
    /// A decimal the module reads as `float`.
    Decimal,
}

#[derive(Debug)]
pub struct Setting {
    /// The module's config key, as in its `.conf.dist`.
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    pub default: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug)]
pub struct Module {
    /// The catalog name; the settings show while this module is selected.
    pub module: &'static str,
    pub title: &'static str,
    pub settings: &'static [Setting],
    /// Config keys the module passes to `urand(low, high)`: low must not exceed high.
    pub ordered: &'static [(&'static str, &'static str)],
    /// Values a server type without this module writes, so an image built before the change
    /// behaves the same until it is rebuilt.
    pub off: &'static [(&'static str, &'static str)],
}

const fn setting(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        kind,
        default,
        min: None,
        max: None,
    }
}
const fn at_least(mut setting: Setting, min: f64) -> Setting {
    setting.min = Some(min);
    setting
}

/// mod-playerbots (conf/playerbots.conf.dist at the pinned revision). Population and turnover:
/// see packaging/server/README.md for why these knobs.
pub const PLAYERBOTS: Module = Module {
    module: "mod-playerbots",
    title: "Playerbots",
    settings: &[
        setting("Playerbots.RandomBotAutologin", "Random bots log in", "Random bots fill the world. Off: only the bots you add yourself.", Kind::Switch, "1"),
        at_least(setting("Playerbots.MinRandomBots", "Online random bots: at least", "Playerbots keeps a number between these two online and re-picks it every half hour to two hours. Bot accounts (ten characters each) are created on the first start after raising it, which takes a while.", Kind::Whole, "1000"), 0.0),
        at_least(setting("Playerbots.MaxRandomBots", "Online random bots: at most", "", Kind::Whole, "2000"), 0.0),
        at_least(setting("Playerbots.AddClassAccountPoolSize", "Addclass bot accounts", "Accounts of ready-made bots for .playerbots bot addclass <class> (one bot of each class per account). 0 turns the command off.", Kind::Whole, "50"), 0.0),
        at_least(setting("Playerbots.RandomBotMinLevel", "Lowest random bot level", "New and restarted random bots start between these levels.", Kind::Whole, "1"), 1.0),
        at_least(setting("Playerbots.RandomBotMaxLevel", "Highest random bot level", "Above the realm's level cap counts as the cap.", Kind::Whole, "80"), 1.0),
        setting("Playerbots.DowngradeMaxLevelBot", "Restart bots that reach the highest level", "At its next re-randomize a bot at the highest level starts again at the lowest, in a starting zone. Bots below it keep their level.", Kind::Switch, "1"),
        at_least(setting("Playerbots.MinRandomBotRandomizeTime", "Re-randomize after at least", "Each bot re-randomizes (new gear and spec, and the restart above) between these times. Upstream's 14 days keeps a bot at the top level for up to two weeks.", Kind::Seconds, "7200"), 0.0),
        at_least(setting("Playerbots.MaxRandomBotRandomizeTime", "Re-randomize after at most", "", Kind::Seconds, "172800"), 0.0),
        setting("Playerbots.EnablePeriodicOnlineOffline", "Rotate which bots are online", "Creates more bots than are online (the ratio below) and swaps them in and out over their in-world times. Off: the same bots stay online.", Kind::Switch, "0"),
        at_least(setting("Playerbots.PeriodicOnlineOfflineRatio", "Bots created per online bot", "Only with rotation on.", Kind::Decimal, "2.0"), 1.0),
        at_least(setting("Playerbots.RandomBotGuildCount", "Random bot guilds", "Random bots form up to this many guilds of their own (up to 15 bots each). 0: random bots stay out of guilds.", Kind::Whole, "20"), 0.0),
    ],
    ordered: &[
        ("Playerbots.MinRandomBotRandomizeTime", "Playerbots.MaxRandomBotRandomizeTime"),
        ("Playerbots.RandomBotMinLevel", "Playerbots.RandomBotMaxLevel"),
    ],
    off: &[
        ("Playerbots.RandomBotAutologin", "0"),
        ("Playerbots.MinRandomBots", "0"),
        ("Playerbots.MaxRandomBots", "0"),
        ("Playerbots.AddClassAccountPoolSize", "0"),
        ("Playerbots.RandomBotGuildCount", "0"),
    ],
};

pub const MODULES: &[Module] = &[PLAYERBOTS];

impl Setting {
    pub fn env(&self) -> String {
        env_name(self.key)
    }
    /// The saved value, or the default when the list does not name it.
    pub fn value<'a>(&'a self, environment: &'a Environment) -> &'a str {
        environment.get(&self.env()).unwrap_or(self.default)
    }
    fn number(&self, value: &str) -> Result<f64, String> {
        let value = value.trim();
        let parsed = match self.kind {
            Kind::Whole | Kind::Seconds => value.parse::<i32>().ok().map(f64::from),
            Kind::Switch => {
                matches!(value, "0" | "1").then(|| if value == "1" { 1.0 } else { 0.0 })
            }
            Kind::Decimal => value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .map(f64::from),
        };
        parsed.ok_or_else(|| {
            format!(
                "{}: {}",
                self.label,
                match self.kind {
                    Kind::Whole => "enter a whole number",
                    Kind::Seconds => "enter a whole number of seconds",
                    Kind::Switch => "enter 0 or 1",
                    Kind::Decimal => "enter a number",
                }
            )
        })
    }
    pub fn check(&self, value: &str) -> Result<(), String> {
        let number = self.number(value)?;
        if let Some(min) = self.min.filter(|min| number < *min) {
            return Err(format!("{}: at least {min}", self.label));
        }
        if let Some(max) = self.max.filter(|max| number > *max) {
            return Err(format!("{}: at most {max}", self.label));
        }
        Ok(())
    }
    /// A reading aid for durations ("2 days"), when the value is valid.
    pub fn describe(&self, value: &str) -> Option<String> {
        if self.kind != Kind::Seconds {
            return None;
        }
        let seconds = value.trim().parse::<i64>().ok()?;
        let (amount, unit) = if seconds >= 86400 && seconds % 3600 == 0 {
            (seconds as f64 / 86400.0, "day")
        } else if seconds >= 3600 {
            (seconds as f64 / 3600.0, "hour")
        } else {
            (seconds as f64 / 60.0, "minute")
        };
        let amount = (amount * 10.0).round() / 10.0;
        Some(format!(
            "{amount} {unit}{}",
            if amount == 1.0 { "" } else { "s" }
        ))
    }
}

impl Module {
    pub fn setting(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.key == key)
    }
    /// Reset every typed setting to its default (a server type that builds this module).
    pub fn apply_defaults(&self, environment: &mut Environment) {
        for setting in self.settings {
            environment.set(setting.env(), setting.default);
        }
    }
    /// Write the values of a server without this module.
    pub fn apply_off(&self, environment: &mut Environment) {
        for (key, value) in self.off {
            environment.set(env_name(key), *value);
        }
    }
}

/// Every typed setting in range, and every ordered pair in order (defaults filling gaps).
pub fn validate(environment: &Environment) -> Result<(), String> {
    for module in MODULES {
        for setting in module.settings {
            if let Some(value) = environment.get(&setting.env()) {
                setting.check(value)?;
            }
        }
        for (low, high) in module.ordered {
            let (Some(low), Some(high)) = (module.setting(low), module.setting(high)) else {
                continue;
            };
            let (Ok(a), Ok(b)) = (
                low.number(low.value(environment)),
                high.number(high.value(environment)),
            ) else {
                continue;
            };
            if a > b {
                return Err(format!(
                    "{} must not be more than {}.",
                    low.label,
                    high.label.to_lowercase()
                ));
            }
        }
    }
    Ok(())
}
