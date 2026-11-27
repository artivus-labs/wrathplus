//! The dungeon finder's census state: a catalog, the lock and queue snapshots, and an LFG party.

use wrathplus_ui::script::*;

use super::ME;

pub(super) fn seed(s: &mut UiScript) {
    // The dungeon finder: a catalog row, a queue and a proposal.
    s.set_wrath_lfg_catalog(
        std::sync::Arc::new(WrathLfgCatalog {
            dungeons: vec![
                WrathLfgDungeon {
                    id: 1,
                    name: "Ragefire Chasm".into(),
                    dungeon_type: 1,
                    min_level: 15,
                    target_min_level: 15,
                    target_max_level: 21,
                    ..Default::default()
                },
                WrathLfgDungeon {
                    id: 2,
                    name: s.eval("return LFG_TYPE_RANDOM_DUNGEON").unwrap_or_default(),
                    dungeon_type: 6,
                    min_level: 15,
                    target_min_level: 15,
                    target_max_level: 80,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        Some(2),
    );
    s.set_wrath_lfg_lock_snapshot(std::sync::Arc::new(WrathLfgLockSnapshot {
        player: Some(WrathLfgPlayerLocks {
            locks: vec![WrathLfgLock {
                entry: 0x0100_0001,
                reason: 2,
            }],
            // The reward preview sits on the random row, the rows `GetLFGDungeonRewardInfo` reads.
            previews: vec![WrathLfgPreview {
                entry: 0x0600_0002,
                money_base: 100,
                xp_base: 1000,
                items: vec![WrathLfgRewardItem {
                    item: 6948,
                    display: 1,
                    count: 1,
                }],
                ..Default::default()
            }],
        }),
        party: None,
    }));
    s.set_wrath_lfg_queue(std::sync::Arc::new(WrathLfgQueueSnapshot {
        // The record the getters read while grouped (`0x552f34`), as the fixture is below.
        party: Some(WrathLfgUpdate {
            update_type: 5,
            joined: true,
            queued: true,
            dungeons: vec![0x0100_0001],
            ..Default::default()
        }),
        role_check: Some(WrathLfgRoleCheck {
            state: 2,
            is_beginning: false,
            dungeons: vec![0x0100_0001],
            members: vec![WrathLfgRoleCheckMember {
                guid: ME,
                ready: true,
                roles: 2,
                level: 80,
                name: Some("Benchone".into()),
            }],
        }),
        completion: Some(WrathLfgCompletion {
            dungeon: 0x0100_0001,
            strangers: 2,
            money_base: 100,
            xp_base: 1000,
            items: vec![WrathLfgCompletionItem {
                item: 6948,
                display: 1,
                count: 1,
            }],
            ..Default::default()
        }),
        queue: Some(WrathLfgQueueStatus {
            dungeon: 0x0100_0001,
            avg_wait: 300,
            my_wait: 120,
            tank_needs: 1,
            healer_needs: 1,
            dps_needs: 3,
            ..Default::default()
        }),
        proposal: Some(WrathLfgProposal {
            dungeon: 0x0100_0001,
            state: 1,
            id: 7,
            rows: vec![WrathLfgProposalRow {
                roles: 2,
                is_self: true,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }));

    // An LFG party of one other member (`0xbd1998`), leaderless: no party channel shifts the list.
    s.set_wrath_group(WrathGroup {
        lfg_dungeon_entry: 0x0100_0001,
        ..Default::default()
    });
    s.set_party(PartyState {
        members: vec![PartyMemberInfo {
            name: "Pal".into(),
            guid: 0x0100_0000_0000_0003,
        }],
        ..Default::default()
    });
}
