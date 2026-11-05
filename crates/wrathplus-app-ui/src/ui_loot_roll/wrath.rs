//! Lossless native roll inbox; presentation drains its updates separately.
pub(super) mod feed;
#[path = "wrath_lifecycle.rs"]
mod lifecycle;
mod outcomes;

use crate::net::NetHandlerApp;
use bevy::prelude::*;
use wrathplus_protocol::wrath_items::RollMetadata;
use wrathplus_protocol::{wrath_roll::Roll, SessionEvent, SessionEventKind};

/// Native query outcomes are cached across rolls, with ordered receipts for pending starts.
/// A negative response is explicit: presentation drops the pending roll (`0x6fb1f0`).
#[derive(Resource, Default)]
pub(super) struct NativeRollTemplates {
    pub metadata: std::collections::HashMap<u32, Option<RollMetadata>>,
    pub receipts: std::collections::VecDeque<(u32, Option<RollMetadata>)>,
}

#[derive(Resource, Default)]
pub(super) struct NativeRolls {
    /// A start carries the authoritative map/count/vote mask, never inferred from a template.
    pub active: Vec<Roll>,
    /// Pending native announcements retain auto-pass and disenchant rather than legacy projection.
    pub updates: std::collections::VecDeque<Roll>,
}

impl NativeRolls {
    fn receive(&mut self, roll: Roll) {
        match &roll {
            Roll::Start { item, .. } => {
                self.active.retain(|r| {
                    !matches!(r, Roll::Start { item: old, .. }
                    if old.guid == item.guid && old.slot == item.slot)
                });
                self.active.push(roll.clone());
            }
            Roll::Won { item, .. } | Roll::AllPassed { item } => {
                self.active.retain(|r| {
                    !matches!(r, Roll::Start { item: old, .. }
                    if old.guid == item.guid && old.slot == item.slot)
                });
            }
            Roll::Vote { .. } => {}
        }
        // Bounded until the native presentation consumer drains this queue.
        if self.updates.len() == 256 {
            self.updates.pop_front();
        }
        self.updates.push_back(roll);
    }
}

pub(super) fn register(app: &mut App) {
    app.add_systems(
        Startup,
        feed::load_limits.after(wrathplus_assets::AssetSet::Open),
    )
    .init_resource::<feed::Presentation>()
    .add_systems(
        Update,
        (feed::feed, feed::feed_item_limits)
            .in_set(crate::app_core::schedule::UiFeed)
            .run_if(crate::app_core::ui_state::ingame_ui_up),
    )
    .init_resource::<NativeRolls>()
    .init_resource::<NativeRollTemplates>()
    .net_handler(SessionEventKind::WrathItemRollMetadata, on_template)
    .net_handler(SessionEventKind::WrathLootRoll, on_roll)
    .net_handler(SessionEventKind::Disconnected, on_reset);
}
fn on_roll(In(event): In<SessionEvent>, mut rolls: ResMut<NativeRolls>) {
    if let SessionEvent::WrathLootRoll(roll) = event {
        rolls.receive(roll);
    }
}
fn on_template(In(event): In<SessionEvent>, mut templates: ResMut<NativeRollTemplates>) {
    if let SessionEvent::WrathItemRollMetadata { entry, metadata } = event {
        templates.metadata.insert(entry, metadata.clone());
        templates.receipts.push_back((entry, metadata));
    }
}
fn on_reset(
    In(_): In<SessionEvent>,
    mut rolls: ResMut<NativeRolls>,
    mut templates: ResMut<NativeRollTemplates>,
    mut presentation: ResMut<feed::Presentation>,
) {
    *presentation = feed::Presentation::default();
    *rolls = NativeRolls::default();
    *templates = NativeRollTemplates::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::wrath_roll::Item;
    #[test]
    fn native_template_outcomes_are_ordered_lossless_and_disconnect_scoped() {
        let mut app = App::new();
        register(&mut app);
        let metadata = RollMetadata {
            flags2: u32::MAX,
            required_disenchant_skill: -1,
            item_limit_category: 123,
        };
        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![
                SessionEvent::WrathItemRollMetadata {
                    entry: 42,
                    metadata: Some(metadata.clone()),
                },
                SessionEvent::WrathItemRollMetadata {
                    entry: 42,
                    metadata: None,
                },
                SessionEvent::WrathItemRollMetadata {
                    entry: 43,
                    metadata: Some(metadata.clone()),
                },
            ],
        );
        let mut templates = app.world_mut().resource_mut::<NativeRollTemplates>();
        assert_eq!(templates.metadata.get(&42), Some(&None));
        assert_eq!(templates.metadata.get(&43), Some(&Some(metadata.clone())));
        assert_eq!(
            templates.receipts.pop_front(),
            Some((42, Some(metadata.clone())))
        );
        assert_eq!(templates.receipts.pop_front(), Some((42, None)));
        assert_eq!(templates.receipts.pop_front(), Some((43, Some(metadata))));
        assert!(templates.receipts.is_empty());
        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![SessionEvent::Disconnected {
                reason: "test".into(),
                end: wrathplus_protocol::SessionEnd::Lost,
            }],
        );
        let templates = app.world().resource::<NativeRollTemplates>();
        assert!(templates.metadata.is_empty());
        assert!(templates.receipts.is_empty());
    }

    fn item() -> Item {
        Item {
            guid: 42,
            slot: 7,
            entry: 2589,
            suffix_factor: 876,
            random_property: -17,
        }
    }
    #[test]
    fn native_owner_retains_start_vote_resolution_and_clears_on_disconnect() {
        let mut app = App::new();

        register(&mut app);
        let start = Roll::Start {
            item: item(),
            map: 571,
            count: 1000,
            countdown_ms: 60000,
            vote_mask: 15,
        };
        let vote = Roll::Vote {
            item: item(),
            player: 88,
            number: 128,
            kind: 3,
            auto_pass: true,
        };
        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![
                SessionEvent::WrathLootRoll(start.clone()),
                SessionEvent::WrathLootRoll(vote.clone()),
            ],
        );
        let rolls = app.world().resource::<NativeRolls>();
        assert_eq!(rolls.active, vec![start.clone()]);
        assert_eq!(
            rolls.updates.iter().cloned().collect::<Vec<_>>(),
            vec![start, vote]
        );
        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![SessionEvent::WrathLootRoll(Roll::Won {
                item: item(),
                player: 88,
                number: 100,
                kind: 3,
            })],
        );
        assert!(app.world().resource::<NativeRolls>().active.is_empty());
        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![SessionEvent::Disconnected {
                reason: "test".into(),
                end: wrathplus_protocol::SessionEnd::Lost,
            }],
        );
        assert!(app.world().resource::<NativeRolls>().updates.is_empty());
    }
}
