//! Single native stable owner: retain the raw FIFO and apply admitted packets to the UI session.
use crate::names::NameCache;
use crate::net::{NetCommands, NetHandlerApp};
use bevy::prelude::*;
use std::collections::{HashSet, VecDeque};
use wrathplus_protocol::{types::StabledPet, wrath_stable::Packet, SessionEvent, SessionEventKind};

use super::{net, StableErrors, StableOpen};

#[derive(Resource, Default)]
pub(crate) struct NativeStablePackets(pub(crate) VecDeque<Packet>);

pub(super) fn register(app: &mut App) {
    app.init_resource::<NativeStablePackets>()
        .net_handler(SessionEventKind::WrathStable, receive)
        .net_handler(SessionEventKind::Disconnected, clear);
}

fn receive(
    In(event): In<SessionEvent>,
    mut packets: ResMut<NativeStablePackets>,
    mut open: ResMut<StableOpen>,
    mut names: ResMut<NameCache>,
    mut errors: ResMut<StableErrors>,
    commands: Res<NetCommands>,
    mut gossip: ResMut<crate::ui_gossip::GossipState>,
    mut quest: ResMut<crate::app_core::state::QuestGiver>,
) {
    let SessionEvent::WrathStable { packet } = event else {
        return;
    };
    packets.0.push_back(packet.clone());
    match packet {
        Packet::List {
            npc,
            purchased_slots,
            pets,
        } => {
            let Some(rows) = ui_rows(&pets, purchased_slots) else {
                warn!("ui_stable: native list has unsupported flags or inconsistent rows; raw packet retained");
                return;
            };
            net::list_stabled_pets(npc, purchased_slots, rows, &mut open, &mut names);
            crate::ui_gossip::end_interaction(&mut gossip, &mut quest);
        }
        Packet::Result { result } => {
            // AzerothCore NPCHandler.cpp StableResultCode: the success/money codes match.
            // 6 is a silent failure; 12 is the Wrath-only exotic-creature refusal.
            if result == 12 {
                errors.0.push("PETTAME_CANTCONTROLEXOTIC");
            } else {
                let purchased = open.num_stable_slots;
                net::stable_result(result, &mut open, &mut errors, &commands);
                // Native purchased count is authoritative only in SendStablePet.
                open.num_stable_slots = purchased;
            }
        }
    }
}

/// NPCHandler.cpp SendStablePet emits current/unslotted flags=1 and occupied stable rows
/// flags=2 in stable order, without physical slot indices. Pack inactive rows densely for UI.
fn ui_rows(
    pets: &[wrathplus_protocol::wrath_stable::Pet],
    purchased: u8,
) -> Option<Vec<StabledPet>> {
    let mut current = false;
    let mut inactive = 0u8;
    let mut numbers = HashSet::new();
    pets.iter()
        .map(|pet| {
            if !numbers.insert(pet.pet_number) {
                return None;
            }
            let slot = match pet.flags {
                1 if !current => {
                    current = true;
                    0
                }
                2 => {
                    inactive = inactive.checked_add(1)?;
                    inactive
                }
                _ => return None,
            };
            if inactive > purchased {
                return None;
            }
            Some(StabledPet {
                pet_number: pet.pet_number,
                creature_entry: pet.creature_entry,
                level: pet.level,
                name: String::from_utf8_lossy(&pet.name).into_owned(),
                // 12340's list has no loyalty byte.
                loyalty: 0,
                slot,
            })
        })
        .collect()
}

fn clear(
    In(_): In<SessionEvent>,
    mut packets: ResMut<NativeStablePackets>,
    mut open: ResMut<StableOpen>,
    mut errors: ResMut<StableErrors>,
) {
    packets.0.clear();
    open.clear();
    errors.0.clear();
}

#[cfg(test)]
#[path = "wrath_tests.rs"]
mod tests;
