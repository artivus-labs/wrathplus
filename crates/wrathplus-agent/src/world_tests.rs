//! Tests for `world`: the snapshot's Lua view, orders, and NPC services.

use super::*;

#[test]
fn guids_round_trip_as_hex_text() {
    assert_eq!(guid_text(0xF130_0000_4E00_079A), "0xF13000004E00079A");
    assert_eq!(
        parse_guid("0xF13000004E00079A"),
        Some(0xF130_0000_4E00_079A)
    );
    assert_eq!(parse_guid("F130"), None);
    assert_eq!(parse_guid("0xZZ"), None);
}

#[test]
fn quest_status_bytes_read_as_markers() {
    assert_eq!(quest_marker(8), Some("available"));
    assert_eq!(quest_marker(5), Some("incomplete"));
    assert_eq!(quest_marker(10), Some("reward"));
    assert_eq!(quest_marker(0), None);
}

#[test]
fn lua_reads_the_snapshot_and_sends_orders() {
    let (handles, link) = link();
    {
        let mut s = handles.snapshot.lock().unwrap();
        s.me = Some(Me {
            pos: [1.0, 2.0, 3.0],
            facing: 1.5,
            map: 0,
        });
        s.objects = vec![
            Seen {
                guid: 0x10,
                kind: "unit",
                name: Some("Deputy Willem".into()),
                distance: 12.0,
                npc: vec!["questgiver"],
                quest: Some("available"),
                ..Default::default()
            },
            Seen {
                guid: 0x20,
                kind: "unit",
                distance: 80.0,
                ..Default::default()
            },
        ];
    }
    let lua = Lua::new();
    install(&lua, &handles).unwrap();
    let (name, quest, count_near, facing): (String, String, usize, f32) = lua
        .load(
            "local n = AgentNearby() local near = AgentNearby(50) \
             return n[1].name, n[1].quest, #near, AgentPosition().facing",
        )
        .eval()
        .unwrap();
    assert_eq!(
        (name.as_str(), quest.as_str(), count_near),
        ("Deputy Willem", "available", 1)
    );
    assert_eq!(facing, 1.5);
    let sent: bool = lua
        .load("return AgentInteract('0x0000000000000010')")
        .eval()
        .unwrap();
    assert!(sent);
    assert!(matches!(link.orders.try_recv(), Ok(Order::Interact(0x10))));
    assert!(lua.load("AgentTarget('Deputy')").exec().is_err());
}

#[test]
fn npc_services_come_from_the_flags_and_find_the_nearest_provider() {
    // Goldshire, from AzerothCore's creature_template: Smith Argus (npcflag 83) trains
    // blacksmithing and does not repair; Gunther Weller (4225) sells weapons and repairs.
    assert_eq!(
        services(83),
        ["gossip", "questgiver", "trainer", "professiontrainer"]
    );
    assert_eq!(services(4225), ["gossip", "vendor", "repair"]);
    let (handles, _link) = link();
    handles.snapshot.lock().unwrap().objects = vec![
        Seen {
            guid: 0x1,
            kind: "unit",
            name: Some("Smith Argus".into()),
            distance: 5.0,
            npc: services(83),
            title: Some("Blacksmithing Trainer".into()),
            ..Default::default()
        },
        Seen {
            guid: 0x2,
            kind: "unit",
            name: Some("Gunther Weller".into()),
            distance: 40.0,
            npc: services(4225),
            ..Default::default()
        },
    ];
    let lua = Lua::new();
    install(&lua, &handles).unwrap();
    let (repair, trainer, title, near): (String, String, String, bool) = lua
        .load(
            "return AgentFindNpc('repair').name, AgentFindNpc('trainer').name, \
             AgentNearby()[1].title, AgentFindNpc('repair', 20) == nil",
        )
        .eval()
        .unwrap();
    assert_eq!(
        (repair.as_str(), trainer.as_str(), title.as_str(), near),
        (
            "Gunther Weller",
            "Smith Argus",
            "Blacksmithing Trainer",
            true
        )
    );
    let error = lua.load("AgentFindNpc('blacksmith')").exec().unwrap_err();
    assert!(
        error.to_string().contains("one of gossip, questgiver"),
        "{error}"
    );
}

#[test]
fn creature_ranks_and_critters_reach_lua() {
    // AzerothCore's CreatureEliteType: normal, elite, rare elite, boss, rare.
    assert_eq!(RANK_WORDS[1], "elite");
    assert_eq!(RANK_WORDS[4], "rare");
    let (handles, _link) = link();
    handles.snapshot.lock().unwrap().objects = vec![
        Seen {
            guid: 0x1,
            kind: "unit",
            name: Some("Hogger".into()),
            classification: Some("elite"),
            distance: 5.0,
            ..Default::default()
        },
        Seen {
            guid: 0x2,
            kind: "unit",
            name: Some("Sheep".into()),
            classification: Some("normal"),
            critter: true,
            distance: 9.0,
            ..Default::default()
        },
        // A template not answered yet: no word, not "normal".
        Seen {
            guid: 0x3,
            kind: "unit",
            distance: 12.0,
            ..Default::default()
        },
    ];
    let lua = Lua::new();
    install(&lua, &handles).unwrap();
    let read: String = lua
        .load(
            "local n = AgentNearby() return n[1].classification .. ' ' .. tostring(n[1].critter) .. ' ' \
             .. n[2].classification .. ' ' .. tostring(n[2].critter) .. ' ' .. tostring(n[3].classification)",
        )
        .eval()
        .unwrap();
    assert_eq!(read, "elite nil normal true nil");
}

/// Under a cinematic the list is what the server's camera sees: `AgentNearby` says so in a second
/// value, which `ipairs(AgentNearby())` and `#AgentNearby()` never see.
#[test]
fn nearby_says_when_a_cinematic_is_why_it_is_empty() {
    let (handles, _link) = link();
    let lua = Lua::new();
    install(&lua, &handles).unwrap();
    let quiet: (usize, Option<String>) = lua
        .load("local list, note = AgentNearby(50) return #list, note")
        .eval()
        .unwrap();
    assert_eq!(quiet, (0, None));
    assert!(lua
        .load("return AgentCinematic() == nil")
        .eval::<bool>()
        .unwrap());
    handles.snapshot.lock().unwrap().cinematic = true;
    let held: String = lua.load("return AgentCinematic()").eval().unwrap();
    assert_eq!(held, CINEMATIC_NOTE);
    let (count, note, walked): (usize, String, usize) = lua
        .load(
            "local list, note = AgentNearby(50) local n = 0 \
             for _ in ipairs(AgentNearby()) do n = n + 1 end return #list, note, n",
        )
        .eval()
        .unwrap();
    assert_eq!((count, walked), (0, 0));
    assert_eq!(note, CINEMATIC_NOTE);
    assert!(note.contains("StopCinematic()"));
}

/// A creature's type reaches Lua, so an agent can tell a critter (no XP) from a beast.
#[test]
fn a_units_creature_type_is_in_its_table() {
    let (handles, _link) = link();
    handles.snapshot.lock().unwrap().objects = vec![
        Seen {
            guid: 0x1,
            kind: "unit",
            name: Some("Rabbit".into()),
            creature_type: Some("Critter"),
            ..Default::default()
        },
        Seen {
            guid: 0x2,
            kind: "unit",
            name: Some("Unknown yet".into()),
            ..Default::default()
        },
    ];
    let lua = Lua::new();
    install(&lua, &handles).unwrap();
    let (critter, unknown): (String, bool) = lua
        .load("local n = AgentNearby() return n[1].creature_type, n[2].creature_type == nil")
        .eval()
        .unwrap();
    assert_eq!((critter.as_str(), unknown), ("Critter", true));
}
