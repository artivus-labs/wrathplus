//! The achievement window's progress from native achievement traffic. `SMSG_ALL_ACHIEVEMENT_DATA`
//! at login seeds the earned list; the criteria counters are `wrath_notices`'s, which takes the
//! snapshot, `SMSG_CRITERIA_UPDATE` and `SMSG_CRITERIA_DELETED` in wire order; the player's own
//! `SMSG_ACHIEVEMENT_EARNED` adds an earn; `SMSG_ACHIEVEMENT_DELETED` removes one. Each packet's events fire as
//! 12340's handlers fire them. `SetAchievementComparisonUnit` sends
//! `CMSG_QUERY_INSPECT_ACHIEVEMENTS`, and every `SMSG_RESPOND_INSPECT_ACHIEVEMENTS` reaches the VM,
//! which keeps the asked player's. A server first (`SMSG_SERVER_FIRST_ACHIEVEMENT`) becomes an
//! achievement chat line ([`server_first_line`]). Other players' earns stay in the capture.
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::app_core::session::SelfPlayer;
pub(crate) use crate::net::wrath_achievement_capture::Achievements;
use crate::net::wrath_achievement_capture::{NativeAchievementPackets, NativeCriteriaReceipts};
use crate::net::wrath_notices::WrathProgress;
use crate::net::{ClientCommand, NetCommands};
use bevy::prelude::*;
use wrathplus_protocol::wrath_achievement::{Packet, Snapshot};
use wrathplus_protocol::wrath_chat::Message;
use wrathplus_ui::script::{
    UiScript, WrathAchievementProgress, WrathAchievementRequest, WrathCriteriaProgress,
};

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        Update,
        feed_achievements
            .in_set(crate::app_core::schedule::UiFeed)
            .before(crate::ui_chat::NativeChatFeedSet)
            .run_if(crate::app_core::ui_state::ingame_ui_up),
    );
}

/// A server first's chat line, as the reference's handler (`0x50b010`) builds it: the template
/// `SERVER_FIRST_ACHIEVEMENT`, or `PLAYER_SERVER_FIRST_ACHIEVEMENT` when the link word is set,
/// raised as chat type `0x30` (`ACHIEVEMENT`) with the name as sender and the achievement id, so
/// the chat feed expands `$a`. Labelled: the template's `%s` are filled with the name here. The
/// handler passes the name to its text lookup `0x819d40`, and the player variant's two `%s`
/// would otherwise fail `ChatFrame`'s one-argument `format`; that the lookup fills them is
/// inferred, not traced.
fn server_first_line(template: &str, name: &str, player: u64, achievement: u32) -> Message {
    let text = wrathplus_ui::strings::fill(
        template,
        &[
            wrathplus_ui::strings::Arg::S(name),
            wrathplus_ui::strings::Arg::S(name),
        ],
    );
    Message {
        gm: false,
        chat_type: 0x30,
        language: 0,
        sender_guid: player,
        flags: 0,
        sender_name: Some(name.as_bytes().to_vec()),
        receiver_guid: 0,
        receiver_name: None,
        channel: None,
        text: text.into_bytes(),
        chat_tag: 0,
        achievement_id: Some(achievement),
    }
}

/// The achievement faction of a race: 0 Horde, 1 Alliance (`Achievement.dbc` `Faction`).
fn faction(race: Option<u8>) -> i32 {
    match race {
        Some(2 | 5 | 6 | 8 | 10) => 0,
        Some(1 | 3 | 4 | 7 | 11) => 1,
        _ => -1,
    }
}

/// An achievement's `Order` column, `None` without a row.
type OrderOf<'a> = &'a dyn Fn(u32) -> Option<u32>;

/// `0x5b28d0` (and `0x5b2ab0` for a compared list): an earn of a known achievement goes before
/// the first strictly older one or, among same-minute earns, before the first of lower `Order`
/// (`0x5b29fb`-`0x5b2a06`); an id already listed keeps its place and takes the new date.
fn insert_earned(list: &mut Vec<(u32, u32)>, id: u32, date: u32, order: OrderOf) {
    let Some(rank) = order(id) else {
        return;
    };
    if let Some(entry) = list.iter_mut().find(|(e, _)| *e == id) {
        entry.1 = date;
        return;
    }
    let at = list
        .iter()
        .position(|&(e, d)| d < date || (d == date && order(e).is_some_and(|o| rank > o)))
        .unwrap_or(list.len());
    list.insert(at, (id, date));
}

/// A compared player's progress; `now` dates its counters as `0x5b34a0` does (`0x5b3588`).
fn snapshot_progress(data: &Snapshot, order: OrderOf, now: i64) -> WrathAchievementProgress {
    let mut earned = Vec::new();
    for &(id, date) in &data.earned {
        insert_earned(&mut earned, id, date, order);
    }
    let criteria = data.criteria.iter().map(|c| {
        let dated = now - i64::from(c.creation_elapsed);
        let progress = WrathCriteriaProgress {
            counter: c.counter,
            player: c.player,
            dated,
        };
        (c.id, progress)
    });
    WrathAchievementProgress {
        earned,
        criteria: criteria.collect(),
        faction: -1,
        received: false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn feed_achievements(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<NativeAchievementPackets>,
    mut receipts: ResMut<NativeCriteriaReceipts>,
    mut state: ResMut<Achievements>,
    metadata: Option<Res<crate::ui_achievements::AchievementMetadata>>,
    mut progress: ResMut<WrathProgress>,
    self_guid: Res<SelfGuid>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    mut chat: Option<ResMut<crate::ui_native_chat::NativeChatPackets>>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<WrathAchievementProgress>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let me = self_guid.0.unwrap_or(0);
    let faction = faction(self_q.iter().next().and_then(|s| s.0.unit_race()));
    let order = |id: u32| metadata.as_deref().and_then(|m| m.order(id));
    let out = absorb(&mut packets.0, &mut state, me, &order);
    let receipts: Vec<_> = receipts.0.drain(..).collect();
    state.received |= !receipts.is_empty();
    if let Some(chat) = chat.as_deref_mut() {
        for first in out.firsts {
            let key = if first.raw_link != 0 {
                "PLAYER_SERVER_FIRST_ACHIEVEMENT"
            } else {
                "SERVER_FIRST_ACHIEVEMENT"
            };
            let template = crate::ui_action::global_string(&script, key).unwrap_or_default();
            chat.0.push_back(server_first_line(
                &template,
                &first.name,
                first.player,
                first.achievement,
            ));
        }
    }
    for request in script.take_wrath_achievement_requests() {
        let WrathAchievementRequest::Compare { guid } = request;
        let _ = commands
            .0
            .send(ClientCommand::NativeInspectAchievements { player: guid });
    }
    let criteria = progress
        .criteria
        .iter()
        .filter(|((player, _), _)| *player == me)
        .map(|(&(player, id), row)| {
            let (counter, dated) = (row.counter, row.dated);
            (
                id,
                WrathCriteriaProgress {
                    counter,
                    player,
                    dated,
                },
            )
        })
        .collect();
    let view = WrathAchievementProgress {
        earned: state.earned.clone(),
        criteria,
        faction,
        received: state.received,
    };
    let (last, reset) = last.get_reset(&script);
    if reset || last.as_ref() != Some(&view) {
        *last = Some(view.clone());
        script.set_wrath_achievement_progress(view);
    }
    // Labelled: the order across packet kinds within one frame is by kind, not by arrival.
    for list in &out.lists {
        script.wrath_achievement_list_arrived(list);
    }
    for (criteria, elapsed) in receipts {
        script.wrath_criteria_updated(criteria, elapsed);
    }
    for _ in 0..std::mem::take(&mut progress.criteria_deleted) {
        script.wrath_criteria_deleted();
    }
    for id in out.deleted {
        script.wrath_achievement_deleted(id);
    }
    for id in out.earned {
        script.wrath_achievement_earned(id);
    }
    for (guid, data) in out.inspected {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        script.set_wrath_achievement_comparison(guid, snapshot_progress(&data, &order, now));
    }
}

/// One `SMSG_SERVER_FIRST_ACHIEVEMENT`, for [`server_first_line`].
#[derive(Debug, PartialEq)]
struct ServerFirst {
    name: String,
    player: u64,
    achievement: u32,
    raw_link: u32,
}

/// What one drain of the capture fires, by packet kind.
#[derive(Debug, Default)]
struct Absorbed {
    /// Each `SMSG_ALL_ACHIEVEMENT_DATA`'s criteria as `(id, elapsed)`.
    lists: Vec<Vec<(u32, u32)>>,
    /// The player's own earns that fire `ACHIEVEMENT_EARNED`.
    earned: Vec<u32>,
    /// Earns an `SMSG_ACHIEVEMENT_DELETED` removed.
    deleted: Vec<u32>,
    /// Every inspect response.
    inspected: Vec<(u64, Snapshot)>,
    firsts: Vec<ServerFirst>,
}

/// Take the player's achievement receipts, the inspect responses and the server firsts out of
/// the capture into `state`. Other players' earns stay, in order.
fn absorb(
    packets: &mut std::collections::VecDeque<Packet>,
    state: &mut Achievements,
    me: u64,
    order: OrderOf,
) -> Absorbed {
    let mut out = Absorbed::default();
    let mut kept = std::collections::VecDeque::new();
    for packet in packets.drain(..) {
        match packet {
            Packet::AllData(data) => {
                state.earned.clear();
                for &(id, date) in &data.earned {
                    insert_earned(&mut state.earned, id, date, order);
                }
                state.received = true;
                out.lists
                    .push(data.criteria.iter().map(|c| (c.id, c.elapsed)).collect());
            }
            Packet::Earned {
                player,
                achievement,
                packed_time,
                raw_unknown,
            } if player == me => {
                insert_earned(&mut state.earned, achievement, packed_time, order);
                // A nonzero trailing dword suppresses the untrack and the event (`0x5b30c5`).
                if raw_unknown == 0 && order(achievement).is_some() {
                    out.earned.push(achievement);
                }
            }
            Packet::AchievementDeleted { achievement } => {
                if let Some(at) = state.earned.iter().position(|(e, _)| *e == achievement) {
                    state.earned.remove(at);
                    out.deleted.push(achievement);
                }
            }
            Packet::CriteriaDeleted { .. } => {}
            Packet::Inspect { player, data } => out.inspected.push((player, data)),
            Packet::ServerFirst {
                name,
                player,
                achievement,
                raw_link,
            } => out.firsts.push(ServerFirst {
                name,
                player,
                achievement,
                raw_link,
            }),
            other => kept.push_back(other),
        }
    }
    *packets = kept;
    out
}

#[cfg(test)]
#[path = "wrath_achievements_tests.rs"]
mod tests;
