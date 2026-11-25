use super::*;

#[test]
fn a_unit_targeted_cast_writes_count_spell_flags_mask_and_packed_guid() {
    let body = encode_cast_spell(1, 78, &CastTarget::Unit(0x05));
    assert_eq!(body, [1, 78, 0, 0, 0, 0, 2, 0, 0, 0, 0x01, 0x05]);
    let body = encode_cast_spell(2, 6603, &CastTarget::None);
    assert_eq!(body, [2, 0xCB, 0x19, 0, 0, 0, 0, 0, 0, 0]);
    // Resurrection (2006) on a friendly corpse, then a hostile one: mask, then the packed guid.
    let body = encode_cast_spell(3, 2006, &CastTarget::CorpseAlly(0xF100_0000_0000_0007));
    assert_eq!(
        body,
        [3, 0xD6, 0x07, 0, 0, 0, 0x00, 0x80, 0, 0, 0x81, 0x07, 0xF1]
    );
    let body = encode_cast_spell(3, 2006, &CastTarget::CorpseEnemy(0x07));
    assert_eq!(body, [3, 0xD6, 0x07, 0, 0, 0, 0x00, 0x02, 0, 0, 0x01, 0x07]);
}

/// A death knight's Plague Strike: power word, then runes 0b111111 before and 0b111010
/// after, so runes 0 and 2 went down with 0 and 51 of 255 run; an observer gets no runes.
#[test]
fn our_spell_go_rune_list_spends_runes() {
    use crate::wrath_spell_control::RunePacket;
    let mut body = vec![0x01, 0x10, 0x01, 0x10, 1];
    body.extend(45462u32.to_le_bytes());
    body.extend((0x100 | CAST_FLAG_POWER_LEFT_SELF | CAST_FLAG_RUNE_LIST).to_le_bytes());
    body.extend(0u32.to_le_bytes());
    body.extend([0, 0]); // no hits, no misses
    body.extend(0u32.to_le_bytes()); // self-targeted: an empty target mask
    body.extend(100u32.to_le_bytes()); // runic power left
    body.extend([0b11_1111, 0b11_1010, 0, 51]);
    let runes = |self_guid| -> Vec<RunePacket> {
        decode(SMSG_SPELL_GO, &body, Some(self_guid))
            .unwrap()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::WrathRunes { packet } => Some(packet),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        runes(0x10),
        [RunePacket::Spend {
            before: 0b11_1111,
            after: 0b11_1010,
            passed: vec![0, 51]
        }]
    );
    assert!(runes(0x99).is_empty());
    // A truncated tail keeps the spell-go and drops the runes.
    body.truncate(body.len() - 1);
    let events = decode(SMSG_SPELL_GO, &body, Some(0x10)).unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::SpellGo { .. })));
    assert!(!events
        .iter()
        .any(|e| matches!(e, SessionEvent::WrathRunes { .. })));
}

/// A hunter's Auto Shot (75) at unit 0x05: power word, then the ammo (Rough Arrow display 5996,
/// inventory type 24) that the client flies to the target. Spell start carries it the same way.
#[test]
fn ranged_shots_carry_the_ammo_display() {
    let ammo = |op: u16, timestamp: bool| -> Option<u32> {
        let mut body = vec![0x01, 0x10, 0x01, 0x10, 0];
        body.extend(75u32.to_le_bytes());
        body.extend((0x100 | CAST_FLAG_PROJECTILE | CAST_FLAG_POWER_LEFT_SELF).to_le_bytes());
        body.extend(0u32.to_le_bytes()); // timestamp (go) or cast time (start)
        if timestamp {
            body.extend([1, 5, 0, 0, 0, 0, 0, 0, 0, 0]); // one hit (0x05), no misses
        }
        body.extend(TARGET_FLAG_UNIT.to_le_bytes());
        body.extend([0x01, 0x05]);
        body.extend(100u32.to_le_bytes()); // power left
        body.extend(5996u32.to_le_bytes());
        body.extend(24u32.to_le_bytes());
        decode(op, &body, Some(0x10))
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                SessionEvent::SpellGo {
                    ammo_display_id, ..
                }
                | SessionEvent::SpellStart {
                    ammo_display_id, ..
                } => Some(ammo_display_id),
                _ => None,
            })?
    };
    assert_eq!(ammo(SMSG_SPELL_GO, true), Some(5996));
    assert_eq!(ammo(SMSG_SPELL_START, false), Some(5996));
}

#[test]
fn our_spell_go_is_also_the_success_result() {
    // Caster 0x10 twice, count 1, Heroic Strike, flags, timestamp, one hit, no misses,
    // unit target 0x05.
    let mut body = vec![0x01, 0x10, 0x01, 0x10, 1];
    body.extend(78u32.to_le_bytes());
    body.extend(0x100u32.to_le_bytes());
    body.extend(0u32.to_le_bytes());
    body.push(1);
    body.extend(5u64.to_le_bytes());
    body.push(0);
    body.extend(TARGET_FLAG_UNIT.to_le_bytes());
    body.extend([0x01, 0x05]);
    let events = decode(SMSG_SPELL_GO, &body, Some(0x10)).unwrap();
    assert!(matches!(
        &events[..],
        [
            SessionEvent::CastResult {
                spell_id: 78,
                success: true,
                ..
            },
            SessionEvent::SpellGo {
                caster: 0x10,
                target: Some(5),
                ..
            }
        ]
    ));
    let others = decode(SMSG_SPELL_GO, &body, Some(0x99)).unwrap();
    assert_eq!(others.len(), 1);
}

#[test]
fn a_known_player_name_decodes() {
    // Packed guid 0x05 (mask 0x01, byte 0x05), known, "Bob", realm "", human male warrior.
    let body = [0x01, 0x05, 0, b'B', b'o', b'b', 0, 0, 1, 0, 1, 0];
    let events = decode(SMSG_NAME_QUERY_RESPONSE, &body, None).unwrap();
    assert!(matches!(
        &events[..],
        [SessionEvent::PlayerName { guid: 5, name, race: 1, gender: 0, class: 1 }] if name == "Bob"
    ));
}

#[test]
fn action_buttons_unpack_all_144_slots() {
    let mut body = vec![1];
    for slot in 0..ACTION_BUTTONS as u32 {
        body.extend(if slot == 143 { 78u32 } else { 0 }.to_le_bytes());
    }
    let events = decode(SMSG_ACTION_BUTTONS, &body, None).unwrap();
    let [SessionEvent::ActionButtons { buttons }] = &events[..] else {
        panic!()
    };
    assert_eq!(
        (buttons[0].slot, buttons[0].action, buttons[0].kind),
        (143, 78, 0)
    );
}

#[test]
fn initial_spells_keep_ids_past_u16() {
    let mut body = vec![0, 2, 0];
    for id in [6603u32, 70_000] {
        body.extend(id.to_le_bytes());
        body.extend(0u16.to_le_bytes());
    }
    body.extend(0u16.to_le_bytes());
    let events = decode(SMSG_INITIAL_SPELLS, &body, None).unwrap();
    let [SessionEvent::SpellBook { spell_ids, .. }] = &events[..] else {
        panic!()
    };
    assert_eq!(spell_ids, &[6603, 70_000]);
}

/// A running cooldown survives the relog; the server's count may include entries it skipped.
#[test]
fn initial_spells_carry_running_cooldowns_and_tolerate_an_overcount() {
    let mut body = vec![0, 1, 0];
    body.extend(59752u32.to_le_bytes());
    body.extend(0u16.to_le_bytes());
    body.extend(2u16.to_le_bytes()); // two listed, one written
    body.extend(59752u32.to_le_bytes());
    body.extend(0u16.to_le_bytes());
    body.extend(0u16.to_le_bytes());
    body.extend(95_000u32.to_le_bytes());
    body.extend(0u32.to_le_bytes());
    let events = decode(SMSG_INITIAL_SPELLS, &body, None).unwrap();
    let [SessionEvent::SpellBook { cooldowns, .. }] = &events[..] else {
        panic!()
    };
    assert_eq!(cooldowns.len(), 1);
    assert_eq!(
        (cooldowns[0].spell_id, cooldowns[0].spell_cd_ms),
        (59752, 95_000)
    );
}

#[test]
fn a_pet_cast_is_the_casters_full_guid_then_the_cast_body() {
    let at = [1.0f32, 2.0, 3.0];
    let body = encode_pet_cast_spell(0xF140_0000_0000_0050, 7, 31707, &CastTarget::Dest(at));
    assert_eq!(&body[..8], &0xF140_0000_0000_0050u64.to_le_bytes());
    assert_eq!(
        &body[8..],
        encode_cast_spell(7, 31707, &CastTarget::Dest(at)).as_slice()
    );
    // count, spell, flags 0, DEST mask, a packed zero transport guid, xyz.
    assert_eq!(body[8], 7);
    assert_eq!(&body[13..18], &[0, 0x40, 0, 0, 0]);
    assert_eq!(body[18], 0);
    assert_eq!(body.len(), 8 + 1 + 4 + 1 + 4 + 1 + 12);
}

#[test]
fn spell_failed_other_keeps_cast_count_and_reason_with_strict_framing() {
    // Packed caster 0x42, cast count 3, spell 133, result 0x28.
    let body = [0x01, 0x42, 3, 0x85, 0, 0, 0, 0x28];
    let events = decode(SMSG_SPELL_FAILED_OTHER, &body, None).unwrap();
    let [SessionEvent::SpellFailedOther {
        caster,
        spell_id,
        cast_count,
        reason,
    }] = &events[..]
    else {
        panic!()
    };
    assert_eq!(
        (*caster, *spell_id, *cast_count, *reason),
        (0x42, 133, Some(3), Some(0x28))
    );
    for end in 0..body.len() {
        assert!(
            decode(SMSG_SPELL_FAILED_OTHER, &body[..end], None).is_err(),
            "prefix {end}"
        );
    }
    assert!(decode(
        SMSG_SPELL_FAILED_OTHER,
        &[body.as_slice(), &[0]].concat(),
        None
    )
    .is_err());
}

#[test]
fn xp_gain_reads_the_recruit_a_friend_byte_and_party_kill_and_exploration_are_strict() {
    let mut kill = 0x42u64.to_le_bytes().to_vec();
    kill.extend(120u32.to_le_bytes());
    kill.push(0);
    kill.extend(100u32.to_le_bytes());
    kill.extend(1.0f32.to_le_bytes());
    kill.push(0); // refer-a-friend
    let [SessionEvent::XpGain(x)] = &decode(SMSG_LOG_XPGAIN, &kill, None).unwrap()[..] else {
        panic!("one gain")
    };
    assert_eq!((x.victim, x.total, x.base, x.kill), (0x42, 120, 100, true));
    let mut other = 0u64.to_le_bytes().to_vec();
    other.extend(50u32.to_le_bytes());
    other.extend([1, 0]);
    let [SessionEvent::XpGain(x)] = &decode(SMSG_LOG_XPGAIN, &other, None).unwrap()[..] else {
        panic!("one gain")
    };
    assert_eq!((x.total, x.base, x.kill), (50, 50, false));
    // The 1.12 shape (no refer-a-friend byte) and an extra byte are both refused.
    assert!(decode(SMSG_LOG_XPGAIN, &other[..other.len() - 1], None).is_err());
    assert!(decode(SMSG_LOG_XPGAIN, &[&other[..], &[0]].concat(), None).is_err());

    let pk = [1u64.to_le_bytes(), 2u64.to_le_bytes()].concat();
    assert!(matches!(
        &decode(SMSG_PARTYKILLLOG, &pk, None).unwrap()[..],
        [SessionEvent::PartyKillLog(l)] if (l.killer, l.victim) == (1, 2)
    ));
    let ex = [12u32.to_le_bytes(), 300u32.to_le_bytes()].concat();
    assert!(matches!(
        &decode(SMSG_EXPLORATION_EXPERIENCE, &ex, None).unwrap()[..],
        [SessionEvent::ExplorationXp(e)] if (e.area_id, e.xp) == (12, 300)
    ));
    assert!(decode(SMSG_EXPLORATION_EXPERIENCE, &ex[..7], None).is_err());
}

#[test]
fn a_creature_query_reads_acs_whole_body_and_the_unknown_entry() {
    let mut b = 3100u32.to_le_bytes().to_vec();
    for text in ["Kobold Vermin", "", "", "", "", "Directions"] {
        b.extend(text.as_bytes());
        b.push(0);
    }
    for v in [0x10u32, 7, 0, 1, 6, 9, 3140, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    b.extend(1.0f32.to_le_bytes());
    b.extend(1.0f32.to_le_bytes());
    b.push(0);
    let short = b.clone();
    for v in [1307u32, 750, 42, 43, 44, 45].iter().chain(&[100]) {
        b.extend(v.to_le_bytes());
    }
    let events = decode(SMSG_CREATURE_QUERY_RESPONSE, &b, None).unwrap();
    let [SessionEvent::CreatureName {
        entry: 3100,
        name: Some(name),
        subname: None,
        creature_type: Some(7),
        rank: 1,
        display_id: 3140,
        quest_links,
        ..
    }] = &events[..]
    else {
        panic!("one creature: {events:?}")
    };
    assert_eq!(name, "Kobold Vermin");
    assert_eq!(quest_links.kill_credit, [6, 9]);
    assert_eq!(quest_links.quest_items, [1307, 750, 42, 43, 44, 45]);
    for n in 0..b.len() {
        assert!(
            decode(SMSG_CREATURE_QUERY_RESPONSE, &b[..n], None).is_err(),
            "prefix {n}"
        );
    }
    assert!(
        decode(SMSG_CREATURE_QUERY_RESPONSE, &short, None).is_err(),
        "the quest items and movement id are part of the body"
    );
    assert!(decode(SMSG_CREATURE_QUERY_RESPONSE, &[&b[..], &[0]].concat(), None).is_err());
    let gone = decode(
        SMSG_CREATURE_QUERY_RESPONSE,
        &(3100u32 | 0x8000_0000).to_le_bytes(),
        None,
    )
    .unwrap();
    assert!(matches!(
        &gone[..],
        [SessionEvent::CreatureName { entry: 3100, name: None, quest_links, .. }]
            if *quest_links == crate::types::CreatureQuestLinks::default()
    ));
}
