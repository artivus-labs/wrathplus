use super::WrathLfgQueueRequest as R;
use super::*;
use crate::script::{
    PartyMemberInfo, PartyState, UnitState, WrathGroup, WrathLfgCatalog, WrathLfgDungeon,
};

fn vm() -> UiScript {
    // The OS clock the throttles read, held still: the lock-gate test installs the same.
    super::super::set_wrath_lfg_request_clock(Arc::new(|| 1_000_000));
    let mut s = UiScript::new().unwrap();
    s.register_cvars([("lfgSelectedRoles", "0")]);
    s.set_unit(
        "player",
        Some(UnitState {
            guid: 1,
            exists: true,
            class: Some("Warrior".into()),
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
        "GetLFGQueuedList()[7]==true and GetLFGQueuedList(nil)[9]==true",
        "IsListedInLFR()==true",
        "select('#', GetLFGTypes())==7 and select(7, GetLFGTypes())=='LFG_TYPE_RANDOM_DUNGEON'",
        "select(1, GetLFGRoleUpdate())==true and select(2, GetLFGRoleUpdate())==1 and select(3, GetLFGRoleUpdate())==2",
        "select(1, GetLFGRoleUpdateSlot(1))==1 and select(2, GetLFGRoleUpdateSlot(1))==7",
        "select('#', GetLFGRoleUpdateSlot(2))==0",
        "select('#', GetLFGRoleUpdateMember(1))==8 and select(3, GetLFGRoleUpdateMember(1))=='Me'",
        "select(5, GetLFGRoleUpdateMember(1))==true and select(6, GetLFGRoleUpdateMember(1))==true and select(7, GetLFGRoleUpdateMember(1))==false",
        "select('#', GetLFGRoleUpdateMember(2))==8 and select(1, GetLFGRoleUpdateMember(2))==false and select(2, GetLFGRoleUpdateMember(2))==79",
        "select(4, GetLFGRoleUpdateMember(1))=='Warrior' and select(3, GetLFGRoleUpdateMember(2))==nil and select(5, GetLFGRoleUpdateMember(2))==nil",
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
        "select(2, GetLFGCompletionRewardItem(0.6))==3 and select('#', GetLFGCompletionRewardItem(2))==0",
    ] {
        assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
    }
    s.run("local p = CreateFrame('Frame') local t = CreateFrame('GameTooltip', 'LfgTip', p) t:SetOwner(p, 'ANCHOR_NONE') t:SetLFGCompletionReward(1) t:SetLFGCompletionReward(9) t:SetLFGDungeonReward(7, 1)").unwrap();
    for usage in [
        "GetLFGRoleUpdateSlot()",
        "GetLFGRoleUpdateMember()",
        "GetLFGProposalEncounter('x')",
        "GetLFGCompletionRewardItem()",
    ] {
        assert!(s.run(usage).is_err(), "{usage}");
    }
    for (call, usage) in [
        ("GetLFGQueuedList(1)", "Usage: GetLFDQueuedList([table])"),
        (
            "LfgTip:SetLFGDungeonReward(7)",
            "Usage: Tooltip:SetLFGDungeonReward(",
        ),
        (
            "LfgTip:SetLFGCompletionReward()",
            "Usage: Tooltip:SetLFGCompletionReward(",
        ),
    ] {
        assert!(
            s.run(call).unwrap_err().to_string().contains(usage),
            "{call}"
        );
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

    // `0x554960`: a vote in progress, three votes per window from the seeded counter, our answer
    // at once (`0xbea860`) until a boot update replaces it.
    s.run("for i = 1, 4 do SetLFGBootVote(true) end assert(select(3, GetLFGBootProposal())==true)")
        .unwrap();
    assert_eq!(s.take_wrath_popup_requests().len(), 3);
    s.set_wrath_lfg_queue(Arc::new(snapshot()));
    assert!(s
        .eval::<bool>("return select(3, GetLFGBootProposal())")
        .unwrap());
    // The victim's name resolving republishes without a packet, and keeps it.
    let mut named = snapshot();
    named.boot.as_mut().unwrap().victim_name = Some("Resolved".into());
    s.set_wrath_lfg_queue(Arc::new(named));
    assert!(s
        .eval::<bool>("return select(3, GetLFGBootProposal())")
        .unwrap());
    let mut update = snapshot();
    update.boot.as_mut().unwrap().time_left = 30;
    s.set_wrath_lfg_queue(Arc::new(update));
    assert!(!s
        .eval::<bool>("return select(3, GetLFGBootProposal())")
        .unwrap());
    let mut over = snapshot();
    over.boot = None;
    s.set_wrath_lfg_queue(Arc::new(over));
    s.run("SetLFGBootVote(true)").unwrap();
    assert!(
        s.take_wrath_popup_requests().is_empty(),
        "no vote in progress"
    );
}

#[test]
fn group_lfg_state_backfill_and_deserter() {
    let mut s = vm();
    assert!(s
        .eval::<bool>("return IsPartyLFG()==nil and select('#', IsInLFGDungeon())==0 and HasLFGRestrictions()==false and CanPartyLFGBackfill()==false")
        .unwrap());
    let group = |lfg_flags| WrathGroup {
        lfg_dungeon_entry: 1 << 24 | 7,
        lfg_flags,
        lfg_restricted: true,
        ..Default::default()
    };
    s.set_wrath_group(group(0));
    s.set_wrath_lfg_location(Some((600, 1)));
    // Backfill needs 1..3 party members (`0xbd1998`), then the LFG byte's bit 2 clear.
    assert!(s
        .eval::<bool>(
            "return CanPartyLFGBackfill()==false and select('#', GetPartyLFGBackfillInfo())==0"
        )
        .unwrap());
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
        .eval::<bool>("local n, id, t = GetPartyLFGBackfillInfo() return IsPartyLFG()==1 and IsInLFGDungeon()==true and HasLFGRestrictions()==true and CanPartyLFGBackfill()==true and n=='Halls' and id==7 and t==1")
        .unwrap());
    s.set_wrath_group(group(2));
    s.set_wrath_lfg_location(Some((601, 1)));
    assert!(s
        .eval::<bool>("return IsInLFGDungeon()==false and CanPartyLFGBackfill()==false")
        .unwrap());
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
        "SetLFGComment('lfm'); JoinLFG(); JoinLFG(); ClearAllLFGDungeons(); SetLFGDungeon(7) \
         JoinLFG(); SetLFGComment('new'); LeaveLFG(); LeaveLFG()",
    )
    .unwrap();
    let join = R::Join {
        roles: 3,
        no_partial_clear: false,
        achievements: false,
        dungeons: vec![1 << 24 | 7],
        needs: [0; 3],
        comment: "lfm".into(),
    };
    assert_eq!(
        s.take_wrath_lfg_queue_requests(),
        vec![
            join.clone(),
            join.clone(),
            R::SetComment { text: "new".into() },
            R::Leave,
        ],
        "a selection changed after a join resends it (`0xacd883`)"
    );
    s.run("ClearAllLFGDungeons(); JoinLFG()").unwrap();
    assert_eq!(s.take_ui_errors(), vec!["ERR_LFG_NO_SLOTS_SELECTED"]);
    // `0x7e1830`: pipes dropped, cut at a line break (a real one or `\n`), 63 characters kept.
    let comment = |s: &mut UiScript, code: &str| {
        s.run(code).unwrap();
        s.model_mut().wrath_lfg.queue.comment.clone()
    };
    assert_eq!(comment(&mut s, r"SetLFGComment('l|f|m\\nmore')"), "lfm");
    assert_eq!(comment(&mut s, "SetLFGComment('a|b\\rc')"), "ab");
    assert_eq!(
        comment(&mut s, "SetLFGComment(('x'):rep(70))"),
        "x".repeat(63)
    );
    // `0x559b70`: a dungeon drops a raid and a raid a dungeon; a repeat changes nothing alone,
    // and in a party a raid clears first.
    let pick = |s: &mut UiScript, code: &str| {
        s.model_mut().wrath_lfg.queue.pending = false;
        s.run(code).unwrap();
        let q = &s.model_mut().wrath_lfg.queue;
        (q.selection.clone(), q.pending)
    };
    assert_eq!(
        pick(&mut s, "SetLFGDungeon(9) SetLFGDungeon(7)"),
        (vec![1 << 24 | 7], true)
    );
    assert_eq!(pick(&mut s, "SetLFGDungeon(9)"), (vec![2 << 24 | 9], true));
    assert_eq!(pick(&mut s, "SetLFGDungeon(9)"), (vec![2 << 24 | 9], false));
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Pal".into(),
            guid: 2,
        }],
        ..Default::default()
    });
    assert_eq!(pick(&mut s, "SetLFGDungeon(9)"), (vec![2 << 24 | 9], true));
    s.set_party(PartyState::default());
    // `0x5593a0` drops the named entry alone; `0x552e90` shows the local state.
    let local = "local a, b, c, d, e, f, g, h = GetLFGInfoLocal() \
                 return select('#', GetLFGInfoLocal()), a, b, c, #d, e, f + g + h";
    assert_eq!(
        s.eval::<(i64, bool, bool, bool, i64, i64, i64)>(local)
            .unwrap(),
        (8, false, false, false, 63, 1, 0)
    );
    assert_eq!(
        pick(&mut s, "ClearLFGDungeon(1, 9) ClearLFGDungeon(2, 7)"),
        (vec![2 << 24 | 9], false)
    );
    assert_eq!(pick(&mut s, "ClearLFGDungeon(2, '9')"), (vec![], true));
    assert!(s
        .run("ClearLFGDungeon('x', {})")
        .unwrap_err()
        .to_string()
        .contains("Usage: ClearLFGDungeon(type, id)"));

    // Role check: the counter starts at 2 (`0xacdd54`), so the first call tests the clock and
    // resets it; two more pass, then nothing within ten seconds.
    s.run("assert(CompleteLFGRoleCheck(true)==true); assert(CompleteLFGRoleCheck(false)==true); assert(CompleteLFGRoleCheck(true)==true); assert(CompleteLFGRoleCheck(true)==nil)").unwrap();
    let answers = s.take_wrath_lfg_queue_requests();
    assert_eq!(answers.len(), 3);
    assert_eq!(answers[1], R::RoleCheckAnswer { roles: 0 });
    assert_eq!(answers[2], R::RoleCheckAnswer { roles: 3 });

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

    // Teleport: `0x71afb0` tests the pet-in-combat flag `0x800` or a swing target, not the
    // in-combat flag; then fatigue (`0x513f20(0)`), each refusal an error and false.
    s.run("assert(LFGTeleport(true)==true)").unwrap();
    let player = |flags| UnitState {
        guid: 1,
        exists: true,
        in_combat: true,
        flags,
        ..Default::default()
    };
    s.set_unit("player", Some(player(0)));
    s.run("assert(LFGTeleport(false)==true)").unwrap();
    s.set_unit("player", Some(player(0x800)));
    s.run("assert(LFGTeleport(false)==false)").unwrap();
    s.set_unit("player", Some(player(0)));
    // Three calls a window: the next two start a fresh one.
    s.model_mut().wrath_lfg.queue.teleport = Gate::INITIAL;
    s.model_mut().current_spells = vec![6603];
    s.run("assert(LFGTeleport(false)==false)").unwrap();
    s.model_mut().current_spells.clear();
    let drain = crate::script::WrathMirrorTimer {
        value: 60_000,
        max_value: 60_000,
        scale: -1,
        paused: false,
        label: String::new(),
        started: 0.0,
    };
    s.set_wrath_mirror_timer(0, Some(drain));
    s.run("assert(LFGTeleport(false)==false)").unwrap();
    s.set_wrath_mirror_timer(0, None);
    assert_eq!(
        s.take_ui_errors(),
        vec![
            "ERR_NOT_IN_COMBAT",
            "ERR_NOT_IN_COMBAT",
            "ERR_NOT_WHILE_FATIGUED"
        ]
    );
    assert_eq!(
        s.take_wrath_lfg_queue_requests(),
        vec![R::Teleport { out: true }, R::Teleport { out: false }]
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
    // `0x55a0c0`: nothing without a party to backfill (`0xbd1998`), else true.
    s.run("assert(select('#', PartyLFGStartBackfill())==0)")
        .unwrap();
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Pal".into(),
            guid: 2,
        }],
        ..Default::default()
    });
    s.run("SetLFGDungeon(9) assert(PartyLFGStartBackfill()==true)")
        .unwrap();
    match s.take_wrath_lfg_queue_requests().as_slice() {
        [R::Join {
            dungeons, roles, ..
        }] => assert_eq!((dungeons.clone(), *roles), (vec![1 << 24 | 7], 3)),
        other => panic!("{other:?}"),
    }
    s.run("SetLFGRoles(true) assert(select('#', PartyLFGStartBackfill())==0)")
        .unwrap();
    assert_eq!(s.take_ui_errors(), vec!["ERR_LFG_NO_ROLES_SELECTED"]);
    // A role bit past the byte still passes the role test; the join send then masks it away.
    s.run("SetCVar('lfgSelectedRoles', '256') assert(PartyLFGStartBackfill()==true)")
        .unwrap();
    assert!(s.take_ui_errors().is_empty() && s.take_wrath_lfg_queue_requests().is_empty());
}
