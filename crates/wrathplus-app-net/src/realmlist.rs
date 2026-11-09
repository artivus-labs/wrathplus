//! The realmlist: the `host[:port]` WrathPlus dials for the logon handshake.
//!
//! Build 12340 loads `Config.wtf` first (`0x406c70` → `0x4067f0`), then registers `realmList`
//! (help "Address of realm list server", `0x6b0bf0`) and runs `data\<locale>\realmlist.wtf`, else
//! `realmlist.wtf` beside the executable, as a console script: that file's `set realmlist` wins
//! over `Config.wtf`'s saved `SET realmList` at every launch. There is no UI for it.
//!
//! WrathPlus keeps that order ([`realmlist_wtf`], [`apply_realmlist_wtf`]): `config.toml` holds the
//! saved CVar as `Config.wtf` does, and the first `realmlist.wtf` found overrides it. Deviation:
//! the login screen edits it, and an edit writes `$WRATHPLUS_HOME/realmlist.wtf`, searched first, so
//! WrathPlus never writes into the game install ([`write_realmlist_wtf`]).
//!
//! `--server` (or `$WOW_HOST`) wins for the session, is never written to the file, and is taken
//! verbatim, not through [`normalize`].

use bevy::prelude::*;

/// Installs [`Realmlist`], which must exist before `cvars::load_config` applies the saved value.
pub struct RealmlistPlugin;

/// `realmList`'s change callback: a value that is not an address is ignored with a warning.
pub fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut realmlist: ResMut<Realmlist>) {
    if !ev.is(CVAR_REALMLIST) {
        return;
    }
    match normalize(&ev.new) {
        Some(address) => realmlist.set(&address),
        None => warn!("cvar realmList: unusable realmlist '{}' ignored", ev.new),
    }
}

/// The CVar rows this feature owns ([`crate::cvars::RegisterCvars`]), each with the
/// reference's register site and its default's standing ([`crate::cvars::Registered`]).
pub const CVARS: &[crate::cvars::Registered] = {
    use crate::cvars::deviates;
    &[
        // The logon server address (register site `0x5ab6a6`), a string row judged by
        // `realmlist::on_cvar`.
        deviates(
            CVAR_REALMLIST,
            DEFAULT_REALMLIST,
            "us.logon.worldofwarcraft.com:3724",
            "that host has not resolved since 2019, so shipping it makes every first launch a \
             DNS failure; WrathPlus dials the machine it is running on",
        ),
    ]
};

/// `$WOW_HOST` is the session's realmlist, which a test run must never write into the file.
pub(crate) const SESSION_LEVER: crate::cvars::SessionLever = crate::cvars::SessionLever {
    name: "WOW_HOST",
    taken: || crate::launch_plan::plan().server.is_some(),
    read: |world| {
        let v = world.get_resource::<Realmlist>();
        vec![(CVAR_REALMLIST, v.map(|r| r.address().to_string()))]
    },
};

impl Plugin for RealmlistPlugin {
    fn build(&self, app: &mut App) {
        crate::cvars::RegisterCvars::register_cvars(app, CVARS);
        crate::cvars::RegisterCvars::register_session_lever(app, SESSION_LEVER);
        app.add_observer(on_cvar);
        app.init_resource::<Realmlist>();
        app.add_systems(Startup, apply_realmlist_wtf.after(crate::cvars::CvarLoad));
    }
}

/// The override file's name.
const REALMLIST_WTF: &str = "realmlist.wtf";

/// Where a `realmlist.wtf` is looked for, first found wins: WrathPlus's own (the login edit's), then
/// the install's `Data\<locale>\realmlist.wtf` and the one beside its executable, 12340's order.
fn realmlist_wtf_candidates() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = crate::local_state::home() {
        out.push(home.join(REALMLIST_WTF));
    }
    if let Some(data) = wrathplus_formats::wow_data() {
        let locale = match wrathplus_formats::DataProfile::from_env() {
            Ok(wrathplus_formats::DataProfile::Wrath { locale }) => locale,
            Err(_) => "enUS".to_string(),
        };
        out.push(data.join(&locale).join(REALMLIST_WTF));
        if let Some(root) = data.parent() {
            out.push(root.join(REALMLIST_WTF));
        }
    }
    out
}

/// The address a `realmlist.wtf` sets: its last `set realmlist <value>` line, as the console runs
/// the file top to bottom. Other lines (`set patchlist …`) are skipped.
fn parse_realmlist_wtf(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter(|line| {
            strip_prefix_ci(line.trim(), "set")
                .filter(|rest| rest.starts_with(|c: char| c.is_whitespace()))
                .and_then(|rest| strip_prefix_ci(rest.trim_start(), "realmlist"))
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_whitespace() || c == '='))
        })
        .find_map(normalize)
}

/// The first `realmlist.wtf` that sets an address ([`realmlist_wtf_candidates`]).
pub(crate) fn realmlist_wtf() -> Option<String> {
    realmlist_wtf_candidates().into_iter().find_map(|path| {
        let address = parse_realmlist_wtf(&std::fs::read_to_string(&path).ok()?)?;
        info!("realmlist: {} sets {address}", path.display());
        Some(address)
    })
}

/// At startup, after `config.toml` is folded in: the override file wins, as 12340's does over
/// `Config.wtf`. A `--server`/`$WOW_HOST` address outranks both as the session's starting
/// address (the launcher's default); the login screen can still change it.
fn apply_realmlist_wtf(mut realmlist: ResMut<Realmlist>) {
    if realmlist.from_launch {
        return;
    }
    if let Some(address) = realmlist_wtf() {
        realmlist.set(&address);
    }
}

/// The login edit's persistence: `$WRATHPLUS_HOME/realmlist.wtf`, in the file's own syntax, so the
/// next launch finds it before the install's.
pub fn write_realmlist_wtf(address: &str) -> std::io::Result<()> {
    let Some(home) = crate::local_state::home() else {
        return Ok(()); // a unit test with no state folder
    };
    std::fs::create_dir_all(&home)?;
    std::fs::write(
        home.join(REALMLIST_WTF),
        format!("set realmlist {address}\n"),
    )
}

/// The CVar name, the reference's spelling.
pub const CVAR_REALMLIST: &str = "realmList";

/// Deviation: the local machine's server, not the reference's
/// `us.logon.worldofwarcraft.com:3724`, because that host no longer resolves.
pub const DEFAULT_REALMLIST: &str = "127.0.0.1";

/// The dialog box's `letters` cap. Deviation: 64, where the reference's login boxes cap at 16
/// (`AccountLogin.xml`), because a hostname needs more.
pub const MAX_LETTERS: usize = 64;

/// The address the next logon attempt dials, as `host[:port]` (the port defaults to
/// [`wrathplus_protocol::AUTH_PORT`]). It travels on each [`crate::net::LoginRequest`], so an edit
/// never repoints an attempt in flight.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct Realmlist {
    address: String,
    /// `--server`/`$WOW_HOST` gave the starting address, so `realmlist.wtf` does not replace it at
    /// startup. It is only a default: the login screen's edits still apply.
    from_launch: bool,
}

impl Default for Realmlist {
    fn default() -> Self {
        // `--server` or `$WOW_HOST`.
        match crate::launch_plan::plan().server {
            // Verbatim, not normalized.
            Some(host) => Realmlist {
                address: host,
                from_launch: true,
            },
            _ => Realmlist {
                address: DEFAULT_REALMLIST.to_string(),
                from_launch: false,
            },
        }
    }
}

impl Realmlist {
    /// A realmlist at `address` with no launch default, for tests, since [`Default`] reads
    /// `$WOW_HOST`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn at(address: &str) -> Self {
        Realmlist {
            address: address.to_string(),
            from_launch: false,
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// Points at an already normalized `address`.
    pub fn set(&mut self, address: &str) {
        self.address = address.to_string();
    }
}

/// Normalizes typed or pasted text into a `host[:port]`, checking syntax only; reachability
/// surfaces in the `LOGIN_FAILED` dialog. A pasted `realmlist.wtf` line
/// (`SET realmlist "logon.example.org"`) is unwrapped to its value.
pub fn normalize(input: &str) -> Option<String> {
    let mut s = input.trim();

    // `SET realmlist <value>` or `set realmlist = <value>`.
    if let Some(rest) = strip_prefix_ci(s, "set") {
        if rest.starts_with(|c: char| c.is_whitespace()) {
            if let Some(value) = strip_prefix_ci(rest.trim_start(), "realmlist") {
                s = value.trim_start().strip_prefix('=').unwrap_or(value).trim();
            }
        }
    }
    // One matched pair of quotes only; `trim_matches` would eat a run of them.
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s = &s[1..s.len() - 1];
    }
    let s = s.trim();

    if s.is_empty() || s.chars().count() > MAX_LETTERS {
        return None;
    }
    // Whitespace or control characters mean the paste brought a second word along.
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    // Mirrors `host_port`'s split: a single colon must carry a valid port; two or more colons
    // are an IPv6 literal, left intact.
    if let Some((host, port)) = s.rsplit_once(':') {
        if !host.contains(':') && (host.is_empty() || port.parse::<u16>().is_err()) {
            return None;
        }
    }
    Some(s.to_string())
}

/// `s` without `prefix`, case-insensitively; `str::get` refuses a non-boundary split.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &s[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_realmlist_wtf_sets_its_last_realmlist_line() {
        let file = "set realmlist 10.0.0.1\r\nset patchlist localhost\r\nSET realmList \"127.0.0.1:3725\"\r\n";
        assert_eq!(parse_realmlist_wtf(file).as_deref(), Some("127.0.0.1:3725"));
        assert_eq!(parse_realmlist_wtf("set patchlist x\n"), None);
        assert_eq!(
            parse_realmlist_wtf("set realmlistbn x\n"),
            None,
            "not realmListbn"
        );
        assert_eq!(parse_realmlist_wtf(""), None);
    }

    #[test]
    fn the_login_edit_is_found_first_on_the_next_launch() {
        // `WRATHPLUS_HOME` is process-wide: hold the lock every env-overriding test shares.
        let _l = crate::local_state::test_env::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("azb-realmlist-{}", std::process::id()));
        let _home =
            crate::local_state::test_env::EnvGuard::set("WRATHPLUS_HOME", dir.to_str().unwrap());
        // A VPS's non-default logon port survives the file and reaches the dial.
        write_realmlist_wtf("192.168.1.1:1337").unwrap();
        assert_eq!(
            realmlist_wtf_candidates().first(),
            Some(&dir.join(REALMLIST_WTF))
        );
        assert_eq!(realmlist_wtf().as_deref(), Some("192.168.1.1:1337"));
        let startup = |realmlist: Realmlist| {
            let mut app = App::new();
            app.insert_resource(realmlist)
                .add_systems(Update, apply_realmlist_wtf);
            app.update();
            app.world().resource::<Realmlist>().address().to_string()
        };
        let address = startup(Realmlist::at(DEFAULT_REALMLIST));
        assert_eq!(address, "192.168.1.1:1337");
        assert_eq!(
            wrathplus_protocol::host_port(&address, wrathplus_protocol::AUTH_PORT),
            ("192.168.1.1", 1337)
        );
        // `--server vps.example.org:2337` outranks the file at startup, port and all.
        let launched = startup(Realmlist {
            address: "vps.example.org:2337".into(),
            from_launch: true,
        });
        assert_eq!(
            wrathplus_protocol::host_port(&launched, wrathplus_protocol::AUTH_PORT),
            ("vps.example.org", 2337)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bare_host_passes_through() {
        assert_eq!(normalize("localhost").as_deref(), Some("localhost"));
        assert_eq!(
            normalize("  logon.example.org  ").as_deref(),
            Some("logon.example.org"),
        );
        assert_eq!(
            normalize("127.0.0.1:3725").as_deref(),
            Some("127.0.0.1:3725")
        );
    }

    #[test]
    fn a_pasted_wtf_line_is_unwrapped() {
        for line in [
            r#"SET realmlist "logon.example.org""#,
            r#"set realmlist "logon.example.org""#,
            "SET realmlist logon.example.org",
            r#"set realmlist = "logon.example.org""#,
            r#"   SET   realmlist   "logon.example.org"   "#,
        ] {
            assert_eq!(
                normalize(line).as_deref(),
                Some("logon.example.org"),
                "unwrapping {line:?}",
            );
        }
    }

    #[test]
    fn a_host_named_like_the_prefix_is_left_alone() {
        assert_eq!(
            normalize("settings.example.org").as_deref(),
            Some("settings.example.org")
        );
        assert_eq!(normalize("set").as_deref(), Some("set"));
    }

    #[test]
    fn nothing_usable_is_rejected() {
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize(r#"SET realmlist """#), None);
        // A paste that brought a second word along.
        assert_eq!(normalize("logon.example.org and more"), None);
        // Longer than the box can hold.
        assert_eq!(normalize(&"a".repeat(MAX_LETTERS + 1)), None);
    }

    #[test]
    fn a_single_colon_must_carry_a_real_port() {
        assert_eq!(normalize("logon.example.org:notaport"), None);
        assert_eq!(normalize("logon.example.org:"), None);
        assert_eq!(normalize("logon.example.org:99999"), None); // past u16
        assert_eq!(normalize(":3724"), None); // no host
        assert_eq!(
            normalize("logon.example.org:3724").as_deref(),
            Some("logon.example.org:3724"),
        );
    }

    #[test]
    fn an_ipv6_literal_is_left_intact() {
        assert_eq!(normalize("::1").as_deref(), Some("::1"));
        assert_eq!(normalize("fe80::1").as_deref(), Some("fe80::1"));
        // And what normalize passes is what the protocol splits.
        assert_eq!(
            wrathplus_protocol::host_port("::1", wrathplus_protocol::AUTH_PORT),
            ("::1", wrathplus_protocol::AUTH_PORT),
        );
    }

    #[test]
    fn the_default_is_a_host_the_protocol_can_split() {
        let (host, port) =
            wrathplus_protocol::host_port(DEFAULT_REALMLIST, wrathplus_protocol::AUTH_PORT);
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, wrathplus_protocol::AUTH_PORT);
    }

    #[test]
    fn a_launch_address_is_a_default_the_login_screen_can_change() {
        let mut r = Realmlist {
            address: "127.0.0.1:13724".into(),
            from_launch: true,
        };
        r.set("wrathplus.artivus.com.au");
        assert_eq!(r.address(), "wrathplus.artivus.com.au");
    }

    #[test]
    fn a_realmlist_takes_writes() {
        let mut r = Realmlist::at(DEFAULT_REALMLIST);
        r.set("logon.example.org");
        assert_eq!(r.address(), "logon.example.org");
    }
}
