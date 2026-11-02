//! Dungeon finder L1: locks and random reward previews (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-LFG-L1-CONTRACT`,
//! lifecycle evidence `https://github.com/artivus-labs/wrathplus/wiki/WRATH-LFG-L1-LIFECYCLE`). This is the sole reducer of the lock
//! receipts in [`super::NativeLfgPackets`]; other LFG families stay queued for their owners.
//!
//! - Receipts: `PLAYER_INFO` (`0x36F`) replaces the self block, `PARTY_INFO` (`0x372`) the party
//!   block; each publishes its snapshot and presentation, then fires `LFG_LOCK_INFO_RECEIVED` and
//!   `LFG_UPDATE_RANDOM_INFO`, once per receipt (`0x55d0cd`, `0x55d0d9`; `0x55d120`, `0x55d12c`).
//!   `LFG_DISABLED` (`0x55c6e0`) invalidates both without events; `LFG_JOIN_RESULT` result 6
//!   replaces and validates the party block without events (`0x55ccbc`).
//! - Session: entering the world is the full initialization `0x55d440` (both blocks invalid) and a
//!   new native-session epoch; a new VM is seeded with the current inputs, never a replay.
//! - Attach (`0x528010` → `0x557240`): each in-game UI construction sends `GET_STATUS`, then the
//!   player lock request, outside the Lua request gates; it clears no validity.
//! - Requests: the UI's process-wide gates decide; App sends each accepted one (`0x36E`/`0x371`,
//!   empty bodies) and injects the OS boot-relative millisecond clock the gates read.
//! - Item callbacks (`0x552950`): every uncached `GetLFGDungeonRewardInfo` call is one interest in
//!   an ordered queue bounded at [`INTEREST_BOUND`], tagged with the session epoch. Answers come
//!   from [`crate::items::Items::take_answers`], in arrival order, after `feed_item_stats` has set
//!   the template: a found answer publishes the presentation, then fires one
//!   `LFG_UPDATE_RANDOM_INFO` per matching interest in order; an unknown entry drops them silently.
//!   Host deviation (labelled): an interest whose template the host already holds but the VM does
//!   not (the two stores are one cache natively) asks the VM's store and completes the next frame.
use std::collections::VecDeque;
use std::sync::Arc;

use bevy::prelude::*;
use wrathplus_protocol::wrath_lfg::{self, DungeonLock, Packet, PartyMemberLocks, PlayerInfo};
use wrathplus_ui::script::{
    UiScript, WrathLfgItemRequest, WrathLfgLock, WrathLfgLockRequest, WrathLfgLockSnapshot,
    WrathLfgMemberLocks, WrathLfgPlayerLocks, WrathLfgPresentation, WrathLfgPreview,
    WrathLfgRewardItem,
};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::app_core::session::SelfPlayer;
use crate::app_core::vm_memo::VmMemo;
use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::names::NameCache;
use crate::net::{ClientCommand, NetCommands};

/// The interest queue's bound; the oldest interest drops, with no event, past it.
pub(crate) const INTEREST_BOUND: usize = 256;

/// The reducer's state, the authority every VM is seeded from.
#[derive(Resource, Default)]
pub(crate) struct LfgL1 {
    snapshot: Arc<WrathLfgLockSnapshot>,
    presentation: Arc<WrathLfgPresentation>,
    /// The native-session epoch: entering or leaving the world cancels older interests.
    epoch: u64,
    interests: VecDeque<(u64, WrathLfgItemRequest)>,
    /// Interests whose template the host held: asked into the VM this frame, answered next.
    host_held: Vec<u32>,
    host_held_next: Vec<u32>,
    /// L2's stores ([`super::l2`]), reset with the session.
    queue: super::l2::LfgQueue,
    /// The snapshot and location last published to the VM.
    queue_pushed: Option<Arc<wrathplus_ui::script::WrathLfgQueueSnapshot>>,
    location: Option<(i32, i32)>,
}

/// The OS boot-relative millisecond count as `u32`, the reference's `GetTickCount` input to the
/// request gates (`0x86d400`); a process-relative clock would reject the first call.
pub(super) fn boot_clock_ms() -> u32 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` only writes the passed `timespec`.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000) as u32
}

/// The reducer's state and its session boundaries; the feed is scheduled by the plugin.
pub(super) fn register(app: &mut App) {
    use crate::net::NetHandlerApp;
    use wrathplus_protocol::SessionEventKind;
    app.init_resource::<LfgL1>()
        .net_handler(SessionEventKind::Connected, reset_session)
        .net_handler(SessionEventKind::Disconnected, reset_session);
}

/// World entry (`0x55d440`) and exit (`0x55af20`): both blocks invalid, a new epoch.
pub(super) fn reset_session(
    In(event): In<wrathplus_protocol::SessionEvent>,
    mut l1: ResMut<LfgL1>,
) {
    use wrathplus_protocol::SessionEvent;
    if !matches!(
        event,
        SessionEvent::Connected { .. } | SessionEvent::Disconnected { .. }
    ) {
        return;
    }
    let epoch = l1.epoch.wrapping_add(1);
    *l1 = LfgL1 {
        epoch,
        ..LfgL1::default()
    };
}

fn locks(rows: &[DungeonLock]) -> Vec<WrathLfgLock> {
    rows.iter()
        .map(|l| WrathLfgLock {
            entry: l.entry,
            reason: l.reason,
        })
        .collect()
}

fn player_block(info: &PlayerInfo) -> WrathLfgPlayerLocks {
    WrathLfgPlayerLocks {
        locks: locks(&info.locks),
        previews: info
            .rewards
            .iter()
            .map(|r| WrathLfgPreview {
                entry: r.entry,
                done: r.done,
                // Wire order `+14 +18 +1c +20`, the getter's base/variance pairs (`0x557aa0`).
                money_base: r.money,
                xp_base: r.xp,
                money_var: r.reserved[0],
                xp_var: r.reserved[1],
                items: r
                    .items
                    .iter()
                    .map(|i| WrathLfgRewardItem {
                        item: i.item,
                        display: i.display,
                        count: i.count,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn party_block(members: &[PartyMemberLocks]) -> Vec<WrathLfgMemberLocks> {
    members
        .iter()
        .map(|m| WrathLfgMemberLocks {
            guid: m.guid,
            locks: locks(&m.locks),
        })
        .collect()
}

/// What one lock-family packet does to the snapshot: `Some(fire)` when it applies, `fire` being
/// the receipt event pair; `None` for any other family.
fn apply(snapshot: &mut WrathLfgLockSnapshot, packet: &Packet) -> Option<bool> {
    match packet {
        Packet::PlayerInfo(info) => {
            snapshot.player = Some(player_block(info));
            Some(true)
        }
        Packet::PartyInfo(members) => {
            snapshot.party = Some(party_block(members));
            Some(true)
        }
        Packet::Dungeon(wrath_lfg::dungeon::Packet::Disabled) => {
            snapshot.player = None;
            snapshot.party = None;
            Some(false)
        }
        Packet::Queue(wrath_lfg::queue::Packet::JoinResult {
            result: 6, locks, ..
        }) => {
            snapshot.party = Some(party_block(locks.as_deref().unwrap_or_default()));
            Some(false)
        }
        _ => None,
    }
}

/// The derived inputs: names of the party GUIDs, icons and links of the preview items.
fn presentation(
    snapshot: &WrathLfgLockSnapshot,
    self_guid: u64,
    level: u32,
    names: Option<&NameCache>,
    commands: &NetCommands,
    items: &Items,
    icons: Option<&ItemDisplays>,
) -> WrathLfgPresentation {
    let mut out = WrathLfgPresentation {
        self_guid,
        ..Default::default()
    };
    for member in snapshot.party.iter().flatten() {
        if let Some(name) = names.and_then(|n| n.resolve(member.guid, commands)) {
            out.guid_names.insert(member.guid, name.to_string());
        }
    }
    let rewards = snapshot
        .player
        .iter()
        .flat_map(|p| &p.previews)
        .flat_map(|p| &p.items);
    for reward in rewards {
        if let Some(icon) = icons
            .and_then(|i| i.catalog.get(reward.display))
            .and_then(|d| d.icon.clone())
        {
            out.display_icons.insert(reward.display, icon);
        }
        if let Some(t) = items.template_cached(reward.item) {
            let link =
                crate::items::wrath_item_link(reward.item, &t.name, t.quality, [0; 4], 0, 0, level);
            out.reward_links.insert(reward.item, link);
        }
    }
    out
}

/// L2's inputs beside L1's.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct L2Inputs<'w> {
    /// The system lines' chat log; `Option` for harnesses, as the group and catalog.
    chat: Option<ResMut<'w, crate::app_core::chat::ChatLog>>,
    group: Option<Res<'w, crate::app_core::group::GroupState>>,
    catalog: Option<Res<'w, super::catalog::Catalog>>,
    /// `IsInLFGDungeon`'s location: our map and the instance's difficulty.
    map: Option<Res<'w, wrathplus_world::world_map::CurrentMap>>,
    instance: Option<Res<'w, crate::ui_static_popup::InstanceLock>>,
}

/// The per-frame L1 feed, after `feed_item_stats` so an answered template is in the VM before its
/// callback events.
#[allow(clippy::too_many_arguments)]
pub(super) fn feed(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<super::NativeLfgPackets>,
    mut l1: ResMut<LfgL1>,
    items: Res<Items>,
    commands: Res<NetCommands>,
    names: Option<Res<NameCache>>,
    self_guid: Res<SelfGuid>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    icons: Option<Res<ItemDisplays>>,
    mut memo: Local<VmMemo<()>>,
    mut l2: L2Inputs,
) {
    let Some(mut script) = script else {
        return;
    };
    let l1 = &mut *l1;
    // The native self object's level, 0 without one (contract: links fall back to level 0).
    let level = self_q
        .single()
        .ok()
        .and_then(|s| s.0.unit_level())
        .unwrap_or(0);
    let me = self_guid.0.unwrap_or(0);
    let derive = |snapshot: &WrathLfgLockSnapshot, items: &Items| {
        Arc::new(presentation(
            snapshot,
            me,
            level,
            names.as_deref(),
            &commands,
            items,
            icons.as_deref(),
        ))
    };
    let send = |command| {
        let _ = commands.0.send(command);
    };

    // A new VM: the current inputs, no replay; then the attach refresh, GET_STATUS first.
    if memo.get_reset(&script).1 {
        script.set_wrath_lfg_lock_snapshot(Arc::clone(&l1.snapshot));
        script.set_wrath_lfg_presentation(Arc::clone(&l1.presentation));
        if let Some(pushed) = &l1.queue_pushed {
            script.set_wrath_lfg_queue(Arc::clone(pushed));
        }
        script.set_wrath_lfg_location(l1.location);
        if self_guid.0.is_some() {
            send(ClientCommand::LfgCommand(
                wrath_lfg::commands::Command::GetStatus,
            ));
            send(ClientCommand::LfgRequestPlayerLockInfo);
        }
    }

    // The L2 verbs' sends (contract: no optimistic state from a send).
    for request in script.take_wrath_lfg_queue_requests() {
        use wrath_lfg::commands::{Command as C, Join};
        use wrathplus_ui::script::WrathLfgQueueRequest as Q;
        send(ClientCommand::LfgCommand(match request {
            Q::Join {
                roles,
                no_partial_clear,
                achievements,
                dungeons,
                needs,
                comment,
            } => C::Join(Join {
                roles: u32::from(roles),
                no_partial_clear,
                achievements,
                slots: dungeons,
                needs,
                comment,
            }),
            Q::Leave => C::Leave,
            Q::RoleCheckAnswer { roles } => C::SetRoles(roles),
            Q::Proposal { id, accept } => C::ProposalResult {
                proposal_id: id,
                accept,
            },
            Q::Teleport { out } => C::Teleport { out },
            Q::SetComment { text } => C::SetComment(text),
        }));
    }

    for request in script.take_wrath_lfg_lock_requests() {
        send(match request {
            WrathLfgLockRequest::Player => ClientCommand::LfgRequestPlayerLockInfo,
            WrathLfgLockRequest::Party => ClientCommand::LfgRequestPartyLockInfo,
        });
    }

    // New interests: the host-held ones are asked into the VM's store; the rest ask the server.
    for request in script.take_wrath_lfg_item_requests() {
        if l1.interests.len() == INTEREST_BOUND {
            l1.interests.pop_front();
        }
        l1.interests.push_back((l1.epoch, request));
        if items.template_cached(request.item).is_some() {
            script.ask_item_templates([request.item]);
            l1.host_held_next.push(request.item);
        } else {
            let _ = items.template(request.item, 0, &commands);
        }
    }

    // Receipts, in order: L1's lock stores, then L2's; other families stay queued for their owners.
    let mut rest = VecDeque::with_capacity(packets.0.len());
    while let Some(packet) = packets.0.pop_front() {
        let mut snapshot = (*l1.snapshot).clone();
        let locks = apply(&mut snapshot, &packet);
        if let Some(fire) = locks {
            l1.snapshot = Arc::new(snapshot);
            l1.presentation = derive(&l1.snapshot, &items);
            script.set_wrath_lfg_lock_snapshot(Arc::clone(&l1.snapshot));
            script.set_wrath_lfg_presentation(Arc::clone(&l1.presentation));
            if fire {
                script.fire_event("LFG_LOCK_INFO_RECEIVED", vec![]);
                script.fire_event("LFG_UPDATE_RANDOM_INFO", vec![]);
            }
        }
        let name = |guid: u64| {
            names
                .as_deref()
                .and_then(|n| n.resolve(guid, &commands))
                .map(str::to_string)
        };
        let dungeon = |entry: u32| {
            l2.catalog.as_deref().and_then(|c| {
                c.0.dungeons
                    .iter()
                    .find(|d| d.id == entry & 0xff_ffff)
                    .map(|d| (d.name.clone(), d.id, d.dungeon_type as u32))
            })
        };
        let inputs = super::l2::Inputs {
            me,
            in_group: l2.group.as_ref().is_some_and(|g| g.in_group),
            now: script.now(),
            name: &name,
            dungeon: &dungeon,
        };
        let queue = l1.queue.apply(&packet, &inputs);
        if let Some(effects) = &queue {
            // The snapshot first, so the events' handlers read the new stores.
            let snapshot = Arc::new(l1.queue.snapshot(&name));
            script.set_wrath_lfg_queue(Arc::clone(&snapshot));
            l1.queue_pushed = Some(snapshot);
            super::l2::run(&mut script, l2.chat.as_deref_mut(), effects);
        }
        if locks.is_none() && queue.is_none() {
            rest.push_back(packet);
        }
    }
    packets.0 = rest;

    // Answers in arrival order, then the host-held completions asked last frame.
    let held: Vec<(u32, bool)> = std::mem::take(&mut l1.host_held)
        .into_iter()
        .map(|item| (item, true))
        .collect();
    l1.host_held = std::mem::take(&mut l1.host_held_next);
    for (item, found) in items.take_answers().into_iter().chain(held) {
        if !l1.interests.iter().any(|(_, r)| r.item == item) {
            continue;
        }
        if found {
            let next = derive(&l1.snapshot, &items);
            if next != l1.presentation {
                l1.presentation = next;
                script.set_wrath_lfg_presentation(Arc::clone(&l1.presentation));
            }
        }
        let epoch = l1.epoch;
        let before = l1.interests.len();
        l1.interests
            .retain(|(e, r)| !(*e == epoch && r.item == item));
        if found {
            for _ in l1.interests.len()..before {
                script.fire_event("LFG_UPDATE_RANDOM_INFO", vec![]);
            }
        }
    }

    // Names land without a receipt: republish the queue without events.
    let name = |guid: u64| {
        names
            .as_deref()
            .and_then(|n| n.resolve(guid, &commands))
            .map(str::to_string)
    };
    if l1.queue_pushed.is_some() {
        let snapshot = l1.queue.snapshot(&name);
        if l1.queue_pushed.as_deref() != Some(&snapshot) {
            let snapshot = Arc::new(snapshot);
            script.set_wrath_lfg_queue(Arc::clone(&snapshot));
            l1.queue_pushed = Some(snapshot);
        }
    }
    let location = l2.map.as_deref().map(|m| {
        let difficulty = l2
            .instance
            .as_deref()
            .and_then(|i| i.difficulty)
            .unwrap_or(0);
        (m.0 as i32, difficulty as i32)
    });
    if location != l1.location {
        l1.location = location;
        script.set_wrath_lfg_location(location);
    }

    // Names and the level move without a receipt: republish without events.
    let next = derive(&l1.snapshot, &items);
    if next != l1.presentation {
        l1.presentation = next;
        script.set_wrath_lfg_presentation(Arc::clone(&l1.presentation));
    }
}

#[cfg(test)]
#[path = "l1_tests.rs"]
mod tests;
