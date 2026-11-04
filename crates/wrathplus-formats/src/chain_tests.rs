use super::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn synthetic_data_tree(root_names: &[&str], locale_dir: &str, locale_names: &[&str]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "wrathplus-wrath-chain-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let locale = root.join(locale_dir);
    fs::create_dir_all(&locale).unwrap();
    for name in root_names {
        fs::write(root.join(name), []).unwrap();
    }
    for name in locale_names {
        fs::write(locale.join(name), []).unwrap();
    }
    root
}

fn relative_plan(root: &Path, locale: &str) -> Vec<String> {
    plan_with(root, locale, false)
}

fn plan_with(root: &Path, locale: &str, nonstock: bool) -> Vec<String> {
    wrath_mount_plan(root, locale, nonstock)
        .unwrap()
        .into_iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn wrath_mount_plan_matches_335_profile_and_numeric_patch_layers() {
    let root = synthetic_data_tree(
        &[
            "PATCH-10.mpq",
            "lichking.MPQ",
            "common.MPQ",
            "patch-3.MPQ",
            "expansion.MPQ",
            "patch.MPQ",
            "common-2.MPQ",
            "patch-2.MPQ",
            "patch-not-a-number.MPQ",
        ],
        "enUS",
        &[
            "patch-enUS-4.MPQ",
            "lichking-speech-enUS.MPQ",
            "locale-enUS.MPQ",
            "expansion-locale-enUS.MPQ",
            "patch-enUS.MPQ",
            "speech-enUS.MPQ",
            "patch-enUS-3.MPQ",
            "lichking-locale-enUS.MPQ",
            "expansion-speech-enUS.MPQ",
            "patch-enUS-2.MPQ",
        ],
    );
    assert_eq!(
        relative_plan(&root, "enUS"),
        owned(&[
            "common.MPQ",
            "common-2.MPQ",
            "expansion.MPQ",
            "lichking.MPQ",
            "enUS/locale-enUS.MPQ",
            "enUS/speech-enUS.MPQ",
            "enUS/expansion-locale-enUS.MPQ",
            "enUS/expansion-speech-enUS.MPQ",
            "enUS/lichking-locale-enUS.MPQ",
            "enUS/lichking-speech-enUS.MPQ",
            "patch.MPQ",
            "patch-2.MPQ",
            "patch-3.MPQ",
            "enUS/patch-enUS.MPQ",
            "enUS/patch-enUS-2.MPQ",
            "enUS/patch-enUS-3.MPQ",
        ]),
        "a repack's patch-10 and patch-enUS-4 stay out by default"
    );
    let opted_in = plan_with(&root, "enUS", true);
    assert_eq!(
        opted_in[opted_in.len() - 6..],
        owned(&[
            "patch-3.MPQ",
            "PATCH-10.mpq",
            "enUS/patch-enUS.MPQ",
            "enUS/patch-enUS-2.MPQ",
            "enUS/patch-enUS-3.MPQ",
            "enUS/patch-enUS-4.MPQ",
        ]),
        "and mount in numeric order when asked for"
    );
    fs::remove_dir_all(root).unwrap();
}

/// The installed client mounts Blizzard's archives alone by default: a repack's
/// `patch-enUS-4.MPQ` (its GlueXML among them) stays out unless asked for.
#[test]
fn the_installed_default_chain_is_stock() {
    let data = crate::wow_data_or_skip!();
    if std::env::var(NONSTOCK_PATCHES_ENV).is_ok_and(|v| v == "1") {
        return;
    }
    let chain = Chain::open_wrath(&data, "enUS").unwrap();
    assert!(chain.contains(r"Interface\GlueXML\GlueXML.toc"));
    assert!(
        !chain.contains(r"Interface\GlueXML\TheraWoW.xml"),
        "the repack's glue is not mounted"
    );
}

#[test]
fn wrath_mount_plan_resolves_locale_and_archives_case_insensitively() {
    let root = synthetic_data_tree(
        &["COMMON.mpq", "Patch-2.mpq", "PATCH.MPQ"],
        "ENus",
        &["LOCALE-ENUS.mpq", "PATCH-enUS.MPQ"],
    );
    assert_eq!(
        relative_plan(&root, "enus"),
        owned(&[
            "COMMON.mpq",
            "ENus/LOCALE-ENUS.mpq",
            "PATCH.MPQ",
            "Patch-2.mpq",
            "ENus/PATCH-enUS.MPQ",
        ])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn wrath_locale_rejects_path_components_and_non_locale_shapes() {
    for locale in ["../x", "en/US", "en\\US", ".enU", "en1S", "enusx", ""] {
        assert!(
            validate_wrath_locale(locale).is_err(),
            "accepted {locale:?}"
        );
    }
    assert!(validate_wrath_locale("enUS").is_ok());
}
