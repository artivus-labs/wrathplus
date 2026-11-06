//! The resolver's guid table (`0x60abf0`'s inputs, [`UnitGuids`]) as the app's aura feed seats it
//! beside the unit snapshots each frame (`ui_aura::units`), for tests whose fixture runs no aura
//! feed.

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use wrathplus_ui::script::{UiScript, UnitGuids, UnitState};

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
