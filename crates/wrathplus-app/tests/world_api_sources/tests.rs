use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "world-api-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, text: &str) {
        let file = self.0.join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn consumers(&self) -> BTreeSet<String> {
        let tests = test_module_files(&self.0);
        super::super::rs_files(&self.0)
            .into_iter()
            .filter(|file| {
                !tests.contains(&file.strip_prefix(&self.0).unwrap().display().to_string())
            })
            .flat_map(|file| {
                super::super::paths_in(&std::fs::read_to_string(file).unwrap(), true, true)
                    .into_iter()
                    .map(|(item, _)| item)
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

const LEAK: &str = "use wrathplus_world::world_point::init_world_point_resources;\n";

#[test]
fn test_only_files_inherit_through_explicit_and_default_module_paths() {
    let f = Fixture::new();
    f.write(
        "lib.rs",
        "mod ui;\n#[cfg(test)]\n#[allow(dead_code)]\n\n// fixture\n#[path = \"suite.rs\"]\nmod checks;\n",
    );
    f.write("ui.rs", "#[cfg(test)]\nmod checks;\n");
    f.write("ui/checks.rs", "mod nested;\n");
    f.write("ui/checks/nested/mod.rs", "mod leaf;\n");
    f.write("ui/checks/nested/leaf.rs", LEAK);
    // Explicit paths are relative to the containing file, not its default module directory.
    f.write("suite.rs", "#[path = \"login.rs\"]\nmod login;\n");
    f.write("login.rs", "mod child;\n");
    f.write("login/child.rs", LEAK);
    assert_eq!(
        test_module_files(&f.0),
        BTreeSet::from([
            "ui/checks.rs".into(),
            "ui/checks/nested/mod.rs".into(),
            "ui/checks/nested/leaf.rs".into(),
            "suite.rs".into(),
            "login.rs".into(),
            "login/child.rs".into(),
        ])
    );
    assert!(f.consumers().is_empty());
}

#[test]
fn production_modules_still_expose_the_real_leak() {
    let f = Fixture::new();
    f.write(
        "lib.rs",
        "#[cfg(test)]\nfn fixture() {}\nmod tests;\n#[path = \"live.rs\"]\npub(crate) mod live;\n",
    );
    // Neither a test-like filename nor a preceding, unrelated cfg(test) hides production.
    f.write("tests.rs", LEAK);
    f.write("live.rs", "mod child;\n");
    f.write("live/child.rs", LEAK);
    assert!(test_module_files(&f.0).is_empty());
    assert_eq!(
        f.consumers(),
        BTreeSet::from(["world_point::init_world_point_resources".into()])
    );
}

#[test]
fn a_file_shared_with_production_and_its_children_are_not_test_only() {
    let f = Fixture::new();
    f.write(
        "lib.rs",
        "#[cfg(test)]\nmod suite;\n#[path = \"shared.rs\"]\nmod live;\n",
    );
    f.write("suite.rs", "#[path = \"shared.rs\"]\nmod helper;\n");
    f.write("shared.rs", "mod child;\n");
    f.write("shared/child.rs", LEAK);
    assert_eq!(test_module_files(&f.0), BTreeSet::from(["suite.rs".into()]));
    assert_eq!(
        f.consumers(),
        BTreeSet::from(["world_point::init_world_point_resources".into()])
    );
}
