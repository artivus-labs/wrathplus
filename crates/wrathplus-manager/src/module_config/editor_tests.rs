use super::*;
use crate::Manager;

#[test]
fn dirty_editor_cancel_requires_explicit_discard() {
    let mut manager = Manager::default();
    manager.config_editor.open = true;
    manager.config_editor.selected = Some(PathBuf::from("not-read.conf"));
    let _ = manager.config_update(Input::Edit(text_editor::Action::Edit(
        text_editor::Edit::Insert('x'),
    )));
    let _ = manager.config_update(Input::Cancel);
    assert!(manager.config_editor.open);
    assert!(manager.config_editor.dirty);
    assert!(manager.config_editor.notice.contains("Discard"));
    assert_eq!(manager.config_editor.content.text(), "x");
    let _ = manager.config_update(Input::Open {
        module: "other".into(),
        name: "other.conf".into(),
    });
    assert_eq!(manager.config_editor.content.text(), "x");
    let _ = manager.config_update(Input::Discard);
    assert!(!manager.config_editor.open);
    assert!(manager.config_editor.content.text().is_empty());
}

#[test]
fn action_debug_does_not_expose_pasted_secrets() {
    let input = Input::Edit(text_editor::Action::Edit(text_editor::Edit::Paste(
        std::sync::Arc::new("private-token".into()),
    )));
    assert!(!format!("{input:?}").contains("private-token"));
}

#[test]
fn modal_template_open_cancel_save_and_existing_config_conflict() {
    use std::fs;
    let f = super::tests::Fixture::new();
    let template = f.core.join("modules/mod-playerbots/conf/bots.conf.dist");
    fs::create_dir_all(template.parent().unwrap()).unwrap();
    fs::write(&template, "Original = template\n").unwrap();
    let mut catalog = crate::modules::Catalog::load(&f.config, &f.core).unwrap();
    catalog.set_enabled("mod-playerbots", false).unwrap();
    catalog.save(&f.config).unwrap();
    let detected = discover(&f.config, &f.core, &catalog.entries()[0]).unwrap();
    assert_eq!(
        detected.len(),
        1,
        "Disabled modules still expose their config"
    );
    assert_eq!(detected[0].name, "bots.conf");
    let mut manager = Manager {
        config: f.config.clone(),
        ..Manager::default()
    };
    let _ = manager.server_update(crate::server::Input::Source(
        f.core.to_string_lossy().into(),
    ));
    let open = || Input::Open {
        module: "mod-playerbots".into(),
        name: "bots.conf".into(),
    };
    let _ = manager.config_update(open());
    assert!(manager.config_editor.open);
    assert!(manager.config_editor.selected.is_some());
    assert!(!f.path().exists(), "Opening template must be read-only");
    let _ = manager.config_update(Input::Cancel);
    assert!(!manager.config_editor.open);
    assert!(!f.path().exists());
    let _ = manager.config_update(open());
    let _ = manager.config_update(Input::Save);
    assert!(!manager.config_editor.open);
    assert_eq!(
        fs::read_to_string(f.path()).unwrap(),
        "Original = template\n"
    );
    assert_eq!(
        fs::read_to_string(&template).unwrap(),
        "Original = template\n"
    );
    fs::write(f.path(), "Local = existing\n").unwrap();
    let _ = manager.config_update(open());
    assert_eq!(manager.config_editor.content.text(), "Local = existing\n");
    fs::write(f.path(), "External = changed\n").unwrap();
    let _ = manager.config_update(Input::Save);
    assert!(manager.config_editor.open);
    assert!(manager.config_editor.notice.contains("externally"));
    assert_eq!(
        fs::read_to_string(f.path()).unwrap(),
        "External = changed\n"
    );
    let _ = manager.config_update(Input::Discard);
    let _ = manager.config_update(open());
    let _ = manager.config_update(Input::Save);
    assert!(!manager.config_editor.open);
}
