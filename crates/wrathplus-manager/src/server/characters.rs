//! Character export and import for the managed realm, using AzerothCore's own `.pdump` files so
//! characters move to and from any AzerothCore server. The server does the work (ID remapping,
//! name checks); the Manager drives it over SOAP on 127.0.0.1 with its own admin account, and moves
//! the dump file in and out of the worldserver container. Blocking: run off the UI thread.
use super::settings::{Settings, PROJECT};
use crate::setup;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

type Result<T> = std::result::Result<T, String>;

/// Published on 127.0.0.1 only (compose.yaml); the container listens on 7878.
const SOAP_ADDRESS: &str = "127.0.0.1:17878";
const ADMIN: &str = "WPMANAGER";
/// The dump's name inside the worldserver's working directory (`PlayerDump.DisallowPaths`).
const EXPORT_FILE: &str = "wrathplus-export.pdump";
const IMPORT_FILE: &str = "wrathplus-import.pdump";
/// Accounts that are not players: Playerbots' random bots, the auction seller, the Manager.
const SERVICE_ACCOUNTS: &str =
    "username NOT LIKE 'RNDBOT%' AND username NOT IN ('AHBOT', 'WPMANAGER')";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Character {
    pub name: String,
    pub level: u8,
    pub race: u8,
    pub class: u8,
    pub account: String,
    pub online: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Roster {
    pub characters: Vec<Character>,
    pub accounts: Vec<String>,
}

/// The players' characters and accounts on the managed realm (bots and service accounts left out).
pub(crate) fn roster() -> Result<Roster> {
    let rows = mysql(&format!(
        "SELECT c.name, c.level, c.race, c.class, a.username, c.online \
         FROM acore_characters.characters c JOIN acore_auth.account a ON a.id = c.account \
         WHERE a.{SERVICE_ACCOUNTS} AND c.deleteDate IS NULL ORDER BY a.username, c.name;\n\
         SELECT '--';\n\
         SELECT username FROM acore_auth.account WHERE {SERVICE_ACCOUNTS} ORDER BY username;"
    ))?;
    parse_roster(&rows)
}

pub(super) fn parse_roster(output: &str) -> Result<Roster> {
    let mut roster = Roster::default();
    let mut accounts = false;
    for line in output.lines().filter(|l| !l.is_empty()) {
        if line == "--" {
            accounts = true;
            continue;
        }
        if accounts {
            roster.accounts.push(line.to_owned());
            continue;
        }
        let f: Vec<_> = line.split('\t').collect();
        let [name, level, race, class, account, online] = f[..] else {
            return Err(format!("Unexpected character row: {line:?}"));
        };
        let number = |s: &str| s.parse::<u8>().map_err(|e| format!("{s:?}: {e}"));
        roster.characters.push(Character {
            name: name.into(),
            level: number(level)?,
            race: number(race)?,
            class: number(class)?,
            account: account.into(),
            online: online == "1",
        });
    }
    Ok(roster)
}

/// Write `name`'s dump and save it to `destination`.
pub(crate) fn export(config: &setup::Config, name: &str, destination: &Path) -> Result<String> {
    check_name(name)?;
    let world = container("worldserver")?;
    // DisallowOverwrite: clear a leftover from an interrupted export first.
    exec(&world, &["rm", "-f", EXPORT_FILE], None)?;
    let result = soap(config, &format!(".pdump write {EXPORT_FILE} {name}"));
    let dump = result.and_then(|_| exec(&world, &["cat", EXPORT_FILE], None));
    let _ = exec(&world, &["rm", "-f", EXPORT_FILE], None);
    let dump = dump?;
    if dump.is_empty() {
        return Err(format!("The server wrote an empty dump for {name}"));
    }
    std::fs::write(destination, dump)
        .map_err(|e| format!("Save {}: {e}", destination.display()))?;
    Ok(format!("Exported {name} to {}", destination.display()))
}

/// Load a dump into `account`, optionally under a new name. The server picks fresh IDs.
pub(crate) fn import(
    config: &setup::Config,
    source: &Path,
    account: &str,
    rename: Option<&str>,
) -> Result<String> {
    check_account(account)?;
    if let Some(name) = rename {
        check_name(name)?;
    }
    let dump = std::fs::read(source).map_err(|e| format!("Read {}: {e}", source.display()))?;
    if !looks_like_dump(&dump) {
        return Err(format!(
            "{} is not a character dump (.pdump write output)",
            source.display()
        ));
    }
    let world = container("worldserver")?;
    exec(
        &world,
        &["sh", "-c", &format!("cat > {IMPORT_FILE}")],
        Some(&dump),
    )?;
    let command = match rename {
        Some(name) => format!(".pdump load {IMPORT_FILE} {account} {name}"),
        None => format!(".pdump load {IMPORT_FILE} {account}"),
    };
    let result = soap(config, &command);
    let _ = exec(&world, &["rm", "-f", IMPORT_FILE], None);
    result?;
    Ok(format!(
        "Imported {} into {account}",
        rename.map_or_else(|| "the character".to_owned(), str::to_owned)
    ))
}

/// A dump is lines of `INSERT INTO` statements; anything else is refused before the server sees it.
pub(super) fn looks_like_dump(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines().any(|l| {
        l.starts_with("INSERT INTO `characters`") || l.starts_with("INSERT INTO characters")
    })
}

/// Character names are letters only (the server applies its own stricter rules on load).
pub(super) fn check_name(name: &str) -> Result<()> {
    if (2..=12).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphabetic()) {
        Ok(())
    } else {
        Err(format!(
            "{name:?} is not a valid character name (2–12 letters)"
        ))
    }
}

pub(super) fn check_account(account: &str) -> Result<()> {
    if (1..=16).contains(&account.len()) && account.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(format!("{account:?} is not a valid account name"))
    }
}

/// The Manager's SOAP admin: a random password kept in the private server state, and the account
/// (re)written to match on every call, so a fresh database or a lost file heals itself.
fn admin(config: &setup::Config) -> Result<String> {
    let path = Settings::state(config).join("manager-admin.secret");
    let password = match std::fs::read_to_string(&path) {
        Ok(saved) if saved.trim().len() == 16 => saved.trim().to_owned(),
        _ => {
            let password: String = (0..16)
                .map(|_| {
                    char::from(b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"[rand::random::<usize>() % 32])
                })
                .collect();
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
            let mut file = options.open(&path).map_err(|e| e.to_string())?;
            file.write_all(password.as_bytes())
                .map_err(|e| e.to_string())?;
            password
        }
    };
    let user = wrathplus_srp::NormalizedString::new(ADMIN).map_err(|e| e.to_string())?;
    let pass = wrathplus_srp::NormalizedString::new(&password).map_err(|e| e.to_string())?;
    let (salt, verifier) = wrathplus_srp::generate_account(&user, &pass);
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
    mysql(&format!(
        "INSERT INTO acore_auth.account (username, salt, verifier, email, reg_mail, expansion) \
         VALUES ('{ADMIN}', X'{s}', X'{v}', '', '', 2) \
         ON DUPLICATE KEY UPDATE salt = X'{s}', verifier = X'{v}';\n\
         REPLACE INTO acore_auth.account_access (id, gmlevel, RealmID, comment) \
         SELECT id, 3, -1, 'WrathPlus Manager character transfer' FROM acore_auth.account \
         WHERE username = '{ADMIN}';",
        s = hex(&salt),
        v = hex(&verifier),
    ))?;
    Ok(password)
}

/// One worldserver command over SOAP; a SOAP fault is the command's error message.
fn soap(config: &setup::Config, command: &str) -> Result<String> {
    let password = admin(config)?;
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:ns1=\"urn:AC\">\
         <SOAP-ENV:Body><ns1:executeCommand><command>{}</command></ns1:executeCommand></SOAP-ENV:Body>\
         </SOAP-ENV:Envelope>",
        escape(command)
    );
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {SOAP_ADDRESS}\r\nAuthorization: Basic {}\r\n\
         Content-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        base64(format!("{ADMIN}:{password}").as_bytes()),
        body.len()
    );
    let address = SOAP_ADDRESS.parse().map_err(|e| format!("{e}"))?;
    let timeout = std::time::Duration::from_secs(60);
    let mut stream = std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(5))
        .map_err(|e| format!("The server's command port is not answering ({e}). Restart the server so it picks up the character-transfer settings."))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream
        .take(1024 * 1024)
        .read_to_string(&mut response)
        .map_err(|e| format!("Command port: {e}"))?;
    parse_soap(&response)
}

pub(super) fn parse_soap(response: &str) -> Result<String> {
    let status = response.split_whitespace().nth(1).unwrap_or("");
    let between = |open: &str, close: &str| {
        let start = response.find(open)? + open.len();
        let end = response[start..].find(close)? + start;
        Some(unescape(&response[start..end]).trim().to_owned())
    };
    match status {
        "200" => Ok(between("<result>", "</result>").unwrap_or_default()),
        "401" | "403" => Err("The server refused the Manager's admin account".into()),
        _ => Err(between("<faultstring>", "</faultstring>")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Command failed (HTTP {status})"))),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape(s: &str) -> String {
    s.replace("&#xA;", "\n")
        .replace("&#xD;", "")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub(super) fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                char::from(TABLE[(n >> (18 - 6 * i)) as usize & 63])
            } else {
                '='
            });
        }
    }
    out
}

/// The running container for one of this project's services.
fn container(service: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["ps", "-q", "--filter"])
        .arg(format!("label=com.docker.compose.project={PROJECT}"))
        .arg("--filter")
        .arg(format!("label=com.docker.compose.service={service}"))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run Docker: {e}"))?;
    let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if id.is_empty() || id.contains('\n') {
        return Err(format!(
            "The server's {service} is not running. Start the server first."
        ));
    }
    Ok(id)
}

/// Run a command in a container, optionally feeding stdin; stdout is returned.
fn exec(container: &str, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new("docker")
        .args(["exec", "-i", container])
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run Docker: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// SQL as root inside the MySQL container; the password never leaves it.
fn mysql(sql: &str) -> Result<String> {
    let output = exec(
        &container("mysql")?,
        &[
            "sh",
            "-c",
            "MYSQL_PWD=$(cat /run/instance-secrets/mysql-root) exec mysql -uroot -N -B",
        ],
        Some(sql.as_bytes()),
    )?;
    String::from_utf8(output).map_err(|e| format!("MySQL output: {e}"))
}
