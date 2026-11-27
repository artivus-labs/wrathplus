//! The arena-team reducer: sole owner of the lossless arena-team capture, folded into the
//! client's three-slot team table (`0xc0f840`) and published to the VM for `PVPFrame.lua`'s
//! bindings (`wrathplus_ui::script::wrath_arena`).
//!
//! - The slots follow the player's `PLAYER_FIELD_ARENA_TEAM_INFO` ids (`0x5a3700`): a slot whose
//!   id changes to a team asks for its roster (`0x5a3600`, CMSG `0x34D`, at most every 10 s a
//!   slot), and `ARENA_TEAM_UPDATE` fires. The member list stays until the new roster lands.
//! - `SMSG_ARENA_TEAM_ROSTER` (`0x5a3e10`) replaces the matching slot's members, counts the online
//!   ones and fires `ARENA_TEAM_ROSTER_UPDATE`; `SMSG_ARENA_TEAM_STATS` (`0x5a2d50`) stores the
//!   slot's stats and fires `ARENA_TEAM_UPDATE`. A packet for a team no slot holds is dropped.
//! - `SMSG_ARENA_TEAM_QUERY_RESPONSE` fills the team record `GetArenaTeam` reads, and
//!   `ARENA_TEAM_UPDATE` fires when a slot holds the team (`0x5a28c0`, the cache callback).
//! - `SMSG_ARENA_TEAM_INVITE` fires `ARENA_TEAM_INVITE_REQUEST(inviter, team)`.
//!
//! Unresolved, labelled: the reference asks for a team record lazily, on `GetArenaTeam`'s cache
//! miss (`0x680170`); this reducer asks once per team as its id appears. The descriptor-change
//! site (`0x6cdda2`) is untraced, so `ARENA_TEAM_UPDATE` fires on any change to the 21 descriptor
//! words. The roster's class name is the display name without the sex-specific form
//! (`0x72aab0`). The sort, the filter and the selection reset with the VM, where the reference's
//! survive a reload.
use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::net::{ClientCommand, NetCommands, NetHandlerApp};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use wrathplus_protocol::types::Field;
use wrathplus_protocol::{
    wrath_arena_team::{Command, Member, Packet},
    SessionEvent, SessionEventKind,
};
use wrathplus_ui::script::{
    ArenaMemberView, ArenaTeamInfoView, ArenaTeamStatsView, ArenaTeamView, InspectArenaTeamView,
    ScriptValue, UiScript,
};

#[derive(Resource, Default)]
pub(crate) struct NativeArenaTeamPackets(pub(crate) VecDeque<Packet>);

/// `SMSG_ARENA_TEAM_EVENT`s seen by the arena lines' drain, each `0x5a28e0(slot, 1)` for every
/// slot ([`ArenaTable::mark_stale`]).
#[derive(Resource, Default)]
pub(crate) struct ArenaRosterStale(pub(crate) usize);

/// `0x5a3600`'s per-slot throttle (`0xc0f7f0`).
const ROSTER_THROTTLE_SECS: f64 = 10.0;
/// `0x47b480`'s 48-byte name buffer, terminator included.
const NAME_BYTES: usize = 47;

/// What one step asks of the session and the VM.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Effects {
    pub(crate) sends: Vec<Command>,
    /// The slots whose rosters landed, each published and firing `ARENA_TEAM_ROSTER_UPDATE`.
    pub(crate) rosters: Vec<usize>,
    pub(crate) events: Vec<(&'static str, Vec<ScriptValue>)>,
}

/// The client's team table and team-record cache.
#[derive(Resource, Default)]
pub(crate) struct ArenaTable {
    descriptor: [[u32; 7]; 3],
    members: [Vec<ArenaMemberView>; 3],
    stats: [ArenaTeamStatsView; 3],
    records: HashMap<u32, ArenaTeamInfoView>,
    queried: HashSet<u32>,
    /// When each slot may next ask for its roster, on the UI clock.
    next_roster: [Option<f64>; 3],
}

impl ArenaTable {
    fn team_id(&self, slot: usize) -> u32 {
        self.descriptor[slot][0]
    }

    fn slot_of(&self, team_id: u32) -> Option<usize> {
        (0..3).find(|&s| self.team_id(s) == team_id)
    }

    /// `0x5a3600`: ask for the slot's roster unless it has no team or asked under 10 s ago.
    fn ask_roster(&mut self, slot: usize, now: f64, fx: &mut Effects) {
        let team_id = self.team_id(slot);
        if team_id == 0 || self.next_roster[slot].is_some_and(|t| now < t) {
            return;
        }
        fx.sends.push(Command::Roster { team_id });
        self.next_roster[slot] = Some(now + ROSTER_THROTTLE_SECS);
    }

    /// Follow the player's descriptor (`0x5a3700`).
    pub(crate) fn sync_descriptor(&mut self, words: [[u32; 7]; 3], now: f64, fx: &mut Effects) {
        if words == self.descriptor {
            return;
        }
        let was = self.descriptor;
        self.descriptor = words;
        for slot in 0..3 {
            let id = words[slot][0];
            if id != 0 && id != was[slot][0] {
                self.ask_roster(slot, now, fx);
            }
            if id != 0 && !self.records.contains_key(&id) && self.queried.insert(id) {
                fx.sends.push(Command::Query { team_id: id });
            }
        }
        fx.events.push(("ARENA_TEAM_UPDATE", Vec::new()));
    }

    /// `0x5a28e0(slot, 1)` for each slot, as the event handler `0x6cc980` calls it: the slot's
    /// throttle clears and `ARENA_TEAM_ROSTER_UPDATE(1)` fires, on which `PVPFrame.lua:44-48`
    /// re-asks the shown team's roster.
    pub(crate) fn mark_stale(&mut self, fx: &mut Effects) {
        for slot in 0..3 {
            self.next_roster[slot] = None;
            fx.events
                .push(("ARENA_TEAM_ROSTER_UPDATE", vec![ScriptValue::Int(1)]));
        }
    }

    /// The frame's `ArenaTeamRoster` calls.
    pub(crate) fn request_roster(&mut self, slot: u8, now: f64, fx: &mut Effects) {
        if usize::from(slot) < 3 {
            self.ask_roster(usize::from(slot), now, fx);
        }
    }

    pub(crate) fn apply(&mut self, packet: Packet, fx: &mut Effects) {
        match packet {
            Packet::Roster {
                team_id, members, ..
            } => {
                let Some(slot) = self.slot_of(team_id) else {
                    return;
                };
                self.members[slot] = members.iter().map(member).collect();
                fx.rosters.push(slot);
            }
            Packet::Stats {
                team_id,
                rating,
                week_games,
                week_wins,
                season_games,
                season_wins,
                rank,
            } => {
                let Some(slot) = self.slot_of(team_id) else {
                    return;
                };
                self.stats[slot] = ArenaTeamStatsView {
                    rating,
                    week_games,
                    week_wins,
                    season_games,
                    season_wins,
                    rank,
                };
                fx.events.push(("ARENA_TEAM_UPDATE", Vec::new()));
            }
            Packet::Query {
                team_id,
                name,
                team_type,
                emblem,
            } => {
                self.queried.remove(&team_id);
                self.records.insert(
                    team_id,
                    ArenaTeamInfoView {
                        name: String::from_utf8_lossy(&name).into_owned(),
                        size: team_type,
                        emblem,
                    },
                );
                if team_id != 0 && self.slot_of(team_id).is_some() {
                    fx.events.push(("ARENA_TEAM_UPDATE", Vec::new()));
                }
            }
            Packet::Invite { inviter, team } => fx.events.push((
                "ARENA_TEAM_INVITE_REQUEST",
                vec![
                    ScriptValue::Str(String::from_utf8_lossy(&inviter).into_owned()),
                    ScriptValue::Str(String::from_utf8_lossy(&team).into_owned()),
                ],
            )),
        }
    }

    /// The inspected player's teams joined with their query records, asking once for a record
    /// not yet held (`0x5e8030`'s cache lookup).
    pub(crate) fn inspect_view(
        &mut self,
        teams: [Option<crate::ui_battlefield::InspectedArenaTeam>; 3],
        fx: &mut Effects,
    ) -> [Option<InspectArenaTeamView>; 3] {
        teams.map(|t| {
            let t = t.filter(|t| t.team_id != 0)?;
            let Some(info) = self.records.get(&t.team_id).cloned() else {
                if self.queried.insert(t.team_id) {
                    fx.sends.push(Command::Query { team_id: t.team_id });
                }
                return None;
            };
            Some(InspectArenaTeamView {
                info,
                rating: t.rating,
                season_games: t.season_games,
                season_wins: t.season_wins,
                player_games: t.member_games,
                player_rating: t.personal_rating,
            })
        })
    }

    /// The three slots as the bindings read them.
    pub(crate) fn teams(&self) -> [ArenaTeamView; 3] {
        std::array::from_fn(|s| ArenaTeamView {
            descriptor: self.descriptor[s],
            stats: self.stats[s],
            info: (self.team_id(s) != 0)
                .then(|| self.records.get(&self.team_id(s)).cloned())
                .flatten(),
        })
    }

    pub(crate) fn roster(&self, slot: usize) -> Vec<ArenaMemberView> {
        self.members[slot].clone()
    }
}

/// One roster row (`0x5a3e10`'s reads).
fn member(m: &Member) -> ArenaMemberView {
    let name = &m.name[..m.name.len().min(NAME_BYTES)];
    let class = crate::char_select::class_name(m.class);
    ArenaMemberView {
        guid: m.guid,
        name: String::from_utf8_lossy(name).into_owned(),
        rank: m.captain_flag,
        level: m.level,
        class: (class != "?").then(|| class.to_string()),
        online: m.online != 0,
        week_games: m.week_games,
        week_wins: m.week_wins,
        season_games: m.season_games,
        season_wins: m.season_wins,
        rating: m.personal_rating,
        gdf: m.extra_bits.map_or([0.0; 2], |w| w.map(f32::from_bits)),
    }
}

pub struct UiArenaTeamPlugin;
impl Plugin for UiArenaTeamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeArenaTeamPackets>()
            .init_resource::<ArenaTable>()
            .init_resource::<ArenaRosterStale>()
            .net_handler(SessionEventKind::WrathArenaTeam, receive)
            .net_handler(SessionEventKind::Disconnected, clear)
            .add_systems(Update, feed_arena.in_set(UnitFeed));
    }
}
fn receive(In(event): In<SessionEvent>, mut packets: ResMut<NativeArenaTeamPackets>) {
    if let SessionEvent::WrathArenaTeam { packet } = event {
        packets.0.push_back(packet);
    }
}
fn clear(
    In(_): In<SessionEvent>,
    mut packets: ResMut<NativeArenaTeamPackets>,
    mut table: ResMut<ArenaTable>,
    mut stale: ResMut<ArenaRosterStale>,
) {
    packets.0.clear();
    stale.0 = 0;
    *table = ArenaTable::default();
}

#[derive(Default)]
struct Published {
    teams: Option<[ArenaTeamView; 3]>,
    inspect: Option<[Option<InspectArenaTeamView>; 3]>,
}

/// The inspected player, as the inspect window resolves it.
#[derive(bevy::ecs::system::SystemParam)]
struct Inspected<'w> {
    target: Res<'w, crate::ui_inspect::InspectTarget>,
    selection: Res<'w, crate::app_core::state::Selection>,
    group: Res<'w, crate::app_core::group::GroupState>,
    teams: Res<'w, crate::ui_battlefield::InspectArenaTeams>,
}

impl Inspected<'_> {
    fn teams(&self) -> [Option<crate::ui_battlefield::InspectedArenaTeam>; 3] {
        self.target
            .token
            .as_deref()
            .and_then(|t| {
                crate::target::unit_tokens::player_token_guid(t, &self.selection, &self.group)
            })
            .and_then(|g| self.teams.0.get(&g))
            .copied()
            .unwrap_or_default()
    }
}

/// Drain the capture into the table, then publish what changed.
fn feed_arena(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<NativeArenaTeamPackets>,
    mut table: ResMut<ArenaTable>,
    mut stale: ResMut<ArenaRosterStale>,
    me: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    clock: Res<crate::app_core::ui_flags::UiClock>,
    inspected: Inspected,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<Published>>,
) {
    let now = clock.ui_now;
    let mut fx = Effects::default();
    let mut script = script;
    if let Some(script) = script.as_deref_mut() {
        for slot in script.take_wrath_arena_roster_requests() {
            table.request_roster(slot, now, &mut fx);
        }
    }
    if let Some(store) = me.iter().next() {
        let word = |s: u16, i: u16| {
            store
                .0
                .get_at(Field::PlayerArenaTeamInfo11, 7 * s + i)
                .unwrap_or(0)
        };
        let words = std::array::from_fn(|s| std::array::from_fn(|i| word(s as u16, i as u16)));
        table.sync_descriptor(words, now, &mut fx);
    }
    for packet in packets.0.drain(..) {
        table.apply(packet, &mut fx);
    }
    for _ in 0..std::mem::take(&mut stale.0) {
        table.mark_stale(&mut fx);
    }
    let inspect = table.inspect_view(inspected.teams(), &mut fx);
    for command in fx.sends.drain(..) {
        let _ = commands.0.send(ClientCommand::ArenaTeamCommand(command));
    }
    let Some(mut script) = script else { return };
    let (memory, vm_reset) = memory.get_reset(&script);
    let teams = table.teams();
    if memory.teams.as_ref() != Some(&teams) {
        script.set_wrath_arena_teams(teams.clone());
        memory.teams = Some(teams);
    }
    // The stats and the record both fire `INSPECT_HONOR_UPDATE` (`0x5e7b00`, `0x5e7640`).
    if memory.inspect.as_ref() != Some(&inspect) {
        script.set_inspect_arena_teams(inspect.clone());
        memory.inspect = Some(inspect);
        if !vm_reset {
            script.fire_event("INSPECT_HONOR_UPDATE", Vec::new());
        }
    }
    if vm_reset {
        for slot in 0..3 {
            script.set_wrath_arena_roster(slot, table.roster(slot));
        }
    }
    for slot in fx.rosters {
        script.set_wrath_arena_roster(slot, table.roster(slot));
        script.fire_event("ARENA_TEAM_ROSTER_UPDATE", Vec::new());
    }
    for (event, args) in fx.events {
        script.fire_event(event, args);
    }
}

#[cfg(test)]
#[path = "ui_arena_team_tests.rs"]
mod tests;
