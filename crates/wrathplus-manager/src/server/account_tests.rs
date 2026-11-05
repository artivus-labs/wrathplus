use super::super::characters::parse_soap;
use super::super::guide::Mark;
use super::super::status::Health;
use super::*;

const PASSWORD: &str = "Sw0rdfish!";

fn ok(result: &str) -> Result<String> {
    parse_soap(&format!(
        "HTTP/1.1 200 OK\r\n\r\n<SOAP-ENV:Body><ns1:executeCommandResponse><result>{result}</result></ns1:executeCommandResponse>"
    ))
}

fn fault(message: &str) -> Result<String> {
    parse_soap(&format!(
        "HTTP/1.1 500 Internal Server Error\r\n\r\n<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client</faultcode><faultstring>{message}</faultstring></SOAP-ENV:Fault>"
    ))
}

/// A realm answering the way AzerothCore's SOAP does: `create` with `on_create`, and the
/// expansion flag finding no account `missing` times first.
fn realm(
    on_create: Result<String>,
    mut missing: usize,
) -> (
    impl FnMut(&str) -> Result<String>,
    std::rc::Rc<std::cell::RefCell<Vec<String>>>,
) {
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = sent.clone();
    let run = move |command: &str| {
        log.borrow_mut().push(command.to_owned());
        if command.starts_with(".account create ") {
            on_create.clone()
        } else if missing > 0 {
            missing -= 1;
            fault("Account &apos;PLAYER&apos; does not exist")
        } else {
            ok("The expansion of account PLAYER is now 2.")
        }
    };
    (run, sent)
}

#[test]
fn a_new_account_gets_wrath_content_once_the_realm_has_it() {
    let (mut run, sent) = realm(ok("Account created: PLAYER"), 2);
    let waits = std::cell::Cell::new(0);
    let created = create_with(&mut run, &|| waits.set(waits.get() + 1), "PLAYER", PASSWORD);
    assert_eq!(created, Ok(Created::New("PLAYER".into())));
    let sent = sent.borrow();
    assert_eq!(sent[0], format!(".account create PLAYER {PASSWORD}"));
    assert_eq!(sent.len(), 4);
    assert!(sent[1..].iter().all(|c| c == ".account set addon PLAYER 2"));
    assert_eq!(waits.get(), 2);
}

#[test]
fn an_existing_account_is_kept_and_reported_as_found() {
    let (mut run, sent) = realm(fault("Account with this name already exist!"), 0);
    let created = create_with(&mut run, &|| {}, "PLAYER", PASSWORD);
    assert_eq!(created, Ok(Created::Existing("PLAYER".into())));
    // Nothing changes its password; it still gets Wrath content.
    assert_eq!(sent.borrow().len(), 2);
    assert!(!sent.borrow()[1].contains(PASSWORD));
}

#[test]
fn failures_stop_and_never_repeat_the_password() {
    let (mut run, sent) = realm(Err(format!("echoed {PASSWORD}")), 0);
    let error = create_with(&mut run, &|| {}, "PLAYER", PASSWORD).unwrap_err();
    assert!(!error.contains(PASSWORD), "{error}");
    assert_eq!(sent.borrow().len(), 1);
    let refused = parse_soap("HTTP/1.1 401 Unauthorized\r\n\r\n");
    let (mut run, _) = realm(refused, 0);
    assert!(create_with(&mut run, &|| {}, "PLAYER", PASSWORD)
        .unwrap_err()
        .contains("refused"));
    // The flag gives up after twenty tries.
    let (mut run, sent) = realm(ok("Account created: PLAYER"), 100);
    assert!(create_with(&mut run, &|| {}, "PLAYER", PASSWORD).is_err());
    assert_eq!(sent.borrow().len(), 21);
}

#[test]
fn names_and_passwords_are_checked_before_anything_is_sent() {
    for bad in ["", "has space", "seventeen-chars-x", "tab\there"] {
        let (mut run, sent) = realm(ok(""), 0);
        let error = create_with(&mut run, &|| {}, "PLAYER", bad).unwrap_err();
        assert!(bad.is_empty() || !error.contains(bad), "{error}");
        assert!(sent.borrow().is_empty());
    }
    assert_eq!(check_name(" player1 "), Ok("PLAYER1".into()));
    assert!(check_name("no-dashes").is_err() && check_name("").is_err());
    assert!(check_password(DEFAULT_PASSWORD).is_ok());
    assert_eq!(default_name(Some("shannon.s".into())), "SHANNONS");
    assert_eq!(default_name(Some("名前".into())), "PLAYER");
    assert_eq!(default_name(None), "PLAYER");
    assert_eq!(
        default_name(Some("averyveryverylongname".into())),
        "AVERYVERYVERYLON"
    );
    let shown = format!("{:?}", AccountInput::Password(PASSWORD.into()));
    assert!(!shown.contains(PASSWORD));
}

#[test]
fn the_account_is_kept_privately_for_the_login_prefill() {
    let root = std::env::temp_dir().join(format!("server-account-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut config = setup::Config::defaults();
    config.repo = root.clone();
    config.data = std::path::PathBuf::new();
    assert_eq!(local(&config), None);
    remember(&config, "PLAYER", PASSWORD).unwrap();
    assert_eq!(local(&config), Some(("PLAYER".into(), PASSWORD.into())));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(local_path(&config))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let mut server = Server::default();
    server.load_account(&config);
    assert_eq!(server.account().created.as_deref(), Some("PLAYER"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_step_waits_for_the_realm_then_reports_the_account() {
    let mut manager = Manager::default();
    let step = |manager: &Manager| {
        let steps = manager.server_state.guide(true);
        (
            steps[4].mark,
            steps[4].action.as_ref().map(|(_, m)| m.is_some()),
        )
    };
    manager.server_state.loaded = true;
    assert_eq!(step(&manager), (Mark::Later, None));
    // Not running: Create does nothing.
    let _ = manager.account_update(AccountInput::Create);
    assert!(!manager.server_state.account.busy);

    manager.server_state.health = Health::Running;
    let _ = manager.account_update(AccountInput::Name("player".into()));
    let _ = manager.account_update(AccountInput::Password(String::new()));
    assert_eq!(step(&manager), (Mark::Now, Some(false)));
    let _ = manager.account_update(AccountInput::Password(PASSWORD.into()));
    assert_eq!(step(&manager), (Mark::Now, Some(true)));

    let _ = manager.account_update(AccountInput::Finished(Err("The server refused".into())));
    assert_eq!(step(&manager).0, Mark::Now);
    assert!(matches!(&manager.server_state.account.notice, Some(Err(_))));

    let _ = manager.account_update(AccountInput::Finished(Ok(Created::Existing(
        "PLAYER".into(),
    ))));
    assert_eq!(step(&manager).0, Mark::Done);
    let note = manager.events.last().unwrap();
    assert!(note.contains("already exists") && !note.contains(PASSWORD));
    assert!(manager.server_state.guide(true)[4]
        .detail
        .starts_with("PLAYER"));
}
