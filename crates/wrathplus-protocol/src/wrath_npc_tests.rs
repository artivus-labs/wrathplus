use super::*;

fn cstr(out: &mut Vec<u8>, s: &str) {
    out.extend(s.as_bytes());
    out.push(0);
}

#[test]
fn a_gossip_menu_keeps_its_options_and_quests_and_its_id_rides_the_select() {
    let mut b = 0x42u64.to_le_bytes().to_vec();
    b.extend(7u32.to_le_bytes()); // menu id
    b.extend(100u32.to_le_bytes()); // text id
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend([3, 0]);
    b.extend(25u32.to_le_bytes());
    cstr(&mut b, "Train me.");
    cstr(&mut b, "Are you sure?");
    b.extend(1u32.to_le_bytes());
    b.extend(783u32.to_le_bytes());
    b.extend(2u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.push(0);
    cstr(&mut b, "A Threat Within");
    let events = decode(SMSG_GOSSIP_MESSAGE, &b).unwrap();
    let [SessionEvent::GossipMenu {
        npc,
        text_id,
        options,
        quests,
        quest_info,
    }] = &events[..]
    else {
        panic!("{events:?}")
    };
    assert_eq!((*npc, *text_id), (0x42, 100));
    let o = &options[0];
    assert_eq!((o.message.as_str(), o.box_money), ("Train me.", 25));
    assert_eq!(o.box_text, "Are you sure?");
    assert_eq!(quest_info.len(), 1);
    assert_eq!(quest_info[0].flags, 0);
    assert_eq!(quests[0], (783, 2, 1, "A Threat Within".to_string()));
    let select = encode_gossip_select_option(0x42, 0, None);
    assert_eq!(&select[8..12], &7u32.to_le_bytes());
    assert_eq!(select.len(), 16, "no code for an uncoded option");
    let coded = encode_gossip_select_option(0x42, 3, Some("1234"));
    assert_eq!(&coded[12..], b"\x03\0\0\x001234\0");
}

#[test]
fn reward_statuses_keep_native_minimap_dot_distinction() {
    use crate::types::dialog_status as d;
    let guid = 0xf130_0001_2300_4567u64;
    for (native, expected) in [(9, d::REWARD_OLD), (10, d::REWARD2)] {
        let mut single = guid.to_le_bytes().to_vec();
        single.push(native);
        assert!(
            matches!(decode(SMSG_QUESTGIVER_STATUS, &single).unwrap()[0],
            SessionEvent::QuestGiverStatus { npc, status } if npc == guid && status == expected)
        );
        let mut multiple = 1u32.to_le_bytes().to_vec();
        multiple.extend(&single);
        assert!(
            matches!(decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &multiple).unwrap()[0],
            SessionEvent::QuestGiverStatus { npc, status } if npc == guid && status == expected)
        );
        for cut in 0..single.len() {
            assert!(decode(SMSG_QUESTGIVER_STATUS, &single[..cut]).is_err());
        }
        for cut in 0..multiple.len() {
            assert!(decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &multiple[..cut]).is_err());
        }
    }
}

#[test]
fn giver_markers_map_onto_the_1_12_statuses() {
    use crate::types::dialog_status as d;
    let mut b = 2u32.to_le_bytes().to_vec();
    b.extend(1u64.to_le_bytes());
    b.push(8);
    b.extend(2u64.to_le_bytes());
    b.push(10);
    let events = decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &b).unwrap();
    assert!(
        matches!(events[0], SessionEvent::QuestGiverStatus { npc: 1, status } if status == d::AVAILABLE)
    );
    assert!(
        matches!(events[2], SessionEvent::QuestGiverStatus { npc: 2, status } if status == d::REWARD2)
    );
}

#[test]
fn quest_details_read_through_the_3_3_5_header_to_money_and_spell() {
    let mut b = 0x42u64.to_le_bytes().to_vec();
    b.extend(0u64.to_le_bytes());
    b.extend(783u32.to_le_bytes());
    cstr(&mut b, "A Threat Within");
    cstr(&mut b, "details");
    cstr(&mut b, "objectives");
    b.push(0);
    b.extend(0u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.push(0);
    b.extend(0u32.to_le_bytes()); // choices
    b.extend(1u32.to_le_bytes()); // rewards
    b.extend([11u32, 1, 99].iter().flat_map(|v| v.to_le_bytes()));
    b.extend(25u32.to_le_bytes()); // money
    b.extend(40u32.to_le_bytes()); // xp
    b.extend(0u32.to_le_bytes()); // honor
    b.extend(0f32.to_le_bytes());
    b.extend(0u32.to_le_bytes()); // spell
    b.extend([0u32, 12, 3, 5].iter().flat_map(|v| v.to_le_bytes()));
    let events = decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b).unwrap();
    let [SessionEvent::QuestDetail(d)] = &events[..] else {
        panic!("{events:?}")
    };
    assert_eq!((d.quest_id, d.money, d.rewards[0].item_id), (783, 25, 11));
    assert_eq!(d.objectives, "objectives");
    let native = d.wrath.as_ref().unwrap();
    assert_eq!(
        (
            native.reward_xp,
            native.title_id,
            native.bonus_talents,
            native.arena_points
        ),
        (40, 12, 3, 5)
    );
    for cut in 0..b.len() {
        assert!(decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b[..cut]).is_err());
    }
}
#[test]
fn greeting_retains_flags_repeatable_and_details_auto_accept_is_not_auto_finish() {
    let mut b = 1u64.to_le_bytes().to_vec();
    cstr(&mut b, "hello");
    b.extend([0u32, 0].iter().flat_map(|v| v.to_le_bytes()));
    b.push(1);
    b.extend([7u32, 4, 80, 0x1000].iter().flat_map(|v| v.to_le_bytes()));
    b.push(1);
    cstr(&mut b, "daily");
    let [SessionEvent::QuestGreeting(list)] = &decode(SMSG_QUESTGIVER_QUEST_LIST, &b).unwrap()[..]
    else {
        panic!()
    };
    let q = list.quests[0].wrath.as_ref().unwrap();
    assert_eq!(q.flags, 0x1000);
    assert!(q.repeatable);

    let mut b = 1u64.to_le_bytes().to_vec();
    b.extend(0u64.to_le_bytes());
    b.extend(7u32.to_le_bytes());
    for text in ["quest", "details", "objectives"] {
        cstr(&mut b, text);
    }
    b.push(1);
    b.extend([0x1000u32, 5].iter().flat_map(|v| v.to_le_bytes()));
    b.push(1);
    b.extend([0u32, 0, 10, 100, 250].iter().flat_map(|v| v.to_le_bytes()));
    b.extend(0.5f32.to_le_bytes());
    b.extend([12u32, 0, 13, 2, 3].iter().flat_map(|v| v.to_le_bytes()));
    let [SessionEvent::QuestDetail(d)] = &decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b).unwrap()[..]
    else {
        panic!()
    };
    assert_eq!(d.auto_finish, 0);
    let q = d.wrath.as_ref().unwrap();
    assert!(q.auto_accept && q.finished);
    assert_eq!(
        (q.suggested_players, q.reward_honor, q.honor_multiplier),
        (5, 250, 0.5)
    );
}
#[test]
fn offer_and_progress_retain_native_columns_at_their_own_offsets() {
    let mut b = 1u64.to_le_bytes().to_vec();
    b.extend(7u32.to_le_bytes());
    cstr(&mut b, "quest");
    cstr(&mut b, "reward");
    b.push(1);
    b.extend(
        [0x1000u32, 5, 1, 25, 4, 0, 0, 100, 900, 250]
            .iter()
            .flat_map(|v| v.to_le_bytes()),
    );
    b.extend(0.5f32.to_le_bytes());
    b.extend([8u32, 12, 0, 13, 2, 3].iter().flat_map(|v| v.to_le_bytes()));
    let [SessionEvent::QuestOffer(o)] = &decode(SMSG_QUESTGIVER_OFFER_REWARD, &b).unwrap()[..]
    else {
        panic!()
    };
    let q = o.wrath.as_ref().unwrap();
    assert_eq!(o.auto_finish, 1);
    assert_eq!(
        (o.money, o.reward_spell, q.flags, q.suggested_players),
        (100, 12, 0x1000, 5)
    );
    assert_eq!(
        (
            q.reward_xp,
            q.reward_honor,
            q.title_id,
            q.bonus_talents,
            q.arena_points
        ),
        (900, 250, 13, 2, 3)
    );
    for cut in 0..b.len() {
        assert!(decode(SMSG_QUESTGIVER_OFFER_REWARD, &b[..cut]).is_err());
    }

    let mut b = 1u64.to_le_bytes().to_vec();
    b.extend(7u32.to_le_bytes());
    cstr(&mut b, "quest");
    cstr(&mut b, "progress");
    b.extend(
        [0u32, 4, 1, 0x1000, 5, 100, 0, 3, 4, 8, 16]
            .iter()
            .flat_map(|v| v.to_le_bytes()),
    );
    let [SessionEvent::QuestProgress(p)] = &decode(SMSG_QUESTGIVER_REQUEST_ITEMS, &b).unwrap()[..]
    else {
        panic!()
    };
    assert!(p.is_complete);
    assert_eq!(p.required_money, 100);
    let q = p.wrath.as_ref().unwrap();
    assert_eq!((q.flags, q.suggested_players), (0x1000, 5));
    for cut in 0..b.len() {
        assert!(decode(SMSG_QUESTGIVER_REQUEST_ITEMS, &b[..cut]).is_err());
    }
}

#[test]
fn a_quest_template_keeps_its_pvp_objective() {
    // The 26 header columns: id, method, level, ..., title id 21, players slain 22.
    let mut header = [0u32; 26];
    header[0] = 9_001;
    header[21] = 7;
    header[22] = 10;
    let mut b: Vec<u8> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
    b.extend([0u8; 4 * (8 + 12 + 15 + 4)]); // rewards, choices, reputations, the POI point
    for s in ["Kill them", "objectives", "details", "", "end"] {
        cstr(&mut b, s);
    }
    b.extend([0u8; 4 * (16 + 12)]); // objective quads, required items
    for _ in 0..4 {
        cstr(&mut b, "");
    }
    let events = decode(SMSG_QUEST_QUERY_RESPONSE, &b).unwrap();
    let [SessionEvent::QuestTemplate(t)] = &events[..] else {
        panic!("one template")
    };
    assert_eq!((t.quest_id, t.title.as_str()), (9_001, "Kill them"));
    let native = t.wrath.as_ref().unwrap();
    assert_eq!((native.title_id, native.players_slain), (7, 10));
    for cut in 0..b.len() {
        assert!(decode(SMSG_QUEST_QUERY_RESPONSE, &b[..cut]).is_err());
    }
}

#[test]
fn the_former_1_12_arms_decode_strictly_and_add_item_is_empty_from_ac() {
    macro_rules! same {
        ($a:expr, $b:expr) => {
            assert_eq!(format!("{:?}", $a), format!("{:?}", $b))
        };
    }
    use SessionEvent as E;
    let one = |op: u16, body: &[u8]| decode(op, body).unwrap();
    same!(one(SMSG_GOSSIP_COMPLETE, &[]), [E::GossipComplete]);
    assert!(decode(SMSG_GOSSIP_COMPLETE, &[0]).is_err());
    same!(one(SMSG_QUESTLOG_FULL, &[]), [E::QuestLogFull]);
    same!(
        one(SMSG_QUESTGIVER_QUEST_INVALID, &13u32.to_le_bytes()),
        [E::QuestGiverInvalid { reason: 13 }]
    );
    same!(
        one(
            SMSG_QUESTGIVER_QUEST_FAILED,
            &[5u32.to_le_bytes(), 4u32.to_le_bytes()].concat()
        ),
        [E::QuestGiverFailed {
            quest_id: 5,
            reason: 4
        }]
    );
    same!(
        one(SMSG_QUESTUPDATE_FAILEDTIMER, &5u32.to_le_bytes()),
        [E::QuestFailed {
            quest_id: 5,
            timed: true
        }]
    );
    same!(
        one(SMSG_QUESTUPDATE_FAILED, &5u32.to_le_bytes()),
        [E::QuestFailed {
            quest_id: 5,
            timed: false
        }]
    );
    same!(
        one(SMSG_QUESTUPDATE_COMPLETE, &5u32.to_le_bytes()),
        [E::QuestObjectivesComplete { quest_id: 5 }]
    );
    let mut kill = Vec::new();
    for v in [5u32, 0x8000_0010, 2, 6] {
        kill.extend(v.to_le_bytes());
    }
    kill.extend(0x99u64.to_le_bytes());
    same!(
        one(SMSG_QUESTUPDATE_ADD_KILL, &kill),
        [E::QuestObjectiveKill {
            quest_id: 5,
            entry: 0x8000_0010,
            count: 2,
            required: 6
        }]
    );
    assert!(
        decode(SMSG_QUESTUPDATE_ADD_KILL, &kill[..16]).is_err(),
        "no guid"
    );
    same!(
        one(SMSG_QUESTUPDATE_ADD_ITEM, &[]),
        [E::QuestObjectiveItem {
            item_id: 0,
            count: 0
        }]
    );
    same!(
        one(
            SMSG_QUESTUPDATE_ADD_ITEM,
            &[2589u32.to_le_bytes(), 3u32.to_le_bytes()].concat()
        ),
        [E::QuestObjectiveItem {
            item_id: 2589,
            count: 3
        }]
    );
    let mut confirm = 5u32.to_le_bytes().to_vec();
    cstr(&mut confirm, "Escort");
    confirm.extend(0x77u64.to_le_bytes());
    assert!(matches!(
        &one(SMSG_QUEST_CONFIRM_ACCEPT, &confirm)[..],
        [E::QuestConfirmAccept(c)] if (c.quest_id, c.title.as_str(), c.sender) == (5, "Escort", 0x77)
    ));
    let mut poi = 99u32.to_le_bytes().to_vec();
    poi.extend(1.5f32.to_le_bytes());
    poi.extend((-2.0f32).to_le_bytes());
    poi.extend(6u32.to_le_bytes());
    poi.extend(0u32.to_le_bytes());
    cstr(&mut poi, "Bank");
    assert!(matches!(
        &one(SMSG_GOSSIP_POI, &poi)[..],
        [E::GossipPoi(p)] if p.pos == [1.5, -2.0] && p.icon == 6 && p.name == "Bank"
    ));
    let push = [&0x66u64.to_le_bytes()[..], &[3]].concat();
    assert!(matches!(
        &one(MSG_QUEST_PUSH_RESULT, &push)[..],
        [E::QuestPushResult { member: 0x66, msg }] if msg.0 == 3
    ));
    let mut text = 77u32.to_le_bytes().to_vec();
    for i in 0..8 {
        text.extend((if i == 0 { 1.0f32 } else { 0.0 }).to_le_bytes());
        cstr(&mut text, if i == 0 { "Hello $N" } else { "" });
        cstr(&mut text, "");
        text.extend([0u8; 4 + 24]);
    }
    let events = one(SMSG_NPC_TEXT_UPDATE, &text);
    let [E::NpcGreeting {
        text_id: 77,
        blocks,
    }] = &events[..]
    else {
        panic!("one greeting")
    };
    same!((blocks.len(), blocks[0].male.as_str()), (8, "Hello $N"));
    assert!(decode(SMSG_NPC_TEXT_UPDATE, &[&text[..], &[0]].concat()).is_err());
}
