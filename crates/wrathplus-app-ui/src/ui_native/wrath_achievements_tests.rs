use super::*;
use std::collections::VecDeque;
use wrathplus_protocol::wrath_achievement::Criteria;

fn snapshot(earned: &[(u32, u32)], criteria: &[(u32, u64)]) -> Snapshot {
    Snapshot {
        earned: earned.to_vec(),
        criteria: criteria
            .iter()
            .map(|&(id, counter)| Criteria {
                id,
                counter,
                player: 1,
                raw_flags: 0,
                packed_time: 0,
                elapsed: 0,
                creation_elapsed: 0,
            })
            .collect(),
    }
}

#[test]
fn own_receipts_leave_the_capture_and_others_keep_their_order() {
    let me = 0x10;
    let mut state = Achievements::default();
    // Achievement.dbc `Order`; 99 has no row.
    let order = |id: u32| match id {
        99 => None,
        5 => Some(2),
        _ => Some(1),
    };
    let earn = |achievement, packed_time, raw_unknown| Packet::Earned {
        player: me,
        achievement,
        packed_time,
        raw_unknown,
    };
    let other_earn = Packet::Earned {
        player: 0x20,
        achievement: 7,
        packed_time: 1,
        raw_unknown: 0,
    };
    let first = Packet::ServerFirst {
        name: "Guild".into(),
        player: 0x20,
        achievement: 457,
        raw_link: 1,
    };
    let mut packets = VecDeque::from([
        Packet::AllData(snapshot(
            &[(6, 100), (9, 300), (99, 300)],
            &[(40, 3), (41, 5)],
        )),
        other_earn.clone(),
        earn(3, 200, 0),
        earn(8, 200, 0),
        earn(8, 200, 0),
        earn(5, 200, 1),
        first.clone(),
        Packet::CriteriaDeleted { criteria: 41 },
        Packet::CriteriaDeleted { criteria: 41 },
        Packet::CriteriaDeleted { criteria: 50 },
        Packet::AchievementDeleted { achievement: 6 },
        Packet::AchievementDeleted { achievement: 6 },
        Packet::Inspect {
            player: 0x30,
            data: snapshot(&[(1, 1), (2, 1)], &[]),
        },
    ]);
    let out = absorb(&mut packets, &mut state, me, &order, &|id| id == 50);
    assert_eq!(out.lists, [vec![(40, 0), (41, 0)]]);
    assert_eq!(
        out.earned,
        [3, 8, 8],
        "every earn fires; a trailing dword silences one"
    );
    assert_eq!(out.deleted, [6], "a deletion fires when it removed an earn");
    assert_eq!(
        out.criteria_deleted, 2,
        "41 once, and 50 from the live counters"
    );
    assert_eq!(out.inspected.len(), 1, "every response reaches the VM");
    assert_eq!(packets, [other_earn]);
    assert_eq!(
        out.firsts,
        [ServerFirst {
            name: "Guild".into(),
            player: 0x20,
            achievement: 457,
            raw_link: 1,
        }]
    );
    assert!(state.received);
    // Newest first; same-minute earns by `Order`, higher first, then arrival (`0x5b28d0`).
    assert_eq!(state.earned, [(9, 300), (5, 200), (3, 200), (8, 200)]);
    assert_eq!(state.criteria.get(&40), Some(&3));
    assert_eq!(state.deleted, [41, 41, 50]);

    let view = snapshot_progress(&out.inspected[0].1, -1, &order);
    assert_eq!(view.earned, [(1, 1), (2, 1)]);
    let mut packets = VecDeque::from([Packet::Inspect {
        player: 0x30,
        data: snapshot(&[(1, 1)], &[(40, 9)]),
    }]);
    let out = absorb(&mut packets, &mut state, me, &order, &|_| false);
    let (guid, data) = &out.inspected[0];
    assert_eq!(*guid, 0x30);
    assert_eq!(
        snapshot_progress(data, -1, &order).criteria.get(&40),
        Some(&9)
    );
    assert!(packets.is_empty());
}

#[test]
fn races_map_to_the_achievement_factions() {
    assert_eq!(faction(Some(10)), 0, "blood elf is Horde");
    assert_eq!(faction(Some(11)), 1, "draenei is Alliance");
    assert_eq!(faction(None), -1);
}

#[test]
fn a_server_first_is_an_achievement_line_named_by_its_earner() {
    let line = server_first_line(
        "|Hplayer:%s|h[%s]|h has earned the achievement $a!",
        "Arthas",
        0x20,
        457,
    );
    assert_eq!(
        (line.chat_type, line.sender_guid, line.achievement_id),
        (0x30, 0x20, Some(457))
    );
    assert_eq!(line.sender_name.as_deref(), Some(&b"Arthas"[..]));
    assert_eq!(
        line.text,
        b"|Hplayer:Arthas|h[Arthas]|h has earned the achievement $a!"
    );
    let guild = server_first_line("%s has earned the achievement $a!", "Guild", 0x20, 457);
    assert_eq!(guild.text, b"Guild has earned the achievement $a!");
}
