//! Ordered native achievement receipts. The player's own progress and the asked comparison feed
//! `crate::ui_native::wrath_achievements` through [`Achievements`]; other players' earns and
//! server firsts stay here.
use super::NetHandlerApp;
use bevy::prelude::*;
use std::collections::{BTreeMap, VecDeque};
use wrathplus_protocol::{wrath_achievement::Packet, SessionEvent, SessionEventKind};

#[derive(Resource, Default)]
pub struct NativeAchievementPackets(pub VecDeque<Packet>);

/// Each `SMSG_CRITERIA_UPDATE`'s `(criteria, elapsed)` in arrival order: 12340 fires its events
/// per packet (`0x5b3160`). The counters themselves are `WrathProgress`'s.
#[derive(Resource, Default)]
pub struct NativeCriteriaReceipts(pub VecDeque<(u32, u32)>);

/// The player's achievement progress the feed reduces the receipts into.
#[derive(Resource, Default)]
pub struct Achievements {
    /// Earned achievements and their packed dates, in the client's list order.
    pub earned: Vec<(u32, u32)>,
    /// The login snapshot's counters; later updates come from `WrathProgress`.
    pub criteria: BTreeMap<u32, u64>,
    /// Criteria the server deleted since the snapshot.
    pub deleted: Vec<u32>,
    /// An all-data or criteria-update packet has arrived (`[0xc1f454]`).
    pub received: bool,
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<NativeAchievementPackets>()
        .init_resource::<NativeCriteriaReceipts>()
        .init_resource::<Achievements>()
        .net_handler(SessionEventKind::WrathAchievement, receive)
        .net_handler(SessionEventKind::CriteriaUpdate, receive_criteria)
        .net_handler(SessionEventKind::Connected, clear)
        .net_handler(SessionEventKind::Disconnected, clear);
}

fn receive(In(event): In<SessionEvent>, mut packets: ResMut<NativeAchievementPackets>) {
    if let SessionEvent::WrathAchievement { packet } = event {
        packets.0.push_back(packet);
    }
}

fn receive_criteria(In(event): In<SessionEvent>, mut receipts: ResMut<NativeCriteriaReceipts>) {
    if let SessionEvent::CriteriaUpdate {
        criteria, elapsed, ..
    } = event
    {
        receipts.0.push_back((criteria, elapsed));
    }
}

fn clear(
    In(event): In<SessionEvent>,
    mut packets: ResMut<NativeAchievementPackets>,
    mut receipts: ResMut<NativeCriteriaReceipts>,
    mut achievements: ResMut<Achievements>,
) {
    if matches!(
        event,
        SessionEvent::Connected { .. } | SessionEvent::Disconnected { .. }
    ) {
        packets.0.clear();
        receipts.0.clear();
        clear_progress(&mut achievements);
    }
}

/// A session boundary forgets the progress.
pub fn clear_progress(state: &mut Achievements) {
    *state = Achievements::default();
}
