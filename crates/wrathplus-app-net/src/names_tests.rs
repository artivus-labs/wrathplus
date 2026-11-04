use super::*;
use crossbeam_channel::TryRecvError;

/// Compose a guid the way the server does: `counter | (entry << 24) | (high << 48)`.
fn compose(high: u16, entry: u32, counter: u32) -> u64 {
    u64::from(counter) | (u64::from(entry) << 24) | (u64::from(high) << 48)
}

fn commands() -> (NetCommands, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (NetCommands(tx), rx)
}

#[test]
fn wrath_misses_ask_the_server_once() {
    let (cmds, rx) = commands();
    let cache = NameCache::default();
    let player = compose(guid::HIGH_PLAYER, 0, 7);
    let creature = compose(guid::HIGH_UNIT, 69, 1);

    assert_eq!(cache.resolve(player, &cmds), None);
    assert_eq!(cache.resolve(creature, &cmds), None);
    let asked: Vec<_> = rx.try_iter().collect();
    assert!(matches!(
        &asked[..],
        [
            ClientCommand::NameQuery { .. },
            ClientCommand::CreatureQuery { entry: 69, .. }
        ]
    ));
    assert!(cache.players.is_pending(player));
    assert!(cache.creatures.is_pending(69));
}

/// An ask must not move the counter, or an outstanding query would hold every gated feed open.
#[test]
fn the_generation_counts_landings_never_asks() {
    let (cmds, _rx) = commands();
    let mut cache = NameCache::default();
    let g0 = cache.generation();

    let player = compose(0, 0, 7);
    assert_eq!(cache.resolve(player, &cmds), None);
    assert_eq!(
        cache.generation(),
        g0,
        "the miss asked, and asking is not a landing"
    );

    cache.insert_player(player, "WrathPlus".into(), None);
    let landed = cache.generation();
    assert_ne!(g0, landed, "the answer landing is the edge");

    assert_eq!(cache.resolve(player, &cmds), Some("WrathPlus"));
    assert_eq!(cache.generation(), landed, "a cache hit moves nothing");

    cache.insert_pet(137, "Voidwalker".into());
    let pet = cache.generation();
    assert_ne!(landed, pet);
    cache.forget_pet(137);
    assert_ne!(
        pet,
        cache.generation(),
        "the rename eviction is a landing-shaped edge"
    );
}

/// A summoned pet asks `CMSG_PET_NAME_QUERY` by the pet number in its guid; its entry slot
/// names no creature template.
#[test]
fn a_pet_asks_the_pet_query_not_the_creature_query() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    let voidwalker = compose(guid::HIGH_PET, 137, 9);

    assert_eq!(cache.resolve(voidwalker, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::PetNameQuery {
            pet_number: 137,
            ..
        })
    ));
    assert_eq!(cache.resolve(voidwalker, &cmds), None);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    cache.insert_pet(137, "Voidwalker".into());
    assert_eq!(cache.resolve(voidwalker, &cmds), Some("Voidwalker"));
    assert_eq!(cache.peek(voidwalker), Some("Voidwalker"));
    // Pet names are per-spawn, so a disconnect drops them.
    cache.clear_pending();
    assert_eq!(cache.peek(voidwalker), None);
}

/// A stable list seeds the same `(pet_number, name)` pair the pet-name query answers, so an
/// unstabled pet is named on arrival: `PetStable.lua:163` stores `UnitName("pet")` unguarded
/// and `:178` hands it to `GameTooltip:SetText`, which raises on nil. vmangos packs the pet
/// number into the guid's entry slot (`Pet.cpp:2250`).
#[test]
fn a_stable_list_names_the_pet_before_it_is_ever_summoned() {
    let (cmds, rx) = commands();
    let cache = NameCache::default();
    let rex = compose(guid::HIGH_PET, 7, 42);

    assert_eq!(cache.resolve(rex, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::PetNameQuery { pet_number: 7, .. })
    ));

    let mut warm = NameCache::default();
    warm.insert_pet(7, "Rex".into());
    assert_eq!(warm.resolve(rex, &cmds), Some("Rex"));
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "a seeded pet must not re-ask"
    );
}

/// The negative answer survives too, so an unknown entry is not re-asked every login.
#[test]
fn the_cache_round_trips_through_its_file_format() {
    let mut cache = NameCache::default();
    cache.insert_creature(
        69,
        Some(CreatureRecord {
            name: "Stable Master Kitrik".into(),
            subname: Some("Stable Master".into()),
            creature_type: 7,
            pet_family: 0,
            rank: 1,
            type_flags: 0x10,
            civilian: true,
            racial_leader: false,
            display_id: 533,
            modifiers: None,
        }),
    );
    cache.insert_creature(1234, None); // the server flagged the entry unknown

    let text = cache.to_tsv("Hydraxian Waterlords");
    let back = NameCache::from_tsv(&text, "Hydraxian Waterlords").expect("header matches");

    // Present-and-None, not absent.
    assert!(back.creatures.answered(1234));
    let rec = back.creature_record(69).expect("creature survived");
    assert_eq!(rec.name, "Stable Master Kitrik");
    assert_eq!(rec.subname.as_deref(), Some("Stable Master"));
    assert_eq!((rec.creature_type, rec.rank, rec.type_flags), (7, 1, 0x10));
    assert_eq!((rec.civilian, rec.racial_leader), (true, false));
    assert_eq!(rec.display_id, 533, "the model a stabled pet is drawn from");
    assert!(back.creature_record(1234).is_none());
    assert_eq!(back.len(), 2);

    // `Some("")` would paint an empty tooltip line.
    cache.insert_creature(
        70,
        Some(CreatureRecord {
            name: "Boar".into(),
            subname: None,
            creature_type: 1,
            pet_family: 5,
            rank: 0,
            type_flags: 0,
            civilian: false,
            racial_leader: false,
            display_id: 1,
            modifiers: None,
        }),
    );
    let back = NameCache::from_tsv(&cache.to_tsv("R"), "R").expect("header");
    assert_eq!(back.creature_record(70).unwrap().subname, None);
}

/// The Wrath template modifiers survive the file bit-exactly; a line without them reads
/// `None`, never a fabricated 1.0.
#[test]
fn creature_modifiers_round_trip_through_the_file() {
    let mut cache = NameCache::default();
    let record = |modifiers| CreatureRecord {
        name: "Imp".into(),
        subname: None,
        creature_type: 3,
        pet_family: 23,
        rank: 0,
        type_flags: 0,
        civilian: false,
        racial_leader: false,
        display_id: 4449,
        modifiers,
    };
    cache.insert_creature(416, Some(record(Some([0.3, 1.1]))));
    cache.insert_creature(417, Some(record(None)));
    let back = NameCache::from_tsv(&cache.to_tsv("R"), "R").expect("header");
    assert_eq!(
        back.creature_record(416).unwrap().modifiers,
        Some([0.3, 1.1])
    );
    assert_eq!(back.creature_record(417).unwrap().modifiers, None);
}

/// The reference builds `'WNAM'` (ctor `0x554cd0`) and `'WPNM'` with persistence off; a 1.12
/// install's `WDB/` holds `creaturecache.wdb` and no `namecache.wdb`.
#[test]
fn the_file_keeps_creature_templates_and_no_identities() {
    let mut cache = NameCache::default();
    cache.insert_player(0x11, "Moamtester".into(), Some((1, 2, 0)));
    cache.insert_pet(7, "Rex".into());
    cache.insert_creature(
        69,
        Some(CreatureRecord {
            name: "Stable Master Kitrik".into(),
            subname: Some("Stable Master".into()),
            creature_type: 7,
            pet_family: 0,
            rank: 1,
            type_flags: 0x10,
            civilian: true,
            racial_leader: false,
            display_id: 533,
            modifiers: None,
        }),
    );

    let text = cache.to_tsv("R");
    assert!(
        !text.contains("Moamtester") && !text.contains("Rex"),
        "no guid- or spawn-keyed name may reach the disk:\n{text}"
    );
    let back = NameCache::from_tsv(&text, "R").expect("header matches");
    assert_eq!(
        back.peek(0x11),
        None,
        "a player name never survives the file"
    );
    assert_eq!(back.player_traits(0x11), None);
    assert_eq!(
        back.creature_record(69).map(|r| r.name.as_str()),
        Some("Stable Master Kitrik"),
        "the template entry is the key that keeps meaning what it meant",
    );
}

/// The reference's `0x555740` clears the stores whose `[cache+0x39]` byte is 0
/// (`0x55579f`-`0x5557ad` for the player cache at `0xc0e228`).
#[test]
fn world_entry_clears_the_identities_and_keeps_the_templates() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    cache.insert_player(0x11, "Moamtester".into(), Some((1, 2, 0)));
    cache.insert_pet(7, "Rex".into());
    cache.insert_creature(69, None);

    cache.clear_world_session();

    assert_eq!(cache.peek(0x11), None);
    assert_eq!(cache.player_traits(0x11), None);
    assert!(
        cache.creatures.answered(69),
        "templates outlive the world session"
    );
    // The in-flight marks went too.
    assert_eq!(cache.resolve(0x11, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::NameQuery { guid: 0x11 })
    ));
}

#[test]
fn a_name_replaced_without_traits_does_not_keep_the_old_ones() {
    let mut cache = NameCache::default();
    cache.insert_player(0x7, "Moamtester".into(), Some((1, 1, 0)));
    cache.insert_player(0x7, "Corances".into(), None);
    assert_eq!(cache.player_traits(0x7), None);
}

#[test]
fn a_header_that_differs_in_any_field_discards_the_whole_file() {
    let mut cache = NameCache::default();
    cache.insert_creature(1234, None);
    assert!(!cache.is_empty());
    let good = cache.to_tsv("Hydraxian Waterlords");
    assert!(NameCache::from_tsv(&good, "Hydraxian Waterlords").is_some());

    // The realm: our field, not the reference's.
    assert!(NameCache::from_tsv(&good, "Another Realm").is_none());

    let head = good.lines().next().unwrap();
    for (field, bad) in [
        (0, "not-wrathplus"),
        (1, "999"),  // format version
        (2, "5876"), // client build
        (3, "frFR"), // locale
    ] {
        let mut parts: Vec<&str> = head.split('\t').collect();
        parts[field] = bad;
        let text = format!("{}\n{}", parts.join("\t"), good.lines().nth(1).unwrap());
        assert!(
            NameCache::from_tsv(&text, "Hydraxian Waterlords").is_none(),
            "header field {field} = {bad} must discard the file"
        );
    }

    assert!(NameCache::from_tsv("", "R").is_none());
    assert!(NameCache::from_tsv("nonsense", "R").is_none());
    let text = format!("{}\nC\tnotanumber\t1\t2\t0\tX\nc\t1234\n", head);
    let back = NameCache::from_tsv(&text, "Hydraxian Waterlords").expect("header still good");
    assert_eq!(back.len(), 1, "the bad line is skipped, the good one kept");

    // A version-1 file, which held player names, is discarded whole.
    let v1 = format!(
        "{CACHE_MAGIC}\t1\t{CACHE_BUILD}\t{CACHE_LOCALE}\tHydraxian Waterlords\n\
         P\t7\t1\t1\t0\tMoamtester\n"
    );
    assert!(NameCache::from_tsv(&v1, "Hydraxian Waterlords").is_none());
}

#[test]
fn invalidating_a_player_makes_the_next_resolve_ask_again() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    cache.insert_player(0x11, "Aldric".into(), Some((1, 2, 0)));
    assert_eq!(cache.resolve(0x11, &cmds), Some("Aldric"));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)), "no ask");

    let before = cache.generation();
    cache.invalidate_player(0x11);
    assert!(
        cache.generation() != before,
        "an eviction is a landed change"
    );
    assert_eq!(
        cache.player_traits(0x11),
        None,
        "the traits go with the name"
    );

    assert_eq!(cache.resolve(0x11, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::NameQuery { guid: 0x11 })
    ));
}

#[test]
fn creature_miss_queries_once_then_serves_the_answer() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    let wolf_a = compose(guid::HIGH_UNIT, 69, 1);
    let wolf_b = compose(guid::HIGH_UNIT, 69, 2);

    assert_eq!(cache.resolve(wolf_a, &cmds), None);
    // Same entry, different spawn: no second query.
    assert_eq!(cache.resolve(wolf_b, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::CreatureQuery { entry: 69, .. })
    ));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

    cache.insert_creature(
        69,
        Some(CreatureRecord {
            name: "Young Wolf".into(),
            subname: None,
            creature_type: 0,
            pet_family: 0,
            rank: 0,
            type_flags: 0,
            civilian: false,
            racial_leader: false,
            display_id: 0,
            modifiers: None,
        }),
    );
    assert_eq!(cache.resolve(wolf_b, &cmds), Some("Young Wolf"));
}

#[test]
fn player_negative_answer_is_cached() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    let g = compose(guid::HIGH_PLAYER, 0, 7);

    assert_eq!(cache.resolve(g, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::NameQuery { guid }) if guid == g
    ));
    assert_eq!(cache.answered_player_name(g), None, "still pending");
    cache.insert_player(g, String::new(), None); // server: unknown guid
    assert_eq!(cache.resolve(g, &cmds), None);
    assert_eq!(
        cache.answered_player_name(g),
        Some(""),
        "completed negative"
    );
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    cache.insert_player(g, "Known".into(), None);
    assert_eq!(cache.answered_player_name(g), Some("Known"));
    cache.clear_world_session();
    assert_eq!(cache.answered_player_name(g), None);
}

#[test]
fn clear_pending_allows_a_reconnect_reask() {
    let (cmds, rx) = commands();
    let mut cache = NameCache::default();
    let g = compose(guid::HIGH_UNIT, 100, 1);

    assert_eq!(cache.resolve(g, &cmds), None);
    let _ = rx.try_recv();
    // A disconnect dropped the answer.
    cache.clear_pending();
    assert_eq!(cache.resolve(g, &cmds), None);
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::CreatureQuery { entry: 100, .. })
    ));
}

#[path = "names_pet_descriptor_tests.rs"]
mod pet_descriptor;
