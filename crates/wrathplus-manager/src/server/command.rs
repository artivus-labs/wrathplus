//! Only this disposable Compose project can be addressed; no shell or destructive verbs.
use super::settings::{Settings, PROJECT};
use crate::setup;
use std::process::Command;

pub fn compose(config: &setup::Config, settings: &Settings, args: &[&str]) -> Command {
    let directory = config.repo.join("packaging/server");
    let mut command = Command::new("docker");
    command
        .args(["compose", "--project-name", PROJECT, "--project-directory"])
        .arg(&directory)
        .arg("-f")
        .arg(directory.join("compose.yaml"))
        .args(args)
        .env(
            "ACORE_SOURCE",
            if settings.source.as_os_str().is_empty() {
                &directory
            } else {
                &settings.source
            },
        )
        .env("MODULE_SOURCES", settings.source.join("modules"))
        .env("MODULE_CONFIG", crate::module_config::directory(config))
        .env("SPIKE_DOCKERFILE", directory.join("Dockerfile"))
        .env("SERVER_RECIPE", &directory)
        .env(
            "SERVER_DATA",
            if settings.data.as_os_str().is_empty() {
                &directory
            } else {
                &settings.data
            },
        )
        .env("SERVER_STATE", Settings::state(config))
        // Never adopt an inherited Compose file, project, profile or env-file override.
        .env_remove("COMPOSE_FILE")
        .env_remove("COMPOSE_PROJECT_NAME")
        .env_remove("COMPOSE_PROFILES")
        .env_remove("COMPOSE_ENV_FILES")
        .env("COMPOSE_DISABLE_ENV_FILE", "1")
        .env("BUILDKIT_PROGRESS", "plain")
        .current_dir(&directory);
    // A profile exposes companion status even when it was started outside Manager.
    command.env("COMPOSE_PROFILES", "chatter");
    // Runtime commands never inherit an arbitrary working-tree build context.
    command.env("CHATTER_SOURCE", &directory);
    command
}
pub const BUILD: &[&str] = &["build", "playerbots-build"];
pub const MYSQL: &[&str] = &["up", "-d", "mysql"];
pub const BOOTSTRAP: &[&str] = &["run", "--rm", "bootstrap"];
pub const START: &[&str] = &["up", "-d", "authserver", "worldserver"];
pub const STOP: &[&str] = &["stop", "llm-chatter-bridge", "worldserver", "authserver"];
pub const STATUS: &[&str] = &["ps", "--all", "--format", "json"];

pub fn failed(rows: &[serde_json::Value]) -> bool {
    rows.iter().any(|row| {
        ["mysql", "authserver", "worldserver"]
            .iter()
            .any(|service| row["Service"] == *service)
            && (row["State"] == "exited" || row["Health"] == "unhealthy")
    })
}
pub const LOGS: &[&str] = &[
    "logs",
    "--no-color",
    "--tail",
    "150",
    "llm-chatter-bridge",
    "mysql",
    "authserver",
    "worldserver",
];

pub fn services(json: &str) -> Result<Vec<serde_json::Value>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    if json.trim_start().starts_with('[') {
        serde_json::from_str(json).map_err(|e| format!("Compose status JSON: {e}"))
    } else {
        json.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(|e| format!("Compose status JSON: {e}")))
            .collect()
    }
}
pub fn ready(rows: &[serde_json::Value]) -> bool {
    ["mysql", "authserver", "worldserver"]
        .iter()
        .all(|service| {
            rows.iter().any(|row| {
                row["Service"] == *service
                    && row["State"] == "running"
                    && row["Health"] == "healthy"
            })
        })
}
pub fn summary(rows: &[serde_json::Value]) -> String {
    let servers: Vec<_> = rows
        .iter()
        .filter(|r| {
            matches!(
                r["Service"].as_str(),
                Some("mysql" | "authserver" | "worldserver" | "llm-chatter-bridge")
            )
        })
        .collect();
    if servers.is_empty() {
        return "Server is stopped".into();
    }
    servers
        .iter()
        .map(|r| {
            format!(
                "{}: {} ({})",
                r["Service"].as_str().unwrap_or("?"),
                r["State"].as_str().unwrap_or("?"),
                r["Health"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("readiness unknown")
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Global image lookup only; never touches containers or an arbitrary realm.
pub fn local_image() -> Command {
    let mut command = Command::new("docker");
    command.args([
        "image",
        "ls",
        "--quiet",
        "--no-trunc",
        "wrathplus/playerbots:arm64-spike",
    ]);
    command
}
