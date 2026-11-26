use super::WrathLfgQueueRequest as R;
use super::*;
use crate::script::{
    PartyMemberInfo, PartyState, UnitState, WrathGroup, WrathLfgCatalog, WrathLfgDungeon,
};

fn vm() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.register_cvars([("lfgSelectedRoles", "0")]);
    s.set_unit(
        "player",
        Some(UnitState {
            guid: 1,
            exists: true,
            class_file: Some("WARRIOR".into()),
            ..Default::default()
        }),
    );
    s.model_mut().unit_guids.player = 1;
    s.set_wrath_lfg_catalog(
        Arc::new(WrathLfgCatalog {
            dungeons: vec![
                WrathLfgDungeon {
                    id: 7,
                    name: "Halls".into(),
                    dungeon_type: 1,
                    map_id: 600,
                    difficulty: 1,
                    texture: "HALLS".into(),
                    ..Default::default()
                },
                WrathLfgDungeon {
                    id: 9,
                    name: "Raid".into(),
                    dungeon_type: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        None,
    );
    s.set_wrath_lfg_encounters(Arc::new(vec![
        WrathLfgEncounter {
            map: 600,
            difficulty: 1,
            bit: 0,
            name: "Boss A".into(),
            icon: Some("IconA".into()),
        },
        WrathLfgEncounter {
            map: 600,
            difficulty: 1,
            bit: 3,
            name: "Boss B".into(),
            icon: None,
        },
    ]));
    s
}

fn snapshot() -> WrathLfgQueueSnapshot {
    WrathLfgQueueSnapshot {
        player: Some(WrathLfgUpdate {
            joined: true,
            queued: true,
            dungeons: vec![1 << 24 | 7, 2 << 24 | 9],
            comment: "hi".into(),
            ..Default::default()
        }),
        queue: Some(WrathLfgQueueStatus {
            dungeon: 1 << 24 | 7,
            avg_wait: 60,
            tank_wait: -1,
            healer_wait: 30,
            dps_wait: 90,
            my_wait: 45,
            tank_needs: 1,
            healer_needs: 0,
            dps_needs: 2,
            queued_at_ms: 5000,
        }),
        role_check: Some(WrathLfgRoleCheck {
            state: 2,
            dungeons: vec![1 << 24 | 7],
            members: vec![
                WrathLfgRoleCheckMember {
                    guid: 1,
                    ready: true,
                    roles: 3,
                    level: 80,
                    name: Some("Me".into()),
                },
                WrathLfgRoleCheckMember {
                    guid: 2,
                    level: 79,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        proposal: Some(WrathLfgProposal {
            dungeon: 1 << 24 | 7,
            id: 44,
            completed_mask: 1 << 3,
            rows: vec![
                WrathLfgProposalRow {
                    roles: 4,
                    ..Default::default()
                },
                WrathLfgProposalRow {
                    roles: 3,
                    is_self: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        boot: Some(WrathLfgBoot {
            in_progress: true,
            did_vote: true,
            my_vote: false,
            victim_name: Some("Bad".into()),
            victim_realm: Some("Elsewhere".into()),
            total_votes: 3,
            boot_votes: 2,
            time_left: 40,
            reason: "afk".into(),
            ..Default::default()
        }),
        completion: Some(WrathLfgCompletion {
            dungeon: 1 << 24 | 7,
            strangers: 2,
            money_base: 100,
            xp_base: 200,
            money_var: 10,
            xp_var: 20,
            items: vec![WrathLfgCompletionItem {
                item: 5,
                display: 6,
                count: 3,
            }],
        }),
        ..Default::default()
    }
}

#[test]
fn getters_read_the_snapshot_in_native_shapes() {
    let mut s = vm();
    assert_eq!(
        s.eval::<u32>("return select('#', GetLFGQueueStats())")
            .unwrap(),
        0
    );
    assert!(s
        .eval::<bool>("return GetLFGBootProposal()==false and select('#', GetLFGBootProposal())==1")
        .unwrap());
    s.set_wrath_lfg_queue(Arc::new(snapshot()));
    for check in [
        "select('#', GetLFGQueueStats())==13",
        "select(1, GetLFGQueueStats())==true and select(2, GetLFGQueueStats())==0",
        "select(3, GetLFGQueueStats())==1 and select(5, GetLFGQueueStats())==2",
        "select(6, GetLFGQueueStats())==1 and select(7, GetLFGQueueStats())=='Halls'",
        "select(9, GetLFGQueueStats())==-1 and select(12, GetLFGQueueStats())==45",
        "select(13, GetLFGQueueStats())==5",
        "select('#', GetLFGInfoServer())==10 and select(1, GetLFGInfoServer())==false",
        "select(6, GetLFGInfoServer())=='hi' and select(7, GetLFGInfoServer())==2",
        "GetLFGQueuedList()[7]==true and GetLFGQueuedList()[9]==true",
        "IsListedInLFR()==true",
        "select(1, GetLFGRoleUpdate())==true and select(2, GetLFGRoleUpdate())==1 and select(3, GetLFGRoleUpdate())==2",
        "select(1, GetLFGRoleUpdateSlot(1))==1 and select(2, GetLFGRoleUpdateSlot(1))==7",
        "select('#', GetLFGRoleUpdateSlot(2))==0",
        "select('#', GetLFGRoleUpdateMember(1))==8 and select(3, GetLFGRoleUpdateMember(1))=='Me'",
        "select(5, GetLFGRoleUpdateMember(1))==true and select(6, GetLFGRoleUpdateMember(1))==true and select(7, GetLFGRoleUpdateMember(1))==false",
        "select(1, GetLFGRoleUpdateMember(2))==2 and select(2, GetLFGRoleUpdateMember(2))==false and select(3, GetLFGRoleUpdateMember(2))==79",
        "select('#', GetLFGProposal())==12 and select(3, GetLFGProposal())==7 and select(4, GetLFGProposal())=='Halls'",
        "select(6, GetLFGProposal())=='TANK' and select(8, GetLFGProposal())==2 and select(9, GetLFGProposal())==1",
        "select(10, GetLFGProposal())==5 and select(11, GetLFGProposal())==true",
        "select(1, GetLFGProposalMember(1))==false and select(2, GetLFGProposalMember(1))=='HEALER' and select('#', GetLFGProposalMember(1))==7",
        "select(2, GetLFGProposalMember(5))=='UNKNOWN' and select('#', GetLFGProposalMember(6))==0",
        "select(1, GetLFGProposalEncounter(1))=='Boss A' and select(3, GetLFGProposalEncounter(1))==false",
        "select(2, GetLFGProposalEncounter(2))==nil and select(3, GetLFGProposalEncounter(2))==true",
        "select('#', GetLFGProposalEncounter(3))==3 and GetLFGProposalEncounter(3)==nil",
        "select(4, GetLFGBootProposal())=='Bad-Elsewhere' and select(7, GetLFGBootProposal())==40 and select(8, GetLFGBootProposal())=='afk'",
        "select('#', GetLFGCompletionReward())==9 and select(4, GetLFGCompletionReward())==100 and select(5, GetLFGCompletionReward())==10",
        "select(6, GetLFGCompletionReward())==200 and select(8, GetLFGCompletionReward())==2 and select(9, GetLFGCompletionReward())==1",
        "select(2, GetLFGCompletionRewardItem(1))==3 and select('#', GetLFGCompletionRewardItem(2))==0",
    ] {
        assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
    }
    s.run("local p = CreateFrame('Frame') local t = CreateFrame('GameTooltip', 'LfgTip', p) t:SetOwner(p, 'ANCHOR_NONE') t:SetLFGCompletionReward(1) t:SetLFGCompletionReward(9) t:SetLFGDungeonReward(7, 1)").unwrap();
    assert!(
        s.run("LfgTip:SetLFGDungeonReward(7)").is_err()
            && s.run("LfgTip:SetLFGCompletionReward()").is_err()
    );
    for usage in [
        "GetLFGRoleUpdateSlot()",
        "GetLFGProposalEncounter('x')",
        "GetLFGCompletionRewardItem()",
        "GetLFGQueuedList(1)",
    ] {
        assert!(s.run(usage).is_err(), "{usage}");
    }
    // Grouped: the party record, which is absent.
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Pal".into(),
            guid: 2,
        }],
        leader_guid: 1,
        own_guid: 1,
        ..Default::default()
    });
    assert!(s
        .eval::<bool>("return select('#', GetLFGQueueStats())==0 and select(1, GetLFGInfoServer())==true and IsListedInLFR()==false")
        .unwrap());
}

#[test]
fn group_lfg_state_backfill_and_deserter() {
    let mut s = vm();
    assert!(s
        .eval::<bool>("return IsPartyLFG()==nil and select('#', IsInLFGDungeon())==0 and HasLFGRestrictions()==false and CanPartyLFGBackfill()==false")
        .unwrap());
    s.set_wrath_group(WrathGroup {
        lfg_dungeon_entry: 1 << 24 | 7,
        lfg_status: 2,
        lfg_restricted: true,
        ..Default::default()
    });
    s.set_wrath_lfg_location(Some((600, 1)));
    assert!(s
        .eval::<bool>("local n, id, t = GetPartyLFGBackfillInfo() return IsPartyLFG()==1 and IsInLFGDungeon()==true and HasLFGRestrictions()==true and CanPartyLFGBackfill()==true and n=='Halls' and id==7 and t==1")
        .unwrap());
    s.set_wrath_lfg_location(Some((601, 1)));
    assert!(s.eval::<bool>("return IsInLFGDungeon()==false").unwrap());
    s.model_mut().player_auras = vec![crate::script::AuraState {
        spell_id: 71041,
        ..Default::default()
    }];
    assert!(s
        .eval::<bool>("return UnitHasLFGDeserter('player')==true and UnitHasLFGRandomCooldown('player')==false and select('#', UnitHasLFGDeserter('party1'))==0")
        .unwrap());
    assert!(s.run("UnitHasLFGDeserter()").is_err());
}

#[test]
fn selection_roles_and_verbs_follow_the_native_gates() {
    let mut s = vm();
    s.set_now(100.0);
    s.run("JoinLFG()").unwrap();
    s.run("SetLFGDungeon(7); SetLFGDungeon(7); SetLFGDungeon(99); JoinLFG()")
        .unwrap();
    assert_eq!(
        s.take_ui_errors(),
        vec!["ERR_LFG_NO_SLOTS_SELECTED", "ERR_LFG_NO_ROLES_SELECTED"]
    );
    // A warrior cannot heal: the healer bit is masked off.
    s.run("SetLFGRoles(true, true, true, false); local l,t,h,d = GetLFGRoles(); assert(l and t and not h and not d)").unwrap();
    s.run(
        "SetLFGComment('lfm'); JoinLFG(); JoinLFG(); SetLFGComment('new'); LeaveLFG(); LeaveLFG()",
    )
    .unwrap();
    assert_eq!(
        s.take_wrath_lfg_queue_requests(),
        vec![
            R::Join {
                roles: 3,
                no_partial_clear: false,
                achievements: false,
                dungeons: vec![1 << 24 | 7],
                needs: [0; 3],
                comment: "lfm".into(),
            },
            R::SetComment { text: "new".into() },
            R::Leave,
        ]
    );
    s.run("ClearAllLFGDungeons(); JoinLFG()").unwrap();
    assert_eq!(s.take_ui_errors(), vec!["ERR_LFG_NO_SLOTS_SELECTED"]);

    // Role check: the third call opens a window (its stamp starts at 0); two more pass, then
    // nothing until ten seconds after that.
    s.run("assert(CompleteLFGRoleCheck(true)==true); assert(CompleteLFGRoleCheck(false)==true); for i = 1, 3 do assert(CompleteLFGRoleCheck(true)==true) end; assert(CompleteLFGRoleCheck(true)==nil)").unwrap();
    s.set_now(109.0);
    s.run("assert(CompleteLFGRoleCheck(true)==nil)").unwrap();
    s.set_now(110.0);
    s.run("assert(CompleteLFGRoleCheck(true)==true)").unwrap();
    let answers = s.take_wrath_lfg_queue_requests();
    assert_eq!(answers.len(), 6);
    assert_eq!(answers[1], R::RoleCheckAnswer { roles: 0 });
    assert_eq!(answers[5], R::RoleCheckAnswer { roles: 3 });

    // Proposal: accept marks our row; reject drops it.
    s.set_wrath_lfg_queue(Arc::new(snapshot()));
    s.run("AcceptProposal(); assert(select(7, GetLFGProposal())==true); RejectProposal(); assert(select('#', GetLFGProposal())==0); AcceptProposal()").unwrap();
    assert_eq!(
        s.take_wrath_lfg_queue_requests(),
        vec![
            R::Proposal {
                id: 44,
                accept: true
            },
            R::Proposal {
                id: 44,
                accept: false
            },
        ]
    );

    // Teleport: refused in combat with the native error.
    s.run("assert(LFGTeleport(true)==true)").unwrap();
    s.set_unit(
        "player",
        Some(UnitState {
            guid: 1,
            exists: true,
            in_combat: true,
            ..Default::default()
        }),
    );
    s.run("assert(LFGTeleport(false)==false)").unwrap();
    assert_eq!(s.take_ui_errors(), vec!["ERR_NOT_IN_COMBAT"]);
    assert_eq!(
        s.take_wrath_lfg_queue_requests(),
        vec![R::Teleport { out: true }]
    );

    // Backfill: the group's dungeon becomes the selection and the join goes out again.
    s.set_wrath_group(WrathGroup {
        lfg_dungeon_entry: 1 << 24 | 7,
        ..Default::default()
    });
    s.set_unit(
        "player",
        Some(UnitState {
            guid: 1,
            exists: true,
            class_file: Some("WARRIOR".into()),
            ..Default::default()
        }),
    );
    s.set_now(200.0);
    s.run("SetLFGDungeon(9) PartyLFGStartBackfill()").unwrap();
    match s.take_wrath_lfg_queue_requests().as_slice() {
        [R::Join {
            dungeons, roles, ..
        }] => assert_eq!((dungeons.clone(), *roles), (vec![1 << 24 | 7], 3)),
        other => panic!("{other:?}"),
    }
}
