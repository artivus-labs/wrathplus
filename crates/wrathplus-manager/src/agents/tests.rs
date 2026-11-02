use super::*;

fn agent(account: &str, port: u16) -> Agent {
    Agent {
        name: account.into(),
        account: account.into(),
        port,
    }
}

#[test]
fn agent_names_become_safe_account_names() {
    assert_eq!(account_for("Scout").unwrap(), "AGENTSCOUT");
    assert_eq!(account_for(" mr. bot 2 ").unwrap(), "AGENTMRBOT2");
    assert_eq!(
        account_for("averyveryverylongname").unwrap(),
        "AGENTAVERYVERYVE"
    );
    assert!(account_for("!!").is_err());
}

#[test]
fn each_agent_gets_its_own_control_port() {
    assert_eq!(next_port(&[]), FIRST_PORT);
    let list = [agent("A", FIRST_PORT), agent("B", FIRST_PORT + 2)];
    assert_eq!(next_port(&list), FIRST_PORT + 1);
}

#[test]
fn the_registry_and_secrets_round_trip_privately() {
    let dir = std::env::temp_dir().join(format!("wrathplus-agents-{}", std::process::id()));
    let mut config = Config::defaults();
    config.repo = dir.clone();
    std::fs::create_dir_all(config.state_root()).unwrap();
    assert!(load(&config).unwrap().is_empty());
    save(&config, &[agent("AGENTSCOUT", FIRST_PORT)]).unwrap();
    assert_eq!(load(&config).unwrap()[0].port, FIRST_PORT);
    let token = home(&config, "AGENTSCOUT").join("token");
    write_private(&token, "abc").unwrap();
    assert_eq!(read_private(&token).unwrap(), "abc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let view = view_url(&config, &agent("AGENTSCOUT", FIRST_PORT)).unwrap();
    assert_eq!(
        view,
        format!("http://127.0.0.1:{FIRST_PORT}/view?token=abc")
    );
    let mcp = mcp_command(&config, &agent("AGENTSCOUT", FIRST_PORT)).unwrap();
    assert!(mcp.contains("WRATHPLUS_AGENT_TOKEN=abc") && mcp.contains("agents/mcp/src/index.ts"));
    assert!(forget(&config, "AGENTSCOUT").unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Against the running managed realm, using the manager.json at WRATHPLUS_LIVE_MANAGER_JSON:
/// create an agent (a real account), check its files, then delete the account and forget it.
#[test]
#[ignore = "needs the managed realm running"]
fn live_create_an_agent_then_remove_it() {
    let path =
        std::env::var("WRATHPLUS_LIVE_MANAGER_JSON").expect("set WRATHPLUS_LIVE_MANAGER_JSON");
    let config = crate::setup::load_config(std::path::Path::new(&path)).expect("manager config");
    let agent = create(&config, "Livecheck").expect("create");
    assert_eq!(agent.account, "AGENTLIVECHECK");
    let dir = home(&config, &agent.account);
    assert_eq!(read_private(&dir.join("token")).unwrap().len(), 32);
    let reply =
        crate::server::admin_command(&config, ".account delete AGENTLIVECHECK").expect("delete");
    eprintln!("{reply}");
    forget(&config, &agent.account).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
