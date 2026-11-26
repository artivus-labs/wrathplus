//! Build 12340's native receipts that feed the FrameXML VM directly: each module drains or
//! reduces the state the wire layer captured (its namesake or capture module under `crate::net`)
//! and publishes it into `UiScript`, so it lives with the feed layer (`tests/ui_script_wall.rs`).
//! Each re-exports the moved capture types under its old path.

pub(crate) mod wrath_achievements;
pub(crate) mod wrath_appearance;
pub(crate) mod wrath_arena;
pub(crate) mod wrath_death_capture;
pub(crate) mod wrath_difficulty;
pub(crate) mod wrath_equipment_sets;
pub(crate) mod wrath_item_refunds;
pub(crate) mod wrath_movie;
pub mod wrath_quest_marker;
pub mod wrath_quest_pois;
pub mod wrath_quests_completed_capture;
pub(crate) mod wrath_raid_browser_feed;
pub(crate) mod wrath_sockets;
pub mod wrath_state_feed;
pub(crate) mod wrath_threat;
mod wrath_tracked_saved;
pub mod wrath_ui_timer_capture;

/// The feed halves of the native receipts the wire layer captures (`crate::net`): each publishes
/// its family's state into the VM in the feed phase.
pub struct NativeFeedsPlugin;

impl bevy::prelude::Plugin for NativeFeedsPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        // In the order `NetPlugin` registered them before the capture halves moved to `crate::net`.
        wrath_quests_completed_capture::register(app);
        wrath_ui_timer_capture::register(app);
        wrath_item_refunds::register(app);
        wrath_equipment_sets::register(app);
        wrath_sockets::register(app);
        wrath_achievements::register(app);
        wrath_tracked_saved::register(app);
        wrath_appearance::register(app);
        wrath_raid_browser_feed::register(app);
        wrath_quest_pois::register(app);
        wrath_quest_marker::register(app);
        wrath_death_capture::register(app);
        wrath_movie::register(app);
        wrath_threat::register(app);
        wrath_difficulty::register(app);
        wrath_arena::register(app);
    }
}
