//! Group session state: the `SMSG_GROUP_LIST` mirror and the party system lines. vmangos sends no
//! text for party events, so, as the reference does through `DisplayError` (`0x496720`), each
//! opcode composes its own line and `SMSG_GROUP_LIST` composes joins and leaves from what the list
//! finds held: the party slots (`0x5e6a40`) and the raid roster (`0x4ba5f0`, `0x4ba550`).

#[cfg(test)]
use crate::app_core::{errors::UiError, group::ListOutcome};
use bevy::prelude::*;
#[cfg(test)]
use wrathplus_protocol::types::{
    party_operation, party_result, GroupMemberEntry, PartyMemberStatsInfo,
};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;

/// An unstreamed member's race, class and sex off the name cache.
mod cached_traits;
pub mod feed;
pub(crate) mod net;
mod pets;
mod ready_close;
mod records;
pub(crate) mod wrath;
pub(crate) use feed::{
    synthetic_raid, synthetic_roster, GROUPTYPE_RAID, PARTY_TOKENS, RAID_PET_TOKENS, RAID_TOKENS,
};

pub struct UiPartyPlugin;

impl Plugin for UiPartyPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        // A UI-only harness has no sound stack; the list handler queues the join chime.
        app.init_resource::<GroupState>()
            .init_resource::<crate::sound::MessageSounds>()
            .add_systems(
                Update,
                (
                    feed::feed_party.in_set(UiFeed),
                    feed::drain_party.after(UiInput),
                    ready_close::close_own_ready_check,
                ),
            );
    }
}

#[cfg(test)]
mod unstreamed_tests;
pub(crate) use crate::app_core::group::GroupState;

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, guid: u64) -> GroupMemberEntry {
        GroupMemberEntry {
            name: name.into(),
            guid,
            status: 1,
            flags: 0,
        }
    }

    #[test]
    fn every_list_counts_whatever_it_changes() {
        let mut g = GroupState::default();
        let list = |g: &mut GroupState, members: Vec<GroupMemberEntry>, leader| {
            g.apply_list(0, 0, members, leader, None, Some(ME));
            g.lists_applied
        };
        assert_eq!(g.lists_applied, 0, "no list yet");

        let alice = member("Alice", 0xA11CE);
        assert_eq!(list(&mut g, vec![alice.clone()], 0xA11CE), 1);
        assert_eq!(
            list(&mut g, vec![alice.clone()], 0xA11CE),
            2,
            "the same list again"
        );
        let away = GroupMemberEntry {
            status: 0,
            ..alice.clone()
        };
        assert_eq!(list(&mut g, vec![away], 0xA11CE), 3, "a status alone");
        assert_eq!(list(&mut g, vec![alice], ME), 4, "a leader alone");
        assert_eq!(list(&mut g, vec![], ME), 5, "a leader with no members");
        assert_eq!(list(&mut g, vec![], 0), 6, "the all-zero list");
        assert_eq!(list(&mut g, vec![], 0), 7, "and another, with nobody held");

        g.leave_group();
        assert_eq!(g.lists_applied, 7, "leaving is not a list");
        g.clear_session();
        assert_eq!(g.lists_applied, 0);
    }

    /// The raid roster writer signals after each rebuild (`0x4babef`), and nothing gates the signal
    /// on a difference from the roster it replaces, so a list that moves a member's status alone, or
    /// nothing, counts.
    #[test]
    fn every_raid_list_counts_a_roster_update_whatever_it_changes() {
        use wrathplus_protocol::types::member_status;
        let mut g = GroupState::default();
        let raid = |g: &mut GroupState, members: Vec<GroupMemberEntry>| {
            g.apply_list(1, 0, members, 1, None, Some(ME));
            g.roster_updates
        };
        assert_eq!(g.roster_updates, 0, "no list yet");

        let (alice, bob) = (sub("Alice", 1, 0), sub("Bob", 2, 1));
        assert_eq!(
            raid(&mut g, vec![alice.clone(), bob.clone()]),
            1,
            "seats the raid"
        );
        assert_eq!(
            raid(&mut g, vec![alice.clone(), bob.clone()]),
            2,
            "the same list again"
        );
        let dead = GroupMemberEntry {
            status: member_status::ONLINE | member_status::DEAD,
            ..bob
        };
        assert_eq!(raid(&mut g, vec![alice.clone(), dead]), 3, "a status alone");
        assert_eq!(raid(&mut g, vec![alice]), 4, "a member gone");
        assert_eq!(
            raid(&mut g, vec![]),
            5,
            "a raid list naming nobody still rebuilds"
        );

        g.leave_group();
        assert_eq!(g.roster_updates, 5, "leaving is not a list");
        g.clear_session();
        assert_eq!(g.roster_updates, 0);
    }

    /// A list that is not a raid runs no roster writer, so a party's list signals nothing; one that
    /// finds a raid held drops it and signals once (`0x4ba57b`), the all-zero list among them, and
    /// with none held there is nothing to say.
    #[test]
    fn a_list_that_is_not_a_raid_counts_a_roster_update_only_for_the_raid_it_drops() {
        let mut g = GroupState::default();
        let list = |g: &mut GroupState, group_type: u8, members: Vec<GroupMemberEntry>, leader| {
            g.apply_list(group_type, 0, members, leader, None, Some(ME));
            g.roster_updates
        };
        let party = vec![sub("Alice", 1, 0)];

        assert_eq!(list(&mut g, 0, party.clone(), 1), 0, "a party's list");
        assert_eq!(list(&mut g, 0, party.clone(), 1), 0, "and again");
        assert_eq!(
            list(&mut g, 0, vec![], 0),
            0,
            "the all-zero list with no raid held"
        );

        assert_eq!(
            list(&mut g, 1, party.clone(), 1),
            1,
            "the party becomes a raid"
        );
        assert_eq!(
            list(&mut g, 0, party.clone(), 1),
            2,
            "the raid becomes a party"
        );
        assert_eq!(list(&mut g, 0, party.clone(), 1), 2, "nothing held to drop");

        assert_eq!(list(&mut g, 1, party, 1), 3);
        assert_eq!(
            list(&mut g, 0, vec![], 0),
            4,
            "the all-zero list ends the raid"
        );
        assert_eq!(
            list(&mut g, 0, vec![], 0),
            4,
            "and a second has none to end"
        );
    }

    /// The rebuild counts the rows whose name is still to come and signals only with none
    /// (`0x4babdf`); the list lands all the same, and a list that drops a raid asks no name.
    #[test]
    fn a_raid_list_with_a_name_pending_counts_no_roster_update() {
        let mut g = GroupState::default();
        let raid = vec![sub("Alice", 1, 0), sub("Bob", 2, 1)];
        let awaiting = |g: &mut GroupState, group_type: u8, pending: bool| {
            g.apply_list_awaiting(group_type, 0, raid.clone(), 1, None, Some(ME), pending);
            g.roster_updates
        };

        assert_eq!(awaiting(&mut g, 1, true), 0, "a name pending");
        assert_eq!(
            g.group_type, GROUPTYPE_RAID,
            "the roster is rebuilt regardless"
        );
        assert_eq!(g.members.len(), 2);
        assert_eq!(awaiting(&mut g, 1, false), 1, "every name held");
        assert_eq!(awaiting(&mut g, 1, true), 1, "pending again");
        assert_eq!(
            awaiting(&mut g, 0, true),
            2,
            "the drop does not wait on a name"
        );
    }

    #[test]
    fn leaving_the_group_keeps_the_session_tickets() {
        let mut g = GroupState {
            in_group: true,
            leader: 0xA11CE,
            members: vec![member("Alice", 0xA11CE)],
            ..Default::default()
        };
        g.raid_targets[0] = 0xA11CE;
        g.apply_ready_check_request(false);
        assert_eq!((g.ready_check, g.ready_check_requests), (1, 1));

        g.apply_list(0, 0, vec![], 0, None, None);

        assert!(!g.in_group);
        assert!(g.members.is_empty());
        assert_eq!(g.leader, 0);
        assert_eq!(g.raid_targets, [0; 8]);
        assert_eq!(
            (g.ready_check, g.ready_check_requests),
            (1, 1),
            "no READY_CHECK edge"
        );

        g.clear_session();
        assert_eq!(g.ready_check, 0);
    }

    #[test]
    fn the_open_form_pops_for_a_member_and_stays_quiet_for_the_leader() {
        let mut g = GroupState {
            leader: 0xA11CE,
            members: vec![member("Alice", 0xA11CE), member("Bob", 0xB0B)],
            ..Default::default()
        };
        g.apply_ready_check_answer(0xB0B, true);

        assert_eq!(
            g.apply_ready_check_request(false),
            vec![UiError::s("ERR_RAID_LEADER_READY_CHECK_START_S", "Alice")]
        );
        assert_eq!((g.ready_check, g.ready_check_requests), (1, 1));
        assert!(
            g.ready_check_answers.is_empty(),
            "a request starts a fresh log"
        );

        assert!(
            g.apply_ready_check_request(true).is_empty(),
            "the leader's echo"
        );
        assert_eq!(
            (g.ready_check, g.ready_check_requests),
            (1, 2),
            "no popup for the leader"
        );

        g.apply_ready_check_answer(0xB0B, false);
        assert_eq!(g.ready_check_answers, vec![(0xB0B, false)]);
    }

    /// Our own guid, for the lists that ask whether we lead.
    const ME: u64 = 0x5E1F;

    fn sub(name: &str, guid: u64, subgroup: u8) -> GroupMemberEntry {
        GroupMemberEntry {
            flags: subgroup,
            ..member(name, guid)
        }
    }

    fn lines(lines: Vec<UiError>, invite_accept: bool) -> ListOutcome {
        ListOutcome {
            lines,
            invite_accept,
        }
    }

    #[test]
    fn joining_a_party_names_nobody_already_in_it() {
        let mut g = GroupState::default();
        let party = vec![member("Alice", 1), member("Bob", 2)];
        assert_eq!(
            g.apply_list(0, 0, party.clone(), 1, None, Some(ME)),
            lines(vec![], true),
            "seated into no group: the chime alone"
        );
        assert!(g.in_group);

        let grown = [party.clone(), vec![member("Carol", 3)]].concat();
        assert_eq!(
            g.apply_list(0, 0, grown.clone(), 1, None, Some(ME)),
            lines(vec![UiError::s("ERR_JOINED_GROUP_S", "Carol")], true),
            "a member new to a party we held"
        );
        assert_eq!(
            g.apply_list(0, 0, party.clone(), 1, None, Some(ME)),
            lines(vec![UiError::s("ERR_LEFT_GROUP_S", "Carol")], false)
        );
        // An unchanged resync (loot/status churn re-sends the list) shows nothing.
        assert_eq!(
            g.apply_list(0, 0, party, 1, None, Some(ME)),
            ListOutcome::default()
        );
    }

    #[test]
    fn a_group_we_form_names_the_member_the_list_names_last() {
        let mut g = GroupState::default();
        // A solo leader's list names nobody, and holds no group.
        assert_eq!(
            g.apply_list(0, 0, vec![], ME, None, Some(ME)),
            ListOutcome::default()
        );
        assert_eq!(
            g.apply_list(0, 0, vec![member("Alice", 1)], ME, None, Some(ME)),
            lines(vec![UiError::s("ERR_JOINED_GROUP_S", "Alice")], true)
        );

        let mut g = GroupState::default();
        assert_eq!(
            g.apply_list(
                0,
                0,
                vec![member("Alice", 1), member("Bob", 2)],
                ME,
                None,
                Some(ME)
            ),
            lines(vec![UiError::s("ERR_JOINED_GROUP_S", "Bob")], true),
            "one line, for the last member"
        );
        // Not ours: nobody is named.
        let mut g = GroupState::default();
        assert_eq!(
            g.apply_list(0, 0, vec![member("Alice", 1)], 1, None, Some(ME)),
            lines(vec![], true)
        );
    }

    #[test]
    fn an_opcode_that_ends_the_group_leaves_the_empty_list_nobody_to_name() {
        type Ending = fn(&mut GroupState) -> Vec<UiError>;
        let endings: [(Ending, &str); 3] = [
            (
                |g| {
                    g.apply_command_result(party_operation::LEAVE, "Us", party_result::OK)
                        .into_iter()
                        .collect()
                },
                "ERR_LEFT_GROUP_YOU",
            ),
            (|g| g.apply_uninvited(), "ERR_UNINVITE_YOU"),
            (|g| g.apply_destroyed(), "ERR_GROUP_DISBANDED"),
        ];
        for (end, key) in endings {
            let mut g = GroupState::default();
            g.apply_list(
                0,
                0,
                vec![member("Alice", 1), member("Bob", 2)],
                1,
                None,
                None,
            );
            assert_eq!(end(&mut g), vec![UiError::key(key)]);
            assert_eq!(
                g.apply_list(0, 0, Vec::new(), 0, None, None),
                ListOutcome::default(),
                "{key} emptied the slots"
            );
            assert!(!g.in_group);
        }

        // Ungrouped, the disband is silent (the `0x4e86d0` gate).
        assert!(GroupState::default().apply_destroyed().is_empty());

        // vmangos's two-member collapse sends nothing first: the survivor sees the leaver go.
        let mut g = GroupState::default();
        g.apply_list(0, 0, vec![member("Alice", 1)], 1, None, None);
        assert_eq!(
            g.apply_list(0, 0, Vec::new(), 0, None, None),
            lines(vec![UiError::s("ERR_LEFT_GROUP_S", "Alice")], false)
        );
    }

    #[test]
    fn joining_a_raid_prints_the_joined_line_alone() {
        let raid = vec![sub("Alice", 1, 0), sub("Bob", 2, 1), sub("Carol", 3, 1)];
        let mut g = GroupState::default();
        assert_eq!(
            g.apply_list(1, 0, raid.clone(), 1, None, Some(ME)),
            lines(vec![UiError::key("ERR_RAID_YOU_JOINED")], true),
            "Alice shares our subgroup, so the chime plays"
        );
        let mut g = GroupState::default();
        assert_eq!(
            g.apply_list(1, 2, raid, 1, None, Some(ME)),
            lines(vec![UiError::key("ERR_RAID_YOU_JOINED")], false),
            "nobody in our subgroup, no chime"
        );
    }

    #[test]
    fn a_raid_list_names_arrivals_and_departures_in_raid_words_alone() {
        let mut g = GroupState::default();
        g.apply_list(
            1,
            0,
            vec![sub("Alice", 1, 0), sub("Bob", 2, 1)],
            1,
            None,
            None,
        );
        // Dave lands in our subgroup: the raid line, no party line and no chime.
        assert_eq!(
            g.apply_list(
                1,
                0,
                vec![sub("Alice", 1, 0), sub("Bob", 2, 1), sub("Dave", 4, 0)],
                1,
                None,
                None
            ),
            lines(vec![UiError::s("ERR_RAID_MEMBER_ADDED_S", "Dave")], false)
        );
        assert_eq!(
            g.apply_list(
                1,
                0,
                vec![sub("Alice", 1, 0), sub("Dave", 4, 0)],
                1,
                None,
                None
            ),
            lines(vec![UiError::s("ERR_RAID_MEMBER_REMOVED_S", "Bob")], false)
        );
    }

    #[test]
    fn leaving_a_raid_prints_the_left_line_alone() {
        let raid = vec![sub("Alice", 1, 0), sub("Bob", 2, 1), sub("Carol", 3, 0)];
        let mut g = GroupState::default();
        g.apply_list(1, 0, raid.clone(), 1, None, None);
        assert_eq!(
            g.apply_command_result(party_operation::LEAVE, "Us", party_result::OK),
            Some(UiError::key("ERR_LEFT_GROUP_YOU"))
        );
        assert_eq!(
            g.apply_list(0, 0, Vec::new(), 0, None, None).lines,
            vec![UiError::key("ERR_RAID_YOU_LEFT")]
        );

        // Removed with no opcode first: our subgroup's slots go in party words, then the raid.
        let mut g = GroupState::default();
        g.apply_list(1, 0, raid, 1, None, None);
        assert_eq!(
            g.apply_list(0, 0, Vec::new(), 0, None, None).lines,
            vec![
                UiError::s("ERR_LEFT_GROUP_S", "Alice"),
                UiError::s("ERR_LEFT_GROUP_S", "Carol"),
                UiError::key("ERR_RAID_YOU_LEFT"),
            ]
        );
    }

    #[test]
    fn a_party_becoming_a_raid_and_back_prints_only_the_raid_edges() {
        let mut g = GroupState::default();
        g.apply_list(0, 0, vec![member("Alice", 1)], 1, None, None);
        assert_eq!(
            g.apply_list(1, 0, vec![member("Alice", 1)], 1, None, None),
            lines(vec![UiError::key("ERR_RAID_YOU_JOINED")], false)
        );
        assert_eq!(
            g.apply_list(0, 0, vec![member("Alice", 1)], 1, None, None),
            lines(vec![UiError::key("ERR_RAID_YOU_LEFT")], false)
        );
    }

    #[test]
    fn party_slots_filter_to_own_subgroup() {
        let mut g = GroupState::default();
        let mut m2 = member("Bob", 2);
        m2.flags = 0x01; // subgroup 1
        let mut m3 = member("Carol", 3);
        m3.flags = 0x80; // subgroup 0, assistant: the filter ignores 0x80
        g.apply_list(1, 0x00, vec![member("Alice", 1), m2, m3], 1, None, None);
        let slots: Vec<&str> = g.party_slots().map(|m| m.name.as_str()).collect();
        assert_eq!(slots, vec!["Alice", "Carol"]);
    }

    #[test]
    fn invite_lines() {
        let mut g = GroupState::default();
        assert_eq!(
            g.apply_invited("Bob"),
            vec![UiError::strings("ERR_INVITED_TO_GROUP_SS", &["Bob", "Bob"])]
        );
        assert_eq!(g.pending_invite.as_deref(), Some("Bob"));
        assert_eq!(
            g.apply_invited_while_grouped("Dan"),
            vec![UiError::strings(
                "ERR_INVITED_ALREADY_IN_GROUP_SS",
                &["Dan", "Dan"]
            )]
        );
        assert_eq!(
            g.pending_invite.as_deref(),
            Some("Bob"),
            "no popup for that one"
        );
        assert_eq!(
            g.apply_command_result(party_operation::INVITE, "Carol", party_result::OK),
            Some(UiError::s("ERR_INVITE_PLAYER_S", "Carol"))
        );
        assert_eq!(
            g.apply_command_result(
                party_operation::INVITE,
                "Carol",
                party_result::ALREADY_IN_GROUP
            ),
            Some(UiError::s("ERR_ALREADY_IN_GROUP_S", "Carol"))
        );
        assert_eq!(
            g.apply_command_result(party_operation::INVITE, "Xz", party_result::BAD_PLAYER_NAME),
            Some(UiError::s("ERR_BAD_PLAYER_NAME_S", "Xz"))
        );
        assert_eq!(
            g.apply_declined("Carol"),
            vec![UiError::s("ERR_DECLINE_GROUP_S", "Carol")]
        );
    }

    #[test]
    fn leader_lines() {
        let mut g = GroupState::default();
        assert_eq!(
            g.apply_leader_changed("Alice", Some("WrathPlus")),
            vec![UiError::s("ERR_NEW_LEADER_S", "Alice")]
        );
        assert_eq!(
            g.apply_leader_changed("WrathPlus", Some("WrathPlus")),
            vec![UiError::key("ERR_NEW_LEADER_YOU")]
        );
    }

    #[test]
    fn stats_merge_and_retention() {
        let mut g = GroupState::default();
        g.apply_list(0, 0, vec![member("Alice", 1)], 1, None, None);
        g.apply_stats(
            1,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(50),
                max_hp: Some(100),
                ..Default::default()
            },
        );
        g.apply_stats(
            1,
            false,
            PartyMemberStatsInfo {
                cur_hp: Some(60),
                ..Default::default()
            },
        );
        let s = g.stats.get(&1).unwrap();
        assert_eq!(s.cur_hp, Some(60));
        assert_eq!(s.max_hp, Some(100), "delta merge keeps unmentioned fields");

        g.apply_stats(
            1,
            true,
            PartyMemberStatsInfo {
                cur_hp: Some(70),
                ..Default::default()
            },
        );
        let s = g.stats.get(&1).unwrap();
        assert_eq!(s.cur_hp, Some(70));
        assert_eq!(s.max_hp, None, "FULL replaces the snapshot outright");

        g.apply_list(0, 0, Vec::new(), 0, None, None);
        assert!(g.stats.is_empty());
    }

    #[test]
    fn raid_target_board() {
        let mut g = GroupState::default();
        g.apply_raid_target(7, 0x99); // skull
        assert_eq!(g.raid_targets[7], 0x99);
        g.apply_raid_target(7, 0); // clear
        assert_eq!(g.raid_targets[7], 0);
        g.apply_raid_target(3, 0x11);
        g.apply_raid_target_list(&[(0, 0x22), (5, 0x33)]);
        assert_eq!(g.raid_targets[0], 0x22);
        assert_eq!(g.raid_targets[5], 0x33);
        assert_eq!(g.raid_targets[3], 0, "the list form resets absent icons");
    }

    #[test]
    fn a_raid_to_party_conversion_clears_the_raid_target_board() {
        let mut g = GroupState::default();
        g.apply_list(1, 0, vec![member("Ally", 0x22)], 0x22, None, None);
        g.apply_raid_target(7, 0x22);
        assert_eq!(g.raid_targets[7], 0x22, "marked while a raid");

        g.apply_list(
            1,
            0,
            vec![member("Ally", 0x22), member("Bee", 0x33)],
            0x22,
            None,
            None,
        );
        assert_eq!(
            g.raid_targets[7], 0x22,
            "a roster change inside a raid keeps the marks"
        );

        g.apply_list(0, 0, vec![member("Ally", 0x22)], 0x22, None, None);
        assert_eq!(
            g.raid_targets, [0; 8],
            "the raid flag clearing empties the board"
        );
    }

    /// Asserted by message id, since the English cannot tell a right key from a wrong one: build
    /// 12340's ids (`0xac8af0`) for the rows the 1.12 arms pushed; the 12340 arms are untraced.
    #[test]
    fn every_party_result_names_the_message_id_the_reference_pushes() {
        const TABLE: &[(u32, u16, bool)] = &[
            (party_result::BAD_PLAYER_NAME, 0x4f, true),
            (party_result::TARGET_NOT_IN_GROUP, 0x51, true),
            (party_result::GROUP_FULL, 0x53, false),
            (party_result::ALREADY_IN_GROUP, 0x43, true),
            (party_result::NOT_IN_GROUP, 0x50, false),
            (party_result::NOT_LEADER, 0x54, false),
            (party_result::WRONG_FACTION, 0x10d, false),
            (party_result::IGNORING_YOU, 0x150, true),
        ];
        let mut g = GroupState::default();
        for &(result, id, takes_name) in TABLE {
            let msg = g
                .apply_command_result(party_operation::INVITE, "Zed", result)
                .unwrap_or_else(|| panic!("result {result} showed nothing"));
            let row = wrathplus_ui::messages::by_key(msg.key)
                .unwrap_or_else(|| panic!("result {result} named {}, not a catalog row", msg.key));
            assert_eq!(row.id, id, "result {result} -> {} (id {})", msg.key, row.id);
            // The arms the reference passes the name to are exactly the `_S` keys.
            assert_eq!(
                msg.arg_s().is_some(),
                takes_name,
                "result {result} name fill"
            );
            assert_eq!(
                msg.key.ends_with("_S"),
                takes_name,
                "result {result} _S suffix"
            );
        }

        // The two `OK` arms, where the operation decides.
        let mut ok = |op| g.apply_command_result(op, "Zed", party_result::OK);
        assert_eq!(
            wrathplus_ui::messages::by_key(ok(party_operation::INVITE).unwrap().key)
                .unwrap()
                .id,
            0x3f
        );
        assert_eq!(
            wrathplus_ui::messages::by_key(ok(party_operation::LEAVE).unwrap().key)
                .unwrap()
                .id,
            0x4a
        );
    }

    #[test]
    fn the_wrong_faction_refusal_is_the_red_line_and_the_rest_are_chat() {
        use wrathplus_ui::messages::MsgKind;
        let mut g = GroupState::default();
        let mut kind = |r| {
            wrathplus_ui::messages::kind_of(
                g.apply_command_result(party_operation::INVITE, "Zed", r)
                    .unwrap()
                    .key,
            )
        };
        assert_eq!(kind(party_result::WRONG_FACTION), MsgKind::Error);
        for r in [
            party_result::BAD_PLAYER_NAME,
            party_result::TARGET_NOT_IN_GROUP,
            party_result::GROUP_FULL,
            party_result::ALREADY_IN_GROUP,
            party_result::NOT_IN_GROUP,
            party_result::NOT_LEADER,
            party_result::IGNORING_YOU,
        ] {
            assert_eq!(kind(r), MsgKind::Chat, "result {r} should be a chat line");
        }
    }

    /// Three inputs show nothing: the reference's default arm (`0x5e6a06`) returns with no call.
    #[test]
    fn the_silent_inputs_show_nothing() {
        let mut g = GroupState::default();
        // 1: every result past the table (`dec eax; cmp eax,7; ja`, unsigned).
        for r in [9u32, 10, 42, u32::MAX] {
            assert!(
                g.apply_command_result(party_operation::INVITE, "Zed", r)
                    .is_none(),
                "result {r} must be silent"
            );
        }
        // 2: `OK` for an operation other than invite or leave.
        for op in [1u32, 3, 99] {
            assert!(g
                .apply_command_result(op, "Zed", party_result::OK)
                .is_none());
        }
        // 3: an invite ack with no name (`0x5e6923`).
        assert!(g
            .apply_command_result(party_operation::INVITE, "", party_result::OK)
            .is_none());
        // A named one still prints: the guard is the name.
        assert!(g
            .apply_command_result(party_operation::INVITE, "Zed", party_result::OK)
            .is_some());
    }

    #[test]
    fn party_result_keys_resolve_to_the_real_1_12_sentences() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");

        let mut g = GroupState::default();
        for r in [
            party_result::BAD_PLAYER_NAME,
            party_result::TARGET_NOT_IN_GROUP,
            party_result::GROUP_FULL,
            party_result::ALREADY_IN_GROUP,
            party_result::NOT_IN_GROUP,
            party_result::NOT_LEADER,
            party_result::WRONG_FACTION,
            party_result::IGNORING_YOU,
        ] {
            let msg = g
                .apply_command_result(party_operation::INVITE, "Zed", r)
                .unwrap();
            let text: String = s.lua().globals().get(msg.key).expect(msg.key);
            assert!(!text.is_empty(), "{} resolves empty", msg.key);
            assert_eq!(
                text.contains("%s"),
                msg.arg_s().is_some(),
                "{} vs its fill",
                msg.key
            );
        }
    }

    #[test]
    fn the_roster_lines_resolve_and_carry_their_catalog_row() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");

        let mut g = GroupState::default();
        let mut lines = Vec::new();
        // Driven through every producer, so a new line cannot skip the checks below.
        let mut list =
            |g: &mut GroupState, group_type: u8, members: &[(&str, u64)], leader: u64| {
                let members = members.iter().map(|(n, guid)| member(n, *guid)).collect();
                lines.extend(
                    g.apply_list(group_type, 0, members, leader, None, None)
                        .lines,
                );
            };
        list(&mut g, 0, &[("Alice", 1)], 1);
        list(&mut g, 0, &[("Alice", 1), ("Bob", 2)], 1);
        list(&mut g, 0, &[("Alice", 1)], 1);
        list(&mut g, 1, &[("Alice", 1)], 1);
        list(&mut g, 1, &[("Alice", 1), ("Dave", 4)], 1);
        list(&mut g, 1, &[("Alice", 1)], 1);
        lines.extend(g.apply_invited("Bob"));
        lines.extend(g.apply_invited_while_grouped("Dan"));
        lines.extend(g.apply_declined("Carol"));
        lines.extend(g.apply_leader_changed("Alice", Some("Us")));
        lines.extend(g.apply_leader_changed("Us", Some("Us")));
        lines.extend(g.apply_uninvited());
        lines.extend(g.apply_destroyed());
        g.leader = 1;
        lines.extend(g.apply_ready_check_request(false));
        lines.extend(g.apply_list(0, 0, Vec::new(), 0, None, None).lines);

        let mut keys: Vec<&str> = lines.iter().map(|m| m.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys,
            [
                "ERR_DECLINE_GROUP_S",
                "ERR_GROUP_DISBANDED",
                "ERR_INVITED_ALREADY_IN_GROUP_SS",
                "ERR_INVITED_TO_GROUP_SS",
                "ERR_JOINED_GROUP_S",
                "ERR_LEFT_GROUP_S",
                "ERR_NEW_LEADER_S",
                "ERR_NEW_LEADER_YOU",
                "ERR_RAID_LEADER_READY_CHECK_START_S",
                "ERR_RAID_MEMBER_ADDED_S",
                "ERR_RAID_MEMBER_REMOVED_S",
                "ERR_RAID_YOU_JOINED",
                "ERR_RAID_YOU_LEFT",
                "ERR_UNINVITE_YOU",
            ],
            "all fourteen keys this window can raise are exercised below"
        );

        for msg in &lines {
            let text: String = s.lua().globals().get(msg.key).expect(msg.key);
            assert!(!text.is_empty(), "{} resolves empty", msg.key);
            assert!(
                wrathplus_ui::messages::by_key(msg.key).is_some(),
                "{} is not a catalog row, so its surface and sound would be a guess",
                msg.key
            );
            assert_eq!(
                text.contains("%s"),
                msg.arg_s().is_some(),
                "{} vs its fill",
                msg.key
            );
        }
        assert_eq!(
            wrathplus_ui::messages::by_key("ERR_DECLINE_GROUP_S").and_then(|r| r.sound),
            Some("igPlayerInviteDecline"),
            "the cue the chat-log path was dropping"
        );
    }
}
