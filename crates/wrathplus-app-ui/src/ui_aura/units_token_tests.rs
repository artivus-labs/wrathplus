    // == Any token the resolver parses ==

    /// `OBJECT_FIELD_TYPE`; a unit's typemask is `0x9`, a player's `0x19`.
    const TYPE: u16 = Field::ObjectType.index();
    /// `UNIT_FIELD_TARGET`'s low dword; the high half is the next index.
    const TARGET: u16 = Field::UnitTarget.index();
    const MOB: u64 = 0xF130_0000_0000_0001;
    const ADD: u64 = 0xF130_0000_0000_0002;

    /// A unit of typemask `ty` targeting `target`.
    fn unit_fields(ty: u32, target: u64) -> Vec<(u16, u32)> {
        vec![
            (TYPE, ty),
            (TARGET, target as u32),
            (TARGET + 1, (target >> 32) as u32),
        ]
    }

    /// We target `MOB`, which targets party1, who targets `ADD`, which targets us; the mouse is over
    /// `ADD`, and our pet targets `MOB`. Each carries its own debuff: us 6788, `MOB` 589, party1
    /// 11976, `ADD` 770.
    fn chain_app() -> App {
        let mut app = party_app();
        app.init_resource::<crate::target::Hovered>();
        let me = app.world_mut().resource::<GuidIndex>().0[&ME];
        *app.world_mut().get_mut::<ObjectStore>(me).unwrap() =
            ObjectStore(ObjectFields::from_pairs(&unit_fields(0x19, MOB)));
        stream(&mut app, MOB, &unit_fields(0x9, MEMBER));
        stream(&mut app, MEMBER, &unit_fields(0x19, ADD));
        stream(&mut app, ADD, &unit_fields(0x9, ME));
        stream(&mut app, PET, &unit_fields(0x9, MOB));
        for (guid, debuff) in [
            (ME, 6788),
            (MOB, 589),
            (MEMBER, 11976),
            (ADD, 770),
            (PET, 1),
        ] {
            packet(&mut app, guid, &[(32, debuff, false)]);
        }
        app.world_mut()
            .resource_mut::<crate::app_core::state::PetBar>()
            .spells
            .pet_guid = PET;
        let (mob, add) = {
            let index = &app.world().resource::<GuidIndex>().0;
            (index[&MOB], index[&ADD])
        };
        let mut selection = app
            .world_mut()
            .resource_mut::<crate::app_core::state::Selection>();
        (selection.target, selection.guid) = (Some(mob), Some(MOB));
        let mut hovered = app.world_mut().resource_mut::<crate::target::Hovered>();
        (hovered.target, hovered.guid) = (Some(add), Some(ADD));
        app.update();
        app
    }

    /// `UnitDebuff` reads the list of whatever unit the token names, through the resolver
    /// (`0x60abf0` via `0x6110a0`): the mouseover, a member's target, any depth of `target` chain,
    /// in any case, and the player's own list at the end of a chain that comes back to us.
    #[test]
    fn every_token_reads_the_list_of_the_unit_it_names() {
        let mut app = chain_app();
        for (token, want) in [
            ("mouseover", 770),
            ("MouseOver", 770),
            ("party1target", 770),
            ("PARTY1TARGET", 770),
            ("target", 589),
            ("targettarget", 11976),
            ("targettargettarget", 770),
            ("targettargettargettarget", 6788),
            ("pettarget", 589),
            ("playertarget", 589),
            ("mouseovertargettarget", 589),
            ("PLAYER", 6788),
            ("Player", 6788),
        ] {
            assert_eq!(listed(&mut app, token, true), [want], "{token}");
        }
        assert_eq!(
            listed(&mut app, "PLAYER", true),
            listed(&mut app, "player", true)
        );
    }

    /// `UNIT_AURA` fires for the tokens it fired for before, never for a chain or the mouseover,
    /// which the reference's fan-out (`0x515c50`) does not list.
    #[test]
    fn unit_aura_still_fires_for_the_fixed_tokens_alone() {
        let mut app = chain_app();
        let mut first = seen(&mut app);
        first.sort();
        assert_eq!(first, ["party1", "pet", "target", "targettarget"]);

        // `ADD`'s list moves: it is the mouseover, party1's target and three hops off ours, and
        // no token that fires names it.
        packet(&mut app, ADD, &[(32, 770, false), (33, 589, false)]);
        app.update();
        assert!(seen(&mut app).is_empty());
        assert_eq!(listed(&mut app, "mouseover", true), [770, 589]);
        assert_eq!(listed(&mut app, "party1target", true), [770, 589]);
    }

    /// A nearer GameObject wins the pick, and the publisher writes its guid, which the resolver
    /// rejects as a unit (`0x515bd9 je`): the unit behind it is no longer the mouseover.
    #[test]
    fn a_nearer_gameobject_takes_the_mouseover_from_the_unit_behind_it() {
        let mut app = chain_app();
        assert_eq!(listed(&mut app, "mouseover", true), [770]);
        app.world_mut()
            .resource_mut::<crate::target::Hovered>()
            .distance = 10.0;
        let chest = app.world_mut().spawn_empty().id();
        app.insert_resource(crate::target::HoveredObject {
            target: Some(chest),
            guid: Some(0xF110_0000_0000_0004),
            distance: 5.0,
        });
        app.update();
        assert!(listed(&mut app, "mouseover", true).is_empty());
        assert!(listed(&mut app, "mouseovertarget", true).is_empty());
    }

    /// A raid member's target that is out of view but on the roster reads its record, as
    /// `UnitDebuff`'s roster walk finds it (`0x519b25`); a hop off a unit not held is nobody.
    #[test]
    fn a_raid_members_target_reads_whatever_the_guid_holds() {
        let mut app = raid_app();
        stats_of(&mut app, FAR, true, online(vec![(32, 589)]));
        // `raid3` is 0x2002, in view and targeting `FAR`, which is not.
        stream(&mut app, 0x2002, &unit_fields(0x19, FAR));
        packet(&mut app, 0x2002, &[(32, 770, false)]);
        app.update();
        assert_eq!(listed(&mut app, "raid3", true), [770]);
        assert_eq!(listed(&mut app, "raid3target", true), [589]);
        assert_eq!(listed(&mut app, "raid5", true), [589]);
        assert!(listed(&mut app, "raid5target", true).is_empty());
        assert!(listed(&mut app, "raid3targettarget", true).is_empty());
    }
