//! Agents on the managed realm (issue #63): each agent has its own account and its own headless
//! client, driven by an external AI agent through the client's control port (`--agent-port`).
//! Accounts are created over the realm's admin command port; passwords and port tokens live in
//! private files beside the Manager state and never reach a command line.
#[cfg(test)]
mod tests;
mod ui;

pub(crate) use ui::Input;

use crate::setup::{self, Config};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

type Result<T> = std::result::Result<T, String>;

/// The managed realm's auth address and name (`packaging/server/runtime.sh`).
const REALM_ADDRESS: &str = "127.0.0.1:13724";
const REALM_NAME: &str = "WrathPlus Local";
/// Control ports are handed out from here up, one per agent.
const FIRST_PORT: u16 = 17950;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Agent {
    /// What the player called it; also the base of the account name.
    pub name: String,
    pub account: String,
    pub port: u16,
}

/// The saved agents and the clients running for them.
#[derive(Default)]
pub(crate) struct Agents {
    pub list: Vec<Agent>,
    pub running: BTreeMap<String, Child>,
    /// The last exit per account, shown until the next start.
    pub exits: BTreeMap<String, String>,
    pub draft: String,
    pub busy: bool,
    pub notice: String,
    pub loaded: bool,
}

fn root(config: &Config) -> PathBuf {
    config.state_root().join("agents")
}

fn registry(config: &Config) -> PathBuf {
    root(config).join("agents.json")
}

/// One agent's private directory: its password, port token, client home and log.
pub(crate) fn home(config: &Config, account: &str) -> PathBuf {
    root(config).join(account)
}

pub(crate) fn load(config: &Config) -> Result<Vec<Agent>> {
    match std::fs::read(registry(config)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("agents.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("agents.json: {e}")),
    }
}

fn save(config: &Config, list: &[Agent]) -> Result<()> {
    std::fs::create_dir_all(root(config)).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(registry(config), bytes).map_err(|e| format!("agents.json: {e}"))
}

/// `Scout` → `AGENTSCOUT`: letters and digits only, at most 16, as account names allow.
pub(crate) fn account_for(name: &str) -> Result<String> {
    let base: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase();
    if base.is_empty() {
        return Err("Give the agent a name with letters or digits.".into());
    }
    Ok(format!("AGENT{base}").chars().take(16).collect())
}

/// The lowest free control port at or above [`FIRST_PORT`] not already given to an agent.
fn next_port(list: &[Agent]) -> u16 {
    (FIRST_PORT..)
        .find(|p| !list.iter().any(|a| a.port == *p))
        .unwrap_or(FIRST_PORT)
}

fn secret(length: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..length)
        .map(|_| char::from(ALPHABET[rand::random::<usize>() % ALPHABET.len()]))
        .collect()
}

/// Write a private (0600) file.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut f| f.write_all(contents.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn read_private(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Make the account on the realm (the server must be running) and remember the agent.
/// Blocking: runs off the UI thread.
pub(crate) fn create(config: &Config, name: &str) -> Result<Agent> {
    let mut list = load(config)?;
    let account = account_for(name)?;
    if list.iter().any(|a| a.account == account) {
        return Err(format!("An agent already uses the account {account}."));
    }
    let password = secret(14);
    crate::server::admin_command(config, &format!(".account create {account} {password}"))?;
    // Wrath content needs the expansion flag on the account. The realm creates accounts
    // asynchronously, so the flag can briefly find no account yet.
    let addon = format!(".account set addon {account} 2");
    let mut attempt = 1;
    while let Err(e) = crate::server::admin_command(config, &addon) {
        if attempt >= 20 || !e.contains("not exist") {
            return Err(e);
        }
        attempt += 1;
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let dir = home(config, &account);
    write_private(&dir.join("password"), &password)?;
    write_private(&dir.join("token"), &secret(32))?;
    let agent = Agent {
        name: name.trim().to_owned(),
        account,
        port: next_port(&list),
    };
    list.push(agent.clone());
    save(config, &list)?;
    Ok(agent)
}

/// Forget an agent in the Manager. The realm account and its characters stay.
pub(crate) fn forget(config: &Config, account: &str) -> Result<Vec<Agent>> {
    let mut list = load(config)?;
    list.retain(|a| a.account != account);
    save(config, &list)?;
    Ok(list)
}

/// Start the agent's headless client: logged in to the managed realm, control port open.
pub(crate) fn start(config: &Config, agent: &Agent) -> Result<Child> {
    let binary = setup::discover(config).ok_or("Build the client first (Client page).")?;
    let dir = home(config, &agent.account);
    let password = read_private(&dir.join("password"))?;
    let token = read_private(&dir.join("token"))?;
    std::fs::create_dir_all(dir.join("home")).map_err(|e| e.to_string())?;
    if std::net::TcpListener::bind(("127.0.0.1", agent.port)).is_err() {
        return Err(format!(
            "Port {} is in use; is this agent's client already running?",
            agent.port
        ));
    }
    let log = std::fs::File::create(dir.join("client.log")).map_err(|e| e.to_string())?;
    let mut command = Command::new(binary);
    command
        .current_dir(&dir)
        .env("WOW_DATA", &config.data)
        .env("WOW_CLIENT_PROFILE", "wrath")
        .env("WOW_LOCALE", &config.locale)
        .env("BENILLA_HOME", dir.join("home"))
        .env("WOW_HEADLESS", "1")
        .env("WOW_AGENT_TOKEN", token)
        .env("WRATHPLUS_AGENT_PASSWORD", password)
        .envs(crate::server::mmaps(config).map(|dir| ("WOW_NAV_MMAPS", dir)))
        .args(["--server", REALM_ADDRESS, "--account", &agent.account])
        .args(["--password-env", "WRATHPLUS_AGENT_PASSWORD"])
        .args([
            "--realm",
            REALM_NAME,
            "--agent-port",
            &agent.port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    command
        .spawn()
        .map_err(|e| format!("Start {}'s client: {e}", agent.name))
}

/// The control port's base URL.
pub(crate) fn url(agent: &Agent) -> String {
    format!("http://127.0.0.1:{}", agent.port)
}

/// The web view address, token included (it only opens the read-only page).
pub(crate) fn view_url(config: &Config, agent: &Agent) -> Result<String> {
    let token = read_private(&home(config, &agent.account).join("token"))?;
    Ok(format!("{}/view?token={token}", url(agent)))
}

/// A `claude mcp add` line for the agent's MCP server (`agents/mcp`), token included.
pub(crate) fn mcp_command(config: &Config, agent: &Agent) -> Result<String> {
    let token = read_private(&home(config, &agent.account).join("token"))?;
    let server = config.repo.join("agents/mcp/src/index.ts");
    Ok(format!(
        "claude mcp add wrathplus-{} -e WRATHPLUS_AGENT_URL={} -e WRATHPLUS_AGENT_TOKEN={token} -- bun run {}",
        agent.name.to_ascii_lowercase().replace(char::is_whitespace, "-"),
        url(agent),
        server.display()
    ))
}

/// Open an address in the player's browser. No shell.
pub(crate) fn open(address: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut command = Command::new("xdg-open");
    command
        .arg(address)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open the browser: {e}"))
}
