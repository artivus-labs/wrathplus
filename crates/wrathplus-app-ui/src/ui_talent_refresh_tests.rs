use super::*;
use wrathplus_formats::{SpellCatalog, TalentTabInfo};
use wrathplus_protocol::{types::Field, ObjectFields};

fn packet(app: &mut App, unspent: u32, ranks: &[(u32, u8)]) {
    packet_groups(app, unspent, 0, &[ranks]);
}

/// A player `SMSG_TALENTS_INFO` with `groups`, the 0-based `active` one active, no glyphs.
fn packet_groups(app: &mut App, unspent: u32, active: u8, groups: &[&[(u32, u8)]]) {
    let mut body = vec![0];
    body.extend(unspent.to_le_bytes());
    body.extend([groups.len() as u8, active]);
    for ranks in groups {
        body.push(ranks.len() as u8);
        for &(id, rank) in *ranks {
            body.extend(id.to_le_bytes());
            body.push(rank);
        }
        body.push(0); // glyph count
    }
    let events = wrathplus_protocol::wrath_progress::decode(
        wrathplus_protocol::wrath_progress::SMSG_TALENTS_INFO,
        &body,
    )
    .unwrap();
    crate::net::handlers::dispatch(app.world_mut(), events);
}

#[test]
fn talent_packet_repaints_open_page_and_learns_next_rank_without_tab_switch() {
    let mut app = App::new();
    // Offline net plugin registers the production TalentsInfo handler; it opens no socket.
    app.add_plugins(crate::net::NetPlugin { connect: false });
    app.init_resource::<PlayerActions>();
    let talent = Talent {
        id: 10,
        tab: 161,
        row: 0,
        col: 0,
        ranks: [100, 101, 102, 0, 0],
        prereq_talent: 0,
        prereq_rank: 0,
        required_spell: 0,
        exceptional: false,
    };
    app.insert_resource(Talents {
        catalog: TalentCatalog::from_rows(
            vec![TalentTabInfo {
                id: 161,
                name: "Arms".into(),
                race_mask: 511,
                class_mask: 1,
                pet_talent_mask: 0,
                background: String::new(),
            }],
            vec![talent],
        ),
    });
    let mut spells = Spells::empty_for_tests();
    spells.catalog = SpellCatalog::from_displays(Default::default());
    app.insert_resource(spells);
    app.world_mut().spawn((
        SelfPlayer,
        ObjectStore(ObjectFields::from_field_slots(&[
            (Field::UnitBytes0, 0, 1 | (1 << 8)),
            (Field::PlayerCharacterPoints1, 0, 99),
        ])),
    ));
    let script = UiScript::new().unwrap();
    script.run("paints=0; local f=CreateFrame('Frame'); f:RegisterEvent('PLAYER_TALENT_UPDATE'); f:SetScript('OnEvent', function() paints=paints+1; local _,_,_,_,rank=GetTalentInfo(1,1); paintedRank=rank; paintedPoints=GetUnspentTalentPoints() end)").unwrap();
    app.insert_non_send_resource(script);
    // Keep the real feed's Local memo/change ticks between runs without running the net ladder.
    let mut schedule = Schedule::default();
    schedule.add_systems(feed_talents);
    packet(&mut app, 2, &[]);
    schedule.run(app.world_mut());
    packet(&mut app, 1, &[(10, 0)]);
    schedule.run(app.world_mut());
    {
        let script = app.world_mut().non_send_resource_mut::<UiScript>();
        assert_eq!(
            script
                .eval::<(u32, u32, u32)>("return paintedRank,paintedPoints,paints")
                .unwrap(),
            (1, 1, 2)
        );
        assert!(script.errors().is_empty(), "{:?}", script.errors());
        script.run("LearnTalent(1,1)").unwrap();
    }
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx));
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(drain_talent_learns)
        .unwrap();
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientCommand::LearnTalent {
            talent_id: 10,
            rank: 1
        })
    ));
    packet(&mut app, 1, &[(10, 0)]);
    schedule.run(app.world_mut());
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<u32>("return paints")
            .unwrap(),
        2,
        "duplicate packet does not repaint"
    );
    packet(&mut app, 2, &[]); // respec clears ranks even if the spellbook is delayed
    schedule.run(app.world_mut());
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<(u32, u32)>("return paintedRank,paintedPoints")
            .unwrap(),
        (0, 2)
    );
}

#[test]
fn player_rank_uses_active_spec_not_pet_or_delayed_spellbook() {
    use wrathplus_protocol::events::TalentGroup;
    let t = Talent {
        id: 10,
        tab: 161,
        row: 0,
        col: 0,
        ranks: [100, 101, 102, 0, 0],
        prereq_talent: 0,
        prereq_rank: 0,
        required_spell: 0,
        exceptional: false,
    };
    let known = BTreeSet::from([102]);
    let mut progress = WrathProgress::default();
    assert_eq!(player_rank(&t, &known, Some(&progress)), 3);
    progress.talents.insert(
        true,
        (
            0,
            0,
            vec![TalentGroup {
                talents: vec![(10, 0)],
                glyphs: vec![],
            }],
        ),
    );
    assert_eq!(
        player_rank(&t, &known, Some(&progress)),
        3,
        "pet packet cannot replace player state"
    );
    progress.talents.insert(
        false,
        (
            2,
            1,
            vec![
                TalentGroup {
                    talents: vec![(10, 2)],
                    glyphs: vec![],
                },
                TalentGroup {
                    talents: vec![(10, 0)],
                    glyphs: vec![],
                },
            ],
        ),
    );
    assert_eq!(player_rank(&t, &known, Some(&progress)), 1);
    assert_eq!(player_talent_state(Some(&progress)).unwrap().0, 2);
    progress.talents.get_mut(&false).unwrap().2[1]
        .talents
        .clear();
    assert_eq!(
        player_rank(&t, &known, Some(&progress)),
        0,
        "respec ignores stale learned spells"
    );
}

/// 12340 keeps the packet's group count and active group (`0x5c9740` into `[0xc20ff8]` and
/// `[0xc20ff4]`), which `GetNumTalentGroups` and `GetActiveTalentGroup` answer.
#[test]
fn the_talents_packet_sets_the_group_count_and_active_group() {
    let mut app = App::new();
    app.add_plugins(crate::net::NetPlugin { connect: false });
    app.init_resource::<PlayerActions>();
    app.insert_non_send_resource(UiScript::new().unwrap());
    let mut schedule = Schedule::default();
    schedule.add_systems(feed_talents);
    let groups = |app: &mut App| {
        let script = app.world_mut().non_send_resource_mut::<UiScript>();
        script
            .eval::<(u32, u32)>("return GetNumTalentGroups(), GetActiveTalentGroup()")
            .unwrap()
    };
    schedule.run(app.world_mut());
    assert_eq!(groups(&mut app), (0, 1), "none before the packet");
    packet_groups(&mut app, 0, 1, &[&[], &[]]);
    schedule.run(app.world_mut());
    assert_eq!(groups(&mut app), (2, 2));
}

/// The handler fires `ACTIVE_TALENT_GROUP_CHANGED` with (new, old) when the active group moves
/// (`0x5c9ed7`-`0x5c9f00`), each 1-based, or 0 while there are no groups.
#[test]
fn a_new_active_group_fires_active_talent_group_changed() {
    let mut app = App::new();
    app.add_plugins(crate::net::NetPlugin { connect: false });
    app.init_resource::<PlayerActions>();
    let script = UiScript::new().unwrap();
    script
        .run(
            "seen = {} local f = CreateFrame('Frame') \
             f:RegisterEvent('ACTIVE_TALENT_GROUP_CHANGED') \
             f:SetScript('OnEvent', function(_, _, new, old) seen[#seen + 1] = new .. ':' .. old end)",
        )
        .unwrap();
    app.insert_non_send_resource(script);
    let mut schedule = Schedule::default();
    schedule.add_systems(feed_talents);
    schedule.run(app.world_mut());
    for active in [1, 0, 0] {
        packet_groups(&mut app, 0, active, &[&[], &[]]);
        schedule.run(app.world_mut());
    }
    let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
    script.tick(0.0);
    assert_eq!(
        script
            .eval::<String>("return table.concat(seen, ' ')")
            .unwrap(),
        "2:0 1:2",
        "the first packet moves from no groups; a repeat fires nothing"
    );
}
