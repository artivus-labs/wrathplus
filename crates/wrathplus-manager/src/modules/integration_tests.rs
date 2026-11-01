use super::*;
#[test]
fn content_change_is_detected_and_staged_content_is_immutable() {
    let f = super::tests::Fixture::new();
    let source = f.core.join("modules/mod-playerbots");
    std::fs::write(source.join("src/main.cpp"), "before").unwrap();
    let plan = prepare(&f.config, &f.core).unwrap();
    let staged = stage(&f.config, &plan).unwrap();
    let before = fingerprint(&source).unwrap();
    assert_eq!(
        before,
        fingerprint(&staged.path.join("mod-playerbots")).unwrap()
    );
    std::fs::write(source.join("src/main.cpp"), "after").unwrap();
    assert_ne!(before, fingerprint(&source).unwrap());
    assert_eq!(
        before,
        fingerprint(&staged.path.join("mod-playerbots")).unwrap()
    );
}
#[test]
fn addon_import_rejects_server_data_overlap_before_network_or_write() {
    let mut f = super::tests::Fixture::new();
    f.config.repo = f.config.repo.canonicalize().unwrap();
    std::fs::create_dir_all(f.config.state_root()).unwrap();
    std::fs::write(
        f.config.state_root().join("manager-server.json"),
        serde_json::to_vec(&serde_json::json!({"data": f.config.repo})).unwrap(),
    )
    .unwrap();
    assert!(clone_addon(&f.config, "owner/repository")
        .unwrap_err()
        .contains("overlaps"));
    assert!(!f.config.state_root().join("managed-addon-sources").exists());
}
