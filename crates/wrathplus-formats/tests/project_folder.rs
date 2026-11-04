//! A launcher's recorded project folder is where a dev build looks for the install, in place of
//! WrathPlus's checkout, while the addon corpus stays WrathPlus's. Recording is process-wide and every
//! test resolves through it, so this one test has a file, and so a process, of its own.

use std::path::Path;

#[test]
fn a_recorded_launcher_folder_is_where_a_dev_build_looks() {
    let tmp = std::env::temp_dir().join(format!("wrathplus-project-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // This process's own environment: `WOW_DATA=` would empty the ladder, and an override would
    // move the corpus.
    std::env::remove_var("WOW_DATA");
    std::env::remove_var("WRATHPLUS_ADDON_CORPUS");
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();

    // A player build's stamp is empty and records nothing.
    wrathplus_formats::set_project_folder("");
    if cfg!(feature = "dev") {
        assert_eq!(wrathplus_formats::project_folder(), Some(checkout));
    }

    wrathplus_formats::set_project_folder(tmp.to_str().unwrap());
    wrathplus_formats::set_project_folder("/a/later/call/changes/nothing");
    if cfg!(feature = "dev") {
        assert_eq!(wrathplus_formats::project_folder(), Some(tmp.as_path()));
        let c = wrathplus_formats::candidates();
        assert_eq!(c.first(), Some(&tmp.join("WoW/Data")), "{c:?}");
        assert!(
            !c.contains(&checkout.join("WoW/Data")),
            "WrathPlus's checkout is still looked in for the install: {c:?}"
        );
        assert_eq!(
            wrathplus_formats::addon_corpus_candidates(),
            vec![checkout.join("wow-addons-vanilla")],
            "the corpus is WrathPlus's whoever the launcher is"
        );
    } else {
        assert_eq!(wrathplus_formats::project_folder(), None);
    }
    std::fs::remove_dir_all(&tmp).ok();
}
