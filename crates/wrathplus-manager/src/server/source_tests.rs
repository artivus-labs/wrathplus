use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);

#[test]
fn pins_are_the_revisions_the_build_checks() {
    let compose = include_str!("../../../../packaging/server/compose.yaml");
    assert!(compose.contains(&format!("CORE_REV: \"{CORE_REV}\"")));
    assert!(compose.contains(&format!("PLAYERBOTS_REV: \"{PLAYERBOTS_REV}\"")));
    let docs = include_str!("../../../../docs/CLIENT-SETUP.md");
    assert!(docs.contains(&format!("github.com/{CORE_REPOSITORY}/tree/Playerbot")));
    assert!(docs.contains(&format!("github.com/{PLAYERBOTS_REPOSITORY}")));
    let pins = pins(Path::new("/state/core"));
    assert_eq!(
        pins[1].path,
        Path::new("/state/core/modules/mod-playerbots")
    );
}

#[test]
fn fetch_is_one_pinned_commit_over_https_and_never_forces_a_used_copy() {
    let pin = Pin {
        repository: CORE_REPOSITORY,
        revision: CORE_REV,
        path: "/state/core dir".into(),
    };
    let [fetch, checkout] = fetch_args(&pin, false);
    assert_eq!(fetch[..3], ["-C", "/state/core dir", "fetch"]);
    assert!(fetch.iter().any(|a| a == "--depth=1"));
    assert_eq!(
        fetch[fetch.len() - 2..],
        [
            "https://github.com/mod-playerbots/azerothcore-wotlk.git".to_owned(),
            CORE_REV.to_owned()
        ]
    );
    assert_eq!(checkout[2..], ["checkout", "--detach", CORE_REV]);
    let [_, fresh] = fetch_args(&pin, true);
    assert_eq!(fresh[2..], ["checkout", "--detach", "--force", CORE_REV]);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(dir: &Path, file: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet"]);
    std::fs::write(dir.join(file), "x").unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "--quiet", "-m", "pin"]);
    git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn verify_mirrors_the_build_cleanliness_checks() {
    let root = std::env::temp_dir().join(format!(
        "server-source-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let core = root.join("core");
    std::fs::create_dir_all(&core).unwrap();
    std::fs::write(core.join(".gitignore"), "/modules/*\n").unwrap();
    git(&core, &["init", "--quiet"]);
    git(&core, &["add", ".gitignore"]);
    let core_rev = repo(&core, "CMakeLists.txt");
    let bots = core.join("modules/mod-playerbots");
    let bots_rev = repo(&bots, "playerbots.cpp");
    fn pins<'a>(core: &Path, core_rev: &'a str, bots_rev: &'a str) -> [Pin<'a>; 2] {
        [
            Pin {
                repository: CORE_REPOSITORY,
                revision: core_rev,
                path: core.to_owned(),
            },
            Pin {
                repository: PLAYERBOTS_REPOSITORY,
                revision: bots_rev,
                path: core.join("modules/mod-playerbots"),
            },
        ]
    }
    let pins_now = pins(&core, &core_rev, &bots_rev);
    verify(&core, &pins_now).unwrap();
    // The build allows untracked core files (a build folder, say), as its own check does.
    std::fs::write(core.join("notes.txt"), "mine").unwrap();
    verify(&core, &pins_now).unwrap();
    let wrong = pins(&core, CORE_REV, &bots_rev);
    assert!(verify(&core, &wrong)
        .unwrap_err()
        .contains("needs 93333d6a9eb0"));
    std::fs::write(core.join("CMakeLists.txt"), "edited").unwrap();
    assert!(verify(&core, &pins_now)
        .unwrap_err()
        .contains("CMakeLists.txt"));
    git(&core, &["checkout", "--", "CMakeLists.txt"]);
    std::fs::write(bots.join("extra.cpp"), "edited").unwrap();
    assert!(verify(&core, &pins_now)
        .unwrap_err()
        .contains("mod-playerbots has changed files"));
    std::fs::remove_dir_all(&bots).unwrap();
    assert!(verify(&core, &pins_now)
        .unwrap_err()
        .contains("mod-playerbots is not checked out"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn git_progress_reads_the_latest_figure() {
    let log = "Receiving objects:  12% (1/9)\rReceiving objects:  45% (4/9), 1.2 MiB\r";
    assert_eq!(progress(log), Some(("Receiving files", 45)));
    let log = "Receiving objects: 100% (9/9), done.\nUpdating files:  30% (3/10)\r";
    assert_eq!(progress(log), Some(("Writing files", 30)));
    assert_eq!(progress("--- git fetch ---\n"), None);
}
