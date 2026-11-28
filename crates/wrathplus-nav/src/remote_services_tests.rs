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
        "S 3 vendor,repair,trainer,flightmaster,innkeeper,banker,questgiver-hub,trainer_cooking,trainer_first_aid,trainer_profession,auctioneer,mailbox,portal"
    );
    assert!(all.len() <= crate::remote::MAX_PAYLOAD);
    assert_eq!(parsed(&all), "services 3 8191 0.000");
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
        "31 0 1 4 0 0 0x80 0 0 99 2 0 0 Renegade".to_string(),
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
    over(&spawns, query)
}

/// The module's answer for a query over a spawn table, read back by the client.
fn over(spawns: &str, query: &str) -> Vec<Place> {
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
        collect.push(line("s 2 1 3 vendor 1 10.0 0.0 0.0 1 - - - - - - - A")),
        Ok(None)
    );
    assert!(collect
        .push(line("s 2 3 3 vendor 1 10.0 0.0 0.0 1 - - - - - - - C"))
        .unwrap_err()
        .contains("came apart"));
    assert_eq!(collect.push(line("s 2 0 0")), Ok(Some(Vec::new())));
    for bad in [
        "s 2 1 1 armorer 1 1 2 3 1 - - - - - - - X",
        "s 2 2 1 vendor 1 1 2 3 1 - - - - - - - X",
        "s 2 1 1 vendor 1 1 nan 3 1 - - - - - - - X",
        // The event field is "-", "on" or "off"; an older realm's line has a name word there.
        "s 2 1 1 vendor 1 1 2 3 1 - maybe - - - - - X",
        "s 2 1 1 vendor 1 1 2 3 1 - Brother_Danil",
        "s 2 1 1 vendor 1 1 2 3 1 -",
        // The house field is "-", "alliance", "horde" or "neutral"; the last realm's line (before
        // auctioneers) has a name word there, or nothing.
        "s 2 1 1 auctioneer 1 1 2 3 1 - - goblin - - - - X",
        "s 2 1 1 vendor 1 1 2 3 1 - - Brother_Danil",
        "s 2 1 1 vendor 1 1 2 3 1 - -",
        // The portal fields (area, to, level, needs) are "-" for any other kind and read for a
        // portal; the last realm's line (before portals) has the name there.
        "s 2 1 1 vendor 1 1 2 3 1 - - - Brother Danil",
        "s 2 1 1 mailbox 1 1 2 3 1 - - - Mailbox",
        "s 2 1 1 vendor 1 1 2 3 1 - - - 10.0 - - - X",
        "s 2 1 1 portal 2173 1 2 3 1 - - - - - - - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - ten 369,68.3,2490.9,-4.3 - - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 10.0 369,68.3,2490.9 - - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 10.0 369,68.3,2490.9,-4.3,1 - - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 1x2@0 369,68.3,2490.9,-4.3 - - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 10.0 369,68.3,2490.9,-4.3 low - Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 10.0 369,68.3,2490.9,-4.3 - raid,, Deeprun Tram",
        "s 2 1 1 portal 2173 1 2 3 1 - - - 10.0 369,68.3,2490.9,-4.3 -",
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

#[test]
fn event_spawns_are_told_with_whether_their_event_has_them_out() {
    // Anvilmar: the real mage trainer, a generic "Mage Trainer" of event 7, one of a negative
    // event (out while event 8 is not), and one of two events.
    let spawns = [
        format!("944 0 1 30 0 0 0x33 8 0 {ALLIANCE} Marryk_Nurribit"),
        format!("26326 0 1 10 0 0 0x33 8 0 {ALLIANCE} @7 Mage_Trainer"),
        format!("26327 0 1 20 0 0 0x80 0 0 {ALLIANCE} @-8 Vendor_Off_Season"),
        format!("26328 0 1 25 0 0 0x80 0 0 {ALLIANCE} @7,9 Two_Events"),
    ]
    .join("\n");
    let told = |active: &str| {
        over(
            &spawns,
            &format!("trainer,vendor 0 1 0 0 2000 8 20 {HUMAN} {active}"),
        )
        .into_iter()
        .map(|p| format!("{} {:?}", p.entry, p.event))
        .collect::<Vec<_>>()
    };
    // Nothing on: the event's trainer is off but still told, nearest first like any other.
    assert_eq!(
        told(""),
        [
            "26326 Some(false)",
            "26327 Some(true)",
            "26328 Some(false)",
            "944 None"
        ]
    );
    assert_eq!(
        told("@7,8"),
        [
            "26326 Some(true)",
            "26327 Some(false)",
            "26328 Some(true)",
            "944 None"
        ]
    );
    assert_eq!(told("@9")[2], "26328 Some(true)");
    // On the wire: "-", "on" or "off" before the name.
    let Ok(Reply::Service(line)) =
        decode("s 3 1 1 trainer 26326 10.0 0.0 0.0 1 - off - - - - - Mage Trainer")
    else {
        panic!("decodes");
    };
    let place = line.place.unwrap();
    assert_eq!(
        (place.event, place.name.as_str()),
        (Some(false), "Mage Trainer")
    );
}

/// A Horde player (orc): Horde and player, hostile to monsters.
const ORC: &str = "2 5 4 8";

#[test]
fn auctioneers_are_told_with_the_house_they_serve() {
    // Stormwind's and Orgrimmar's auctioneers, Booty Bay's (goblins: anyone's), and a vendor that
    // is an auctioneer too.
    let spawns = [
        format!("8719 0 1 10 0 0 0x200000 0 0 {ALLIANCE} +alliance Auctioneer_Fitch"),
        format!("8673 0 1 5 0 0 0x200000 0 0 {HORDE} +horde Auctioneer_Thathung"),
        format!("2622 0 1 20 0 0 0x200000 0 0 {GOBLIN} +neutral Auctioneer_Kresky"),
        format!("9999 0 1 30 0 0 0x200080 0 0 {ALLIANCE} +alliance Trader"),
    ]
    .join("\n");
    let told = |me: &str| {
        over(
            &spawns,
            &format!("auctioneer,vendor 0 1 0 0 2000 1 20 {me}"),
        )
        .into_iter()
        .map(|p| format!("{} {} {:?}", p.kind, p.entry, p.house))
        .collect::<Vec<_>>()
    };
    // The other side's house is left out, as any hostile spawn is; the neutral one is told as
    // neutral, and only an auctioneer place carries a house.
    assert_eq!(
        told(HUMAN),
        [
            "auctioneer 8719 Some(\"alliance\")",
            "auctioneer 2622 Some(\"neutral\")",
            "vendor 9999 None",
            "auctioneer 9999 Some(\"alliance\")",
        ]
    );
    assert_eq!(
        told(ORC),
        [
            "auctioneer 8673 Some(\"horde\")",
            "auctioneer 2622 Some(\"neutral\")",
        ]
    );
    // On the wire: the house before the name.
    let Ok(Reply::Service(line)) =
        decode("s 3 1 1 auctioneer 2622 20.0 0.0 0.0 1 - - neutral - - - - Auctioneer Kresky")
    else {
        panic!("decodes");
    };
    let place = line.place.unwrap();
    assert_eq!(
        (place.house.as_deref(), place.name.as_str()),
        (Some("neutral"), "Auctioneer Kresky")
    );
}

#[test]
fn mailboxes_are_gameobjects_told_to_either_side_with_their_events() {
    // Two standing mailboxes, one a game event puts out (a fair's), and an innkeeper beside them.
    let spawns = [
        "143981 0 1 15 0 0 0 0 0 0 0 0 0 +mailbox Mailbox".to_string(),
        "32349 0 1 2500 0 0 0 0 0 0 0 0 0 +mailbox Far_Mailbox".to_string(),
        "187113 0 1 8 0 0 0 0 0 0 0 0 0 +mailbox @110 Fair_Mailbox".to_string(),
        format!("6740 0 1 12 0 0 0x10000 0 0 {ALLIANCE} Innkeeper_Allison"),
    ]
    .join("\n");
    let told = |kinds: &str, me: &str, active: &str| {
        over(&spawns, &format!("{kinds} 0 1 0 0 2000 1 20 {me} {active}"))
            .into_iter()
            .map(|p| format!("{} {} {:?} {:?}", p.kind, p.entry, p.event, p.house))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        told("mailbox", HUMAN, ""),
        [
            "mailbox 187113 Some(false) None",
            "mailbox 143981 None None"
        ]
    );
    // A mailbox has no faction: the Horde is told the same ones, the fair's out while it runs.
    assert_eq!(
        told("mailbox", ORC, "@110"),
        ["mailbox 187113 Some(true) None", "mailbox 143981 None None"]
    );
    // Asked for, or not at all; and an NPC is never a mailbox.
    assert_eq!(told("innkeeper", HUMAN, ""), ["innkeeper 6740 None None"]);
    assert_eq!(told("mailbox,innkeeper", HUMAN, "").len(), 3);
}

#[test]
fn the_nearest_class_trainers_are_told_beyond_the_radius_and_the_cap() {
    // Loch Modan: a mage with no mage trainer within 2000 yards; Ironforge's beyond it, one of
    // them a game event's, and the Horde's nearer still.
    let ironforge = [
        format!("5144 0 1 2500 0 0 0x33 8 0 {ALLIANCE} Bink"),
        format!("5145 0 1 2600 0 0 0x33 8 0 {ALLIANCE} Juli"),
        format!("5146 0 1 2700 0 0 0x33 8 0 {ALLIANCE} Nittlebur"),
        format!("26326 0 1 2400 0 0 0x33 8 0 {ALLIANCE} @7 Mage_Trainer"),
        format!("5880 0 1 2300 0 0 0x33 8 0 {HORDE} Un'Thuwa"),
        format!("1901 0 1 2200 0 0 0x33 1 0 {ALLIANCE} Kelstrum"),
        format!("1300 1 1 10 0 0 0x33 8 0 {ALLIANCE} Elsewhere"),
        format!("1699 0 1 2100 0 0 0x80 0 0 {ALLIANCE} Far_Vendor"),
        format!("5159 0 1 3000 0 0 0x50 0 185:Cooking {ALLIANCE} Daryl"),
        format!("1 0 1 10 0 0 0x80 0 0 {ALLIANCE} Vendor_A"),
        format!("2 0 1 20 0 0 0x80 0 0 {ALLIANCE} Vendor_B"),
        format!("3 0 1 30 0 0 0x80 0 0 {ALLIANCE} Vendor_C"),
    ]
    .join("\n");
    let told = |spawns: &str, kinds: &str, cap: u32, active: &str| {
        over(
            spawns,
            &format!("{kinds} 0 1 0 0 2000 8 {cap} {HUMAN} {active}"),
        )
        .into_iter()
        .map(|p| format!("{} {}", p.kind, p.entry))
        .collect::<Vec<_>>()
    };
    // The nearest two of the mage's own on the map: not the Horde's, the warrior's, another
    // map's, nor one whose event has it off.
    assert_eq!(
        told(&ironforge, "trainer", 20, ""),
        ["trainer 5144", "trainer 5145"]
    );
    assert_eq!(
        told(&ironforge, "trainer", 20, "@7"),
        ["trainer 26326", "trainer 5144"]
    );
    // Only trainers come from beyond the radius, and the cap makes room for them.
    assert_eq!(
        told(&ironforge, "trainer,vendor", 3, ""),
        ["vendor 1", "trainer 5144", "trainer 5145"]
    );
    assert_eq!(
        told(&ironforge, "vendor", 20, ""),
        ["vendor 1", "vendor 2", "vendor 3"]
    );
    // Cooking (and First Aid) the same; trainer_profession is not.
    assert_eq!(
        told(&ironforge, "trainer_cooking", 20, ""),
        ["trainer_cooking 5159"]
    );
    assert!(told(&ironforge, "trainer_profession", 20, "").is_empty());
    // A trainer within the radius counts, one whose event has it off does not (still told).
    let thelsamar = [
        ironforge.clone(),
        format!("944 0 1 100 0 0 0x33 8 0 {ALLIANCE} Near"),
        format!("26327 0 1 50 0 0 0x33 8 0 {ALLIANCE} @7 Near_Event"),
    ]
    .join("\n");
    assert_eq!(
        told(&thelsamar, "trainer", 20, ""),
        ["trainer 26327", "trainer 944", "trainer 5144"]
    );
    // On the wire nothing new: a far trainer is a place like any other.
    let far = over(&ironforge, &format!("trainer 0 1 0 0 2000 8 20 {HUMAN}"));
    assert_eq!((far[0].pos[0], far[0].name.as_str()), (2500.0, "Bink"));
}

/// Every phase: an area trigger has none.
const ALL_PHASES: &str = "4294967295";

#[test]
fn portals_are_told_with_their_volume_destination_and_requirement() {
    // The Deeprun Tram's entrances (AzerothCore's areatrigger and areatrigger_teleport rows):
    // Ironforge's in Tinker Town and Stormwind's, 3500 yards apart on map 0; the tram's own ways
    // out (map 369, one a box); then instance doors with access requirements: Onyxia's Lair (a
    // raid), the Black Morass (a quest, and a key on heroic) and the Pit of Saron (a quest per
    // side), and a vendor beside the Ironforge entrance.
    let spawns = [
        format!("2175 0 {ALL_PHASES} -4840.26 -1330.46 508.172 0 0 0 0 0 0 0 +sphere=10 +to=369,69.2542,10.257,-4.29664 Deeprun_Tram"),
        format!("2173 0 {ALL_PHASES} -8346.46 514.031 96.5989 0 0 0 0 0 0 0 +sphere=10 +to=369,68.3006,2490.91,-4.29647 Deeprun_Tram"),
        format!("2166 369 {ALL_PHASES} 76.027 10.5043 -4.29659 0 0 0 0 0 0 0 +box=9.417,19,20.64,0 +to=0,-4838.95,-1318.46,501.868 Eastern_Kingdoms"),
        format!("2848 0 {ALL_PHASES} -4800 -1300 500 0 0 0 0 0 0 0 +sphere=5 +to=249,29.1,-71.1,-8.2 +raidmap +raidgroup +access=0,80,0,0 +access=1,80,0,0 Onyxia's_Lair"),
        format!("4321 0 {ALL_PHASES} -4810 -1300 500 0 0 0 0 0 0 0 +sphere=5 +to=269,-1496.7,7033.5,32.3 +access=0,66,0,0 +need=0,1,10285,2 +access=1,70,0,0 +need=1,1,10285,2 +need=1,2,30635,2 Black_Morass"),
        format!("5633 0 {ALL_PHASES} -4820 -1300 500 0 0 0 0 0 0 0 +sphere=5 +to=658,435.7,212.4,528.7 +access=0,0,0,200 +need=0,1,24499,0 +need=0,1,24511,1 Pit_of_Saron"),
        format!("5175 0 1 -4835 -1310 508 0x80 0 0 {ALLIANCE} Gnome_Vendor"),
    ]
    .join("\n");
    let told = |query: &str| over(&spawns, query);
    let tinker_town = |kinds: &str, marks: &str| {
        told(&format!(
            "{kinds} 0 1 -4830 -1300 2000 1 20 {HUMAN} {marks}"
        ))
    };
    // Only on the asker's map, within the radius, nearest first; asked for, or not at all.
    let near = tinker_town("portal", "+team=0");
    let entries: Vec<u32> = near.iter().map(|p| p.entry).collect();
    assert_eq!(entries, [5633, 4321, 2848, 2175]);
    assert_eq!(
        tinker_town("vendor", "")
            .iter()
            .map(|p| p.kind.as_str())
            .collect::<Vec<_>>(),
        ["vendor"]
    );
    assert!(tinker_town("vendor", "")[0].portal.is_none());
    // The tram: a 10 yard sphere, to map 369 and where on it, named for the map.
    let tram = &near[3];
    assert_eq!(
        (tram.kind.as_str(), tram.name.as_str()),
        ("portal", "Deeprun Tram")
    );
    assert_eq!(tram.pos, [-4840.3, -1330.5, 508.2]);
    let portal = tram.portal.as_ref().unwrap();
    assert_eq!(portal.area, Area::Sphere { radius: 10.0 });
    assert_eq!((portal.to_map, portal.to), (369, [69.3, 10.3, -4.3]));
    assert_eq!((portal.level_min, portal.level_max), (None, None));
    assert!(portal.needs.is_empty());
    // A raid: its level and a raid group, told and not filtered (the asker is no level 80).
    let onyxia = near[2].portal.as_ref().unwrap();
    assert_eq!(
        (onyxia.level_min, onyxia.needs.as_slice()),
        (Some(80), ["raid".to_string()].as_slice())
    );
    // At the asker's difficulty: the Black Morass's normal and heroic requirements.
    let morass = |marks: &str| {
        let place = tinker_town("portal", marks).remove(1);
        let p = place.portal.unwrap();
        (p.level_min, p.needs)
    };
    assert_eq!(morass(""), (Some(66), vec!["quest:10285".to_string()]));
    assert_eq!(
        morass("+dungeon=1"),
        (
            Some(70),
            vec!["quest:10285".to_string(), "item:30635".to_string()]
        )
    );
    // The raid difficulty is for raids, not dungeons.
    assert_eq!(morass("+raid=1").0, Some(66));
    // Each side is told its own quest; an average item level is a need too.
    let saron = |team: &str| tinker_town("portal", team).remove(0).portal.unwrap().needs;
    assert_eq!(saron("+team=0"), ["quest:24499", "item_level:200"]);
    assert_eq!(saron("+team=1"), ["quest:24511", "item_level:200"]);
    // Inside the tram: its ways out, a box among them.
    let inside = told(&format!("portal 369 1 70 10 2000 1 20 {HUMAN}"));
    let way_out = inside[0].portal.as_ref().unwrap();
    assert_eq!(
        (inside.len(), way_out.area, way_out.to_map),
        (
            1,
            Area::Box {
                size: [9.4, 19.0, 20.6],
                yaw: 0.0
            },
            0
        )
    );
    // On the wire: the four fields before the name.
    let Ok(Reply::Service(line)) = decode(
        "s 3 1 1 portal 4321 -4810.0 -1300.0 500.0 1 - - - 5.0 269,-1496.7,7033.5,32.3 70-72 quest:10285,item:30635 Caverns of Time",
    ) else {
        panic!("decodes");
    };
    let place = line.place.unwrap();
    let p = place.portal.unwrap();
    assert_eq!(
        (place.name.as_str(), p.level_min, p.level_max, p.needs.len()),
        ("Caverns of Time", Some(70), Some(72), 2)
    );
    let Ok(Reply::Service(line)) =
        decode("s 3 1 1 portal 1 0.0 0.0 0.0 1 - - - 5.0 1,0.0,0.0,0.0 -60 - X")
    else {
        panic!("decodes");
    };
    let p = line.place.unwrap().portal.unwrap();
    assert_eq!((p.level_min, p.level_max), (None, Some(60)));
}
