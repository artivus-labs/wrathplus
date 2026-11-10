//! The services wire both ways, and the module's selection (`AgentNavServices.h`, through
//! `csrc/realm_module_shim.cpp`) over a fixture spawn table.

use super::*;
use crate::remote::{decode, Reply};
use std::ffi::{c_char, c_int, CString};

extern "C" {
    fn realm_parse(payload: *const c_char, out: *mut c_char, out_len: c_int) -> c_int;
    fn realm_services(
        spawns: *const c_char,
        query: *const c_char,
        out: *mut c_char,
        out_len: c_int,
    ) -> c_int;
}

fn text(call: impl FnOnce(*mut c_char, c_int) -> c_int) -> String {
    let mut out = vec![0u8; 64 * 1024];
    let len = call(out.as_mut_ptr().cast(), out.len() as c_int);
    assert!(len >= 0 && (len as usize) < out.len());
    String::from_utf8(out[..len as usize].to_vec()).unwrap()
}

fn parsed(line: &str) -> String {
    let line = CString::new(line).unwrap();
    text(|out, len| unsafe { realm_parse(line.as_ptr(), out, len) })
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_module_reads_a_services_request_as_the_client_wrote_it() {
    let all = request(3, &[], None).unwrap();
    assert_eq!(
        all,
        "S 3 vendor,repair,trainer,flightmaster,innkeeper,banker,questgiver-hub,trainer_cooking,trainer_first_aid,trainer_profession"
    );
    assert!(all.len() <= crate::remote::MAX_PAYLOAD);
    assert_eq!(parsed(&all), "services 3 1023 0.000");
    let some = request(4, &words(&["repair", "trainer"]), Some(800)).unwrap();
    assert_eq!(some, "S 4 repair,trainer 800");
    assert_eq!(parsed(&some), "services 4 6 800.000");
    assert!(request(5, &words(&["armorer"]), None)
        .unwrap_err()
        .contains("unknown service kind \"armorer\""));
    for (line, why) in [
        ("S 4", "!fields 4"),
        ("S 4 armorer", "!kinds 4"),
        ("S 4 vendor,", "!kinds 4"),
        ("S 4 vendor 0", "!radius 4"),
        ("S 4 vendor -5", "!radius 4"),
        ("S 4 vendor 9 9", "!fields 4"),
    ] {
        assert_eq!(parsed(line), why, "{line:?}");
    }
    // A realm without the module delivers it back.
    assert_eq!(decode(&all), Ok(Reply::Echo));
}

/// Factions as `faction our friendly hostile [enemies]`: the player is human (Alliance, player).
const HUMAN: &str = "1 3 2 8";
const ALLIANCE: &str = "12 2 3 12";
const HORDE: &str = "29 4 5 10";
const GOBLIN: &str = "120 0 0 0";
const MONSTER: &str = "26 8 0 7";

fn fixture() -> String {
    let rows = [
        format!("1 0 1 10 0 0 0x80 0 0 {ALLIANCE} Brother_Danil"),
        format!("2 0 1 20 0 0 0x1080 0 0 {ALLIANCE} Godric"),
        format!("3 0 1 5 0 0 0x33 1 0 {ALLIANCE} Llane"),
        format!("4 0 1 6 0 0 0x33 8 0 {ALLIANCE} Khelden"),
        format!("5 0 1 7 0 0 0x50 0 0 {ALLIANCE} Herbalist"),
        format!("6 0 1 3 0 0 0x80 0 0 {HORDE} Grunt_Vendor"),
        format!("7 0 1 30 0 0 0x80 0 0 {GOBLIN} Goblin"),
        format!("8 1 1 1 0 0 0x80 0 0 {ALLIANCE} Elsewhere"),
        format!("9 0 2 1 0 0 0x80 0 0 {ALLIANCE} Other_Phase"),
        format!("10 0 1 3000 0 0 0x80 0 0 {ALLIANCE} Far"),
        format!("11 0 1 40 0 0 0x10000 0 0 {ALLIANCE} Innkeeper"),
        format!("12 0 1 50 0 0 0x2000 0 0 {ALLIANCE} Flight"),
        format!("13 0 1 60 0 0 0x20000 0 0 {ALLIANCE} Banker"),
        format!("21 0 1 100 0 0 2 0 0 {ALLIANCE} Giver_A"),
        format!("22 0 1 110 0 0 2 0 0 {ALLIANCE} Giver_B"),
        format!("23 0 1 120 5 0 2 0 0 {ALLIANCE} Giver_C"),
        format!("24 0 1 300 0 0 2 0 0 {ALLIANCE} Lonely"),
        format!("25 0 1 500 0 0 2 0 0 {ALLIANCE} Pair_A"),
        format!("26 0 1 510 0 0 2 0 0 {ALLIANCE} Pair_B"),
        format!("30 0 1 2 0 0 0x80 0 0 {MONSTER} Kobold"),
        // Friendly by mask, but at war with the player's faction.
        format!("31 0 1 4 0 0 0x80 0 0 99 2 0 0 Renegade"),
        // Profession trainers, the profession read off their spells; a weapon master has none.
        format!("40 0 1 8 0 0 0x50 0 185:Cooking {ALLIANCE} Cook"),
        format!("41 0 1 9 0 0 0x50 0 129:First_Aid {ALLIANCE} Medic"),
        format!("42 0 1 11 0 0 0x50 0 164:Blacksmithing {ALLIANCE} Smith"),
        format!("43 0 1 12 0 0 0x10 0 0 {ALLIANCE} Weapon_Master"),
    ];
    rows.join("\n")
}

/// The module's answer for a query over the fixture, read back by the client.
fn lines(query: &str) -> Vec<Place> {
    places(query, false)
}

/// The module's answer for a query over the fixture, read back by the client: the places in order.
fn places(query: &str, enemies_of_31: bool) -> Vec<Place> {
    let mut spawns = fixture();
    if enemies_of_31 {
        spawns = spawns.replace("99 2 0 0 Renegade", "99 2 0 0 1 Renegade");
    }
    let (spawns, query) = (CString::new(spawns).unwrap(), CString::new(query).unwrap());
    let lines =
        text(|out, len| unsafe { realm_services(spawns.as_ptr(), query.as_ptr(), out, len) });
    let mut collect = Collect::default();
    let mut done = None;
    for line in lines.split('\n') {
        assert!(line.len() <= crate::remote::MAX_PAYLOAD);
        let Ok(Reply::Service(line)) = decode(line) else {
            panic!("{line}");
        };
        done = collect.push(line).unwrap();
    }
    done.expect("the last line completes the answer")
}

/// The same as `(kind, entry, name, members)`.
fn answer(query: &str, enemies_of_31: bool) -> Vec<(String, u32, String, u32)> {
    places(query, enemies_of_31)
        .into_iter()
        .map(|p| (p.kind, p.entry, p.name, p.members))
        .collect()
}

fn names(places: &[(String, u32, String, u32)]) -> Vec<String> {
    places
        .iter()
        .map(|(kind, entry, _, members)| format!("{kind} {entry} {members}"))
        .collect()
}

#[test]
fn the_realm_names_the_services_near_the_asker_nearest_first() {
    let warrior = format!("{} 0 1 0 0 2000 1 20 {HUMAN}", KINDS[..7].join(","));
    assert_eq!(
        names(&answer(&warrior, false)),
        [
            // Friendly by its masks, and at war with nobody here.
            "vendor 31 1",
            // The warrior's own class trainer, not the mage's nor a profession trainer.
            "trainer 3 1",
            "vendor 1 1",
            // One spawn, two services.
            "vendor 2 1",
            "repair 2 1",
            // Neutral goblins trade with anyone.
            "vendor 7 1",
            "innkeeper 11 1",
            "flightmaster 12 1",
            "banker 13 1",
            // Three quest givers within 40 yd make a hub; one alone or a pair do not.
            "questgiver-hub 21 3",
        ]
    );
    // Not the Horde's vendor, the kobold, another map or phase, nor one past the radius; and
    // not a faction whose enemies name the player's.
    let at_war = answer(&warrior, true);
    assert_eq!(at_war.len(), 9);
    assert!(at_war.iter().all(|p| p.1 != 31));
    // Names come back whole, spaces and all.
    assert_eq!(answer(&warrior, false)[2].2, "Brother Danil");
}

#[test]
fn the_realm_answers_only_the_kinds_asked_within_the_radius_and_the_cap() {
    let ask = |kinds: &str, radius: u32, class: u32, cap: u32| {
        names(&answer(
            &format!("{kinds} 0 1 0 0 {radius} {class} {cap} {HUMAN}"),
            false,
        ))
    };
    assert_eq!(ask("repair", 2000, 1, 20), ["repair 2 1"]);
    assert_eq!(ask("trainer", 2000, 8, 20), ["trainer 4 1"]);
    assert_eq!(ask("vendor", 15, 1, 20), ["vendor 31 1", "vendor 1 1"]);
    assert_eq!(ask("vendor", 4000, 1, 20).last().unwrap(), "vendor 10 1");
    assert_eq!(ask(&KINDS.join(","), 2000, 1, 3).len(), 3);
    // A weapon master (a tradeskill trainer with no profession) is never a profession trainer.
    assert!(ask("trainer_profession", 2000, 1, 20)
        .iter()
        .all(|p| !p.contains(" 43 ")));
    // Nothing near: one line saying so.
    assert!(ask("banker", 30, 1, 20).is_empty());
}

#[test]
fn answer_lines_out_of_turn_or_unreadable_are_errors() {
    let line = |text: &str| match decode(text) {
        Ok(Reply::Service(line)) => line,
        other => panic!("{text}: {other:?}"),
    };
    let mut collect = Collect::default();
    assert_eq!(
        collect.push(line("s 2 1 3 vendor 1 10.0 0.0 0.0 1 - A")),
        Ok(None)
    );
    assert!(collect
        .push(line("s 2 3 3 vendor 1 10.0 0.0 0.0 1 - C"))
        .unwrap_err()
        .contains("came apart"));
    assert_eq!(collect.push(line("s 2 0 0")), Ok(Some(Vec::new())));
    for bad in [
        "s 2 1 1 armorer 1 1 2 3 1 - X",
        "s 2 2 1 vendor 1 1 2 3 1 - X",
        "s 2 1 1 vendor 1 1 nan 3 1 - X",
        "s 2 1 1 vendor 1 1 2 3 1",
        "s 2 0 0 extra",
        "s 2 1 1 vendor",
    ] {
        assert!(decode(bad).is_err(), "{bad}");
    }
}

#[test]
fn profession_trainers_are_found_by_profession_and_named() {
    let ask = |kinds: &str| {
        let query = format!("{kinds} 0 1 0 0 2000 1 20 {HUMAN}");
        answer(&query, false)
            .into_iter()
            .map(|(kind, entry, _, _)| format!("{kind} {entry}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(ask("trainer_cooking"), ["trainer_cooking 40"]);
    assert_eq!(ask("trainer_first_aid"), ["trainer_first_aid 41"]);
    assert_eq!(
        ask("trainer_profession"),
        [
            "trainer_profession 40",
            "trainer_profession 41",
            "trainer_profession 42"
        ]
    );
    // The profession comes back by name, spaces and all; a class trainer has none.
    let all = lines(&format!(
        "trainer,trainer_profession 0 1 0 0 2000 1 20 {HUMAN}"
    ));
    let professions: Vec<Option<String>> = all.iter().map(|p| p.profession.clone()).collect();
    assert_eq!(
        professions,
        [
            None,
            Some("Cooking".to_string()),
            Some("First Aid".to_string()),
            Some("Blacksmithing".to_string())
        ]
    );
}
