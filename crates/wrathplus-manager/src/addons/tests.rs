use super::*;
use crate::setup::tests::Fixture;
fn addon(root: &Path, folder: &str, name: &str, extra: &str) -> PathBuf {
    let path = root.join(folder);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join(format!("{name}.toc")),
        format!("## Interface: 30300\n## Title: {name}\n{extra}\nmain.lua\n"),
    )
    .unwrap();
    std::fs::write(path.join("main.lua"), "-- synthetic addon only").unwrap();
    std::fs::canonicalize(path).unwrap()
}
#[test]
fn discovers_real_manifests_and_custom_identity_not_pub_records() {
    let f = Fixture::new();
    let root = f.config.data.parent().unwrap().join("Interface/AddOns");
    addon(
        &root,
        "MultiBot",
        "MultiBot",
        "## Version: 4.0\n## SavedVariables: GlobalDB\n## SavedVariablesPerCharacter: CharacterDB",
    );
    std::fs::create_dir_all(root.join("Blizzard_Test")).unwrap();
    std::fs::write(root.join("Blizzard_Test/Blizzard_Test.pub"), "record").unwrap();
    let entries = discover(&f.config, &Settings::default());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].missing, 0);
    assert!(entries[0]
        .requirements(&entries)
        .contains("mod-multibot-bridge"));
    assert_eq!(entries[0].saved, ["GlobalDB", "CharacterDB"]);
    let custom = addon(
        &f.config.repo,
        "MultiBot-Chatless",
        "MultiBot",
        "## Dependencies: Shared\n## OptionalDeps: Extra\n## LoadOnDemand: 1",
    );
    let entry = scan_folder(&custom).remove(0);
    assert_eq!(entry.name, "MultiBot");
    assert!(entry.lod);
    assert_eq!(entry.dependencies, ["Shared"]);
    assert_eq!(entry.optional, ["Extra"]);
}
#[test]
fn registration_links_files_and_removal_preserves_source_and_saved_data() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "custom-source", "Demo", "");
    let settings = Settings {
        folders: vec![source.clone()],
        selected: vec![scan_folder(&source)[0].selection()],
    };
    save_and_apply(&f.config, &settings).unwrap();
    let target = f.config.repo.join("wrathplus-config/AddOns/Demo/main.lua");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "-- synthetic addon only"
    );
    assert_eq!(load(&f.config).unwrap().selected, settings.selected);
    let saved = f.config.repo.join("wrathplus-config/saved/Demo.lua");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, "saved").unwrap();
    save_and_apply(&f.config, &Settings::default()).unwrap();
    assert!(!target.exists());
    assert!(source.join("Demo.toc").exists());
    assert!(saved.exists());
}
#[test]
fn duplicates_unmanaged_files_and_running_client_are_fenced() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "custom", "Demo", "");
    let item = scan_folder(&source)[0].selection();
    let mut settings = Settings {
        selected: vec![item.clone(), item.clone()],
        ..Default::default()
    };
    assert!(save_and_apply(&f.config, &settings)
        .unwrap_err()
        .contains("duplicate"));
    settings.selected.pop();
    let unmanaged = f.config.repo.join("wrathplus-config/AddOns/Demo");
    std::fs::create_dir_all(&unmanaged).unwrap();
    assert!(save_and_apply(&f.config, &settings)
        .unwrap_err()
        .contains("not a Manager-owned"));
    std::fs::remove_dir(unmanaged).unwrap();
    std::fs::write(f.config.target.join("manager/running.lock"), "fixture").unwrap();
    assert!(save_and_apply(&f.config, &settings)
        .unwrap_err()
        .contains("running"));
    assert!(!f.config.repo.join("wrathplus-config/AddOns/Demo").exists());
}
#[test]
fn unsafe_missing_paths_and_version_are_reported_and_ui_does_not_apply() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "bad", "Bad", "../escape.lua\nmissing.xml");
    let entry = scan_folder(&source).remove(0);
    assert_eq!(entry.missing, 2);
    let mut manager = crate::Manager {
        config: f.config.clone(),
        addon_entries: vec![entry],
        ..Default::default()
    };
    manager.toggle_addon(0, true);
    assert!(manager.notice.contains("blockers"));
    assert!(!f.config.repo.join("wrathplus-config/AddOns/Bad").exists());
}

#[test]
fn managed_external_link_executes_through_real_client_lua_loader() {
    use wrathplus_ui::script::{AddOnInfo, UiScript};
    let f = Fixture::new();
    let source = addon(
        &f.config.repo,
        "external source",
        "Demo",
        "## LoadOnDemand: 1",
    );
    std::fs::write(source.join("main.lua"), "local name, private = ...; ManagerLinkLoaded = name; private.ok = true; local f = CreateFrame('Frame', 'ManagerLinkFrame', UIParent); f:RegisterEvent('ADDON_LOADED'); f:SetScript('OnEvent', function(_, _, addon) ManagerLinkEvent = addon end)").unwrap();
    let entry = scan_folder(&source).remove(0);
    save_and_apply(
        &f.config,
        &Settings {
            selected: vec![entry.selection()],
            ..Default::default()
        },
    )
    .unwrap();
    let mut script = UiScript::new().unwrap();
    script.set_instruction_budget(200_000_000);
    script.note_addon_info_reply(&[]);
    script.register_addons(
        vec![AddOnInfo {
            name: "Demo".into(),
            files: vec!["main.lua".into()],
            interface: 30300,
            enabled: true,
            load_on_demand: true,
            ..Default::default()
        }],
        Some(Box::new({
            let root = f.config.state_root().join("AddOns");
            move |rel: &str| {
                let path = Path::new(rel);
                if path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                {
                    return None;
                }
                std::fs::read(root.join(path)).ok()
            }
        })),
        None,
        None,
    );
    script.run("CreateFrame('Frame', 'UIParent'); local ok, why = LoadAddOn('Demo'); assert(ok, why); assert(ManagerLinkLoaded == 'Demo'); assert(ManagerLinkEvent == 'Demo'); assert(ManagerLinkFrame ~= nil)").unwrap();
}

#[test]
fn repeated_directives_and_cp1252_use_the_runtime_parser() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "encoded", "Encoded", "");
    std::fs::write(source.join("caf\u{e9}.lua"), "-- fixture").unwrap();
    std::fs::write(source.join("Encoded.toc"), b"## Interface: 30400\n## Interface: 30300\n## LoadOnDemand: 0\n## LoadOnDemand: 1\ncaf\xe9.lua\n").unwrap();
    let entry = scan_folder(&source).remove(0);
    assert_eq!(entry.interface, 30300);
    assert!(entry.lod);
    assert_eq!(entry.missing, 0);
}
#[test]
fn vanished_source_can_be_unregistered_and_oversized_settings_are_atomic() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "external", "Demo", "");
    let settings = Settings {
        selected: vec![scan_folder(&source)[0].selection()],
        ..Default::default()
    };
    save_and_apply(&f.config, &settings).unwrap();
    let mut oversized = settings.clone();
    oversized.folders = vec![PathBuf::from("x".repeat(65536))];
    assert!(save_and_apply(&f.config, &oversized)
        .unwrap_err()
        .contains("64 KiB"));
    assert_eq!(load(&f.config).unwrap().selected, settings.selected);
    std::fs::remove_dir_all(&source).unwrap();
    let entries = discover(&f.config, &settings);
    assert!(entries
        .iter()
        .any(|e| e.selected(&settings) && e.title.contains("unavailable")));
    save_and_apply(&f.config, &Settings::default()).unwrap();
    assert!(std::fs::symlink_metadata(f.config.repo.join("wrathplus-config/AddOns/Demo")).is_err());
}
#[test]
fn switching_checkouts_does_not_carry_addon_state() {
    let first = Fixture::new();
    let second = Fixture::new();
    let source = addon(&first.config.repo, "external", "Demo", "");
    let mut manager = crate::Manager {
        config: first.config.clone(),
        addon_settings: Settings {
            selected: vec![scan_folder(&source)[0].selection()],
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = manager.update(crate::Message::Repo(
        second.config.repo.to_string_lossy().into_owned(),
    ));
    assert!(manager.addon_settings.selected.is_empty());
    assert!(manager.addon_entries.is_empty());
}

#[cfg(unix)]
#[test]
fn failed_settings_promotion_rolls_back_new_links() {
    let f = Fixture::new();
    let source = addon(&f.config.repo, "external", "Demo", "");
    let settings = Settings {
        selected: vec![scan_folder(&source)[0].selection()],
        ..Default::default()
    };
    // A readable settings symlink into protected assets forces persistence refusal after linking.
    let state = f.config.repo.join("wrathplus-config");
    std::fs::create_dir_all(&state).unwrap();
    let protected = f.config.data.join("settings.json");
    std::fs::write(&protected, "{\"folders\":[],\"selected\":[]}").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&protected, state.join("manager-addons.json")).unwrap();
        assert!(save_and_apply(&f.config, &settings)
            .unwrap_err()
            .contains("read-only"));
        assert!(!state.join("AddOns/Demo").exists());
        assert_eq!(
            std::fs::read_to_string(&protected).unwrap(),
            "{\"folders\":[],\"selected\":[]}"
        );
        assert!(source.join("Demo.toc").exists());
    }
}

#[test]
fn legacy_registrations_links_and_saved_variables_keep_one_home() {
    let f = Fixture::new();
    let legacy = f.config.repo.join("benilla-config");
    std::fs::create_dir(&legacy).unwrap();
    let source = addon(&f.config.repo, "legacy-source", "Demo", "");
    let settings = Settings {
        folders: vec![source.clone()],
        selected: vec![scan_folder(&source)[0].selection()],
    };
    save_and_apply(&f.config, &settings).unwrap();
    let saved = legacy.join("saved/Demo.lua");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, "legacy data").unwrap();
    let new = f.config.repo.join("wrathplus-config");
    std::fs::create_dir(&new).unwrap();
    assert_eq!(load(&f.config).unwrap().selected, settings.selected);
    assert!(discover(&f.config, &settings)
        .iter()
        .any(|e| e.selected(&settings)));
    assert!(legacy.join("AddOns/Demo/main.lua").is_file());
    save_and_apply(&f.config, &Settings::default()).unwrap();
    assert!(!legacy.join("AddOns/Demo").exists());
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "legacy data");
    assert!(source.join("Demo.toc").is_file());
    assert_eq!(std::fs::read_dir(new).unwrap().count(), 0);
}

#[test]
fn manifest_matches_each_directory_and_file_component_case_insensitively() {
    let f = Fixture::new();
    let source = addon(
        &f.config.repo,
        "mixed-case",
        "Demo",
        "sCrIpTs\\nEsTeD\\mAiN.LUA",
    );
    std::fs::create_dir_all(source.join("Scripts/Nested")).unwrap();
    std::fs::write(source.join("Scripts/Nested/Main.lua"), "-- fixture").unwrap();
    assert_eq!(manifest(&source).unwrap().missing, 0);
    assert!(!manifest_file_exists(
        &source,
        Path::new("Scripts/../main.lua")
    ));
    assert!(!manifest_file_exists(&source, Path::new("/main.lua")));
    assert!(!manifest_file_exists(&source, Path::new("C:/main.lua")));
    assert!(!manifest_file_exists(&source, Path::new("Scripts/Nested")));
    #[cfg(unix)]
    {
        let alias = f.config.repo.join("source-link");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert_eq!(manifest(&alias).unwrap().missing, 0);
    }
}

#[test]
fn two_vanished_sources_can_be_removed_sequentially_through_ui() {
    let f = Fixture::new();
    let first = addon(&f.config.repo, "first", "First", "");
    let second = addon(&f.config.repo, "second", "Second", "");
    let settings = Settings {
        selected: vec![
            manifest(&first).unwrap().selection(),
            manifest(&second).unwrap().selection(),
        ],
        ..Default::default()
    };
    save_and_apply(&f.config, &settings).unwrap();
    std::fs::remove_dir_all(&first).unwrap();
    std::fs::remove_dir_all(&second).unwrap();
    let mut manager = crate::Manager {
        config: f.config.clone(),
        ..Default::default()
    };
    manager.refresh_addons();
    let index = manager
        .addon_entries
        .iter()
        .position(|e| e.name == "First")
        .unwrap();
    manager.toggle_addon(index, false);
    assert_eq!(
        manager.addon_settings.selected,
        [settings.selected[1].clone()]
    );
    assert_eq!(
        load(&f.config).unwrap().selected,
        manager.addon_settings.selected
    );
    let root = f.config.state_root().join("AddOns");
    assert!(std::fs::symlink_metadata(root.join("First")).is_err());
    assert_eq!(std::fs::read_link(root.join("Second")).unwrap(), second);
    // Unchanged unavailable registrations still require ownership verification.
    std::fs::remove_file(root.join("Second")).unwrap();
    std::fs::create_dir(root.join("Second")).unwrap();
    assert!(save_and_apply(&f.config, &manager.addon_settings)
        .unwrap_err()
        .contains("not a Manager-owned"));
    std::fs::remove_dir(root.join("Second")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&second, root.join("Second")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&second, root.join("Second")).unwrap();
    let index = manager
        .addon_entries
        .iter()
        .position(|e| e.name == "Second")
        .unwrap();
    manager.toggle_addon(index, false);
    assert!(manager.addon_settings.selected.is_empty());
    assert!(load(&f.config).unwrap().selected.is_empty());
    assert!(std::fs::symlink_metadata(root.join("Second")).is_err());
    // A missing source is not allowed as a new registration.
    assert!(save_and_apply(&f.config, &settings)
        .unwrap_err()
        .contains("No valid TOC"));
}
