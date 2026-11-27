//! Battleground teammate and flag positions on the world map: `MSG_BATTLEGROUND_PLAYER_POSITIONS`
//! resolved into the view behind `GetBattlefieldPosition`, `GetBattlefieldFlagPosition` and
//! `GetBattlefieldMapIconScale`, which `WorldMapFrame.lua` and the battlefield minimap poll.
//!
//! The reference's getter (`0x54c2e0`) does the work per call: it skips the player, the party and
//! the raid roster, prefers a streamed object's live position, and projects under the active queue
//! slot's map, answering `(0, 0)` off it. The app resolves the list that way each frame. A name
//! landing fires `WORLD_MAP_NAME_UPDATE`, as in the reference.

use std::time::{Duration, Instant};

use bevy::prelude::*;

use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_assets::MapCatalogRes;
use wrathplus_protocol::types::{BattlefieldPosition, BattlefieldPositions as PositionsPacket};
use wrathplus_ui::script::{BattlefieldFlagView, BattlefieldPositionView, UiScript};

use crate::app_core::group::GroupState;
use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::EnteredWorldMessage;
use crate::app_core::session::SelfGuid;
use crate::names::NameCache;
use crate::net::{ClientCommand, GuidIndex, NetCommands, NetEntity};
use crate::ui_dialog_verbs::BattlefieldQueue;
use crate::ui_world_map::{project_on_displayed, WorldMapUiData};

/// `RequestBattlefieldPositions` (`0x54dcb0` → `0x54cf60`) sends at most once per 5000 ms (stamp
/// `[0xbea5b4]`), and only with an active queue slot; without one it clears the list.
pub(crate) const REQUEST_THROTTLE: Duration = Duration::from_millis(5000);

/// The last positions packet and the request stamp.
#[derive(Resource, Default)]
pub(crate) struct BattlefieldPositions {
    packet: Option<PositionsPacket>,
    /// The carrier slots (`0xbea170`) in Lua index order.
    carriers: Vec<BattlefieldPosition>,
    last_request: Option<Instant>,
    /// Guids whose name the cache has not answered yet.
    pending_names: Vec<u64>,
}

impl BattlefieldPositions {
    /// `MSG_BATTLEGROUND_PLAYER_POSITIONS` in: kept as sent.
    pub(crate) fn apply(&mut self, mut packet: PositionsPacket) {
        self.carriers = packet.carrier.take().into_iter().collect();
        self.packet = Some(packet);
    }

    /// Build 12340's `MSG_BATTLEGROUND_PLAYER_POSITIONS` (`0x54b3f0`), from the native reducer.
    /// The carriers fill the slots from the last down (`0x54b480`-`0x54b4be`), so the last wire
    /// carrier is index 1; past two the reference writes beyond its pair, here dropped.
    pub(crate) fn apply_wrath(
        &mut self,
        players: Vec<(u64, f32, f32)>,
        carriers: Vec<(u64, f32, f32)>,
    ) {
        let at = |(guid, x, y): (u64, f32, f32)| BattlefieldPosition { guid, x, y };
        self.apply(PositionsPacket {
            players: players.into_iter().map(at).collect(),
            carrier: None,
        });
        self.carriers = carriers.into_iter().rev().take(2).map(at).collect();
    }

    fn clear(&mut self) {
        self.packet = None;
        self.carriers.clear();
        self.pending_names.clear();
    }

    /// The throttle: `true`, and the stamp moves, when a request may go out now.
    fn request_due(&mut self, now: Instant) -> bool {
        let due = self
            .last_request
            .is_none_or(|t| now.saturating_duration_since(t) >= REQUEST_THROTTLE);
        if due {
            self.last_request = Some(now);
        }
        due
    }
}

/// The getter's position source (`0x54c2e0`): a streamed object's live position, else the
/// packet's, as wow `(x, y)`.
fn live_or_packet(
    p: &BattlefieldPosition,
    guids: &GuidIndex,
    unit_pos: &Query<&GlobalTransform, With<NetEntity>>,
) -> (f32, f32) {
    match guids.0.get(&p.guid).and_then(|e| unit_pos.get(*e).ok()) {
        Some(tf) => {
            let w = bevy_to_wow(tf.translation());
            (w[0], w[1])
        }
        None => (p.x, p.y),
    }
}

/// `0x54d010`'s token from the carrier's side (`0x6d6e90`: 0 Horde, 1 Alliance): a Horde
/// carrier holds `AllianceFlag` (`0xacd310`), the pair swapped on Eye of the Storm (map 566).
fn flag_token(side: i8, map: u32) -> Option<&'static str> {
    let side = match side {
        0 | 1 if map == 566 => 1 - side,
        0 | 1 => side,
        _ => return None,
    };
    Some(["AllianceFlag", "HordeFlag"][side as usize])
}

/// The carrier's side: its own race when streamed, else the viewer's for a raid member and the
/// other side for anyone else (`0x549670`).
fn carrier_side(own_race: Option<u8>, raid_member: bool, viewer_race: Option<u8>) -> i8 {
    use crate::ui_unit::race_pvp_team;
    if let Some(race) = own_race {
        return race_pvp_team(race);
    }
    let viewer = viewer_race.map_or(-1, race_pvp_team);
    match (raid_member, viewer) {
        (true, side) => side,
        (false, 0) => 1,
        (false, _) => 0,
    }
}

/// `0x549670`: a guid in the raid roster (`0xbeb568`, count `0xbeb608`), so never a party's.
fn raid_member(group: &GroupState, guid: u64) -> bool {
    group.group_type == crate::app_core::state::GROUPTYPE_RAID
        && group.members.iter().any(|m| m.guid == guid)
}

pub(crate) fn feed_battlefield_positions(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<BattlefieldPositions>,
    queue: Res<BattlefieldQueue>,
    data: Option<Res<WorldMapUiData>>,
    maps: Option<Res<MapCatalogRes>>,
    me: Res<SelfGuid>,
    group: Res<GroupState>,
    guids: Res<GuidIndex>,
    unit_pos: Query<&GlobalTransform, With<NetEntity>>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    objects: crate::net::Objects,
    // The last empty push: with no list the engine is told on change, with one every frame.
    mut last_empty: Local<crate::app_core::vm_memo::VmMemo<Option<(bool, u32)>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let active = queue.active_map();
    // `GetBattlefieldMapIconScale`: the active slot's `Map.dbc` row, map 0's with no slot.
    let icon_scale = maps
        .as_deref()
        .and_then(|m| m.0.battleground(active.unwrap_or(0)))
        .map_or(1.0, |b| b.minimap_icon_scale);
    let BattlefieldPositions {
        packet,
        carriers,
        pending_names,
        ..
    } = &mut *state;
    let (Some(map), Some(packet)) = (active, packet.as_ref()) else {
        let key = Some((false, icon_scale.to_bits()));
        let last = last_empty.get(&script);
        if *last != key {
            *last = key;
            script.set_battlefield_positions(Vec::new(), None, icon_scale);
        }
        return;
    };
    *last_empty.get(&script) = Some((true, icon_scale.to_bits()));

    let selection = script.world_map_selection();
    let project = |x: f32, y: f32| {
        data.as_deref()
            .and_then(|d| project_on_displayed(d, selection, map, x, y))
            .unwrap_or((0.0, 0.0))
    };
    let mut name_landed = false;
    let mut players = Vec::with_capacity(packet.players.len());
    for p in &packet.players {
        if me.0 == Some(p.guid) || group.members.iter().any(|m| m.guid == p.guid) {
            continue;
        }
        let (x, y) = live_or_packet(p, &guids, &unit_pos);
        let name = names.resolve(p.guid, &commands).map(str::to_string);
        match name {
            None => {
                if !pending_names.contains(&p.guid) {
                    pending_names.push(p.guid);
                }
            }
            Some(_) => {
                if let Some(i) = pending_names.iter().position(|g| *g == p.guid) {
                    pending_names.swap_remove(i);
                    name_landed = true;
                }
            }
        }
        players.push(BattlefieldPositionView {
            uv: project(x, y),
            name,
        });
    }
    let race = |guid: u64| objects.object(guid).and_then(|o| o.unit_race());
    let viewer_race = me.0.and_then(race);
    let flags = carriers
        .iter()
        .map(|c| {
            let (x, y) = live_or_packet(c, &guids, &unit_pos);
            let member = raid_member(&group, c.guid);
            BattlefieldFlagView {
                uv: project(x, y),
                token: flag_token(carrier_side(race(c.guid), member, viewer_race), map)
                    .map(str::to_string),
            }
        })
        .collect();
    script.set_battlefield_positions(players, None, icon_scale);
    script.set_battlefield_flag_carriers(flags);
    if name_landed {
        script.fire_event("WORLD_MAP_NAME_UPDATE", Vec::new());
    }
}

fn drain_battlefield_positions(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<BattlefieldPositions>,
    queue: Res<BattlefieldQueue>,
    commands: Res<NetCommands>,
) {
    let Some(mut script) = script else {
        return;
    };
    if script.take_battlefield_position_requests() == 0 {
        return;
    }
    if queue.active_map().is_none() {
        state.clear();
        return;
    }
    if state.request_due(Instant::now()) {
        let _ = commands.0.send(ClientCommand::RequestBattlefieldPositions);
    }
}

pub(crate) fn reset_on_world_enter(
    mut entered: MessageReader<EnteredWorldMessage>,
    mut state: ResMut<BattlefieldPositions>,
) {
    if entered.read().next().is_none() {
        return;
    }
    state.clear();
    state.last_request = None;
}

/// The battlefield map's packet handler.
mod net {
    use bevy::prelude::*;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use super::BattlefieldPositions;
    use crate::net::NetHandlerApp;

    pub(super) fn register(app: &mut App) {
        app.net_handler(SessionEventKind::BattlefieldPositions, on_positions);
    }

    fn on_positions(In(ev): In<SessionEvent>, mut positions: ResMut<BattlefieldPositions>) {
        if let SessionEvent::BattlefieldPositions(packet) = ev {
            positions.apply(packet);
        }
    }
}

pub struct BattlefieldPositionsPlugin;

impl Plugin for BattlefieldPositionsPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        app.init_resource::<BattlefieldPositions>().add_systems(
            Update,
            (
                reset_on_world_enter.before(feed_battlefield_positions),
                feed_battlefield_positions.in_set(UiFeed),
                drain_battlefield_positions.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_throttle_is_the_references_five_seconds() {
        let mut s = BattlefieldPositions::default();
        let t0 = Instant::now();
        assert!(s.request_due(t0), "the first request goes out");
        assert!(!s.request_due(t0 + Duration::from_millis(4999)));
        assert!(s.request_due(t0 + REQUEST_THROTTLE));
        assert!(!s.request_due(t0 + REQUEST_THROTTLE + Duration::from_millis(1)));
    }

    #[test]
    fn the_flag_token_follows_the_carriers_side() {
        // A Horde carrier (side 0) holds the Alliance flag; Eye of the Storm swaps the pair.
        assert_eq!(flag_token(0, 489), Some("AllianceFlag"));
        assert_eq!(flag_token(1, 489), Some("HordeFlag"));
        assert_eq!(flag_token(0, 566), Some("HordeFlag"));
        assert_eq!(flag_token(-1, 489), None);
        // Streamed: its own race (a Tauren); else the viewer's side in the raid, the other out.
        assert_eq!(carrier_side(Some(6), true, Some(1)), 0);
        assert_eq!(carrier_side(None, true, Some(1)), 1);
        assert_eq!(carrier_side(None, false, Some(1)), 0);
        assert_eq!(carrier_side(None, false, Some(2)), 1);
        let mut group = crate::app_core::group::GroupState {
            members: vec![wrathplus_protocol::types::GroupMemberEntry {
                name: String::new(),
                guid: 7,
                status: 1,
                flags: 0,
            }],
            ..Default::default()
        };
        assert!(!raid_member(&group, 7), "a party is no raid roster");
        group.group_type = crate::app_core::state::GROUPTYPE_RAID;
        assert!(raid_member(&group, 7) && !raid_member(&group, 8));
    }

    #[test]
    fn the_last_wire_carrier_is_slot_one() {
        let mut s = BattlefieldPositions::default();
        s.apply_wrath(Vec::new(), vec![(7, 1.0, 2.0), (8, 3.0, 4.0)]);
        let guids: Vec<u64> = s.carriers.iter().map(|c| c.guid).collect();
        assert_eq!(guids, [8, 7]);
        s.apply_wrath(Vec::new(), vec![(7, 1.0, 2.0)]);
        assert_eq!(s.carriers.len(), 1);
    }
}
