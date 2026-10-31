//! Synthetic MPQs only: no installed assets, subprocesses, cargo invocations or game launches.
use super::*;

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) config: Config,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = env::temp_dir().join(format!(
            "wrathplus-manager-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let config = Config {
            repo: root.join("repo with spaces"),
            data: root.join("install/Data"),
            target: root.join("target with spaces"),
            locale: "enUS".into(),
            server: "realm.example:3724".into(),
        };
        fs::create_dir_all(config.repo.join("crates/wrathplus")).unwrap();
        fs::write(config.repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(
            config.repo.join("crates/wrathplus/Cargo.toml"),
            "[package]\nname='wrathplus'\n",
        )
        .unwrap();
        fs::create_dir_all(config.data.join("enUS")).unwrap();
        let this = Self { root, config };
        this.assets();
        this
    }
    fn assets(&self) {
        let entries = vec![
            ("component.wow-enUS.txt", component("12340", "enUS")),
            ("DBFilesClient\\ChrRaces.dbc", dbc()),
            ("DBFilesClient\\ChrClasses.dbc", dbc()),
            ("DBFilesClient\\Map.dbc", dbc()),
            ("DBFilesClient\\Spell.dbc", dbc()),
            (
                "Interface\\FrameXML\\FrameXML.toc",
                b"## Interface: 30300\nUIParent.lua".to_vec(),
            ),
            (
                "Interface\\FrameXML\\UIParent.lua",
                b"-- synthetic fixture, not stock UI".to_vec(),
            ),
            (
                "Interface\\GlueXML\\GlueXML.toc",
                b"## Interface: 30300".to_vec(),
            ),
        ];
        write_mpq(&self.config.data.join("common.MPQ"), &entries, false);
    }
    fn fake_binary(&self) {
        fs::create_dir_all(self.config.binary_path().parent().unwrap()).unwrap();
        fs::write(
            self.config.binary_path(),
            b"synthetic binary, never executed",
        )
        .unwrap();
        make_executable(&self.config.binary_path());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn component(build: &str, locale: &str) -> Vec<u8> {
    format!("<?xml version=\"1.0\"?><componentinfo format=\"1\"><component name=\"wow-{}\" version=\"{build}\" /></componentinfo>", locale.to_ascii_lowercase()).into_bytes()
}
fn dbc() -> Vec<u8> {
    let mut out = b"WDBC".to_vec();
    for word in [1u32, 1, 4, 1, 42] {
        out.extend(word.to_le_bytes());
    }
    out.push(0);
    out
}

// Tiny independent MPQ fixture encoder using the documented Storm hash/encryption mechanism.
// Only stored single-unit files and tombstones are emitted; it never extracts real game data.
fn crypt_table() -> [u32; 0x500] {
    let mut table = [0; 0x500];
    let mut seed = 0x0010_0001u32;
    for low in 0..256 {
        for high in 0..5 {
            seed = (seed * 125 + 3) % 0x2a_aaab;
            let upper = (seed & 0xffff) << 16;
            seed = (seed * 125 + 3) % 0x2a_aaab;
            table[low + high * 256] = upper | (seed & 0xffff);
        }
    }
    table
}
fn hash(name: &str, kind: usize, table: &[u32; 0x500]) -> u32 {
    let (mut a, mut b) = (0x7fed_7fedu32, 0xeeee_eeeeu32);
    for byte in name
        .replace('/', "\\")
        .bytes()
        .map(|b| b.to_ascii_uppercase())
    {
        a = table[kind + byte as usize] ^ a.wrapping_add(b);
        b = (byte as u32)
            .wrapping_add(a)
            .wrapping_add(b)
            .wrapping_add(b << 5)
            .wrapping_add(3);
    }
    a
}
fn encrypt(words: &mut [u32], mut key: u32, table: &[u32; 0x500]) {
    let mut seed = 0xeeee_eeeeu32;
    for word in words {
        seed = seed.wrapping_add(table[0x400 + (key & 255) as usize]);
        let plain = *word;
        *word ^= key.wrapping_add(seed);
        key = (!key << 21).wrapping_add(0x1111_1111) | (key >> 11);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
    }
}
fn write_mpq(path: &Path, entries: &[(&str, Vec<u8>)], deleted: bool) {
    let table = crypt_table();
    let slots = (entries.len() * 2).next_power_of_two().max(4);
    let hash_pos = 32u32;
    let block_pos = hash_pos + slots as u32 * 16;
    let data_pos = block_pos + entries.len() as u32 * 16;
    let mut hashes = vec![u32::MAX; slots * 4];
    let mut blocks = Vec::new();
    let mut data = Vec::new();
    for (index, (name, bytes)) in entries.iter().enumerate() {
        let mut slot = hash(name, 0, &table) as usize & (slots - 1);
        while hashes[slot * 4 + 3] != u32::MAX {
            slot = (slot + 1) & (slots - 1);
        }
        hashes[slot * 4..slot * 4 + 4].copy_from_slice(&[
            hash(name, 0x100, &table),
            hash(name, 0x200, &table),
            0,
            index as u32,
        ]);
        let bytes = if deleted { &[][..] } else { bytes.as_slice() };
        blocks.extend([
            data_pos + data.len() as u32,
            bytes.len() as u32,
            bytes.len() as u32,
            if deleted { 0x8200_0000 } else { 0x8100_0000 },
        ]);
        data.extend_from_slice(bytes);
    }
    encrypt(&mut hashes, hash("(hash table)", 0x300, &table), &table);
    encrypt(&mut blocks, hash("(block table)", 0x300, &table), &table);
    let mut out = b"MPQ\x1a".to_vec();
    out.extend(32u32.to_le_bytes());
    out.extend((data_pos + data.len() as u32).to_le_bytes());
    out.extend(0u16.to_le_bytes()); // MPQ v1
    out.extend(3u16.to_le_bytes()); // 4096-byte sectors (unused by single-unit files)
    for word in [hash_pos, block_pos, slots as u32, entries.len() as u32] {
        out.extend(word.to_le_bytes());
    }
    for word in hashes.into_iter().chain(blocks) {
        out.extend(word.to_le_bytes());
    }
    out.extend(data);
    fs::write(path, out).unwrap();
}

#[test]
fn validates_real_archive_bytes_without_writing_assets_or_state() {
    let f = Fixture::new();
    let archive = f.config.data.join("common.MPQ");
    let before = fs::read(&archive).unwrap();
    let validated = validate(&f.config).unwrap();
    assert!(validated.summary.contains("12340"));
    assert_eq!(validated.config, f.config);
    assert_eq!(fs::read(archive).unwrap(), before);
    assert!(!f.config.state_path().exists());
    assert!(!f.config.target.exists());
}

#[test]
fn filenames_and_loose_files_cannot_attest_assets() {
    let f = Fixture::new();
    fs::write(f.config.data.join("common.MPQ"), b"not an MPQ").unwrap();
    fs::write(
        f.config.data.join("component.wow-enUS.txt"),
        component("12340", "enUS"),
    )
    .unwrap();
    assert!(validate(&f.config).is_err());
    fs::remove_file(f.config.data.join("common.MPQ")).unwrap();
    assert!(validate(&f.config).is_err());
}

#[test]
fn highest_patch_manifest_wins_and_wrong_build_or_locale_is_rejected() {
    let f = Fixture::new();
    let path = f.config.data.join("enUS/patch-enUS-3.MPQ");
    for (build, locale) in [("5875", "enUS"), ("12340", "frFR"), ("123400", "enUS")] {
        write_mpq(
            &path,
            &[("component.wow-enUS.txt", component(build, locale))],
            false,
        );
        assert!(validate(&f.config).is_err());
    }
    write_mpq(
        &path,
        &[("component.wow-enUS.txt", component("12340", "enUS"))],
        false,
    );
    assert!(validate(&f.config).is_ok());
}

#[test]
fn patch_tombstones_and_bad_dbc_never_fall_back_to_base() {
    let f = Fixture::new();
    let path = f.config.data.join("patch-3.MPQ");
    write_mpq(&path, &[("DBFilesClient\\Map.dbc", Vec::new())], true);
    assert!(validate(&f.config).unwrap_err().contains("deleted"));
    write_mpq(
        &path,
        &[("DBFilesClient\\Map.dbc", b"WDBCgarbage".to_vec())],
        false,
    );
    assert!(validate(&f.config).is_err());
    write_mpq(
        &path,
        &[("Interface\\FrameXML\\UIParent.lua", b" \r\n".to_vec())],
        false,
    );
    assert!(validate(&f.config).is_err());
}

#[test]
fn invalid_locale_empty_data_and_output_containment_fail_closed() {
    let f = Fixture::new();
    for locale in ["../enUS", "xxxx", "enus", "", "enUS/../../"] {
        let mut config = f.config.clone();
        config.locale = locale.into();
        assert!(validate(&config).is_err());
    }
    let mut config = f.config.clone();
    config.data = PathBuf::new();
    assert!(validate(&config).is_err());
    for path in [&f.config.data, f.config.data.parent().unwrap(), &f.root] {
        let mut config = f.config.clone();
        config.target = path.join("target");
        if path == f.root {
            config.target = f.root.clone();
        }
        assert!(validate(&config).is_err());
    }
    let mut config = f.config.clone();
    config.target = f.root.join("outside/../install/Data/target");
    assert!(validate(&config).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_outputs_and_state_aliases_cannot_write_to_install() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(&f.config.data, &f.config.target).unwrap();
    assert!(validate(&f.config).is_err());
    fs::remove_file(&f.config.target).unwrap();
    symlink(&f.config.data, f.config.repo.join("benilla-config")).unwrap();
    assert!(validate(&f.config).is_err());
    fs::remove_file(f.config.repo.join("benilla-config")).unwrap();
    f.fake_binary();
    let alias = f.root.join("state-alias");
    symlink(&f.config.data, &alias).unwrap();
    assert!(save_config(&f.config, &alias.join("manager.json")).is_err());
    assert!(!f.config.data.join("manager.json").exists());
}

#[test]
fn persistence_requires_successful_install_and_uses_explicit_path() {
    let f = Fixture::new();
    let state = f.root.join("explicit/settings.json");
    assert!(save_config(&f.config, &state).is_err());
    assert!(!state.exists());
    f.fake_binary();
    save_config(&f.config, &state).unwrap();
    assert_eq!(load_config(&state).unwrap(), f.config);
    assert!(!f.config.state_path().exists());
    fs::write(&state, b"{\"server\":\"x\"}").unwrap();
    assert!(load_config(&state).is_err());
    fs::write(&state, vec![b' '; 65537]).unwrap();
    assert!(load_config(&state).is_err());
}

#[test]
fn commands_keep_user_paths_as_single_arguments_and_clear_overrides() {
    let f = Fixture::new();
    let command = build_command(&f.config, Path::new("cargo"));
    assert_eq!(command.get_program(), "cargo");
    let args: Vec<_> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        &args[..12],
        &[
            "build",
            "--locked",
            "-p",
            "wrathplus",
            "--bin",
            "wrathplus",
            "--no-default-features",
            "--profile",
            "play",
            "--jobs",
            "2",
            "--manifest-path"
        ]
    );
    assert_eq!(args[12], f.config.repo.join("Cargo.toml").to_string_lossy());
    assert_eq!(args[14], f.config.target.to_string_lossy());
    assert!(command
        .get_envs()
        .any(|(k, v)| k == "CARGO_INCREMENTAL" && v == Some(std::ffi::OsStr::new("0"))));
    let state = f.config.repo.join("benilla-config");
    let command = launch_command(&f.config, &state);
    assert_eq!(command.get_program(), f.config.binary_path().as_os_str());
    assert_eq!(command.get_current_dir(), Some(state.as_path()));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![
            std::ffi::OsStr::new("--server"),
            std::ffi::OsStr::new(&f.config.server)
        ]
    );
    let allowed = [
        "WOW_DATA",
        "WOW_CLIENT_PROFILE",
        "WOW_LOCALE",
        "WOW_HOST",
        "BENILLA_HOME",
    ];
    for (key, value) in command.get_envs() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("WOW_") || upper.starts_with("BENILLA_") || upper.contains("PROBE") {
            assert!(value.is_none() || allowed.contains(&upper.as_str()));
        }
    }
}

#[test]
fn locks_serialize_operations_and_running_marker_fences_install() {
    let f = Fixture::new();
    let lock = operation_lock(&f.config).unwrap();
    assert!(operation_lock(&f.config).is_err());
    drop(lock);
    assert!(operation_lock(&f.config).is_ok());
    fs::write(running_path(&f.config), b"12345").unwrap();
    assert!(require_stopped(&f.config).is_err());
    fs::remove_file(running_path(&f.config)).unwrap();
    assert!(require_stopped(&f.config).is_ok());
}

#[test]
fn failed_install_or_persistence_preserves_prior_binary_and_config() {
    let f = Fixture::new();
    f.fake_binary();
    save_config(&f.config, &f.config.state_path()).unwrap();
    let old_binary = fs::read(f.config.binary_path()).unwrap();
    let old_state = fs::read(f.config.state_path()).unwrap();
    assert!(install(&f.config, &f.root.join("missing-build-output")).is_err());
    assert_eq!(fs::read(f.config.binary_path()).unwrap(), old_binary);
    assert_eq!(fs::read(f.config.state_path()).unwrap(), old_state);
    let source = f.root.join("new-binary");
    fs::write(&source, b"new synthetic binary").unwrap();
    // Fail save_config's asset revalidation after promotion, exercising rollback without cargo.
    fs::write(f.config.data.join("common.MPQ"), b"corrupt").unwrap();
    assert!(install(&f.config, &source).is_err());
    assert_eq!(fs::read(f.config.binary_path()).unwrap(), old_binary);
    assert_eq!(fs::read(f.config.state_path()).unwrap(), old_state);
}

#[test]
fn successful_install_promotes_and_only_then_saves() {
    let f = Fixture::new();
    let source = f.root.join("new-binary");
    fs::write(&source, b"synthetic built binary").unwrap();
    make_executable(&source);
    install(&f.config, &source).unwrap();
    assert_eq!(
        fs::read(f.config.binary_path()).unwrap(),
        b"synthetic built binary"
    );
    assert_eq!(load_config(&f.config.state_path()).unwrap(), f.config);
    assert!(!f.config.binary_path().with_extension("backup").exists());
}

#[test]
fn manifest_and_dbc_parsers_reject_substring_attestation() {
    assert!(check_component(&component("12340", "enUS"), "enUS").is_ok());
    assert!(check_component(
        b"<!-- <component name='wow' version='12340' locale='enUS'/> -->",
        "enUS"
    )
    .is_err());
    assert!(check_component(
        b"<component name='wow' fakeversion='12340' locale='enUS'/>",
        "enUS"
    )
    .is_err());
    assert!(check_component(b"<component name='wow' version='12340' locale='enUS'/><component name='wow' version='5875' locale='enUS'/>", "enUS").is_err());
    assert!(check_dbc(&dbc(), "fixture").is_ok());
    let mut bytes = dbc();
    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(check_dbc(&bytes, "fixture").is_err());
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[test]
fn existing_live_build_can_be_used_and_saved_without_copy_or_cargo() {
    let f = Fixture::new();
    let live = f
        .config
        .repo
        .join("target/azb-live/bin")
        .join(binary_name());
    fs::create_dir_all(live.parent().unwrap()).unwrap();
    fs::write(&live, b"synthetic executable; never run").unwrap();
    make_executable(&live);
    assert!(available(&f.config));
    assert!(!f.config.binary_path().exists());
    save_config(&f.config, &f.config.state_path()).unwrap();
    assert_eq!(load_config(&f.config.state_path()).unwrap(), f.config);
    assert_eq!(
        launch_command(&f.config, &f.config.repo).get_program(),
        live.as_os_str()
    );
    assert!(!f.config.binary_path().exists());
    fs::remove_file(&live).unwrap();
    assert!(!available(&f.config));
}
