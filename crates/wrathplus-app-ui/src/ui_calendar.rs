//! The build-12340 calendar from native traffic: the sole reducer of Wire B's
//! `NativeCalendarPackets`, published as Native UI's [`WrathCalendarView`] with the calendar's
//! events fired here, and the sender of the VM's [`WrathCalendarRequest`]s through the
//! transport-only `ClientCommand::NativeCalendar` writers. Evidence: AzerothCore's
//! `CalendarHandler.cpp`/`CalendarMgr.cpp` for what the server sends and reads, the stock
//! `Blizzard_Calendar` for the events' consumers; the binding round is in
//! `wrathplus_ui::script::wrath_calendar`.
//!
//! - `SMSG_CALENDAR_SEND_CALENDAR` replaces the list: each event with the player's invite, the
//!   permanent lockouts (reset at the server's time plus the seconds left), the raid resets
//!   (`relationTime + n·period`) and the holidays, each holiday day its own row. The server's
//!   zone time fixes the zone, so every unix time shows server-local. Fires
//!   `CALENDAR_UPDATE_EVENT_LIST`.
//! - `SMSG_CALENDAR_SEND_EVENT` (sent on a `CalendarOpenEvent`, an add or a copy) opens the
//!   asked-for event (`CALENDAR_OPEN_EVENT` with its calendar type, then on a refresh
//!   `CALENDAR_UPDATE_EVENT`/`_INVITE_LIST`); an add or copy lists it and fires
//!   `CALENDAR_NEW_EVENT(isCopy)`, an add closing the draft.
//! - The invite, status, moderator and removal receipts edit the open event's invitees
//!   (`CALENDAR_UPDATE_INVITE_LIST`) and the player's own rows (`CALENDAR_UPDATE_EVENT_LIST`); an
//!   alert that removes the open event fires `CALENDAR_CLOSE_EVENT`; the lockout receipts edit
//!   the lockouts; `SMSG_CALENDAR_SEND_NUM_PENDING` fires `CALENDAR_UPDATE_PENDING_INVITES`.
//! - `SMSG_CALENDAR_COMMAND_RESULT`'s error raises `CALENDAR_UPDATE_ERROR` with its line.
//!
//! Labelled: the error id → key table is AzerothCore's `CalendarError` names matched to
//! GlobalStrings (the client's own table is not traced); the guild-filter and arena-team member
//! lists fill a draft at level 0, their byte uninterpreted (an open server event ignores them);
//! `0x0442`/`0x0460`/`0x0461` never arrive (AzerothCore sends none), so
//! no status alert, invite note or `CALENDAR_EVENT_ALARM` exists; a holiday lasts its first
//! duration from each start date; system events are never sent.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use bevy::prelude::*;
use wrathplus_formats::holidays::HolidayCatalog;
use wrathplus_protocol::wrath_calendar::{
    AddInvitee, Calendar, Command, Event, EventInvitee, EventRow, Lockout, Packet, RaidReset,
};
use wrathplus_ui::script::{
    ScriptValue as V, UiScript, WrathCalendarDate, WrathCalendarEntry, WrathCalendarEvent,
    WrathCalendarFields, WrathCalendarInvitee, WrathCalendarKind, WrathCalendarRaid,
    WrathCalendarRaidReset, WrathCalendarRequest as R, WrathCalendarTexture, WrathCalendarView,
};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::names::NameCache;
use crate::net::wrath_calendar_capture::NativeCalendarPackets;
use crate::net::{ClientCommand, NetCommands, NetHandlerApp};
use wrathplus_protocol::{SessionEvent, SessionEventKind};
mod draft_list;
mod invite_bits;
mod pending;

/// Wire statuses and ranks.
const INVITED: u8 = 0;
const CONFIRMED: u8 = 3;
const OWNER: u8 = 2;
/// `CALENDAR_FLAG_WITHOUT_INVITES`.
const ANNOUNCEMENT: u32 = 0x40;
/// `SMSG_CALENDAR_SEND_EVENT`'s send types.
const SEND_ADD: u8 = 1;
const SEND_COPY: u8 = 2;

/// What `SMSG_CALENDAR_COMMAND_RESULT`'s line formats its key with (`0x5b9960`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorArg {
    /// The bare template.
    None,
    /// A constant the client passes for its `%d` (the limits it knows).
    Count(u32),
    /// The result's name, for its `%s`.
    Name,
}

/// Build 12340's error switch (`0x6cd380`): AzerothCore's `CalendarError` to the GlobalStrings key
/// and the argument `0x5b9960` formats it with; codes it has no arm for raise nothing.
fn error_key(error: u32) -> Option<(&'static str, ErrorArg)> {
    use ErrorArg::{Count, Name, None as Bare};
    Some(match error {
        1 => ("CALENDAR_ERROR_GUILD_EVENTS_EXCEEDED", Count(100)),
        2 => ("CALENDAR_ERROR_EVENTS_EXCEEDED", Count(30)),
        3 => ("CALENDAR_ERROR_SELF_INVITES_EXCEEDED", Bare),
        4 => ("CALENDAR_ERROR_OTHER_INVITES_EXCEEDED", Name),
        5 => ("CALENDAR_ERROR_PERMISSIONS", Bare),
        6 => ("CALENDAR_ERROR_EVENT_INVALID", Bare),
        7 => ("CALENDAR_ERROR_NOT_INVITED", Bare),
        8 => ("CALENDAR_ERROR_INTERNAL", Bare),
        9 => ("ERR_GUILD_PLAYER_NOT_IN_GUILD", Bare),
        10 => ("CALENDAR_ERROR_ALREADY_INVITED_TO_EVENT_S", Name),
        11 => ("PLAYER_NOT_FOUND", Bare),
        12 => ("CALENDAR_ERROR_NOT_ALLIED", Bare),
        13 => ("ERR_IGNORING_YOU_S", Name),
        14 => ("CALENDAR_ERROR_INVITES_EXCEEDED", Count(100)),
        16 => ("CALENDAR_ERROR_INVALID_DATE", Bare),
        17 => ("CALENDAR_ERROR_INVALID_TIME", Bare),
        19 => ("CALENDAR_ERROR_NEEDS_TITLE", Bare),
        20 => ("CALENDAR_ERROR_EVENT_PASSED", Bare),
        21 => ("CALENDAR_ERROR_EVENT_LOCKED", Bare),
        22 => ("CALENDAR_ERROR_DELETE_CREATOR_FAILED", Bare),
        24 => ("ERR_SYSTEM_DISABLED", Bare),
        25 => ("ERR_RESTRICTED_ACCOUNT", Bare),
        26 => ("CALENDAR_ERROR_ARENA_EVENTS_EXCEEDED", Bare),
        27 => ("CALENDAR_ERROR_RESTRICTED_LEVEL", Bare),
        28 => ("ERR_USER_SQUELCHED", Bare),
        29 => ("CALENDAR_ERROR_NO_INVITE", Bare),
        36 => ("CALENDAR_ERROR_EVENT_WRONG_SERVER", Bare),
        37 => ("CALENDAR_ERROR_INVITE_WRONG_SERVER", Bare),
        38 => ("CALENDAR_ERROR_NO_GUILD_INVITES", Bare),
        39 => ("CALENDAR_ERROR_INVALID_SIGNUP", Bare),
        40 => ("CALENDAR_ERROR_NO_MODERATOR", Bare),
        _ => return None,
    })
}

/// The `LFGDungeons` texture lists, the raid difficulty keys and the holidays, loaded once.
#[derive(Resource, Default)]
pub(crate) struct CalendarCatalog {
    holidays: HolidayCatalog,
    textures: [Vec<WrathCalendarTexture>; 2],
    /// Every `LFGDungeons` row's texture by id, a listed event's icon (`0x5c1040`).
    dungeons: HashMap<i32, String>,
    difficulty_keys: HashMap<(i32, i32), String>,
    max_players: HashMap<(i32, i32), i32>,
}

impl CalendarCatalog {
    /// `MapDifficulty`'s string key and player cap for a map and difficulty.
    pub(crate) fn difficulty(&self, map: u32, difficulty: u32) -> Option<(String, i32)> {
        let key = (map as i32, difficulty as i32);
        Some((
            self.difficulty_keys.get(&key)?.clone(),
            self.max_players.get(&key).copied().unwrap_or(0),
        ))
    }
}

/// One listed event and the player's invite to it.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    row: EventRow,
    /// Invite id, wire status, rank, type.
    invite: Option<(u64, u8, u8, u8)>,
}

#[derive(Resource, Default)]
pub(crate) struct CalendarState {
    /// Server-local minus unix seconds, from the zone time.
    zone: i64,
    /// The server's unix time and when it was read.
    clock: Option<(i64, Instant)>,
    events: Vec<Listed>,
    /// Each lockout with its reset's unix time.
    lockouts: Vec<(Lockout, i64)>,
    relation_time: i64,
    raid_resets: Vec<RaidReset>,
    holidays: Vec<wrathplus_protocol::wrath_calendar::Holiday>,
    pending: u32,
    /// The event or draft the VM opened.
    open: Option<WrathCalendarEvent>,
    /// The id a `CalendarOpenEvent` asked for, until its event lands.
    asked: Option<u64>,
    pushed: Option<Arc<WrathCalendarView>>,
}

impl CalendarState {
    fn now_unix(&self) -> i64 {
        self.clock.map_or_else(
            || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            },
            |(t, at)| t + at.elapsed().as_secs() as i64,
        )
    }

    fn local(&self, unix: i64) -> WrathCalendarDate {
        WrathCalendarDate::from_seconds(unix + self.zone)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct UiCalendarPlugin;
impl Plugin for UiCalendarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CalendarState>()
            .init_resource::<CalendarCatalog>()
            .net_handler(SessionEventKind::Connected, clear)
            .net_handler(SessionEventKind::Disconnected, clear)
            .add_systems(Startup, load.after(wrathplus_assets::AssetSet::Open))
            .add_systems(
                Update,
                feed_calendar
                    .in_set(crate::app_core::schedule::UiFeed)
                    .run_if(crate::app_core::ui_state::ingame_ui_up),
            );
    }
}

fn load(mut catalog: ResMut<CalendarCatalog>, assets: Option<Res<wrathplus_assets::WorldAssets>>) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    if !matches!(
        chain.profile(),
        wrathplus_formats::DataProfile::Wrath { .. }
    ) {
        return;
    }
    match wrathplus_formats::holidays::load_holidays(&mut chain) {
        Ok(h) => catalog.holidays = h,
        Err(error) => warn!("calendar holidays: {error:#}"),
    }
    match wrathplus_formats::lfg::load_lfg_catalogs(&mut chain) {
        Ok(lfg) => {
            for d in lfg.map_difficulties.rows() {
                catalog
                    .difficulty_keys
                    .entry((d.map_id(), d.difficulty()))
                    .or_insert_with(|| d.raw_string.clone());
                catalog
                    .max_players
                    .entry((d.map_id(), d.difficulty()))
                    .or_insert(d.max_players());
            }
            catalog.textures = textures(&lfg, &catalog.difficulty_keys);
            let rows = lfg.dungeons.rows().iter();
            catalog.dungeons = rows.map(|d| (d.id() as i32, d.texture.clone())).collect();
        }
        Err(error) => warn!("calendar dungeons: {error:#}"),
    }
}

/// Labelled (`0xc20f60`'s builder untraced): every raid (`LFGDungeons` type 2) then every
/// dungeon (type 1), in file order.
fn textures(
    lfg: &wrathplus_formats::lfg::LfgCatalogs,
    keys: &HashMap<(i32, i32), String>,
) -> [Vec<WrathCalendarTexture>; 2] {
    let list = |kind: i32| {
        lfg.dungeons
            .rows()
            .iter()
            .filter(|d| d.dungeon_type() == kind)
            .map(|d| WrathCalendarTexture {
                dungeon: d.id() as i32,
                title: d.name.clone(),
                texture: d.texture.clone(),
                expansion: d.expansion(),
                difficulty_key: keys.get(&(d.map_id(), d.difficulty())).cloned(),
            })
            .collect()
    };
    [list(2), list(1)]
}

#[allow(clippy::too_many_arguments)]
fn feed_calendar(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<NativeCalendarPackets>,
    mut state: ResMut<CalendarState>,
    catalog: Res<CalendarCatalog>,
    names: Option<Res<NameCache>>,
    commands: Res<NetCommands>,
    maps: Option<Res<wrathplus_assets::MapCatalogRes>>,
    self_q: Query<(&crate::net::Guid, &ObjectStore), With<SelfPlayer>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let me = self_q.single().ok();
    let my_guid = me.map_or(0, |(g, _)| g.0);
    let mut fired: Vec<(&'static str, Vec<V>)> = Vec::new();
    let (mut clear_pending, mut sent) = (false, Vec::new());
    for packet in packets.0.drain(..) {
        sent.extend(pending::follow_up(&packet));
        clear_pending |= reduce(&mut state, packet, my_guid, &mut fired);
    }
    for request in script.take_wrath_calendar_requests() {
        if let Some(args) = draft_list::redraws(&request) {
            fired.push(("CALENDAR_UPDATE_INVITE_LIST", args));
        }
        let store = me.map(|(_, s)| s);
        sent.extend(apply(&mut state, request, my_guid, store));
    }
    for command in sent {
        let _ = commands.0.send(ClientCommand::NativeCalendar(command));
    }
    let view = Arc::new(build(
        &state,
        &catalog,
        names.as_deref(),
        &commands,
        maps.as_deref(),
        my_guid,
    ));
    if state.pushed.as_ref() != Some(&view) {
        script.set_wrath_calendar(view.clone());
        state.pushed = Some(view);
    }
    for (event, args) in fired {
        let args = args
            .into_iter()
            .map(|a| match a {
                V::Str(key) if event == "CALENDAR_UPDATE_ERROR" => {
                    V::Str(error_line(&script, &key))
                }
                a => a,
            })
            .collect();
        script.fire_event(event, args);
    }
    if clear_pending {
        script.set_wrath_calendar_action_pending(false);
    }
}

/// `key`'s template, its `%s` or `%d` filled with the argument after a NUL when there is one (a
/// bare key is the template as is, `0x5b9960`'s no-argument leg). Labelled: the `|4one:many;`
/// plural escapes are left to the text renderer.
fn error_line(script: &UiScript, packed: &str) -> String {
    let (key, arg) = match packed.split_once('\0') {
        Some((key, arg)) => (key, Some(arg)),
        None => (packed, None),
    };
    let template = script
        .lua()
        .globals()
        .get::<Option<String>>(key)
        .ok()
        .flatten()
        .unwrap_or_else(|| key.to_string());
    match arg {
        Some(arg) if template.contains("%d") => template.replacen("%d", arg, 1),
        Some(arg) => template.replacen("%s", arg, 1),
        None => template,
    }
}

fn invitee(i: &EventInvitee) -> WrathCalendarInvitee {
    WrathCalendarInvitee {
        guid: i.guid,
        level: i.level,
        status: i.status,
        rank: i.rank,
        signup: i.guild_member != 0,
        invite_id: i.invite_id,
        status_time: WrathCalendarDate::from_packed(i.status_time),
        notes: i.text.clone(),
        ..Default::default()
    }
}

fn open_of(e: &Event) -> WrathCalendarEvent {
    WrathCalendarEvent {
        event_id: e.event_id,
        creator: e.creator,
        title: e.title.clone(),
        description: e.description.clone(),
        event_type: e.kind,
        repeat: e.repeat,
        max_invites: e.max_invites,
        dungeon: e.dungeon,
        flags: e.flags,
        date: WrathCalendarDate::from_packed(e.packed_time).unwrap_or_default(),
        lockout: WrathCalendarDate::from_packed(e.timezone_time).unwrap_or_default(),
        invitees: e.invitees.iter().map(invitee).collect(),
        ..Default::default()
    }
}

/// Apply one receipt; `true` when it clears the pending bit ([`pending::clears`]).
fn reduce(
    state: &mut CalendarState,
    packet: Packet,
    me: u64,
    fired: &mut Vec<(&'static str, Vec<V>)>,
) -> bool {
    let list = |fired: &mut Vec<(&'static str, Vec<V>)>| {
        if !fired
            .iter()
            .any(|(e, _)| *e == "CALENDAR_UPDATE_EVENT_LIST")
        {
            fired.push(("CALENDAR_UPDATE_EVENT_LIST", vec![]));
        }
    };
    let invites = |fired: &mut Vec<(&'static str, Vec<V>)>| {
        fired.push(("CALENDAR_UPDATE_INVITE_LIST", vec![]));
    };
    let clears = pending::clears(&packet, |e| error_key(e).is_some());
    match packet {
        Packet::Calendar(c) => {
            calendar(state, c);
            list(fired);
        }
        Packet::Event(e) => {
            let opened = WrathCalendarKind::of_flags(e.flags).token();
            let mut event = open_of(&e);
            if e.send_type == SEND_ADD || e.send_type == SEND_COPY {
                let mine = e.invitees.iter().find(|i| i.guid == me);
                state.events.retain(|l| l.row.event_id != e.event_id);
                state.events.push(Listed {
                    row: EventRow {
                        event_id: e.event_id,
                        title: e.title.clone(),
                        kind: u32::from(e.kind),
                        packed_time: e.packed_time,
                        flags: e.flags,
                        dungeon: e.dungeon,
                        creator: e.creator,
                    },
                    invite: mine.map(|i| (i.invite_id, i.status, i.rank, i.guild_member)),
                });
                let copy = e.send_type == SEND_COPY;
                if !copy && state.open.as_ref().is_some_and(|o| o.event_id == 0) {
                    state.open = None;
                }
                fired.push(("CALENDAR_NEW_EVENT", vec![V::Bool(copy)]));
                list(fired);
                return clears;
            }
            let asked = state.asked == Some(e.event_id);
            let refresh = state
                .open
                .as_ref()
                .is_some_and(|o| o.event_id == e.event_id);
            if asked {
                state.asked = None;
                state.open = Some(event);
                fired.push(("CALENDAR_OPEN_EVENT", vec![V::Str(opened.into())]));
            } else if refresh {
                let revision = state.open.as_ref().map_or(0, |o| o.revision);
                event.revision = revision + 1;
                state.open = Some(event);
                fired.push(("CALENDAR_UPDATE_EVENT", vec![]));
                invites(fired);
            }
        }
        Packet::FilterGuild { members } | Packet::ArenaTeam { members } => {
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == 0) {
                for (guid, _) in members {
                    if !open.invitees.iter().any(|i| i.guid == guid) {
                        open.invitees.push(WrathCalendarInvitee {
                            guid,
                            status: INVITED,
                            ..Default::default()
                        });
                    }
                }
                invites(fired);
            }
        }
        Packet::Invite {
            invitee,
            event_id,
            invite_id,
            level,
            status,
            status_time,
            ..
        } => {
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == event_id) {
                let row = WrathCalendarInvitee {
                    guid: invitee,
                    level,
                    status,
                    invite_id,
                    status_time: status_time.and_then(WrathCalendarDate::from_packed),
                    ..Default::default()
                };
                match open.invitees.iter_mut().find(|i| i.guid == invitee) {
                    Some(i) => *i = row,
                    None => open.invitees.push(row),
                }
                invites(fired);
            }
        }
        Packet::InviteRemoved {
            invitee, event_id, ..
        } => {
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == event_id) {
                open.invitees.retain(|i| i.guid != invitee);
                invites(fired);
            }
        }
        Packet::Status {
            invitee,
            event_id,
            status,
            rank,
            status_time,
            ..
        } => {
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == event_id) {
                if let Some(i) = open.invitees.iter_mut().find(|i| i.guid == invitee) {
                    i.status = status;
                    i.rank = rank;
                    i.status_time = WrathCalendarDate::from_packed(status_time);
                }
                invites(fired);
            }
            if invitee == me {
                if let Some(l) = state.events.iter_mut().find(|l| l.row.event_id == event_id) {
                    if let Some(inv) = l.invite.as_mut() {
                        inv.1 = status;
                        inv.2 = rank;
                    }
                    list(fired);
                }
            }
        }
        Packet::CommandResult { param, error, .. } => {
            if let Some((key, arg)) = error_key(error) {
                let packed = match arg {
                    ErrorArg::None => key.to_string(),
                    ErrorArg::Count(n) => format!("{key}\0{n}"),
                    ErrorArg::Name => format!("{key}\0{param}"),
                };
                fired.push(("CALENDAR_UPDATE_ERROR", vec![V::Str(packed)]));
            }
        }
        Packet::RaidLockoutAdded { lockout, .. } => {
            let reset = state.now_unix() + i64::from(lockout.seconds_left);
            state
                .lockouts
                .retain(|(l, _)| l.instance != lockout.instance);
            state.lockouts.push((lockout, reset));
            list(fired);
        }
        Packet::RaidLockoutRemoved { lockout } => {
            state.lockouts.retain(|(l, _)| {
                (l.map, l.difficulty, l.instance)
                    != (lockout.map, lockout.difficulty, lockout.instance)
            });
            list(fired);
        }
        Packet::RaidLockoutUpdated {
            map,
            difficulty,
            new_seconds,
            ..
        } => {
            let reset = state.now_unix() + i64::from(new_seconds);
            for (l, at) in &mut state.lockouts {
                if (l.map, l.difficulty) == (map, difficulty) {
                    l.seconds_left = new_seconds;
                    *at = reset;
                }
            }
            list(fired);
        }
        Packet::InviteAlert {
            event_id,
            title,
            packed_time,
            flags,
            kind,
            dungeon,
            invite_id,
            status,
            rank,
            creator,
            ..
        } => {
            state.events.retain(|l| l.row.event_id != event_id);
            state.events.push(Listed {
                row: EventRow {
                    event_id,
                    title,
                    kind,
                    packed_time,
                    flags,
                    dungeon,
                    creator,
                },
                invite: Some((invite_id, status, rank, 0)),
            });
            list(fired);
        }
        Packet::InviteRemovedAlert { event_id, .. } | Packet::RemovedAlert { event_id, .. } => {
            state.events.retain(|l| l.row.event_id != event_id);
            if state.open.as_ref().is_some_and(|o| o.event_id == event_id) {
                state.open = None;
                fired.push(("CALENDAR_CLOSE_EVENT", vec![]));
            }
            list(fired);
        }
        Packet::UpdatedAlert {
            event_id,
            flags,
            packed_time,
            kind,
            dungeon,
            title,
            description,
            repeat,
            max_invites,
            ..
        } => {
            if let Some(l) = state.events.iter_mut().find(|l| l.row.event_id == event_id) {
                l.row.title = title.clone();
                l.row.flags = flags;
                l.row.packed_time = packed_time;
                l.row.kind = u32::from(kind);
                l.row.dungeon = dungeon;
            }
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == event_id) {
                open.title = title;
                open.description = description;
                open.flags = flags;
                open.event_type = kind;
                open.dungeon = dungeon;
                open.repeat = repeat;
                open.max_invites = max_invites;
                open.date = WrathCalendarDate::from_packed(packed_time).unwrap_or(open.date);
                open.revision += 1;
                fired.push(("CALENDAR_UPDATE_EVENT", vec![]));
            }
            list(fired);
        }
        Packet::ModeratorStatusAlert {
            invitee,
            event_id,
            rank,
            ..
        } => {
            if let Some(open) = state.open.as_mut().filter(|o| o.event_id == event_id) {
                if let Some(i) = open.invitees.iter_mut().find(|i| i.guid == invitee) {
                    i.rank = rank;
                }
                invites(fired);
            }
            if invitee == me {
                if let Some(inv) = state
                    .events
                    .iter_mut()
                    .find(|l| l.row.event_id == event_id)
                    .and_then(|l| l.invite.as_mut())
                {
                    inv.2 = rank;
                }
            }
        }
        // `0x5b7ce0` announces the server's count only when it changes.
        Packet::NumPending { count } => {
            if std::mem::replace(&mut state.pending, count) != count {
                fired.push(("CALENDAR_UPDATE_PENDING_INVITES", vec![]));
            }
        }
        Packet::ClearPendingAction => {}
    }
    clears
}

fn calendar(state: &mut CalendarState, c: Calendar) {
    let server = i64::from(c.server_time);
    if let Some(local) = WrathCalendarDate::from_packed(c.zone_packed_time) {
        // The zone's offset to the minute the packed time keeps.
        state.zone = ((local.seconds() - server) as f64 / 60.0).round() as i64 * 60;
    }
    state.clock = Some((server, Instant::now()));
    state.events = c
        .events
        .into_iter()
        .map(|row| {
            let invite = c
                .invites
                .iter()
                .find(|i| i.event_id == row.event_id)
                .map(|i| (i.invite_id, i.status, i.rank, i.guild_event));
            Listed { row, invite }
        })
        .collect();
    state.lockouts = c
        .lockouts
        .into_iter()
        .map(|l| {
            let at = server + i64::from(l.seconds_left);
            (l, at)
        })
        .collect();
    state.relation_time = i64::from(c.relation_time);
    state.raid_resets = c.raid_resets;
    state.holidays = c.holidays;
}

fn draft(state: &mut CalendarState) -> Option<&mut WrathCalendarEvent> {
    state.open.as_mut().filter(|o| o.event_id == 0)
}

/// Send one request, or apply it to the draft; the commands it sends.
fn apply(
    state: &mut CalendarState,
    request: R,
    me: u64,
    store: Option<&ObjectStore>,
) -> Vec<Command> {
    match request {
        R::GetCalendar => vec![Command::GetCalendar],
        // The open event stays until the asked one arrives, as 12340's `0xc207ec` does.
        R::GetEvent { event_id } => {
            state.asked = Some(event_id);
            vec![Command::GetEvent { event_id }]
        }
        R::NewDraft { flags, arena_slot } => {
            state.asked = None;
            let creator = (flags & ANNOUNCEMENT == 0).then(|| WrathCalendarInvitee {
                guid: me,
                level: store.and_then(|s| s.0.unit_level()).unwrap_or(0) as u8,
                status: CONFIRMED,
                rank: OWNER,
                ..Default::default()
            });
            state.open = Some(WrathCalendarEvent {
                creator: me,
                flags,
                invitees: creator.into_iter().collect(),
                ..Default::default()
            });
            arena_slot
                .and_then(|slot| slot.checked_sub(1))
                .and_then(|slot| store.and_then(|s| s.0.player_arena_team_id(slot)))
                .filter(|&id| id != 0)
                .map(|team_id| vec![Command::ArenaTeam { team_id }])
                .unwrap_or_default()
        }
        R::AddEvent(f) => {
            let invites = state.open.as_ref().map(|o| {
                o.invitees
                    .iter()
                    .filter(|i| i.guid != me)
                    .map(|i| AddInvitee {
                        guid: i.guid,
                        status: i.status,
                        rank: i.rank,
                    })
                    .collect()
            });
            vec![add_event(&f, invites)]
        }
        R::UpdateEvent {
            event_id,
            invite_id,
            fields: f,
        } => vec![Command::UpdateEvent {
            event_id,
            invite_id,
            title: f.title,
            description: f.description,
            kind: f.event_type,
            repeat: f.repeat,
            max_invites: f.max_invites,
            dungeon: f.dungeon,
            packed_time: f.date.packed(),
            timezone_time: f.lockout.packed(),
            flags: f.flags,
        }],
        R::RemoveEvent {
            event_id,
            invite_id,
            flags,
        } => vec![Command::RemoveEvent {
            event_id,
            invite_id,
            flags,
        }],
        R::CopyEvent {
            event_id,
            invite_id,
            date,
        } => vec![Command::CopyEvent {
            event_id,
            invite_id,
            packed_time: date.packed(),
        }],
        R::Invite {
            event_id,
            invite_id,
            name,
            pre_invite,
            guild_event,
        } => vec![Command::Invite {
            event_id,
            invite_id,
            name,
            pre_invite: u8::from(pre_invite),
            guild_event: u8::from(guild_event),
        }],
        R::RemoveInvite {
            event_id: 0,
            invitee,
            ..
        } => {
            if let Some(open) = draft(state) {
                open.invitees.retain(|i| i.guid != invitee || i.guid == me);
            }
            vec![]
        }
        R::RemoveInvite {
            event_id,
            invitee,
            invite_id,
            owner_invite_id,
        } => vec![Command::RemoveInvite {
            invitee,
            invite_id,
            owner_invite_id,
            event_id,
        }],
        R::SetStatus {
            event_id: 0,
            invitee,
            status,
            ..
        } => {
            if let Some(i) =
                draft(state).and_then(|o| o.invitees.iter_mut().find(|i| i.guid == invitee))
            {
                i.status = status;
            }
            vec![]
        }
        R::SetStatus {
            event_id,
            invitee,
            invite_id,
            owner_invite_id,
            status,
        } => vec![Command::Status {
            invitee,
            event_id,
            invite_id,
            owner_invite_id,
            status,
        }],
        R::SetModerator {
            event_id: 0,
            invitee,
            rank,
            ..
        } => {
            if let Some(i) = draft(state).and_then(|o| {
                o.invitees
                    .iter_mut()
                    .find(|i| i.guid == invitee && i.rank != OWNER)
            }) {
                i.rank = rank;
            }
            vec![]
        }
        R::SetModerator {
            event_id,
            invitee,
            invite_id,
            owner_invite_id,
            rank,
        } => vec![Command::ModeratorStatus {
            invitee,
            event_id,
            invite_id,
            owner_invite_id,
            rank,
        }],
        R::Rsvp {
            event_id,
            invite_id,
            status,
        } => vec![Command::Rsvp {
            event_id,
            invite_id,
            status: u32::from(status),
        }],
        R::SignUp {
            event_id,
            tentative,
        } => vec![Command::Signup {
            event_id,
            tentative: u8::from(tentative),
        }],
        R::Complain { event_id, guid } => vec![Command::Complain { event_id, guid }],
        R::MassInviteGuild {
            min_level,
            max_level,
            min_rank,
        } => vec![Command::GuildFilter {
            min_level,
            max_level,
            min_rank,
        }],
        R::MassInviteArenaTeam { slot } => slot
            .checked_sub(1)
            .and_then(|slot| store.and_then(|s| s.0.player_arena_team_id(slot)))
            .filter(|&id| id != 0)
            .map(|team_id| vec![Command::ArenaTeam { team_id }])
            .unwrap_or_default(),
    }
}

fn add_event(f: &WrathCalendarFields, invites: Option<Vec<AddInvitee>>) -> Command {
    Command::AddEvent {
        title: f.title.clone(),
        description: f.description.clone(),
        kind: f.event_type,
        repeat: f.repeat,
        max_invites: f.max_invites,
        dungeon: f.dungeon,
        packed_time: f.date.packed(),
        raw_packed_time: f.lockout.packed(),
        flags: f.flags,
        invites: if f.flags & ANNOUNCEMENT != 0 {
            None
        } else {
            Some(invites.unwrap_or_default())
        },
    }
}

fn name_of(names: Option<&NameCache>, guid: u64, net: &NetCommands) -> Option<String> {
    if guid == 0 {
        return None;
    }
    names?.resolve(guid, net).map(str::to_string)
}

/// A lockout or reset's title, `Map.dbc`'s name or "" (`0x5c117f`).
fn map_name(maps: Option<&wrathplus_assets::MapCatalogRes>, map: u32) -> String {
    maps.and_then(|m| m.0.name(map)).unwrap_or("").to_string()
}

/// The view from the reduced state.
fn build(
    state: &CalendarState,
    catalog: &CalendarCatalog,
    names: Option<&NameCache>,
    net: &NetCommands,
    maps: Option<&wrathplus_assets::MapCatalogRes>,
    me: u64,
) -> WrathCalendarView {
    // `0x5c0fb0`: a player, guild or announcement row (flags `0x543`) shows its dungeon's texture.
    let texture_of = |flags: u32, dungeon: i32| -> String {
        let found = (flags & 0x543 != 0).then(|| catalog.dungeons.get(&dungeon));
        found.flatten().cloned().unwrap_or_default()
    };
    let mut entries = Vec::new();
    // Holidays first on their days, then the rest by time.
    for h in &state.holidays {
        let dbc = catalog.holidays.get(h.id);
        let name = dbc.map(|d| d.name.clone()).unwrap_or_default();
        let description = dbc.map(|d| d.description.clone()).unwrap_or_default();
        let texture = if h.texture.is_empty() {
            dbc.map(|d| d.texture.clone()).unwrap_or_default()
        } else {
            h.texture.clone()
        };
        let hours = h
            .durations
            .first()
            .copied()
            .filter(|&d| d != 0)
            .unwrap_or(24);
        for &start in h.dates.iter().filter(|&&d| d != 0) {
            let Some(from) = WrathCalendarDate::from_packed(start) else {
                continue;
            };
            let end =
                WrathCalendarDate::from_seconds(from.seconds() + i64::from(hours) * 3600 - 60);
            let first = WrathCalendarDate::new(from.year, from.month, from.day, 0, 0).seconds();
            let days = ((end.seconds() - first) / 86_400 + 1).max(1) as u32;
            for k in 0..days {
                let day = WrathCalendarDate::from_seconds(first + i64::from(k) * 86_400);
                let at = match k {
                    0 => from,
                    k if k + 1 == days => end,
                    _ => day,
                };
                entries.push(WrathCalendarEntry {
                    kind: WrathCalendarKind::Holiday,
                    date: WrathCalendarDate::new(day.year, day.month, day.day, at.hour, at.minute),
                    title: name.clone(),
                    texture: texture.clone(),
                    sequence: (k, days),
                    holiday_description: description.clone(),
                    ..Default::default()
                });
            }
        }
    }
    let mut rest = Vec::new();
    let now = state.local(state.now_unix());
    for l in &state.events {
        let bits = invite_bits::invite_bits(&l.row, l.invite.map(|i| (i.1, i.2, i.3)), me, now);
        let (Some(date), Some(bits)) = (WrathCalendarDate::from_packed(l.row.packed_time), bits)
        else {
            continue;
        };
        rest.push(WrathCalendarEntry {
            kind: WrathCalendarKind::of_flags(l.row.flags),
            date,
            title: l.row.title.clone(),
            event_id: l.row.event_id,
            invite_id: l.invite.map_or(0, |i| i.0),
            creator: l.row.creator,
            invited_by: name_of(names, l.row.creator, net),
            flags: l.row.flags,
            event_type: l.row.kind as u8,
            dungeon: l.row.dungeon,
            texture: texture_of(l.row.flags, l.row.dungeon),
            invite: l.invite.map(|i| (i.1, i.2)),
            bits,
            ..Default::default()
        });
    }
    for (l, at) in &state.lockouts {
        rest.push(WrathCalendarEntry {
            kind: WrathCalendarKind::RaidLockout,
            date: state.local(*at),
            title: map_name(maps, l.map),
            raid: Some(WrathCalendarRaid {
                map: l.map,
                instance: l.instance,
                difficulty: l.difficulty,
                difficulty_key: catalog
                    .difficulty_keys
                    .get(&(l.map as i32, l.difficulty as i32))
                    .cloned()
                    .unwrap_or_default(),
            }),
            ..Default::default()
        });
    }
    rest.sort_by_key(|e| e.date);
    entries.extend(rest);
    let raid_resets = state
        .raid_resets
        .iter()
        .map(|r| WrathCalendarRaidReset {
            map: r.map as u32,
            name: map_name(maps, r.map as u32),
            first: state.relation_time + i64::from(r.offset) + state.zone,
            period: i64::from(r.period),
            difficulty_key: String::new(),
        })
        .collect();
    let event = state.open.as_ref().map(|o| {
        let mut o = o.clone();
        o.creator_name = name_of(names, o.creator, net);
        for i in &mut o.invitees {
            i.name = name_of(names, i.guid, net);
            let class = names.and_then(|n| n.player_traits(i.guid)).map(|t| t.1);
            if let Some((name, file)) = class.and_then(crate::app_core::catalogs::class_names) {
                i.class_name = Some(name.to_string());
                i.class_file = Some(file.to_string());
            }
        }
        o
    });
    WrathCalendarView {
        me,
        entries,
        raid_resets,
        pending_invites: state.pending,
        event,
        textures: catalog.textures.clone(),
        // Labelled: a Wrath account's cap; the account expansion is not read here.
        level_cap: 80,
        gm: false,
    }
}

fn clear(In(event): In<SessionEvent>, mut state: ResMut<CalendarState>) {
    if matches!(
        event,
        SessionEvent::Connected { .. } | SessionEvent::Disconnected { .. }
    ) {
        state.reset();
    }
}

#[cfg(test)]
#[path = "ui_calendar_tests.rs"]
mod tests;
