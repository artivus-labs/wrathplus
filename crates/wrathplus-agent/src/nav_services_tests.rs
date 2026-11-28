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
        "s 1 1 2 vendor 1 110.0 200.0 30.0 1 - - - - - - - Brother Danil",
        "s 1 2 2 trainer_first_aid 41 120.0 200.0 30.0 1 First_Aid - - - - - - Medic",
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

/// Each place carries its distance on the ground from where the character stands, and the list is
/// nearest first from there: after a walk within the answer's lifetime the order follows the
/// character, not the spot the realm was asked from.
#[test]
fn places_come_nearest_first_with_their_distance_from_where_you_stand() {
    let rig = Rig::in_world();
    let call = "AgentServices('vendor')";
    assert_eq!(services(&rig, call), Ok(None));
    rig.realm(&[]);
    rig.realm(&[
        "s 1 1 2 vendor 1 110.0 200.0 30.0 1 - - - - - - - Brother Danil",
        "s 1 2 2 vendor 2 70.0 240.0 30.0 1 - - - - - - - Janos Hammerknuckle",
    ]);
    let read = |rig: &Rig| -> Vec<(String, f64)> {
        let list: Table = rig.lua.load(format!("return {call}")).eval().unwrap();
        list.sequence_values::<Table>()
            .map(|p| {
                let p = p.unwrap();
                (p.get("name").unwrap(), p.get("distance").unwrap())
            })
            .collect()
    };
    assert_eq!(
        read(&rig),
        [
            ("Brother Danil".to_string(), 10.0),
            ("Janos Hammerknuckle".to_string(), 50.0)
        ]
    );
    // Thirty yards on, the same answer, nearest first from here.
    rig.world.snapshot.lock().unwrap().me.as_mut().unwrap().pos = [80.0, 230.0, 30.0];
    assert_eq!(
        read(&rig),
        [
            ("Janos Hammerknuckle".to_string(), 14.1),
            ("Brother Danil".to_string(), 42.4)
        ]
    );
}

/// A spawn a game event brings out comes with `event = true` and whether its event had it out;
/// a standing one with `event = false` and no `event_active`.
#[test]
fn event_spawns_carry_event_and_event_active() {
    let rig = Rig::in_world();
    let call = "AgentServices('trainer')";
    assert_eq!(services(&rig, call), Ok(None));
    rig.realm(&[]);
    rig.realm(&[
        "s 1 1 3 trainer 26326 110.0 200.0 30.0 1 - off - - - - - Mage Trainer",
        "s 1 2 3 trainer 26330 115.0 200.0 30.0 1 - on - - - - - Paladin Trainer",
        "s 1 3 3 trainer 944 120.0 200.0 30.0 1 - - - - - - - Marryk Nurribit",
    ]);
    let fields: Vec<String> = rig
        .lua
        .load(format!(
            "local out = {{}} for _, p in ipairs({call}) do \
               out[#out + 1] = p.name .. ' ' .. tostring(p.event) .. ' ' .. tostring(p.event_active) \
             end return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        fields,
        [
            "Mage Trainer true false",
            "Paladin Trainer true true",
            "Marryk Nurribit false nil"
        ]
    );
}

/// An auctioneer comes with the house it serves; a mailbox (a game object) with none.
#[test]
fn auctioneers_carry_their_house_and_mailboxes_come_too() {
    let rig = Rig::in_world();
    let call = "AgentServices('auctioneer,mailbox')";
    assert_eq!(services(&rig, call), Ok(None));
    assert_eq!(rig.realm(&[]), ["S 1 auctioneer,mailbox"]);
    rig.realm(&[
        "s 1 1 3 mailbox 143981 105.0 200.0 30.0 1 - - - - - - - Mailbox",
        "s 1 2 3 auctioneer 8719 110.0 200.0 30.0 1 - - alliance - - - - Auctioneer Fitch",
        "s 1 3 3 auctioneer 2622 115.0 200.0 30.0 1 - - neutral - - - - Auctioneer Kresky",
    ]);
    let fields: Vec<String> = rig
        .lua
        .load(format!(
            "local out = {{}} for _, p in ipairs({call}) do \
               out[#out + 1] = p.kind .. ' ' .. p.entry .. ' ' .. tostring(p.house) \
             end return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        fields,
        [
            "mailbox 143981 nil",
            "auctioneer 8719 alliance",
            "auctioneer 2622 neutral"
        ]
    );
}

/// A portal comes with its volume, where it leads and what it asks; and a question of other kinds
/// asked meanwhile (wp.services' town while wp.notice asks for portals) goes out after it, each
/// answer kept for its own kinds.
#[test]
fn portals_carry_where_they_lead_and_two_questions_are_answered_apart() {
    let rig = Rig::in_world();
    let portals = "AgentServices('portal')";
    let vendors = "AgentServices('vendor')";
    assert_eq!(services(&rig, portals), Ok(None));
    assert_eq!(services(&rig, vendors), Ok(None));
    assert_eq!(rig.realm(&[]), ["S 1 portal"]);
    rig.realm(&[
        "s 1 1 2 portal 2175 110.0 200.0 30.0 1 - - - 10.0 369,69.3,10.3,-4.3 - - Deeprun Tram",
        "s 1 2 2 portal 2848 120.0 200.0 30.0 1 - - - 9.4x19.0x20.6@1.57 249,29.1,-71.1,-8.2 80 raid,item:30635 Onyxia's Lair",
    ]);
    rig.later(1.0);
    assert_eq!(rig.realm(&[]), ["S 2 vendor"]);
    rig.realm(&["s 2 1 1 vendor 1 110.0 200.0 30.0 1 - - - - - - - Brother Danil"]);
    assert_eq!(
        services(&rig, vendors),
        Ok(Some(vec!["vendor Brother Danil - -".to_string()]))
    );
    let fields: Vec<String> = rig
        .lua
        .load(format!(
            "local out = {{}} for _, p in ipairs({portals}) do \
               out[#out + 1] = table.concat({{ p.kind, p.entry, p.name, tostring(p.radius), \
                 p.box and (p.box.length .. 'x' .. p.box.width .. '@' .. p.box.yaw) or '-', \
                 p.to_map, p.to_x, tostring(p.level), \
                 p.needs and table.concat(p.needs, ',') or '-' }}, ' ') \
             end return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        fields,
        [
            "portal 2175 Deeprun Tram 10 - 369 69.300003051758 nil -",
            "portal 2848 Onyxia's Lair nil 9.3999996185303x19@1.5700000524521 249 29.10000038147 80 raid,item:30635"
        ]
    );
    // Asking for the vendors again does not unsettle the portals' answer.
    assert!(services(&rig, vendors).unwrap().is_some());
    assert!(services(&rig, portals).unwrap().is_some());
    assert!(rig.realm(&[]).is_empty());
}
