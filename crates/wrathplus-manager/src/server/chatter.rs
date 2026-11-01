//! Bundled companion service for mod-llm-chatter, never a second module import.
use super::{command, settings::Settings};
use crate::{modules, setup};
use std::path::PathBuf;

pub(super) fn source(
    config: &setup::Config,
    settings: &Settings,
) -> Result<Option<PathBuf>, String> {
    if settings.source.as_os_str().is_empty() {
        return Ok(None);
    }
    let catalog = modules::Catalog::load(config, &settings.source)?;
    let Some(entry) = catalog
        .entries()
        .iter()
        .find(|e| e.name == "mod-llm-chatter" && e.enabled)
    else {
        return Ok(None);
    };
    let path = match &entry.source {
        modules::Source::Core => settings.source.join("modules/mod-llm-chatter"),
        modules::Source::Local { path } | modules::Source::Github { path, .. } => path.clone(),
    };
    if !path.join("tools/llm_chatter_bridge.py").is_file()
        || !path.join("tools/requirements.txt").is_file()
        || !path.join("tools/chatter_healthcheck.py").is_file()
    {
        return Err("Selected mod-llm-chatter lacks its bundled tools/ bridge".into());
    }
    Ok(Some(path))
}
/// Use the same bounded, private-config/link rejecting snapshot as server builds.
/// Only this snapshot can become a companion image context.
pub(super) fn stage(
    config: &setup::Config,
    settings: &Settings,
) -> Result<modules::Staged, String> {
    let path = source(config, settings)?.ok_or("Chatter is not selected")?;
    modules::stage(
        config,
        &modules::BuildPlan {
            core_path: settings.source.clone(),
            cmake_args: vec![],
            selected: vec![modules::ModuleInput {
                name: "mod-llm-chatter".into(),
                source: path,
            }],
            disabled: vec![],
            requires_staging: true,
        },
    )
}
pub(super) fn failed(rows: &[serde_json::Value]) -> bool {
    rows.iter().any(|r| {
        r["Service"] == "llm-chatter-bridge"
            && (r["State"] == "exited" || r["Health"] == "unhealthy")
    })
}
pub(super) const START: &[&str] = &[
    "up",
    "-d",
    "--no-deps",
    "--force-recreate",
    "--build",
    "llm-chatter-bridge",
];
pub(super) fn ready(rows: &[serde_json::Value]) -> bool {
    rows.iter().any(|r| {
        r["Service"] == "llm-chatter-bridge" && r["State"] == "running" && r["Health"] == "healthy"
    })
}
pub(super) fn summary(rows: &[serde_json::Value], expected: bool) -> String {
    let mut summary = command::summary(rows);
    if expected && !rows.iter().any(|r| r["Service"] == "llm-chatter-bridge") {
        summary.push_str(" · LLM bridge missing: use Start to provision its bundled service");
    }
    summary
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_module_resolves_bundled_service_and_deselection_removes_it() {
        let f = crate::setup::tests::Fixture::new();
        let core = f.config.target.join("core");
        let module = core.join("modules/mod-llm-chatter");
        std::fs::create_dir_all(module.join("src")).unwrap();
        std::fs::write(core.join("CMakeLists.txt"), "").unwrap();
        let settings = Settings {
            source: core.clone(),
            ..Settings::default()
        };
        assert!(source(&f.config, &settings).is_err());
        std::fs::create_dir_all(module.join("tools")).unwrap();
        for file in [
            "llm_chatter_bridge.py",
            "requirements.txt",
            "chatter_healthcheck.py",
        ] {
            std::fs::write(module.join("tools").join(file), "").unwrap();
        }
        assert_eq!(source(&f.config, &settings).unwrap(), Some(module.clone()));
        let staged = stage(&f.config, &settings).unwrap();
        assert!(staged
            .path
            .join("mod-llm-chatter/tools/requirements.txt")
            .is_file());
        std::fs::write(module.join("tools/.env"), "private").unwrap();
        assert!(stage(&f.config, &settings).is_err());
        assert!(module.join("tools/.env").is_file());
        std::fs::remove_file(module.join("tools/.env")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("requirements.txt", module.join("tools/link")).unwrap();
            assert!(stage(&f.config, &settings).is_err());
            std::fs::remove_file(module.join("tools/link")).unwrap();
        }
        let mut catalog = modules::Catalog::load(&f.config, &core).unwrap();
        catalog.set_enabled("mod-llm-chatter", false).unwrap();
        catalog.save(&f.config).unwrap();
        assert!(source(&f.config, &settings).unwrap().is_none());
        assert!(START.contains(&"--no-deps"));
        assert!(START.contains(&"--force-recreate"));
    }
    #[test]
    fn bridge_needs_observed_health_and_missing_is_explicit() {
        assert!(!ready(&[]));
        assert!(summary(&[], true).contains("LLM bridge missing"));
        assert!(!summary(&[], false).contains("LLM bridge"));
        let mut rows = vec![
            serde_json::json!({"Service":"llm-chatter-bridge", "State":"running", "Health":"starting"}),
        ];
        assert!(!ready(&rows));
        rows[0]["Health"] = "healthy".into();
        assert!(ready(&rows));
        assert!(summary(&rows, true).contains("llm-chatter-bridge"));
        rows[0]["State"] = "exited".into();
        assert!(!ready(&rows));
        assert!(failed(&rows));
        // A retained bridge from Stop or a failed/deselected companion cannot
        // fail the realm startup phase; provisioning happens after realm health.
        assert!(!command::failed(&rows));
        let mut realm = ["mysql", "authserver", "worldserver"].into_iter()
            .map(|service| serde_json::json!({"Service":service,"State":"running","Health":"healthy"}))
            .collect::<Vec<_>>();
        realm.extend(rows);
        assert!(command::ready(&realm));
        assert!(!command::failed(&realm));
        assert_eq!(
            super::super::status::Health::observed(&realm),
            super::super::status::Health::Running
        );
        realm[2]["Health"] = "starting".into();
        assert!(!command::ready(&realm));
        assert!(!command::failed(&realm));
    }
}
