//! The native guild state the shared guild events leave out (`wrath_guild::session_events`):
//! 12340's one rank table of bank words (`0xc21e60`), which the bank bindings read and the rank
//! save carries back, and the event log,
//! pushed to Native UI's `set_wrath_guild_events` with names from the name cache and rank names
//! from our guild's identity.
use crate::names::NameCache;
use crate::net::{NetCommands, NetHandlerApp};
use bevy::prelude::*;
use wrathplus_protocol::types::GUILD_RANKS_MAX_COUNT;
use wrathplus_protocol::wrath_guild::{EventLogEntry, Packet, Rank};
use wrathplus_protocol::wrath_guild_bank::Permissions;
use wrathplus_protocol::{SessionEvent, SessionEventKind};
use wrathplus_ui::script::{UiScript, WrathGuildEvent};

#[derive(Resource, Default)]
pub(crate) struct NativeGuild {
    /// Per rank id: every row from the latest roster (`0x5cc6c7`-`0x5cc771`), our own row from a
    /// later `MSG_GUILD_PERMISSIONS` (`0x5cb9f0`), whichever landed last.
    ranks: Vec<Rank>,
    /// The latest `MSG_GUILD_EVENT_LOG_QUERY` answer, newest first.
    log: Vec<EventLogEntry>,
}

impl NativeGuild {
    /// A rank's bank words as the roster last sent them.
    pub(crate) fn rank(&self, rank: u32) -> Option<&Rank> {
        self.ranks.get(usize::try_from(rank).ok()?)
    }

    /// Every rank's bank words, rank 0 (the guild master) first.
    pub(crate) fn ranks(&self) -> &[Rank] {
        &self.ranks
    }

    /// `MSG_GUILD_PERMISSIONS` overwrites its rank's row: flags, money, then each tab's flags and
    /// item limit (`0x5cba2a`-`0x5cbab7`), the limit being AzerothCore's remaining slots today.
    pub(crate) fn apply_permissions(&mut self, p: &Permissions) {
        let Some(rank) = usize::try_from(p.rank)
            .ok()
            .filter(|r| *r < GUILD_RANKS_MAX_COUNT)
        else {
            return;
        };
        let empty = Rank {
            flags: 0,
            money_limit: 0,
            tabs: [(0, 0); 6],
        };
        if self.ranks.len() <= rank {
            self.ranks.resize(rank + 1, empty);
        }
        self.ranks[rank] = Rank {
            flags: p.flags,
            money_limit: p.money_limit,
            tabs: p.tabs.clone().map(|t| (t.rights, t.remaining_items)),
        };
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<NativeGuild>()
        .net_handler(SessionEventKind::WrathGuild, receive)
        .net_handler(SessionEventKind::Connected, clear)
        .net_handler(SessionEventKind::Disconnected, clear)
        .add_systems(
            Update,
            feed_events
                .after(super::GuildFeed)
                .in_set(crate::app_core::schedule::UiFeed)
                .run_if(crate::app_core::ui_state::ingame_ui_up),
        );
}

fn receive(In(event): In<SessionEvent>, mut guild: ResMut<NativeGuild>) {
    if let SessionEvent::WrathGuild { packet } = event {
        match packet {
            Packet::Roster { ranks, .. } => guild.ranks = ranks,
            Packet::EventLog { entries } => guild.log = entries,
            _ => {}
        }
    }
}

fn clear(In(event): In<SessionEvent>, mut guild: ResMut<NativeGuild>) {
    if matches!(
        event,
        SessionEvent::Connected { .. } | SessionEvent::Disconnected { .. }
    ) {
        *guild = NativeGuild::default();
    }
}

/// The log rows as the guild pane shows them; an unnamed player waits for its name query.
fn events(
    log: &[EventLogEntry],
    name: impl Fn(u64) -> Option<String>,
    rank: impl Fn(u8) -> String,
) -> Vec<WrathGuildEvent> {
    log.iter()
        .map(|e| WrathGuildEvent {
            kind: e.kind,
            player1: name(e.player),
            player2: e.other.and_then(&name),
            rank: e.rank.map(&rank),
            seconds_ago: e.seconds_ago,
        })
        .collect()
}

fn feed_events(
    script: Option<NonSendMut<UiScript>>,
    guild: Res<NativeGuild>,
    state: Res<super::GuildState>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<Vec<WrathGuildEvent>>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let rows = events(
        &guild.log,
        |guid| names.resolve(guid, &commands).map(str::to_string),
        |rank| state.own_rank_name(u32::from(rank)).to_string(),
    );
    let (last, reset) = last.get_reset(&script);
    if reset || last.as_ref() != Some(&rows) {
        *last = Some(rows.clone());
        script.set_wrath_guild_events(rows);
    }
}

#[cfg(test)]
#[path = "wrath_tests.rs"]
mod tests;
