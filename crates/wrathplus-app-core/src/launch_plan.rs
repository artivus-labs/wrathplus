//! Which pre-world steps this run skips, and with what: the command line and the `WOW_*`
//! environment folded into one [`LaunchPlan`] that the login screen, the realm list and character
//! select each read for their own step. With nothing given, every step is the player's: the login
//! screen, then the realm list only when no realm is remembered, then character select.
//!
//! Each flag skips only its own step:
//!
//! | Flag | Env | Step it answers |
//! |---|---|---|
//! | `--server <host[:port]>` | `WOW_HOST` | the logon address, for this session only (never saved) |
//! | `--account <name>` | `WOW_USER` | the account box |
//! | `--password-env <VAR>`, `--password-stdin` | `WOW_PASS` | the password box; with an account, the login submits |
//! | `--realm <name>` | `WOW_REALM` | the realm list |
//! | `--character <name>` | | the selected row on character select |
//! | `--enter-world` | | the Enter World press, on the selected row |
//! | `--fill-only` | `WOW_FILL_ONLY=1` | with `--account`/a password, fill the login boxes and stop: the player presses Login |
//! | `--agent-port <port>` | `WOW_AGENT_PORT` | not a step: opens the agent control port (`WOW_AGENT_TOKEN`) |
//!
//! `WOW_CHAR=<name>` is the scripted fast path the probes use: `--character <name> --enter-world`.
//! So `--account X --password-env Y` logs in and stops at character select.
//!
//! The password never travels as an argument: `--password` is refused, because an argument shows
//! in `ps` and in shell history. It is read from the named environment variable or one line of
//! standard input, and [`LaunchPlan`]'s `Debug` never prints it.

use std::sync::OnceLock;

/// The run's answers for the pre-world steps; see the module table. Also a resource, so a test can
/// stand in a plan the environment cannot spell (a character without Enter World).
#[derive(Clone, Default, PartialEq, Eq, bevy::prelude::Resource)]
pub struct LaunchPlan {
    pub server: Option<String>,
    pub account: Option<String>,
    password: Option<String>,
    pub realm: Option<String>,
    pub character: Option<String>,
    pub enter_world: bool,
    /// `--fill-only`: the credentials fill the login screen and the login waits for the player.
    pub fill_only: bool,
    /// `--agent-port <port>`: open the agent control port on 127.0.0.1 (`wrathplus-agent`; the token
    /// comes from `WOW_AGENT_TOKEN`). `0` lets the system pick a free port, logged at start.
    pub agent_port: Option<u16>,
}

impl std::fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchPlan")
            .field("server", &self.server)
            .field("account", &self.account)
            .field("password", &self.password.as_ref().map(|_| "<set>"))
            .field("realm", &self.realm)
            .field("character", &self.character)
            .field("enter_world", &self.enter_world)
            .field("fill_only", &self.fill_only)
            .field("agent_port", &self.agent_port)
            .finish()
    }
}

impl LaunchPlan {
    /// The password, for the login submit only.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Both credentials are given and not only to fill the boxes: the login submits without typing.
    pub fn logs_in(&self) -> bool {
        self.account.is_some() && self.password.is_some() && !self.fill_only
    }

    /// `--fill-only`'s account and password for the login boxes (either may be empty).
    pub fn fill(&self) -> Option<(&str, &str)> {
        self.fill_only.then(|| {
            (
                self.account.as_deref().unwrap_or_default(),
                self.password.as_deref().unwrap_or_default(),
            )
        })
    }

    /// A test plan with these steps answered and no password.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_steps(character: Option<&str>, enter_world: bool) -> Self {
        LaunchPlan {
            character: character.map(str::to_string),
            enter_world,
            ..Default::default()
        }
    }
}

/// The usage text for a refused command line.
pub(crate) const USAGE: &str = "\
usage: wrathplus [--server <host[:port]>] [--account <name>] \
[--password-env <VAR> | --password-stdin] [--realm <name>] [--character <name>] [--enter-world] \
[--fill-only] [--agent-port <port>]
Each flag skips only its own pre-world step; with none, the login screen shows.
--fill-only puts the account and password in the login boxes and waits for the Login press.
--agent-port opens the agent control port on 127.0.0.1 (token in WOW_AGENT_TOKEN).
The password is never an argument: name an environment variable or pipe it on stdin.";

/// Fold `args` (without the program name) and the environment into a plan. `env` reads a
/// variable; `stdin_line` reads one line of standard input, asked only for `--password-stdin`.
pub(crate) fn parse(
    args: impl IntoIterator<Item = String>,
    env: impl Fn(&str) -> Option<String>,
    stdin_line: impl FnOnce() -> Option<String>,
) -> Result<LaunchPlan, String> {
    let given = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    // The environment first; a flag overrides it.
    let mut plan = LaunchPlan {
        server: given(env("WOW_HOST")),
        account: given(env("WOW_USER")),
        password: given(env("WOW_PASS")),
        realm: given(env("WOW_REALM")),
        character: None,
        enter_world: false,
        fill_only: env("WOW_FILL_ONLY").as_deref() == Some("1"),
        agent_port: match given(env("WOW_AGENT_PORT")) {
            Some(port) => Some(agent_port(&port)?),
            None => None,
        },
    };
    if let Some(name) = given(env("WOW_CHAR")) {
        plan.character = Some(name);
        plan.enter_world = true;
    }

    let mut stdin_line = Some(stdin_line);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        // macOS hands a Finder launch `-psn_<n>_<m>`.
        if arg.starts_with("-psn_") {
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| args.next())
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag.as_str() {
            "--server" => plan.server = Some(value("--server")?),
            "--account" => plan.account = Some(value("--account")?),
            "--password-env" => {
                let var = value("--password-env")?;
                plan.password = Some(
                    given(env(&var)).ok_or_else(|| format!("--password-env: ${var} is not set"))?,
                );
            }
            "--password-stdin" => {
                let line = stdin_line.take().and_then(|read| read());
                plan.password = Some(
                    given(line.map(|l| l.trim_end_matches(['\r', '\n']).to_string()))
                        .ok_or("--password-stdin: no password on standard input")?,
                );
            }
            "--realm" => plan.realm = Some(value("--realm")?),
            "--character" => plan.character = Some(value("--character")?),
            "--enter-world" => plan.enter_world = true,
            "--fill-only" => plan.fill_only = true,
            "--agent-port" => plan.agent_port = Some(agent_port(&value("--agent-port")?)?),
            // Refused without echoing what followed it.
            "--password" | "-p" => {
                return Err(format!(
                    "{flag} is refused: an argument shows in ps and shell history; use \
                     --password-env <VAR> or --password-stdin\n{USAGE}"
                ))
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(plan)
}

fn agent_port(value: &str) -> Result<u16, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("--agent-port: {value:?} is not a port number\n{USAGE}"))
}

static PLAN: OnceLock<LaunchPlan> = OnceLock::new();

/// Parse this process's command line once, at launch; the first install wins.
pub fn install_from_process() -> Result<(), String> {
    let plan = parse(
        std::env::args().skip(1),
        |k| std::env::var(k).ok(),
        || {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).ok()?;
            Some(line)
        },
    )?;
    let _ = PLAN.set(plan);
    Ok(())
}

/// This run's plan: the installed one, or (with none installed, as in tests) the environment's,
/// read fresh so a test's env guard is seen.
pub fn plan() -> LaunchPlan {
    PLAN.get().cloned().unwrap_or_else(|| {
        parse(std::iter::empty(), |k| std::env::var(k).ok(), || None).unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Result<LaunchPlan, String> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        parse(
            args.iter().map(|s| s.to_string()),
            |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()),
            || stdin.map(str::to_string),
        )
    }

    #[test]
    fn no_flags_and_no_env_skip_nothing() {
        let plan = run(&[], &[], None).unwrap();
        assert_eq!(plan, LaunchPlan::default());
        assert!(!plan.logs_in());
    }

    #[test]
    fn each_flag_answers_only_its_own_step() {
        let plan = run(
            &["--account", "Bob", "--password-env", "SECRET_VAR"],
            &[("SECRET_VAR", "hunter2")],
            None,
        )
        .unwrap();
        assert!(plan.logs_in());
        assert_eq!(plan.password(), Some("hunter2"));
        assert_eq!(plan.character, None, "stops at character select");
        assert!(!plan.enter_world);

        let plan = run(&["--account=Bob"], &[], None).unwrap();
        assert_eq!(plan.account.as_deref(), Some("Bob"));
        assert!(!plan.logs_in(), "an account alone only fills the box");

        let plan = run(
            &[
                "--server",
                "10.0.0.5:3724",
                "--realm",
                "AzerothCore",
                "--character",
                "Hero",
            ],
            &[],
            None,
        )
        .unwrap();
        assert_eq!(plan.server.as_deref(), Some("10.0.0.5:3724"));
        assert_eq!(plan.realm.as_deref(), Some("AzerothCore"));
        assert_eq!(plan.character.as_deref(), Some("Hero"));
        assert!(!plan.enter_world, "a character is selected, not entered");
        assert!(run(&["--enter-world"], &[], None).unwrap().enter_world);
        // A non-default logon port rides along verbatim (`realmlist::Realmlist`'s pin).
        let plan = run(&["--server=vps.example.org:2337"], &[], None).unwrap();
        assert_eq!(plan.server.as_deref(), Some("vps.example.org:2337"));
        // `--fill-only`: the credentials fill the boxes and nothing submits.
        let fill = run(
            &["--account", "Bob", "--password-env", "V", "--fill-only"],
            &[("V", "hunter2")],
            None,
        )
        .unwrap();
        assert!(!fill.logs_in());
        assert_eq!(fill.fill(), Some(("Bob", "hunter2")));
        assert_eq!(run(&["--account", "Bob"], &[], None).unwrap().fill(), None);
        assert!(run(&[], &[("WOW_FILL_ONLY", "1")], None).unwrap().fill_only);
    }

    #[test]
    fn the_password_comes_from_stdin_or_a_named_variable_never_an_argument() {
        let plan = run(&["--password-stdin"], &[], Some("hunter2\n")).unwrap();
        assert_eq!(plan.password(), Some("hunter2"));
        for refused in [
            &["--password", "hunter2"][..],
            &["--password=hunter2"],
            &["-p", "hunter2"],
        ] {
            let err = run(refused, &[], None).unwrap_err();
            assert!(
                !err.contains("hunter2"),
                "the refusal never echoes it: {err}"
            );
            assert!(err.contains("--password-env"));
        }
        assert!(run(&["--password-env", "UNSET"], &[], None).is_err());
        assert!(run(&["--password-stdin"], &[], Some("\n")).is_err());
        let shown = format!(
            "{:?}",
            run(&["--password-env", "V"], &[("V", "hunter2")], None).unwrap()
        );
        assert!(
            !shown.contains("hunter2") && shown.contains("<set>"),
            "{shown}"
        );
    }

    #[test]
    fn the_wow_environment_maps_onto_the_same_steps_and_flags_win() {
        let env = [
            ("WOW_HOST", "127.0.0.1:3724"),
            ("WOW_USER", "envuser"),
            ("WOW_PASS", "envpass"),
            ("WOW_REALM", "EnvRealm"),
            ("WOW_CHAR", "Benchone"),
        ];
        let plan = run(&[], &env, None).unwrap();
        assert_eq!(plan.server.as_deref(), Some("127.0.0.1:3724"));
        assert!(plan.logs_in());
        assert_eq!(plan.realm.as_deref(), Some("EnvRealm"));
        assert_eq!(plan.character.as_deref(), Some("Benchone"));
        assert!(
            plan.enter_world,
            "WOW_CHAR is the probes' pick-and-enter fast path"
        );
        let plan = run(
            &["--account", "flaguser", "--character", "Other"],
            &env,
            None,
        )
        .unwrap();
        assert_eq!(plan.account.as_deref(), Some("flaguser"));
        assert_eq!(plan.character.as_deref(), Some("Other"));
        // Empty variables are unset.
        assert_eq!(run(&[], &[("WOW_USER", " ")], None).unwrap().account, None);
    }

    #[test]
    fn unknown_or_valueless_arguments_are_refused_with_usage() {
        assert!(run(&["--bogus"], &[], None).unwrap_err().contains("usage:"));
        assert!(run(&["--account"], &[], None)
            .unwrap_err()
            .contains("needs a value"));
        assert!(
            run(&["-psn_0_12345"], &[], None).is_ok(),
            "a Finder launch's argument"
        );
    }
}

#[cfg(test)]
mod agent_port_tests {
    use super::*;

    fn run(args: &[&str], env: &[(&str, &str)]) -> Result<LaunchPlan, String> {
        parse(
            args.iter().map(|s| s.to_string()),
            |k| {
                env.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            },
            || None,
        )
    }

    #[test]
    fn the_agent_port_comes_from_the_flag_or_the_environment() {
        assert_eq!(run(&[], &[]).unwrap().agent_port, None);
        assert_eq!(
            run(&["--agent-port", "17950"], &[]).unwrap().agent_port,
            Some(17950)
        );
        assert_eq!(
            run(&[], &[("WOW_AGENT_PORT", "0")]).unwrap().agent_port,
            Some(0)
        );
        assert_eq!(
            run(&["--agent-port=1"], &[("WOW_AGENT_PORT", "2")])
                .unwrap()
                .agent_port,
            Some(1),
            "the flag overrides the environment"
        );
        assert!(run(&["--agent-port", "lots"], &[]).is_err());
        assert!(run(&[], &[("WOW_AGENT_PORT", "70000")]).is_err());
        assert!(
            !run(&["--agent-port", "1"], &[]).unwrap().logs_in(),
            "not a login step"
        );
    }
}
