//! The resolver's guid table (`0x60abf0`'s inputs, [`UnitGuids`]) as the app's aura feed seats it
//! beside the unit snapshots each frame (`ui_aura::units`), for tests whose fixture runs no aura
//! feed.

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use wrathplus_ui::script::{UiScript, UnitGuids, UnitState};

use crate::app_core::session::ObjectStore;

/// Adds `systems` to `Update` beside the aura feed, which seats the table each frame, and what the
/// feed reads.
pub(crate) fn with_aura_feed<M>(
    app: &mut App,
    systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
) {
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
        .insert_state(crate::app_core::state::ClientState::InWorld)
        .init_resource::<crate::app_core::state::PetBar>()
        .init_resource::<crate::net::GuidIndex>()
        .add_plugins(crate::ui_aura::UiAuraPlugin)
        .add_systems(Update, systems);
}

/// Pushes `target` as the target with the table beside it: us `me` and the target's guid.
pub(crate) fn push_target(s: &mut UiScript, me: u64, target: Option<UnitState>) {
    let guid = target.as_ref().map_or(0, |u| u.guid);
    s.set_unit("target", target);
    s.set_unit_guids(&UnitGuids {
        player: me,
        target: guid,
        ..Default::default()
    });
}

/// Seats the table a world hover leaves: us `me`, `target`, and the picked unit as the mouseover,
/// each held.
pub(crate) fn seat_hover(app: &mut App, me: u64, target: u64, pick: Option<(Entity, u64)>) {
    let mouseover = pick.map_or(0, |(_, guid)| guid);
    let held = [me, target, mouseover].into_iter().filter(|&g| g != 0);
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_unit_guids(&UnitGuids {
            player: me,
            target,
            mouseover,
            held: held.map(|g| (g, 0)).collect(),
            ..Default::default()
        });
}

/// Pushes `player` as the player with the table beside it, its guid in the player slot.
pub(crate) fn push_player(s: &mut UiScript, player: UnitState) {
    s.set_unit_guids(&UnitGuids {
        player: player.guid,
        ..Default::default()
    });
    s.set_unit("player", Some(player));
}

/// Seats the table the aura feed builds from the app's group state with no avatar (`fill_group`):
/// us `me`, the party slots and the raid rows, each with its pet off a held member's descriptor,
/// else off its record.
pub(crate) fn seat_group(app: &mut App, me: u64) {
    let world = app.world();
    let group = world.resource::<crate::app_core::group::GroupState>();
    let index = world.resource::<crate::net::GuidIndex>();
    let store = |m: u64| index.0.get(&m).and_then(|&e| world.get::<ObjectStore>(e));
    let live = |m: u64| store(m).map(|s| &s.0);
    let mut g = UnitGuids {
        player: me,
        raid: crate::app_core::raid_rows::raid_row_guids(group, None),
        ..Default::default()
    };
    for (i, m) in group.party_slots().take(4).enumerate() {
        g.party[i] = m.guid;
        g.party_pets[i] = group.party_pet_guid(m.guid, live(m.guid)).unwrap_or(0);
    }
    let pet = |&m: &u64| group.raid_pet_guid(m, live(m)).unwrap_or(0);
    g.raid_pets = g.raid.iter().map(pet).collect();
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_unit_guids(&g);
}
