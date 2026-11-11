//! The death packet handlers filling [`DeathNet`] and the movement-mode forward for our own mover.
//! The corpse-guid latch's hooks on the object lifecycle live with their caller, the wire layer's
//! object arms (`net::objects`).

use bevy::prelude::*;
use wrathplus_protocol::{MoveMode, SessionEvent, SessionEventKind};

use super::{CorpsePoint, DeathNet, ResurrectOffer};
use crate::app_core::session::SelfGuid;
use crate::net::{MoveModeMessage, NetHandlerApp};

/// Register the death handlers and the session-end listener.
pub(super) fn register(app: &mut App) {
    use SessionEventKind as K;
    app.net_handler(K::CorpseQuery, on_corpse_query)
        .net_handler(K::CorpseReclaimDelay, on_corpse_reclaim_delay)
        .net_handler(K::ResurrectRequest, on_resurrect_request)
        .net_handler(K::SpiritHealerConfirm, on_spirit_healer_confirm)
        .net_handler(K::MoveMode, on_move_mode)
        .net_handler(K::Disconnected, on_session_end);
}

fn on_corpse_query(In(ev): In<SessionEvent>, mut death_net: ResMut<DeathNet>) {
    if let SessionEvent::CorpseQuery {
        found,
        display_map,
        position,
        corpse_map,
    } = ev
    {
        corpse_query(found, display_map, position, corpse_map, &mut death_net);
    }
}

fn on_corpse_reclaim_delay(
    In(ev): In<SessionEvent>,
    mut death_net: ResMut<DeathNet>,
    real_clock: Res<Time<Real>>,
) {
    if let SessionEvent::CorpseReclaimDelay { delay_ms } = ev {
        corpse_reclaim_delay(delay_ms, real_clock.elapsed_secs_f64(), &mut death_net);
    }
}

fn on_resurrect_request(In(ev): In<SessionEvent>, mut death_net: ResMut<DeathNet>) {
    if let SessionEvent::ResurrectRequest {
        caster,
        name,
        sickness,
        has_timer,
    } = ev
    {
        resurrect_request(caster, name, sickness, has_timer, &mut death_net);
    }
}

fn on_spirit_healer_confirm(In(ev): In<SessionEvent>, mut death_net: ResMut<DeathNet>) {
    if let SessionEvent::SpiritHealerConfirm { npc } = ev {
        spirit_healer_confirm(npc, &mut death_net);
    }
}

fn on_move_mode(
    In(ev): In<SessionEvent>,
    self_guid: Res<SelfGuid>,
    player: Option<Res<crate::app_core::player::Player>>,
    mut out: MessageWriter<MoveModeMessage>,
) {
    if let SessionEvent::MoveMode {
        guid, mode, apply, ..
    } = ev
    {
        move_mode(
            guid,
            mode,
            apply,
            &self_guid,
            player.as_ref().and_then(|p| p.foreign_mover),
            &mut out,
        );
    }
}

/// The death stores are session-scoped; a reconnect while dead re-sends the reclaim delay.
fn on_session_end(In(_): In<SessionEvent>, mut death_net: ResMut<DeathNet>) {
    *death_net = DeathNet::default();
}

/// `MSG_CORPSE_QUERY`'s answer; a not-found, asked or pushed at bones conversion, drops the marker.
fn corpse_query(
    found: bool,
    display_map: i32,
    position: [f32; 3],
    corpse_map: u32,
    death_net: &mut DeathNet,
) {
    death_net.corpse = found.then_some(CorpsePoint {
        display_map,
        position,
        corpse_map,
    });
}

/// `SMSG_CORPSE_RECLAIM_DELAY`, anchored at arrival on the clock the feed reads. The 1.12 client
/// re-fires the corpse-range events on each (`0x4962d0`), hence the generation bump.
fn corpse_reclaim_delay(delay_ms: u32, now_secs: f64, death_net: &mut DeathNet) {
    death_net.reclaim_at = Some(now_secs + f64::from(delay_ms) / 1000.0);
    death_net.reclaim_generation = death_net.reclaim_generation.wrapping_add(1);
}

/// `SMSG_RESURRECT_REQUEST`, the RESURRECT popup's data.
fn resurrect_request(
    caster: u64,
    name: String,
    sickness: bool,
    has_timer: bool,
    death_net: &mut DeathNet,
) {
    death_net.resurrect = Some(ResurrectOffer {
        caster,
        name,
        sickness,
        has_timer,
    });
    death_net.resurrect_generation = death_net.resurrect_generation.wrapping_add(1);
}

/// `SMSG_SPIRIT_HEALER_CONFIRM`: the healer's gossip re-sends it on every ask, and the reference
/// fires `CONFIRM_XP_LOSS` per arrival.
fn spirit_healer_confirm(npc: u64, death_net: &mut DeathNet) {
    death_net.ask_spirit_healer(npc);
}

/// The acked movement modes, root, water-walk, feather-fall, hover and can-fly, addressed to our own
/// body or to the unit we drive (`driven`, a vehicle or a possessed unit) and applied by the
/// controller ([`crate::player::wire_in`]).
fn move_mode(
    guid: u64,
    mode: MoveMode,
    apply: bool,
    self_guid: &SelfGuid,
    driven: Option<u64>,
    out: &mut MessageWriter<MoveModeMessage>,
) {
    if self_guid.0 != Some(guid) && driven != Some(guid) {
        return;
    }
    out.write(MoveModeMessage { mode, apply });
}
