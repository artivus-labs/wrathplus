//! The spell-control packets whose native handlers act on our own state, beside their capture
//! FIFO (`crate::net::wrath_spell_control_capture`). One read-only Ghidra round on build 12340
//! traced both through their `0x6b0b80` registrations (https://github.com/artivus-labs/wrathplus/wiki/WRATH-SPELL-CONTROL).
//!
//! `SMSG_MODIFY_COOLDOWN` (AzerothCore `Player::ModifySpellCooldown`, `Player.cpp:11275`): one
//! running cooldown moves by a signed delta in milliseconds. The handler (`0x804010`, registered
//! at `0x81018c`) reads the spell, the guid and the delta, skips a zero spell, then runs
//! `0x802bf0`: the list edit `0x802a10`, then the same flush as `SMSG_SPELL_COOLDOWN`'s `0x802ba0`
//! (`SPELL_UPDATE_COOLDOWN` through `0x53bac0`, the pet bar's events for the pet's guid), which a
//! generation bump drives here. The edit itself is not traced: AzerothCore moves the record's end,
//! so here the spell's recovery moves, and the list is the one the guid names, as for every other
//! cooldown packet.
//!
//! `SMSG_CLEAR_TARGET` (AzerothCore `Spell::EffectForceDeselect`, `SpellEffects.cpp:5019`, to the
//! caster's hostile watchers): the handler (`0x756800`, registered at `0x756c62`) calls
//! `0x5241b0(guid, 1)`, which drops our selection, with its `CMSG_SET_SELECTION` 0, only when it
//! is that caster: the guid-scoped deselect [`crate::target::DeselectGuid`] already is.

use std::time::Duration;

use bevy::prelude::*;
use wrathplus_protocol::{wrath_spell_control::Packet, SessionEvent, SessionEventKind};

use crate::app_core::session::SelfGuid;
use crate::app_core::state::PetBar;
use crate::net::NetHandlerApp;
use crate::spell::Cooldowns;
use crate::target::DeselectGuid;

pub(super) fn register(app: &mut App) {
    app.add_message::<DeselectGuid>()
        .net_handler(SessionEventKind::WrathSpellControl, on_modify_cooldown)
        .net_handler(SessionEventKind::WrathSpellControl, on_clear_target);
}

/// `SMSG_CLEAR_TARGET`: ask the guid-scoped deselect for the caster.
fn on_clear_target(In(event): In<SessionEvent>, mut asks: MessageWriter<DeselectGuid>) {
    if let SessionEvent::WrathSpellControl {
        packet: Packet::ClearTarget { caster },
    } = event
    {
        asks.write(DeselectGuid(caster));
    }
}

fn on_modify_cooldown(
    In(event): In<SessionEvent>,
    self_guid: Res<SelfGuid>,
    mut player: ResMut<Cooldowns>,
    mut pet: ResMut<PetBar>,
) {
    let SessionEvent::WrathSpellControl {
        packet:
            Packet::ModifyCooldown {
                spell_id,
                unit,
                delta_ms,
            },
    } = event
    else {
        return;
    };
    if spell_id == 0 {
        return;
    }
    let store = if self_guid.0 == Some(unit) {
        &mut *player
    } else if pet.has_bar() && pet.guid() == unit {
        &mut pet.cooldowns
    } else {
        return;
    };
    modify(store, spell_id, delta_ms);
}

/// Move every running recovery of `spell_id` by `delta_ms`; a cut past its end just ends it.
fn modify(store: &mut Cooldowns, spell_id: u32, delta_ms: i32) {
    let delta = Duration::from_millis(u64::from(delta_ms.unsigned_abs()));
    let mut touched = false;
    for r in &mut store.records {
        if r.spell_id != spell_id || r.recovery.duration.is_zero() {
            continue;
        }
        r.recovery.duration = if delta_ms < 0 {
            r.recovery.duration.saturating_sub(delta)
        } else {
            r.recovery.duration + delta
        };
        touched = true;
    }
    if touched {
        store.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::cooldowns::Timer;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::Instant;
    use wrathplus_protocol::wrath_pet::PetSpells;

    const ME: u64 = 0x10;
    const PET: u64 = 0xF140_0000_0000_0042;

    fn recovering(store: &mut Cooldowns, spell_id: u32, ms: u64, now: Instant) {
        let none = Timer::none(now);
        let recovery = Timer {
            start: now,
            duration: Duration::from_millis(ms),
        };
        store.add(now, spell_id, 0, recovery, 0, false, none, false, 0, none);
    }

    fn world() -> (World, Instant) {
        let now = Instant::now();
        let mut world = World::new();
        world.insert_resource(SelfGuid(Some(ME)));
        let mut player = Cooldowns::default();
        recovering(&mut player, 133, 8_000, now);
        world.insert_resource(player);
        world.insert_resource(PetBar::default());
        (world, now)
    }

    fn send(world: &mut World, spell_id: u32, unit: u64, delta_ms: i32) {
        let packet = Packet::ModifyCooldown {
            spell_id,
            unit,
            delta_ms,
        };
        world
            .run_system_once_with(
                on_modify_cooldown,
                SessionEvent::WrathSpellControl { packet },
            )
            .unwrap();
    }

    fn recovery_ms(world: &World) -> u128 {
        world.resource::<Cooldowns>().records[0]
            .recovery
            .duration
            .as_millis()
    }

    #[test]
    fn our_guid_moves_our_spells_recovery_both_ways_and_flushes() {
        let (mut world, _) = world();
        let before = world.resource::<Cooldowns>().generation;
        send(&mut world, 133, ME, -3_000);
        assert_eq!(recovery_ms(&world), 5_000, "a cut");
        send(&mut world, 133, ME, 1_500);
        assert_eq!(recovery_ms(&world), 6_500, "and an extension");
        assert_eq!(
            world.resource::<Cooldowns>().generation,
            before + 2,
            "each one flushes the list's events"
        );
        send(&mut world, 133, ME, -60_000);
        assert_eq!(recovery_ms(&world), 0, "a cut past the end ends it");
    }

    #[test]
    fn another_spell_a_zero_spell_or_a_strangers_guid_changes_nothing() {
        let (mut world, _) = world();
        let before = world.resource::<Cooldowns>().generation;
        send(&mut world, 116, ME, -3_000);
        send(&mut world, 0, ME, -3_000);
        send(&mut world, 133, 0x99, -3_000);
        assert_eq!(recovery_ms(&world), 8_000);
        assert_eq!(
            world.resource::<Cooldowns>().generation,
            before,
            "and flushes nothing"
        );
    }

    #[test]
    fn the_pets_guid_moves_the_pets_list() {
        let (mut world, now) = world();
        {
            let mut pet = world.resource_mut::<PetBar>();
            pet.native = Some(PetSpells {
                guid: PET,
                ..Default::default()
            });
            recovering(&mut pet.cooldowns, 17_253, 4_000, now);
        }
        assert!(
            world.resource::<PetBar>().has_bar(),
            "the fixture is a pet with a bar"
        );
        send(&mut world, 17_253, PET, -1_000);
        let pet = world.resource::<PetBar>();
        assert_eq!(
            pet.cooldowns.records[0].recovery.duration.as_millis(),
            3_000
        );
        assert_eq!(recovery_ms(&world), 8_000, "ours is untouched");
    }

    #[test]
    fn a_clear_target_asks_to_deselect_its_caster() {
        let mut world = World::new();
        world.init_resource::<Messages<DeselectGuid>>();
        let packet = Packet::ClearTarget { caster: 0x42 };
        world
            .run_system_once_with(on_clear_target, SessionEvent::WrathSpellControl { packet })
            .unwrap();
        let asks: Vec<u64> = world
            .resource_mut::<Messages<DeselectGuid>>()
            .drain()
            .map(|ask| ask.0)
            .collect();
        assert_eq!(asks, vec![0x42]);
    }
}
