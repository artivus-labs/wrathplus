use super::{filesystem, platform, tools, Report, Status};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "manager-preflight-test-{}-{}-{}",
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn toolchain_and_exact_versions() {
    assert_eq!(
        tools::channel("[toolchain]\nchannel = '1.98.1' # pin\n").unwrap(),
        "1.98.1"
    );
    for input in [
        "channel = \"1.98.1\"",
        "[other]\nchannel = '1.98.1'",
        "[toolchain]\nchannel = 'stable'",
    ] {
        assert!(tools::channel(input).is_err());
    }
    assert!(tools::version("rustc 1.98.1 (abc 2026-01-01)", "rustc", "1.98.1").is_ok());
    assert!(tools::version("cargo 1.98.0 (abc)", "cargo", "1.98.1").is_err());
    assert!(tools::version("rustc 1.98.1-nightly", "rustc", "1.98.1").is_err());
    assert!(tools::version("cargo 1.98.1", "rustc", "1.98.1").is_err());
}

#[test]
fn injected_results_determine_readiness_and_summary() {
    let mut report = Report::default();
    report.result("cargo", Ok("cargo 1.98.1".into()));
    report.add("Disk", Status::Warning, "unmeasured");
    assert!(report.ready());
    report.result("rustc", Err("wrong version".into()));
    assert!(!report.ready());
    assert!(report.summary().contains("[Blocked] rustc: wrong version"));
    assert_eq!(report.checks.clone().len(), 3);
}

#[test]
fn pathext_only_permits_native_executables() {
    assert_eq!(
        tools::names("cargo", true, ".COM;.EXE;.BAT;.CMD;.PS1"),
        ["cargo.COM", "cargo.EXE"]
    );
    assert_eq!(
        tools::names("cargo", true, ".exe;.com"),
        ["cargo.exe", "cargo.com"]
    );
    assert!(tools::names("cargo", true, ".BAT;.CMD").is_empty());
    assert_eq!(tools::names("cargo", false, ".EXE"), ["cargo"]);
}

#[test]
fn discovery_order_and_executable_bit() {
    let dir = Scratch::new();
    let first = dir.0.join("first");
    let second = dir.0.join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    for path in [
        first.join("cargo"),
        second.join("cargo"),
        second.join("cargo.EXE"),
    ] {
        std::fs::write(&path, "fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    assert_eq!(
        tools::in_dirs("cargo", &[first.clone(), second.clone()], false, ""),
        Some(first.join("cargo"))
    );
    assert_eq!(
        tools::in_dirs("cargo", std::slice::from_ref(&second), true, ".BAT;.EXE"),
        Some(second.join("cargo.EXE"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(first.join("cargo"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        assert_eq!(
            tools::in_dirs("cargo", &[first, second.clone()], false, ""),
            Some(second.join("cargo"))
        );
    }
}

#[test]
fn disk_and_wsl_parsing() {
    assert_eq!(filesystem::df_available("Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk 1000 100 900 10% /a path\n"), Some(900 * 1024));
    assert_eq!(filesystem::df_available("Filesystem 1024-blocks Used Available Capacity Mounted on\nnetwork disk 1000 100 900 10% /mnt\n"), Some(900 * 1024));
    assert_eq!(filesystem::df_available("garbage"), None);
    for path in [
        "/mnt/c/project",
        "/home/player/project",
        "\\\\wsl$\\Ubuntu\\home\\me",
        "\\\\wsl.localhost\\Ubuntu\\repo",
    ] {
        assert!(platform::wsl_path(path), "{path}");
    }
    assert!(!platform::wsl_path("C:\\Users\\me\\repo"));
}

#[test]
fn canonical_missing_paths_and_overlap() {
    let dir = Scratch::new();
    let root = std::fs::canonicalize(&dir.0).unwrap();
    assert_eq!(
        filesystem::resolve(&dir.0.join("missing/target")).unwrap(),
        root.join("missing/target")
    );
    assert_eq!(
        filesystem::nearest(&root.join("missing/target")).unwrap(),
        root
    );
    assert!(filesystem::resolve(Path::new("")).is_err());
    assert!(filesystem::resolve(&root.join("../elsewhere")).is_err());
    assert!(filesystem::overlaps(&root, &root.join("Data")));
    assert!(!filesystem::overlaps(
        &root.join("target"),
        &root.join("Data")
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        assert_eq!(
            filesystem::resolve(&root.join("alias/missing")).unwrap(),
            root.join("missing")
        );
    }
}

#[test]
fn write_probes_skip_unknown_assets_and_never_enter_install() {
    let dir = Scratch::new();
    let config = crate::setup::Config {
        repo: dir.0.clone(),
        target: dir.0.join("target"),
        data: PathBuf::new(),
        locale: "enUS".into(),
        server: "localhost".into(),
    };
    let mut report = Report::default();
    filesystem::inspect(&config, &dir.0, &mut report);
    assert!(report
        .checks
        .iter()
        .take(2)
        .all(|c| c.status == Status::Warning));
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    let install = dir.0.join("install");
    std::fs::create_dir(&install).unwrap();
    let config = crate::setup::Config {
        data: install.join("Data"),
        target: install.join("output"),
        ..config
    };
    let mut report = Report::default();
    filesystem::inspect(&config, &dir.0, &mut report);
    assert!(report
        .checks
        .iter()
        .take(2)
        .all(|c| c.status == Status::Blocked));
    assert_eq!(std::fs::read_dir(&install).unwrap().count(), 0);
}

#[test]
fn safe_sibling_probes_cleanup_without_creating_target() {
    let dir = Scratch::new();
    let config = crate::setup::Config {
        repo: dir.0.join("repo"),
        target: dir.0.join("repo/target/nested"),
        data: dir.0.join("install/Data"),
        locale: "enUS".into(),
        server: "localhost".into(),
    };
    std::fs::create_dir(&config.repo).unwrap();
    let mut report = Report::default();
    filesystem::inspect(&config, &config.repo, &mut report);
    assert!(
        report
            .checks
            .iter()
            .take(2)
            .all(|c| c.status == Status::Pass),
        "{}",
        report.summary()
    );
    assert_eq!(std::fs::read_dir(&config.repo).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn bounded_process_exit_and_timeout() {
    use std::ffi::OsStr;
    let dir = Scratch::new();
    // Direct OS executables, never a shell, game, GUI or installer.
    assert!(tools::run(&dir.0, Path::new("/usr/bin/false"), &[]).is_err());
    let start = std::time::Instant::now();
    assert!(
        tools::run(&dir.0, Path::new("/bin/sleep"), &[OsStr::new("30")])
            .unwrap_err()
            .contains("timed out")
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
}
