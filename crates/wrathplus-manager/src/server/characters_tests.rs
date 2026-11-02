use super::characters::*;
use super::characters_ui::{class, race};

#[test]
fn the_roster_reads_characters_then_accounts() {
    let roster = parse_roster(
        "Dongle\t17\t3\t3\tAZBENCH\t0\nWifey\t16\t1\t5\tAZBENCH\t1\n--\nAZBENCH\nPLAYTEST\n",
    )
    .unwrap();
    assert_eq!(roster.accounts, ["AZBENCH", "PLAYTEST"]);
    assert_eq!(roster.characters.len(), 2);
    let wifey = &roster.characters[1];
    assert_eq!(
        (wifey.name.as_str(), wifey.level, wifey.online),
        ("Wifey", 16, true)
    );
    assert_eq!(
        format!("{} {}", race(wifey.race), class(wifey.class)),
        "Human Priest"
    );
    assert!(parse_roster("broken row\n").is_err());
    assert!(parse_roster("--\n").unwrap().characters.is_empty());
}

#[test]
fn soap_results_and_faults_become_messages() {
    let ok = "HTTP/1.1 200 OK\r\n\r\n<SOAP-ENV:Body><ns1:executeCommandResponse><result>Character dumped successfully.&#xA;</result></ns1:executeCommandResponse>";
    assert_eq!(parse_soap(ok).unwrap(), "Character dumped successfully.");
    let fault = "HTTP/1.1 500 Internal Server Error\r\n\r\n<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client</faultcode><faultstring>Account &apos;NOBODY&apos; does not exist</faultstring></SOAP-ENV:Fault>";
    assert_eq!(
        parse_soap(fault).unwrap_err(),
        "Account 'NOBODY' does not exist"
    );
    assert!(parse_soap("HTTP/1.1 401 Unauthorized\r\n\r\n")
        .unwrap_err()
        .contains("refused"));
    assert!(parse_soap("").unwrap_err().contains("Command failed"));
}

#[test]
fn basic_auth_is_standard_base64() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"WPMANAGER:ABC23"), "V1BNQU5BR0VSOkFCQzIz");
}

#[test]
fn names_and_files_are_checked_before_the_server_sees_them() {
    assert!(check_name("Dongle").is_ok());
    for bad in ["D", "Thirteenchars", "Dong le", "Dongle;", "Dóngle"] {
        assert!(check_name(bad).is_err(), "{bad}");
    }
    assert!(check_account("AZBENCH").is_ok());
    assert!(check_account("AZ BENCH").is_err() && check_account("").is_err());
    assert!(looks_like_dump(
        b"IMPORTANT NOTE: ...\nINSERT INTO `characters` VALUES ('1');\n"
    ));
    assert!(!looks_like_dump(b"<html>not a dump</html>"));
}

/// Against the running managed realm (with SOAP on), using the manager.json at
/// WRATHPLUS_LIVE_MANAGER_JSON: export WRATHPLUS_LIVE_EXPORT, import it into
/// WRATHPLUS_LIVE_ACCOUNT as WRATHPLUS_LIVE_RENAME. Run by hand; it adds a character.
#[test]
#[ignore = "needs the managed realm running"]
fn live_export_then_import_a_renamed_copy() {
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
    let config =
        crate::setup::load_config(std::path::Path::new(&var("WRATHPLUS_LIVE_MANAGER_JSON")))
            .expect("manager config");
    let before = roster().expect("roster");
    eprintln!(
        "{} characters, accounts {:?}",
        before.characters.len(),
        before.accounts
    );
    let dir = std::env::temp_dir().join(format!("wrathplus-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("live.pdump");
    eprintln!(
        "{}",
        export(&config, &var("WRATHPLUS_LIVE_EXPORT"), &file).expect("export")
    );
    let rename = var("WRATHPLUS_LIVE_RENAME");
    eprintln!(
        "{}",
        import(
            &config,
            &file,
            &var("WRATHPLUS_LIVE_ACCOUNT"),
            Some(&rename)
        )
        .expect("import")
    );
    assert!(roster()
        .unwrap()
        .characters
        .iter()
        .any(|c| c.name == rename));
}
