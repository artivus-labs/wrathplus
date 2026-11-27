//! Build 12340's battleground feed: the reducer over the native battleground capture
//! ([`NativeBattlegroundPackets`]) and the app half of `PVPBattlegroundFrame.lua`,
//! `BattlefieldFrame.lua` and the world-PvP queue (`/tmp/azb-exp-bg{,2,3}-clean.log`).
//!
//! [`drain_native`] owns the capture: each packet goes to the table the reference's handler
//! writes. `SMSG_BATTLEFIELD_STATUS` (`0x54ae40`) to [`BattlefieldQueue`],
//! `SMSG_BATTLEFIELD_LIST` (`0x54e390`) and `SMSG_GROUP_JOINED_BATTLEGROUND` (`0x54b1c0`) here,
//! `MSG_PVP_LOG_DATA` (`0x54d280`) to the scoreboard, `MSG_BATTLEGROUND_PLAYER_POSITIONS`
//! (`0x54b3f0`) to the positions, and the battlefield-manager family (`0x54b610`, `0x5498c0`,
//! `0x54b680`, `0x5499c0`, `0x54b7a0`) to [`WorldPvp`] through `0x549500`.
//!
//! A list fires, by its shape: the random type from a battlemaster `NPC_PVPQUEUE_ANYWHERE`;
//! a list from anywhere else `PVPQUEUE_ANYWHERE_SHOW`; otherwise `BATTLEFIELDS_SHOW` for a
//! battlemaster's list, and `BATTLEFIELDS_CLOSED` then `BATTLEFIELDS_SHOW` for none (`0x5496d0`).
//!
//! A battlemaster's window is leashed to it ([`crate::ui_battlefield_leash`]). Receipts this
//! reducer leaves go on to [`NativeBattlegroundRest`]; `0x14b`, `0x4e5`, `0x4e8` are undecoded.
//! AzerothCore drops `CMSG_BATTLEFIELD_MGR_QUEUE_REQUEST` (`STATUS_NEVER`), so that request is
//! sent and never answered.
//!
//! The 1.12 kinds (`BattlefieldStatus`, `BattlefieldList`, `PvpLogData`,
//! `GroupJoinedBattleground`) are dropped by an explicit handler: build 12340 delivers them
//! natively, and the 1.12 path is retired.

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_assets::{AssetSet, LockRecover, MapCatalogRes, WorldAssets};
use wrathplus_formats::{Battlemasters, MapCatalog, RANDOM_BATTLEGROUND};
use wrathplus_protocol::types::Field;
use wrathplus_protocol::wrath_battleground::{
    BattlefieldList, Command as Bg, Packet, Rewards, StatusTail,
};
use wrathplus_ui::script::{
    BattlefieldListView, BattlefieldQueueSlot, BattlefieldRequest, BattlegroundRewardsView,
    BattlegroundTypeView, ScriptValue, UiScript, WorldPvpView,
};

use crate::app_core::group::GroupState;
use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::EnteredWorldMessage;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::WorldStates;
use crate::area::AreaTableRes;
use crate::names::NameCache;
use crate::net::wrath_battleground_capture::NativeBattlegroundPackets;
use crate::net::{ClientCommand, NetCommands};
use crate::ui_battlefield_positions::BattlefieldPositions;
use crate::ui_battlefield_score::BattlefieldScoreboard;
use crate::ui_dialog_verbs::{BattlefieldQueue, QueueSlot};

/// The world states the queue getters read (`0x543600`, `0x5435a0`, `0x5a2a40`, `0x5a2a70`).
const WS_WINTERGRASP_QUEUE: u32 = 0x1117;
const WS_WINTERGRASP_WAITING: u32 = 0xed9;
const WS_WINTERGRASP_NEXT: u32 = 0x1102;
const WS_ARENA_SEASON: u32 = 0xc77;
const WS_PREVIOUS_ARENA_SEASON: u32 = 0xf3d;

/// `0x5167e0`'s `AreaTable` flag.
const AREA_FLAG_PVP_POI: u32 = 0x8000;

/// `BattlemasterList.dbc` and `PvPDifficulty.dbc`, absent when the load fails.
#[derive(Resource)]
pub(crate) struct BattlemasterTable(pub(crate) Battlemasters);

/// The native receipts this reducer does not consume (the arena
/// command/event/error replies, arena-unit destroys), in arrival order, for their own consumer;
/// cleared with the capture at connect and disconnect.
#[derive(Resource, Default)]
pub(crate) struct NativeBattlegroundRest(pub(crate) std::collections::VecDeque<Packet>);

/// `MSG_INSPECT_ARENA_TEAMS` answers by inspected player, slot 0-2, for `GetInspectArenaTeamData`
/// (InspectUI); the last answer per slot wins. Zeroed at session end.
#[derive(Resource, Default)]
pub(crate) struct InspectArenaTeams(
    pub(crate) std::collections::HashMap<u64, [Option<InspectedArenaTeam>; 3]>,
);

/// One inspected team as `0x377` carries it: the id and the stats; the name, size and emblem
/// come from the team-query cache ([`crate::ui_arena_team`] joins them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InspectedArenaTeam {
    pub(crate) team_id: u32,
    pub(crate) rating: u32,
    pub(crate) season_games: u32,
    pub(crate) season_wins: u32,
    pub(crate) member_games: u32,
    pub(crate) personal_rating: u32,
}

/// The list window's state.
#[derive(Resource, Default)]
pub struct Battlefield {
    list: Option<BattlefieldList>,
    /// `0xbea53c`: the last list's type, or the type `RequestBattlegroundInstanceInfo` asked for.
    bg_type: u32,
    /// `0xbea568`: the battlemaster the list came from; 0 for none.
    battlemaster: u64,
    /// Events the last lists owe, in order.
    events: Vec<&'static str>,
    dirty: bool,
    /// A battlemaster's window is open and leashed to it (`0x51fce0`).
    leashed: bool,
    /// The group-join verdicts, `(result, guid)`.
    verdicts: Vec<(i32, Option<u64>)>,
    /// Joined (`true`) or left guids awaiting a name.
    players: Vec<(u64, bool)>,
}

impl Battlefield {
    /// `0x54e390`.
    pub(crate) fn apply_list(&mut self, list: BattlefieldList) {
        self.bg_type = list.bg_type;
        let guid = list.battlemaster;
        if list.bg_type == RANDOM_BATTLEGROUND && guid != 0 {
            self.battlemaster = guid;
            self.events.push("NPC_PVPQUEUE_ANYWHERE");
        } else if list.from_where != 0 {
            self.events.push("PVPQUEUE_ANYWHERE_SHOW");
        } else if guid == 0 {
            self.battlemaster = 0;
            self.events.push("BATTLEFIELDS_CLOSED");
            self.events.push("BATTLEFIELDS_SHOW");
        } else {
            self.battlemaster = guid;
            self.events.push("BATTLEFIELDS_SHOW");
        }
        self.list = Some(list);
        self.dirty = true;
    }

    /// `CloseBattlefield` (`0x549b40`) or a broken leash (`0x518d50`): the battlemaster clears.
    pub(crate) fn close(&mut self) {
        self.battlemaster = 0;
        self.leashed = false;
    }

    pub(crate) fn apply_verdict(&mut self, result: i32, guid: Option<u64>) {
        self.verdicts.push((result, guid));
    }

    /// `SMSG_BATTLEGROUND_PLAYER_JOINED`/`_LEFT` (`0x54b510`): one line once the name resolves.
    pub(crate) fn apply_player(&mut self, guid: u64, joined: bool) {
        self.players.push((guid, joined));
    }

    /// The world-enter reset (`0x54ca50`).
    fn clear_session(&mut self) {
        self.list = None;
        self.bg_type = 0;
        self.battlemaster = 0;
        self.dirty = true;
        self.leashed = false;
        self.events.clear();
        self.verdicts.clear();
        self.players.clear();
    }

    pub fn battlemaster(&self) -> Option<u64> {
        (self.battlemaster != 0).then_some(self.battlemaster)
    }

    /// The events owed; a battlemaster's window leashes to it (`0x54970f`, `0x54e52e`).
    fn take_events(&mut self) -> Vec<&'static str> {
        let window = |e: &&str| matches!(*e, "BATTLEFIELDS_SHOW" | "NPC_PVPQUEUE_ANYWHERE");
        self.leashed |= self.battlemaster != 0 && self.events.iter().any(window);
        std::mem::take(&mut self.events)
    }

    /// The battlemaster a window is leashed to.
    pub(crate) fn leashed(&self) -> Option<u64> {
        self.battlemaster().filter(|_| self.leashed)
    }

    /// The listed battleground type, `CMSG_BATTLEMASTER_JOIN`'s `bgTypeId` (0 before a list).
    pub fn bg_type(&self) -> u32 {
        self.bg_type
    }
}

/// The world-PvP queue (`0xbea4a8..b4`) and the battlefield-manager events owed.
#[derive(Resource, Default)]
pub(crate) struct WorldPvp {
    battle_id: u32,
    zone: u32,
    status: u32,
    deadline: u32,
    events: Vec<(&'static str, Vec<ScriptValue>)>,
    /// `SMSG_BATTLEFIELD_MGR_ENTERED`'s `CLEARED_AFK` line.
    cleared_afk: bool,
}

impl WorldPvp {
    /// `0x549500`: a different battle only takes over an idle queue.
    fn set(&mut self, battle_id: u32, status: u32, zone: u32, deadline: u32) {
        if self.battle_id != battle_id && self.zone != 0 && self.status != 0 {
            return;
        }
        if zone != 0 {
            self.zone = zone;
        }
        if deadline != 0 {
            self.deadline = deadline;
        }
        self.battle_id = battle_id;
        self.status = status;
    }

    /// The idle reset `0x5498c0` and `0x5499c0` share.
    fn idle(&mut self, battle_id: u32) {
        if self.battle_id == battle_id || self.zone == 0 || self.status == 0 {
            self.battle_id = battle_id;
            self.status = 0;
        }
    }

    fn apply(&mut self, packet: &Packet, in_zone: impl Fn(u32) -> bool) {
        let int = |v: u32| ScriptValue::Int(i64::from(v));
        let b = ScriptValue::Bool;
        match *packet {
            Packet::BfEntryInvite {
                battle_id,
                zone,
                deadline,
            } => {
                self.set(battle_id, 2, zone, deadline);
                self.events
                    .push(("BATTLEFIELD_MGR_ENTRY_INVITE", vec![int(battle_id)]));
            }
            Packet::BfEntered {
                battle_id,
                raw_bytes,
                clear_afk,
            } => {
                self.cleared_afk |= clear_afk != 0;
                self.idle(battle_id);
                self.events.push((
                    "BATTLEFIELD_MGR_ENTERED",
                    vec![int(battle_id), b(raw_bytes[0] != 0), b(raw_bytes[1] != 0)],
                ));
            }
            Packet::BfQueueResponse {
                battle_id,
                zone,
                accepted,
                logging_in,
                warmup,
            } => {
                self.set(battle_id, u32::from(accepted != 0), zone, 0);
                self.events.push((
                    "BATTLEFIELD_MGR_QUEUE_REQUEST_RESPONSE",
                    vec![
                        int(battle_id),
                        b(accepted != 0),
                        b(warmup == 1),
                        b(in_zone(zone)),
                        b(logging_in != 0),
                    ],
                ));
            }
            Packet::BfEjected {
                battle_id,
                reason,
                battle_status,
                relocated,
            } => {
                if matches!(reason, 1 | 2 | 4 | 5 | 7..=11) {
                    self.idle(battle_id);
                }
                self.events.push((
                    "BATTLEFIELD_MGR_EJECTED",
                    vec![
                        int(battle_id),
                        b(reason == 8),
                        b(relocated != 0),
                        b(battle_status == 2),
                        b(reason == 10),
                    ],
                ));
            }
            Packet::BfQueueInvite {
                battle_id,
                raw_warmup,
            } => self.events.push((
                "BATTLEFIELD_MGR_QUEUE_INVITE",
                vec![int(battle_id), b(raw_warmup == 1)],
            )),
            _ => {}
        }
    }
}

/// `SMSG_GROUP_JOINED_BATTLEGROUND`'s game-error line (`0x54b1c0`), with the named player for the
/// two that carry one; `None` for -1, which shows nothing.
fn verdict_key(result: i32, named: bool, bg_known: bool) -> Option<&'static str> {
    Some(match result {
        -15 => "ERR_IN_NON_RANDOM_BG",
        -14 => "ERR_IN_RANDOM_BG",
        -13 => "ERR_LFG_CANT_USE_BATTLEGROUND",
        -12 | -11 if named => "ERR_BATTLEGROUND_JOIN_TIMED_OUT",
        -12 | -11 => "ERR_BATTLEGROUND_JOIN_FAILED",
        -10 => "ERR_BATTLEGROUND_JOIN_RANGE_INDEX",
        -9 => "ERR_BATTLEGROUND_JOIN_XP_GAIN",
        -8 => "ERR_BATTLEGROUND_NOT_IN_BATTLEGROUND",
        -7 => "ERR_BATTLEGROUND_TEAM_LEFT_QUEUE",
        -6 => "ERR_BATTLEDGROUND_QUEUED_FOR_RATED",
        -5 => "ERR_BATTLEGROUND_CANNOT_QUEUE_FOR_RATED",
        -4 => "ERR_BATTLEGROUND_TOO_MANY_QUEUES",
        -3 => "ERR_ARENA_TEAM_PARTY_SIZE",
        -2 => "ERR_GROUP_JOIN_BATTLEGROUND_DESERTERS",
        -1 => return None,
        _ if bg_known => "ERR_GROUP_JOIN_BATTLEGROUND_S",
        _ => "ERR_GROUP_JOIN_BATTLEGROUND_FAIL",
    })
}

fn rewards(r: &Rewards) -> BattlegroundRewardsView {
    BattlegroundRewardsView {
        has_win: r.has_win != 0,
        win_honor: r.win_honor,
        win_arena: r.win_arena,
        loss_honor: r.loss_honor,
    }
}

/// The player's side (`0x6d6e90`): 0 Horde, 1 Alliance, -1 neither, from the race.
fn side(store: Option<&ObjectStore>) -> i8 {
    store
        .and_then(|s| s.0.unit_race())
        .map_or(-1, crate::ui_unit::race_pvp_team)
}

/// Every `BattlemasterList` type as the VM reads it, for the player's level and side.
fn type_views(
    table: &Battlemasters,
    maps: Option<&MapCatalog>,
    states: &WorldStates,
    level: u32,
    side: i8,
) -> Vec<BattlegroundTypeView> {
    table
        .types()
        .iter()
        .map(|t| {
            let map = u32::try_from(t.maps[0]).ok();
            let row = map.and_then(|m| maps.and_then(|c| c.battleground(m)));
            BattlegroundTypeView {
                id: t.id,
                name: t.name.clone(),
                instance_type: t.instance_type,
                groups_allowed: t.groups_allowed != 0,
                max_group_size: t.max_group_size,
                min_level: t.min_level,
                can_enter: table.bracket_for(t, level).is_some(),
                is_holiday: t.holiday_world_state != 0 && states.get(t.holiday_world_state) != 0,
                description: match side {
                    0 => row.map(|r| r.descriptions[0].clone()),
                    1 => row.map(|r| r.descriptions[1].clone()),
                    _ => None,
                },
                has_map: row.is_some(),
            }
        })
        .collect()
}

/// One slot's VM view at `now`.
fn slot_view(
    slot: Option<&(QueueSlot, Instant)>,
    table: Option<&Battlemasters>,
    maps: Option<&MapCatalog>,
    now: Instant,
) -> BattlefieldQueueSlot {
    let Some((s, at)) = slot else {
        return BattlefieldQueueSlot::default();
    };
    let st = &s.status;
    let type_name = || {
        table
            .and_then(|t| t.get(st.bg_type))
            .map(|t| t.name.clone())
    };
    let map_name = || maps.and_then(|c| c.name(s.map)).map(str::to_string);
    // `0x54bc30`: an invite names its map unless it is the random type with a zero tail word; an
    // active slot names its map; either falls back to the type's name.
    let raw_unknown = match st.tail {
        StatusTail::Invited { raw_unknown, .. } => raw_unknown,
        _ => 0,
    };
    let name = match st.status {
        2 if st.bg_type != RANDOM_BATTLEGROUND || raw_unknown != 0 => map_name().or_else(type_name),
        3 => map_name().or_else(type_name),
        _ => type_name(),
    };
    let ms = |d: std::time::Duration| d.as_millis().min(u128::from(u32::MAX)) as u32;
    BattlefieldQueueSlot {
        status: st.status,
        bg_type: st.bg_type,
        name,
        instance_id: st.client_instance,
        min_level: u32::from(st.min_level),
        max_level: u32::from(st.max_level),
        team_size: u32::from(st.arena_type & 0x7f),
        rated: st.rated != 0,
        port_expiration_secs: s
            .port_deadline
            .map_or(0, |d| ms(d.saturating_duration_since(now)) / 1000),
        estimated_wait_ms: s.queued.map_or(0, |(avg, _)| avg),
        time_waited_ms: s.queued.map_or(0, |(_, waited)| {
            ms(now.saturating_duration_since(*at) + waited)
        }),
    }
}

fn list_view(state: &Battlefield) -> BattlefieldListView {
    let Some(l) = state.list.as_ref() else {
        return BattlefieldListView {
            bg_type: state.bg_type,
            ..Default::default()
        };
    };
    BattlefieldListView {
        bg_type: state.bg_type,
        instances: l.instances.clone(),
        holiday: rewards(&l.rewards),
        random: l.random_rewards.as_ref().map(rewards).unwrap_or_default(),
    }
}

fn load_battlemasters(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = wrathplus_formats::load_battlemasters(&mut assets.chain.lock_recover());
    match loaded {
        Ok(table) => commands.insert_resource(BattlemasterTable(table)),
        Err(e) => warn!(
            "battlefield: BattlemasterList/PvPDifficulty failed to load — the PvP frame lists no \
             battleground types: {e:#}"
        ),
    }
}

/// The reducer: every native battleground receipt in arrival order, to its table.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_native(
    mut packets: ResMut<NativeBattlegroundPackets>,
    mut state: ResMut<Battlefield>,
    mut queue: ResMut<BattlefieldQueue>,
    mut board: ResMut<BattlefieldScoreboard>,
    mut positions: ResMut<BattlefieldPositions>,
    mut world_pvp: ResMut<WorldPvp>,
    mut rest: ResMut<NativeBattlegroundRest>,
    mut inspect: ResMut<InspectArenaTeams>,
    mut honor: ResMut<crate::ui_honor::InspectHonor>,
    maps: Option<Res<MapCatalogRes>>,
    areas: Option<Res<AreaTableRes>>,
    world: wrathplus_world::world_point::WorldPoint,
) {
    if packets.0.is_empty() {
        return;
    }
    let catalog = maps.as_deref().map(|m| &m.0);
    let now = Instant::now();
    let zone = world.area();
    // `0x54b680`: the player's area is the zone, or its parent is.
    let in_zone = |z: u32| {
        zone.is_some_and(|a| {
            a == z
                || areas
                    .as_deref()
                    .and_then(|t| t.0.get(a))
                    .is_some_and(|r| r.zone_id == z)
        })
    };
    for packet in packets.0.drain(..) {
        match packet {
            Packet::Status { slot, status } => queue.apply(
                slot,
                status,
                |map| catalog.and_then(|c| c.instance_type(map)).unwrap_or(0),
                now,
            ),
            Packet::List(list) => state.apply_list(list),
            Packet::PvpLog(log) => board.apply(log),
            Packet::GroupJoined { result, guid } => state.apply_verdict(result, guid),
            Packet::Positions { players, carriers } => positions.apply_wrath(players, carriers),
            p @ (Packet::BfEntryInvite { .. }
            | Packet::BfEntered { .. }
            | Packet::BfQueueResponse { .. }
            | Packet::BfEjected { .. }
            | Packet::BfQueueInvite { .. }) => world_pvp.apply(&p, in_zone),
            Packet::InspectArenaTeam {
                guid,
                slot,
                team_id,
                rating,
                season_games,
                season_wins,
                member_games,
                personal_rating,
            } => {
                if let Some(cell) = inspect
                    .0
                    .entry(guid)
                    .or_default()
                    .get_mut(usize::from(slot))
                {
                    *cell = Some(InspectedArenaTeam {
                        team_id,
                        rating,
                        season_games,
                        season_wins,
                        member_games,
                        personal_rating,
                    });
                }
            }
            Packet::InspectHonor {
                guid,
                raw_points,
                kills,
                today_honor,
                yesterday_honor,
                lifetime_kills,
            } => {
                honor.0 = Some(crate::ui_honor::InspectHonorReply {
                    player_guid: guid,
                    lifetime_rank: raw_points,
                    kills,
                    today_honor,
                    yesterday_honor,
                    lifetime_hk: lifetime_kills,
                })
            }
            other => rest.0.push_back(other),
        }
    }
}

/// `0x54ccc5`-`0x54ccd9`: the type's `MaxGroupSize` below the party without the player
/// (`0x52bd10`) or the raid with the player (`[0xbeb608]`, 0 outside a raid).
fn join_too_large(group: Option<&GroupState>, max: u32) -> bool {
    let Some(g) = group else { return false };
    let raid = usize::from(g.group_type == 1) * (g.members.len() + 1);
    (max as usize) < g.party_slots().count() || (max as usize) < raid
}

/// Runs before the dialog and score feeds fire `UPDATE_BATTLEFIELD_STATUS` and
/// `UPDATE_BATTLEFIELD_SCORE`, so their handlers read this frame's list and slots.
#[allow(clippy::too_many_arguments)]
fn feed_battlefield(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<Battlefield>,
    mut world_pvp: ResMut<WorldPvp>,
    queue: Res<BattlefieldQueue>,
    table: Option<Res<BattlemasterTable>>,
    maps: Option<Res<MapCatalogRes>>,
    areas: Option<Res<AreaTableRes>>,
    states: Res<WorldStates>,
    world: wrathplus_world::world_point::WorldPoint,
    me: Query<&ObjectStore, With<SelfPlayer>>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    mut sink: crate::ui_action::MessageSink,
    mut memo: Local<crate::app_core::vm_memo::VmMemo<Option<Vec<BattlegroundTypeView>>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let catalog = maps.as_deref().map(|m| &m.0);
    let bm = table.as_deref().map(|t| &t.0);
    let now = Instant::now();
    let store = me.iter().next();

    if let Some(bm) = bm {
        let level = store.and_then(|s| s.0.unit_level()).unwrap_or(0);
        let types = type_views(bm, catalog, &states, level, side(store));
        let last = memo.get(&script);
        if last.as_ref() != Some(&types) {
            script.set_battleground_types(types.clone());
            *last = Some(types);
        }
    }
    if std::mem::take(&mut state.dirty) {
        script.set_battlefield_list(list_view(&state));
    }
    let slots = queue
        .slots()
        .each_ref()
        .map(|s| slot_view(s.as_ref(), bm, catalog, now));
    script.set_battlefield_queue(
        slots,
        queue.instance_expiration_ms(now),
        queue.active_instance_type() == 4,
        queue.active_rated(),
    );
    script.set_battlefield_arena_faction(queue.arena_faction != 0);
    let zone_name = areas
        .as_deref()
        .and_then(|a| a.0.name(world_pvp.zone))
        .map(str::to_string);
    let poi = world
        .area()
        .and_then(|a| areas.as_deref().and_then(|t| t.0.get(a)))
        .is_some_and(|r| r.flags & AREA_FLAG_PVP_POI != 0);
    script.set_world_pvp(WorldPvpView {
        status: world_pvp.status,
        zone_name,
        battle_id: world_pvp.battle_id,
        deadline: world_pvp.deadline,
        can_queue_wintergrasp: states.get(WS_WINTERGRASP_QUEUE) != 0,
        wintergrasp_waiting: states.get(WS_WINTERGRASP_WAITING) != 0,
        wintergrasp_next: states.get(WS_WINTERGRASP_NEXT),
        arena_season: states.get(WS_ARENA_SEASON),
        previous_arena_season: states.get(WS_PREVIOUS_ARENA_SEASON),
        subzone_pvp_poi: poi,
    });

    for event in state.take_events() {
        script.fire_event(event, vec![]);
    }

    let mut lines = Vec::new();
    if std::mem::take(&mut world_pvp.cleared_afk) {
        lines.extend(crate::ui_action::keyed_line(&script, "CLEARED_AFK"));
    }
    for (event, args) in std::mem::take(&mut world_pvp.events) {
        script.fire_event(event, args);
    }
    for (result, guid) in std::mem::take(&mut state.verdicts) {
        // `0x54b1c0` looks the name up without waiting for it.
        let name = guid.and_then(|g| names.peek(g)).map(str::to_string);
        let bg_name = bm
            .and_then(|t| t.get(result as u32))
            .map(|t| t.name.clone());
        let Some(key) = verdict_key(result, name.is_some(), bg_name.is_some()) else {
            continue;
        };
        let line = match (key, &name, &bg_name) {
            ("ERR_BATTLEGROUND_JOIN_TIMED_OUT", Some(n), _) => {
                crate::ui_action::keyed_line_s(&script, key, &[n])
            }
            ("ERR_GROUP_JOIN_BATTLEGROUND_S", _, Some(b)) => {
                crate::ui_action::keyed_line_s(&script, key, &[b])
            }
            _ => crate::ui_action::keyed_line(&script, key),
        };
        lines.extend(line);
    }
    let players = std::mem::take(&mut state.players);
    for (guid, joined) in players {
        let Some(name) = names.resolve(guid, &commands).map(str::to_string) else {
            state.players.push((guid, joined));
            continue;
        };
        let line = if joined {
            crate::ui_action::keyed_line_s(&script, "ERR_BG_PLAYER_JOINED_SS", &[&name, &name])
        } else {
            crate::ui_action::keyed_line_s(&script, "ERR_BG_PLAYER_LEFT_S", &[&name])
        };
        lines.extend(line);
    }
    if !lines.is_empty() {
        crate::ui_action::show_messages(&mut script, &mut sink, "ui_battlefield", lines);
    }
}

/// The VM's battleground requests, in call order.
#[allow(clippy::too_many_arguments)]
fn drain_battlefield(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<Battlefield>,
    queue: Res<BattlefieldQueue>,
    table: Option<Res<BattlemasterTable>>,
    maps: Option<Res<MapCatalogRes>>,
    group: Option<Res<GroupState>>,
    current_map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    me: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    mut sink: crate::ui_action::MessageSink,
) {
    let Some(mut script) = script else {
        return;
    };
    let requests = script.take_battlefield_requests();
    if requests.is_empty() {
        return;
    }
    let bm = table.as_deref().map(|t| &t.0);
    let catalog = maps.as_deref().map(|m| &m.0);
    let store = me.iter().next();
    let send = |c: Bg| {
        let _ = commands.0.send(ClientCommand::NativeBattleground(c));
    };
    let mut errors: Vec<&'static str> = Vec::new();
    for request in requests {
        match request {
            BattlefieldRequest::List { bg_type } => {
                // `0x54e5b0`: the can-gain-XP byte is the player-flags bit 25 under the level cap.
                let level = store.and_then(|s| s.0.unit_level()).unwrap_or(0);
                let cap = store
                    .and_then(|s| s.0.get(Field::PlayerMaxLevel))
                    .unwrap_or(0);
                let flags = store.and_then(|s| s.0.get(Field::PlayerFlags)).unwrap_or(0);
                let can_gain_xp = u8::from(level < cap && flags >> 25 & 1 != 0);
                state.bg_type = bg_type;
                state.list.iter_mut().for_each(|l| l.instances.clear()); // `0x54e6a8`
                send(Bg::List {
                    bg_type,
                    from_where: 1,
                    can_gain_xp,
                });
            }
            BattlefieldRequest::Join {
                index,
                as_group,
                rated,
            } => {
                let ty = bm.and_then(|t| t.get(state.bg_type));
                let arena = ty.is_some_and(|t| t.instance_type == 4);
                let max = ty.map(|t| t.max_group_size);
                if as_group && max.is_some_and(|m| join_too_large(group.as_deref(), m)) {
                    errors.push("ERR_GROUP_JOIN_BATTLEGROUND_TOO_MANY");
                    continue;
                }
                // Not from inside a battleground, except into an arena.
                let here = current_map
                    .as_deref()
                    .and_then(|m| catalog.and_then(|c| c.instance_type(m.0)));
                if !arena && here == Some(3) {
                    errors.push("ERR_BATTLEGROUND_NOT_IN_BATTLEGROUND");
                    continue;
                }
                let battlemaster = state.battlemaster;
                if arena {
                    send(Bg::JoinArena {
                        battlemaster,
                        slot: index as u8,
                        as_group: u8::from(as_group),
                        rated: u8::from(rated),
                    });
                } else {
                    let instance_id = state
                        .list
                        .as_ref()
                        .filter(|_| battlemaster != 0)
                        .and_then(|l| usize::try_from(index).ok().and_then(|i| l.instances.get(i)))
                        .copied()
                        .unwrap_or(0);
                    send(Bg::Join {
                        battlemaster,
                        bg_type: state.bg_type,
                        instance_id,
                        as_group: u8::from(as_group),
                    });
                }
            }
            BattlefieldRequest::Port { slot, accept } => {
                if let Some(c) = queue.port_command(slot, accept) {
                    send(c);
                }
            }
            BattlefieldRequest::BfExit { battle_id } => send(Bg::BfExit { battle_id }),
            BattlefieldRequest::BfQueueRequest { battle_id } => {
                send(Bg::BfQueueRequest { battle_id })
            }
            BattlefieldRequest::Close => state.close(),
        }
    }
    let lines: Vec<_> = errors
        .into_iter()
        .filter_map(|key| crate::ui_action::keyed_line(&script, key))
        .collect();
    if !lines.is_empty() {
        crate::ui_action::show_messages(&mut script, &mut sink, "ui_battlefield", lines);
    }
}

/// World enter (`0x54ca50`): the list, the selection and the battlemaster clear, the queue slots
/// stay, and the bodyless `CMSG_BATTLEFIELD_STATUS` goes out, answered slot by slot.
fn reset_on_world_enter(
    mut entered: MessageReader<EnteredWorldMessage>,
    mut state: ResMut<Battlefield>,
    script: Option<NonSendMut<UiScript>>,
    commands: Res<NetCommands>,
) {
    if entered.read().next().is_none() {
        return;
    }
    state.clear_session();
    if let Some(mut script) = script {
        script.reset_battlefield_selection();
    }
    request_battlefield_status(&commands);
}

fn request_battlefield_status(commands: &NetCommands) {
    let _ = commands.0.send(ClientCommand::BattlefieldStatusRequest);
}

/// The battlemaster window's shared packet handlers.
mod net {
    use bevy::prelude::*;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use super::{Battlefield, InspectArenaTeams, NativeBattlegroundRest, WorldPvp};
    use crate::net::NetHandlerApp;

    pub(super) fn register(app: &mut App) {
        use SessionEventKind as K;
        app.net_handler(K::BattlegroundPlayer, on_player)
            .net_handler(K::Connected, on_connect)
            .net_handler(K::Disconnected, on_session_end);
        app.net_handler(K::BattlefieldStatus, retired_1_12)
            .net_handler(K::BattlefieldList, retired_1_12)
            .net_handler(K::PvpLogData, retired_1_12)
            .net_handler(K::GroupJoinedBattleground, retired_1_12);
    }

    /// A 1.12 battleground receipt: build 12340 delivers these natively, so this drops it.
    fn retired_1_12(In(ev): In<SessionEvent>) {
        if matches!(
            ev,
            SessionEvent::BattlefieldStatus(_)
                | SessionEvent::BattlefieldList(_)
                | SessionEvent::PvpLogData(_)
                | SessionEvent::GroupJoinedBattleground { .. }
        ) {
            debug!("battlefield: 1.12 receipt dropped, the native reducer owns it: {ev:?}");
        }
    }

    fn on_connect(In(ev): In<SessionEvent>, mut rest: ResMut<NativeBattlegroundRest>) {
        if matches!(ev, SessionEvent::Connected { .. }) {
            rest.0.clear();
        }
    }

    fn on_player(In(ev): In<SessionEvent>, mut battlefield: ResMut<Battlefield>) {
        if let SessionEvent::BattlegroundPlayer { guid, joined } = ev {
            battlefield.apply_player(guid, joined);
        }
    }

    fn on_session_end(
        In(_): In<SessionEvent>,
        mut battlefield: ResMut<Battlefield>,
        mut world_pvp: ResMut<WorldPvp>,
        mut rest: ResMut<NativeBattlegroundRest>,
        mut inspect: ResMut<InspectArenaTeams>,
    ) {
        battlefield.clear_session();
        *world_pvp = WorldPvp::default();
        rest.0.clear();
        inspect.0.clear();
    }
}

pub struct BattlefieldPlugin;

impl Plugin for BattlefieldPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        crate::ui_battlefield_arena_lines::register(app);
        app.init_resource::<Battlefield>()
            .init_resource::<WorldPvp>()
            .init_resource::<NativeBattlegroundRest>()
            .init_resource::<InspectArenaTeams>()
            .add_systems(Startup, load_battlemasters.after(AssetSet::Open))
            .add_systems(
                Update,
                (
                    reset_on_world_enter.in_set(UiFeed).before(drain_native),
                    drain_native
                        .in_set(UiFeed)
                        .after(crate::ui_battlefield_positions::reset_on_world_enter)
                        .after(crate::ui_battlefield_score::clear_on_world_enter)
                        .before(crate::ui_battlefield_positions::feed_battlefield_positions)
                        .before(feed_battlefield),
                    feed_battlefield
                        .before(crate::ui_battlefield_score::feed_battlefield_score)
                        .before(crate::ui_dialog_verbs::feed_dialog_verbs)
                        .in_set(UiFeed),
                    crate::ui_battlefield_leash::leash_battlemaster
                        .after(feed_battlefield)
                        .in_set(UiFeed)
                        .run_if(crate::app_core::ui_state::ingame_ui_up),
                    drain_battlefield.after(UiInput),
                ),
            );
    }
}

#[cfg(test)]
#[path = "ui_battlefield_tests.rs"]
mod tests;
