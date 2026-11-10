//! `AgentServices` end to end without a server: asked of the realm, read back on the next call,
//! with the realm played by handing its lines to `pump`.

use super::rig::Rig;
use mlua::Table;

/// `AgentServices(...)`'s answer: `pending`, the places as "kind name zone", or the error.
fn services(rig: &Rig, call: &str) -> Result<Option<Vec<String>>, String> {
    let (answer, why): (Option<Table>, Option<String>) =
        rig.lua.load(format!("return {call}")).eval().unwrap();
    let Some(answer) = answer else {
        return Err(why.unwrap());
    };
    if answer.get::<Option<bool>>("pending").unwrap() == Some(true) {
        return Ok(None);
    }
    Ok(Some(
        answer
            .sequence_values::<Table>()
            .map(|p| {
                let p = p.unwrap();
                let get = |k: &str| p.get::<Option<String>>(k).unwrap().unwrap_or("-".into());
                format!(
                    "{} {} {} {}",
                    get("kind"),
                    get("name"),
                    get("zone"),
                    get("profession")
                )
            })
            .collect(),
    ))
}

#[test]
fn agent_services_asks_the_realm_once_and_the_next_call_reads_the_answer() {
    let rig = Rig::in_world();
    let call = "AgentServices({ 'vendor', 'trainer_first_aid' }, 800)";
    assert_eq!(services(&rig, call), Ok(None));
    assert_eq!(rig.realm(&[]), ["S 1 trainer_first_aid,vendor 800"]);
    // Asked again while the answer is on its way: still the one question, written either way.
    assert_eq!(
        services(&rig, "AgentServices('vendor,trainer_first_aid', 800)"),
        Ok(None)
    );
    rig.later(1.0);
    assert!(rig.realm(&[]).is_empty());
    rig.realm(&[
        "s 1 1 2 vendor 1 110.0 200.0 30.0 1 - Brother Danil",
        "s 1 2 2 trainer_first_aid 41 120.0 200.0 30.0 1 First_Aid Medic",
    ]);
    assert_eq!(
        rig.events().last().unwrap(),
        "AGENT_SERVICES_READY [true,2]"
    );
    assert_eq!(
        services(&rig, call),
        Ok(Some(vec![
            "vendor Brother Danil - -".to_string(),
            "trainer_first_aid Medic - First Aid".to_string()
        ]))
    );
    // Unknown kinds are refused before anything is sent.
    assert!(services(&rig, "AgentServices({ 'armorer' })")
        .unwrap_err()
        .contains("unknown service kind \"armorer\""));
}

#[test]
fn agent_services_says_when_the_realm_cannot_answer() {
    let rig = Rig::new();
    assert!(services(&rig, "AgentServices()")
        .unwrap_err()
        .contains("once in the world"));
    assert_eq!(rig.realm(&[]), ["H 1"]);
    // Asked while the probe is out, then the realm turns out to have no module.
    assert_eq!(services(&rig, "AgentServices()"), Ok(None));
    rig.realm(&["H 1"]);
    let why = services(&rig, "AgentServices()").unwrap_err();
    assert!(why.contains("mod-agent-nav"), "{why}");
    assert!(rig
        .events()
        .last()
        .unwrap()
        .starts_with("AGENT_SERVICES_READY [false,"));
    // A realm that answers, then refuses for the rate.
    let rig = Rig::in_world();
    assert_eq!(services(&rig, "AgentServices()"), Ok(None));
    rig.realm(&[]);
    rig.realm(&["e 1 rate 5 a second"]);
    assert!(services(&rig, "AgentServices()")
        .unwrap_err()
        .contains("too many requests"));
}
