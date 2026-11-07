    /// The record is slot-indexed: deltas patch named slots, the halves split at slot 32, and each
    /// walk is ascending slot whatever order the slots filled in.
    #[test]
    fn record_auras_list_in_slot_order_and_split_buffs_from_debuffs() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                auras: Some(vec![(5, 1126)]),
                auras_negative: Some(vec![(40, 770)]),
                ..Default::default()
            },
        );
        app.update();
        let _ = seen(&mut app);
        // A later aura in a lower slot, then a debuff, as two deltas.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras: Some(vec![(2, 21562)]),
                ..Default::default()
            },
        );
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras_negative: Some(vec![(34, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", false), [21562, 1126]);
        assert_eq!(listed(&mut app, "party1", true), [589, 770]);
        assert_eq!(seen(&mut app), ["party1"]);

        // Slot 5 falls off: the delta names it with a 0, and only it goes.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                auras: Some(vec![(5, 0)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "party1", false), [21562]);
        assert_eq!(listed(&mut app, "party1", true), [589, 770]);
        assert_eq!(seen(&mut app), ["party1"]);

        // A delta that moves no aura fires nothing.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(7),
                ..Default::default()
            },
        );
        app.update();
        assert!(seen(&mut app).is_empty());
    }

    /// `partypetN` names the member's `CHARM`, else `SUMMON`, while the member is held, else the
    /// record's pet guid (`0x4e81d0`); the pet's list is its own descriptor while held, else the
    /// pet record's block (`0x496420`, `+0xe4`).
    #[test]
    fn a_party_pet_reads_its_descriptor_or_the_pet_block() {
        let mut app = party_app();
        stats(
            &mut app,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::ONLINE),
                pet_guid: Some(PET),
                pet_auras: Some(vec![(1, 1126)]),
                pet_auras_negative: Some(vec![(32, 770)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "partypet1", false), [1126]);
        assert_eq!(listed(&mut app, "partypet1", true), [770]);
        assert!(listed(&mut app, "partypet2", true).is_empty());
        assert_eq!(seen(&mut app), ["party1", "partypet1"]);

        // A pet-aura delta moves the pet's list and fires for its token alone.
        stats(
            &mut app,
            false,
            PartyMemberStatsInfo {
                pet_auras_negative: Some(vec![(32, 0), (33, 589)]),
                ..Default::default()
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "partypet1", true), [589]);
        assert_eq!(seen(&mut app), ["partypet1"]);

        // Member and pet in view: the pet guid comes off the member's SUMMON, the list off the
        // pet's own packet.
        stream(
            &mut app,
            MEMBER,
            &[(SUMMON, PET as u32), (SUMMON + 1, (PET >> 32) as u32)],
        );
        stream(&mut app, PET, &[]);
        packet(&mut app, PET, &[(35, 11976, false)]);
        app.update();
        assert_eq!(listed(&mut app, "partypet1", true), [11976]);
        assert!(listed(&mut app, "partypet1", false).is_empty());
        assert!(seen(&mut app).contains(&"partypet1".to_string()));
    }

    // == The raid tokens ==

    /// In our subgroup, so `party1` and `raid2`.
    const MATE: u64 = 0x2001;
    /// Subgroup 2, `raid5` only.
    const FAR: u64 = 0x2004;
    const FAR_PET: u64 = 0xF140_0000_0000_0099;

    /// A raid of five: us (row 1, subgroup 1), `MATE` (subgroup 1), two in subgroup 2 and `FAR`
    /// in subgroup 3, in wire order, none streamed and none with a record.
    fn raid_app() -> App {
        let mut app = party_app();
        relist_raid(&mut app, member_status::ONLINE);
        app.update();
        let _ = seen(&mut app);
        app
    }

    /// The raid's list as `raid_app` sends it, with `MATE`'s status byte as given.
    fn relist_raid(app: &mut App, mate: u8) {
        let member = |name: &str, guid, status, subgroup| GroupMemberEntry {
            name: name.into(),
            guid,
            status,
            flags: subgroup,
        };
        let online = member_status::ONLINE;
        app.world_mut().resource_mut::<GroupState>().apply_list(
            crate::app_core::state::GROUPTYPE_RAID,
            0,
            vec![
                member("Mate", MATE, mate, 0),
                member("Two", 0x2002, online, 1),
                member("Three", 0x2003, online, 1),
                member("Far", FAR, online, 2),
            ],
            ME,
            None,
            Some(ME),
        );
    }

    fn online(auras_negative: Vec<(u8, u16)>) -> PartyMemberStatsInfo {
        PartyMemberStatsInfo {
            status: Some(member_status::ONLINE),
            auras_negative: Some(auras_negative),
            ..Default::default()
        }
    }

    /// A raid member in view reads its own packet auras under `raidN`, whatever its subgroup; the
    /// native list applies its own whole-unit gate, which passes a raid member's buffs.
    #[test]
    fn a_live_raid_members_packet_auras_list_under_its_raid_token() {
        let mut app = raid_app();
        stream(&mut app, FAR, &[]);
        packet(&mut app, FAR, &[(0, 1126, true), (33, 589, false)]);
        app.update();
        assert_eq!(listed(&mut app, "raid5", true), [589]);
        assert_eq!(listed(&mut app, "raid5", false), [1126]);
        assert_eq!(seen(&mut app), ["raid5"]);
        for token in ["party1", "party2", "raid6"] {
            assert!(listed(&mut app, token, true).is_empty(), "{token}");
        }
    }

    /// Out of range, `raidN` walks the member's record, which vmangos keeps current for every raid
    /// member we do not see; `raidpetN` walks the pet block the same record names.
    #[test]
    fn an_out_of_range_raid_member_and_its_pet_read_the_record() {
        let mut app = raid_app();
        stats_of(
            &mut app,
            FAR,
            true,
            PartyMemberStatsInfo {
                pet_guid: Some(FAR_PET),
                pet_auras: Some(vec![(1, 1126)]),
                pet_auras_negative: Some(vec![(33, 770)]),
                ..online(vec![(32, 589), (35, 11976)])
            },
        );
        app.update();
        assert_eq!(listed(&mut app, "raid5", true), [589, 11976]);
        let count = app
            .world_mut()
            .non_send_resource::<UiScript>()
            .eval::<i64>(r#"local _, _, _, count = UnitDebuff("raid5", 1) return count"#)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(listed(&mut app, "raidpet5", false), [1126]);
        assert_eq!(listed(&mut app, "raidpet5", true), [770]);
        assert_eq!(seen(&mut app), ["raid5", "raidpet5"]);
        assert!(
            listed(&mut app, "raidpet4", true).is_empty(),
            "a petless member's raidpet token names nobody"
        );
    }

    /// `UNIT_AURA` fires for every token naming a unit whose list moved (`0x515e50`), and for
    /// nothing else; a delta that moves no aura fires nothing.
    #[test]
    fn unit_aura_fires_for_each_token_naming_the_member_whose_list_moved() {
        let mut app = raid_app();
        stats_of(&mut app, MATE, true, online(vec![(32, 589)]));
        stats_of(&mut app, FAR, true, online(vec![(32, 770)]));
        app.update();
        let mut first = seen(&mut app);
        first.sort();
        assert_eq!(first, ["party1", "raid2", "raid5"]);

        stats_of(&mut app, MATE, false, online(vec![(33, 11976)]));
        app.update();
        assert_eq!(seen(&mut app), ["party1", "raid2"]);
        assert_eq!(listed(&mut app, "raid2", true), [589, 11976]);
        assert_eq!(listed(&mut app, "party1", true), [589, 11976]);

        stats_of(
            &mut app,
            FAR,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(7),
                ..Default::default()
            },
        );
        app.update();
        assert!(seen(&mut app).is_empty());
    }

    /// The raid roster's twin of the list write (`0x4ba947`-`0x4ba960`): a status-only raid list
    /// hides an out-of-view row's auras under `raidN` and its party slot alike, and an online list
    /// returns them.
    #[test]
    fn a_status_only_raid_list_takes_an_out_of_view_rows_auras_and_an_online_one_returns_them() {
        let mut app = raid_app();
        stats_of(&mut app, MATE, true, online(vec![(32, 589)]));
        app.update();
        assert_eq!(listed(&mut app, "raid2", true), [589]);
        assert_eq!(listed(&mut app, "party1", true), [589]);

        relist_raid(&mut app, member_status::OFFLINE);
        app.update();
        assert!(listed(&mut app, "raid2", true).is_empty());
        assert!(listed(&mut app, "party1", true).is_empty());

        relist_raid(&mut app, member_status::ONLINE);
        app.update();
        assert_eq!(listed(&mut app, "raid2", true), [589]);
        assert_eq!(listed(&mut app, "party1", true), [589]);
    }

    /// A raid token past the roster, a raid token outside a raid, and an offline member's pet
    /// all answer nil.
    #[test]
    fn a_raid_token_naming_nobody_answers_nil() {
        let mut app = party_app();
        stats(&mut app, true, online(vec![(32, 589)]));
        app.update();
        assert_eq!(listed(&mut app, "party1", true), [589]);
        for token in ["raid1", "raid2"] {
            assert!(
                listed(&mut app, token, true).is_empty(),
                "a party is no raid: {token}"
            );
        }

        let mut app = raid_app();
        stats_of(
            &mut app,
            FAR,
            true,
            PartyMemberStatsInfo {
                status: Some(member_status::OFFLINE),
                pet_guid: Some(FAR_PET),
                pet_auras_negative: Some(vec![(33, 770)]),
                auras_negative: Some(vec![(32, 589)]),
                ..Default::default()
            },
        );
        app.update();
        for token in ["raid5", "raidpet5", "raid6", "raid40", "raidpet40"] {
            assert!(listed(&mut app, token, true).is_empty(), "{token}");
        }
    }

    /// Row 1 is us: the player's own list, as `"player"` answers it.
    #[test]
    fn our_raid_row_mirrors_the_player_list() {
        let mut app = raid_app();
        packet(&mut app, ME, &[(32, 589, false)]);
        app.update();
        assert_eq!(listed(&mut app, "player", true), [589]);
        assert_eq!(listed(&mut app, "raid1", true), [589]);
        assert_eq!(seen(&mut app), ["raid1"]);
    }

    /// A frame where nothing moved builds no list: the feed compares the inputs, not the lists.
    #[test]
    fn an_unchanged_raid_rebuilds_no_list() {
        let mut app = raid_app();
        stats_of(&mut app, FAR, true, online(vec![(32, 589)]));
        app.update();
        let versions = |app: &mut App| {
            app.world_mut()
                .resource_scope(|world, mut mem: Mut<super::super::AuraFeedMemory>| {
                    let script = world.non_send_resource::<UiScript>();
                    mem.vm.get(script).units.versions
                })
        };
        let before = versions(&mut app);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(versions(&mut app), before);
        stats_of(&mut app, FAR, false, online(vec![(33, 770)]));
        app.update();
        assert_eq!(
            versions(&mut app),
            before + 1,
            "one list moved, one rebuilt"
        );
    }
