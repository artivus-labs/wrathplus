use super::test_env::{EnvGuard, ENV_LOCK};
use super::*;

/// `home()` is `None` under `$WOW_CAPTURE`, so `None` here is the capture case.
#[test]
fn there_is_one_addon_root_and_it_is_ours() {
    assert_eq!(
        addons_root_from(Some(PathBuf::from("/state"))),
        Some(PathBuf::from("/state/AddOns"))
    );
    assert_eq!(
        addons_root_from(None),
        None,
        "a capture run has no addon root"
    );
}

/// Every per-character file keys on this token; ASCII names map unchanged.
#[test]
fn distinct_names_never_share_a_token() {
    assert_ne!(file_token("Вася"), file_token("Петя"));
    assert_ne!(file_token("Zoë"), file_token("Zoé"));
    assert_eq!(file_token("Zoë"), "Zoë");
    assert_eq!(file_token("Onehunter"), "Onehunter");
    assert_eq!(file_token("Hydraxian Waterlords"), "Hydraxian_Waterlords");
    assert_eq!(
        file_token("a/b\\c:d"),
        "a_b_c_d",
        "separators never survive"
    );
    assert_eq!(
        file_token("realm-name"),
        "realm_name",
        "the key's own `-` stays unforgeable"
    );
}

/// The layout under the override, the one step of [`home`] whose answer a test can state.
#[test]
fn the_home_law_override_then_the_residents_and_hermetic_captures() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("benilla-ls-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    // 1 · the explicit override.
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::set("BENILLA_HOME", tmp.join(STATE_DIR).to_str().unwrap());
    assert_eq!(home(), Some(tmp.join(STATE_DIR)));
    assert_eq!(
        config_path(),
        Some(tmp.join("wrathplus-config/config.toml"))
    );
    assert_eq!(
        macros_account_path(),
        Some(tmp.join("wrathplus-config/macros/account.txt"))
    );
    assert_eq!(
        saved_variables_path(),
        Some(tmp.join("wrathplus-config/saved-variables.lua"))
    );
    assert_eq!(
        macros_character_path("Hydraxian Waterlords", "Probeone"),
        Some(tmp.join("wrathplus-config/macros/Hydraxian_Waterlords-Probeone.txt"))
    );
    assert_eq!(
        macros_character_path("../evil", "a/b"),
        Some(tmp.join("wrathplus-config/macros/___evil-a_b.txt")),
        "no name can escape the folder"
    );
    assert_eq!(
        camera_character_path("Hydraxian Waterlords", "Probeone"),
        Some(tmp.join("wrathplus-config/camera/Hydraxian_Waterlords-Probeone.txt"))
    );
    assert_eq!(
        chat_character_path("Hydraxian Waterlords", "Probeone"),
        Some(tmp.join("wrathplus-config/chat/Hydraxian_Waterlords-Probeone.txt"))
    );
    assert_eq!(
        layout_character_path("Hydraxian Waterlords", "Probeone"),
        Some(tmp.join("wrathplus-config/layout/Hydraxian_Waterlords-Probeone.txt"))
    );

    assert_eq!(
        saved_account_path(),
        Some(tmp.join("wrathplus-config/account"))
    );
    assert_eq!(shots_path(), Some(tmp.join("wrathplus-config/shots.txt")));
    assert_eq!(
        screenshots_dir(),
        Some(tmp.join("wrathplus-config/Screenshots"))
    );

    // 0 · a capture run resolves nothing, even with an override set.
    let _c2 = EnvGuard::set("WOW_CAPTURE", "ui-color-picker");
    assert_eq!(home(), None);
    assert_eq!(
        saved_account_path(),
        None,
        "a capture reads no saved account"
    );
    assert_eq!(shots_path(), None);
    assert_eq!(
        screenshots_dir(),
        None,
        "a capture writes no player screenshots"
    );
    assert_eq!(
        chat_character_path("Hydraxian Waterlords", "Probeone"),
        None,
        "a capture reads no player's chat look"
    );
    assert_eq!(
        layout_character_path("Hydraxian Waterlords", "Probeone"),
        None,
        "a capture reads no player's window layout"
    );
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn explicit_legacy_custom_and_empty_overrides_are_not_rewritten() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _c = EnvGuard::unset("WOW_CAPTURE");
    for value in [
        "/custom/benilla-config",
        "/custom/wrathplus-config",
        "/custom/state",
        "",
    ] {
        let _h = EnvGuard::set("BENILLA_HOME", value);
        assert_eq!(home(), Some(PathBuf::from(value)));
        assert_eq!(addons_root(), Some(PathBuf::from(value).join("AddOns")));
    }
}

/// A unit test that does not pin `$BENILLA_HOME` resolves no state folder, so no test can write
/// the real one.
#[test]
fn a_test_without_an_override_has_no_state_folder() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::unset("BENILLA_HOME");
    assert_eq!(home(), None);
    assert_eq!(config_path(), None);
}

/// A missing or wrong `$WOW_DATA` does not move or lose the state folder.
#[test]
fn the_state_folder_no_longer_depends_on_finding_the_install() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::unset("BENILLA_HOME");
    let _d = EnvGuard::set("WOW_DATA", "/nonexistent/benilla-test/Data");
    let h = resident_home().expect("a broken install path must not cost the player their config");
    assert!(
        !h.starts_with("/nonexistent"),
        "home() still reads $WOW_DATA: {}",
        h.display()
    );
    assert!(
        h.ends_with(STATE_DIR) || h.ends_with("benilla-config"),
        "{}",
        h.display()
    );
}

/// A player build resolves beside the binary; a dev build to the primary checkout, even from a
/// linked worktree (whose `.git` is a file).
#[test]
fn the_state_folder_lands_where_the_build_says() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _h = EnvGuard::unset("BENILLA_HOME");
    let h = resident_home().expect("the resident folder always resolves");
    assert!(
        h.ends_with(STATE_DIR) || h.ends_with("benilla-config"),
        "{}",
        h.display()
    );

    let Some(here) = wrathplus_formats::project_folder() else {
        // Player build: beside the binary.
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert_eq!(h, wrathplus_formats::default_state_home(&exe_dir));
        return;
    };

    if here.join(".git").is_file() {
        assert_ne!(
            h,
            here.join(STATE_DIR),
            "a linked worktree must not get its own settings folder — the worktrees share one"
        );
        assert!(
            h.parent().unwrap().join(".git").is_dir(),
            "resolved to {}, which is not a primary checkout",
            h.display()
        );
    } else {
        assert_eq!(h, wrathplus_formats::default_state_home(here));
    }
}

/// A project folder that is a linked worktree keeps its state in the primary checkout, so every
/// worktree shares one; any other folder, a crate on top of benilla's included, keeps its own.
#[test]
fn the_state_root_is_the_folder_or_its_primary_checkout() {
    let tmp = std::env::temp_dir().join(format!("benilla-root-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let primary = tmp.join("benilla");
    std::fs::create_dir_all(primary.join(".git/worktrees/pool-3")).unwrap();
    let slot = tmp.join("pool-3");
    std::fs::create_dir_all(&slot).unwrap();
    let gitdir = primary.join(".git/worktrees/pool-3");
    std::fs::write(slot.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    assert_eq!(shared_root(&slot), primary);
    assert_eq!(shared_root(&primary), primary);
    let hello_mod = tmp.join("hello-mod");
    std::fs::create_dir_all(hello_mod.join(".git")).unwrap();
    assert_eq!(shared_root(&hello_mod), hello_mod);
    let plain = tmp.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(shared_root(&plain), plain);
    std::fs::remove_dir_all(&tmp).ok();
}

/// A launcher's recorded folder is where a dev build keeps its state and reads its probe
/// identity; a player build resolves neither from it. Recording is process-wide, so the parent
/// runs this same test in a child process of the test binary, which records and resolves.
#[test]
fn a_recorded_launcher_folder_holds_the_state_and_the_identity() {
    const CHILD: &str = "BENILLA_TEST_LAUNCHER_FOLDER";
    if let Some(dir) = std::env::var_os(CHILD) {
        let dir = PathBuf::from(dir);
        wrathplus_formats::set_project_folder(dir.to_str().unwrap());
        if crate::run_mode::dev_affordances() {
            assert_eq!(resident_home(), Some(dir.join(STATE_DIR)));
            assert_eq!(
                crate::run_mode::declared_identity(),
                Some(crate::run_mode::DeclaredIdentity {
                    user: "probe9".into(),
                    character: "Modchar".into()
                })
            );
        } else {
            let exe_dir = std::env::current_exe().unwrap();
            assert_eq!(
                resident_home(),
                Some(exe_dir.parent().unwrap().join(STATE_DIR))
            );
            assert_eq!(crate::run_mode::declared_identity(), None);
        }
        return;
    }
    let tmp = std::env::temp_dir().join(format!("benilla-launcher-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join(".probe-identity"),
        "WOW_USER=probe9\nWOW_PASS=unused\nWOW_CHAR=Modchar\n",
    )
    .unwrap();
    let name = "local_state::tests::a_recorded_launcher_folder_holds_the_state_and_the_identity";
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, &tmp)
        .env_remove("BENILLA_HOME")
        .env_remove("WOW_CAPTURE")
        .output()
        .unwrap();
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && report.contains("1 passed"),
        "the child run:\n{report}"
    );
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn write_atomic_creates_dirs_and_replaces_whole_files() {
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("benilla-wa-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let path = tmp.join("nested/config.toml");
    write_atomic(&path, "a = \"1\"\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = \"1\"\n");
    write_atomic(&path, "a = \"2\"\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = \"2\"\n");
    assert!(!path.with_extension("tmp").exists(), "tmp renamed away");
    std::fs::remove_dir_all(&tmp).ok();
}
