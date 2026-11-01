use super::*;
#[test]
fn staged_build_contains_only_enabled_modules_and_never_modifies_core() {
    let f = super::tests::Fixture::new();
    std::fs::write(
        f.core.join("modules/mod-playerbots/src/main.cpp"),
        "required",
    )
    .unwrap();
    let mut catalog = Catalog::load(&f.config, &f.core).unwrap();
    catalog.set_enabled("mod-unknown", false).unwrap();
    catalog.save(&f.config).unwrap();
    let plan = prepare(&f.config, &f.core).unwrap();
    let staged = stage(&f.config, &plan).unwrap();
    assert!(staged.path.join("mod-playerbots/src/main.cpp").exists());
    assert!(!staged.path.join("mod-unknown").exists());
    assert!(f.core.join("modules/mod-unknown/src").exists());
    let path = staged.path.clone();
    drop(staged);
    assert!(!path.exists());
}
