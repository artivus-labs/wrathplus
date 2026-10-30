//! Native quest POIs and PvP-kill ticks. As the reference's quest-log rebuild (`0x5e6940`) does,
//! every logged quest without POI data is asked once with `CMSG_QUEST_POI_QUERY` (at most
//! [`MAX_POI_QUERY`] ids a packet). Each `SMSG_QUEST_POI_QUERY_RESPONSE` merges into [`QuestPois`]
//! (`crate::net::wrath_quest_pois`), the logged quests' POIs go to `set_wrath_quest_pois` whole,
//! and `QUEST_POI_UPDATE` fires.
//! Labelled: that event (id `0x297`) fires once per response here; its native fire site is not
//! traced (the quest module's only `push 0x297` is `0x5e7628`, unverified). A PvP-kill tick
//! re-asks the givers' markers, as `SMSG_QUESTUPDATE_ADD_KILL` does; its count lives in the
//! quest-log slot (`crate::ui_quest_log`), whose diff toasts it.
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
pub use crate::net::wrath_quest_pois::QuestPois;
use crate::net::{ClientCommand, NetCommands};
use bevy::prelude::*;
use std::collections::BTreeSet;
use wrathplus_protocol::wrath_quest::Command;
use wrathplus_ui::script::{UiScript, WrathQuestPoi, WrathQuestPoiBlob};

pub fn register(app: &mut App) {
    app.add_systems(
        Update,
        feed_quest_pois
            .in_set(crate::app_core::schedule::UiFeed)
            .run_if(crate::app_core::ui_state::ingame_ui_up),
    );
}

/// The quest ids in the player's log slots, in slot order, each once.
fn logged_quests(store: &wrathplus_protocol::ObjectFields) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    (0..store.player_quest_log_slots())
        .filter_map(|slot| store.player_quest_log(slot))
        .map(|slot| slot.quest_id)
        .filter(|&q| q != 0 && seen.insert(q))
        .collect()
}

/// What `GetQuestWorldMapAreaID` reads beside the POIs: each logged quest's header top zone and
/// the player's world `(x, y)`.
#[derive(Clone, Debug, PartialEq)]
struct QuestMapContext {
    zones: Vec<(u32, u32)>,
    player: Option<(f32, f32)>,
}

/// `0x5e2b60`'s fallback input: a logged quest's quest-log header, a positive `ZoneOrSort` naming
/// an `AreaTable` zone, walked up to its top zone. A `QuestSort` header or an uncached template
/// has none.
fn quest_zones(
    logged: &[u32],
    quest_log: Option<&crate::ui_quest_log::QuestLog>,
    areas: Option<&crate::area::AreaTableRes>,
) -> Vec<(u32, u32)> {
    let (Some(quest_log), Some(areas)) = (quest_log, areas) else {
        return Vec::new();
    };
    logged
        .iter()
        .filter_map(|&q| {
            let zone = u32::try_from(quest_log.zone_or_sort(q)?).ok()?;
            Some((q, areas.0.top_zone(zone)?))
        })
        .collect()
}

#[allow(clippy::type_complexity)] // the map context bundled as one param, under the arg cap
fn feed_quest_pois(
    script: Option<NonSendMut<UiScript>>,
    mut pois: ResMut<QuestPois>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    context: (
        Option<Res<crate::ui_quest_log::QuestLog>>,
        Option<Res<crate::area::AreaTableRes>>,
        Option<Res<crate::app_core::player::Player>>,
    ),
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<Vec<WrathQuestPoi>>>>,
    mut last_context: Local<crate::app_core::vm_memo::VmMemo<Option<QuestMapContext>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let logged = self_q
        .iter()
        .next()
        .map(|s| logged_quests(&s.0))
        .unwrap_or_default();
    for quests in pois.queries(&logged) {
        let _ = commands
            .0
            .send(ClientCommand::NativeQuest(Command::PoiQuery { quests }));
    }
    let view = pois.view(&logged);
    let (last, reset) = last.get_reset(&script);
    let changed = reset || last.as_ref() != Some(&view);
    if changed {
        *last = Some(view.clone());
        script.set_wrath_quest_pois(view);
    }
    let (quest_log, areas, player) = context;
    let now = QuestMapContext {
        zones: quest_zones(&logged, quest_log.as_deref(), areas.as_deref()),
        player: player.map(|p| {
            let wow = wrathplus_assets::coords::bevy_to_wow(p.pos);
            (wow[0], wow[1])
        }),
    };
    let (last_context, reset) = last_context.get_reset(&script);
    if reset || last_context.as_ref() != Some(&now) {
        script.set_wrath_quest_zones(now.zones.clone());
        script.set_wrath_quest_player_position(now.player);
        *last_context = Some(now);
    }
    // Stock WatchFrame.lua:272 rebuilds CURRENT_MAP_QUESTS on this event. Publishing cached
    // POIs into a replacement VM needs the same invalidation as a newly received reply.
    let fresh = std::mem::take(&mut pois.fresh);
    if changed || fresh {
        script.fire_event("QUEST_POI_UPDATE", vec![]);
    }
}

/// `QuestPois`'s methods that need this module (the type lives in `app_core` for the crate split).
pub trait QuestPoisView {
    /// The logged quests' POIs, in log order; an unanswered or POI-less quest has no entry.
    fn view(&self, logged: &[u32]) -> Vec<WrathQuestPoi>;
}

impl QuestPoisView for QuestPois {
    fn view(&self, logged: &[u32]) -> Vec<WrathQuestPoi> {
        logged
            .iter()
            .filter_map(|&quest_id| {
                let pois = self.known.get(&quest_id).filter(|p| !p.is_empty())?;
                Some(WrathQuestPoi {
                    quest_id,
                    blobs: pois
                        .iter()
                        .map(|p| WrathQuestPoiBlob {
                            objective_index: p.objective,
                            map_id: p.map,
                            area_id: p.area,
                            floor: p.floor,
                            points: p.points.clone(),
                        })
                        .collect(),
                })
            })
            .collect()
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "wrath_quest_pois_tests.rs"]
pub mod tests;
