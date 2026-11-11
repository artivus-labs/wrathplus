//! The guild session the world reads and the wire fills: the identity cache (guild names, rank
//! names and crests, asked lazily by guild id), the roster mirror with its sort, and the packet
//! handlers. The guild windows' feed is `crate::ui_guild`; nameplates and the body composite read
//! [`unit_guild_name`] and the crest lookups.

use bevy::prelude::*;
use wrathplus_formats::GuildEmblem;
use wrathplus_protocol::types::{
    guild_event, GuildCommandResult, GuildEventNotice, GuildInfo, GuildQueryResponse, GuildRoster,
    GuildRosterMember, GUILD_RANKS_MAX_COUNT,
};
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::script::{LastOnline, UnitGuild};

use crate::net::{ClientCommand, NetCommands};
use crate::query_cache::QueryCache;

pub(crate) mod lines;
pub(crate) mod sort;
pub use sort::RosterRow;
use sort::{SortField, SortStack};

/// `GuildRoster()`'s silence after a request: the reference's 10 000 ms against `0xb73130`
/// (`0x4d10d0`). The arg1 edge of [`RosterUpdate`] clears it.
pub const ROSTER_REQUEST_THROTTLE_SECS: f64 = 10.0;

/// One guild's identity, from `SMSG_GUILD_QUERY_RESPONSE`.
#[derive(Clone, Debug, Default)]
pub struct Identity {
    /// Empty means no such guild, as the reference reads it (`0x5552ae`: cache insert `0x561070`
    /// or remove `0x561390`); cached as a negative so the query is not re-sent.
    pub name: String,
    /// The ten rank names, index 0 = guild master; empty past the guild's real rank count.
    pub rank_names: [String; GUILD_RANKS_MAX_COUNT],
    /// The tabard's five emblem indices for the body composite: the reference reads this record
    /// from its character compositor (`0x6d6d20`, `0x47a610`) as well as from `GetGuildInfo`.
    pub emblem: GuildEmblem,
}

impl Identity {
    /// The name of rank `index`, or `""`. Deviation: bounded at the ten slots, because the
    /// reference's read (`0x4c93e7`) runs past its array.
    pub fn rank_name(&self, index: u32) -> &str {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.rank_names.get(i))
            .map(String::as_str)
            .unwrap_or_default()
    }

    pub(crate) fn exists(&self) -> bool {
        !self.name.is_empty()
    }
}

/// Which `GUILD_ROSTER_UPDATE` the feed owes. A truthy arg1 makes the stock pane re-request the
/// roster (`FriendsFrame.lua:646-653`). The reference's `0x4d1160` fires it bare for a parsed
/// roster, a show-offline change, a re-sort and a landed guild record (`0x4d0d3c`, `0x4d0f9f`,
/// `0x4d1022`, `0x4d148c`), and with arg1 only for `SMSG_GUILD_EVENT` cases `0x00`-`0x0b`
/// (`0x5e74c4`) and two `SMSG_GUILD_COMMAND_RESULT` arms (`0x5e7792`), clearing the
/// `GuildRoster()` throttle first. arg1 on a parsed roster would re-request forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterUpdate {
    /// Repaint: a roster just applied, a local re-order, a late identity.
    Applied,
    /// The roster is stale: re-request it.
    Stale,
}

impl RosterUpdate {
    /// Whether this edge carries arg1.
    pub fn arg1(self) -> bool {
        self == RosterUpdate::Stale
    }
}

/// The `guildMemberNotify` CVar, Guild Member Alert on the Chat options page (help string at
/// `0x860320`). Off by default: its register site `0x5e24c7` pushes `"0"` (`0x82e570`). Kept apart
/// from [`GuildState`], which is cleared on disconnect.
#[derive(Resource, Default)]
pub struct GuildMemberNotify(pub bool);

/// The `guildShowOffline` CVar (registered at `0x51e1e2`, default "1"): offline members count and
/// sort in place. Its callback (`0x512730` -> `0x5ca870`) re-sorts and fires the roster update.
#[derive(Resource)]
pub struct GuildShowOffline(pub bool);

impl Default for GuildShowOffline {
    fn default() -> Self {
        Self(true)
    }
}

/// The guild session mirror: filled by the packet handlers, read by the feed, reset on disconnect.
#[derive(Resource, Default)]
pub struct GuildState {
    /// Our `PLAYER_GUILDID` (field 191), `0` when guildless; `IsInGuild` reads it, not the roster.
    pub guild_id: u32,
    /// Our `PLAYER_GUILDRANK` (field 192), 0-based, `0` = guild master.
    pub rank_index: u32,
    /// Guild id to identity, asked once, negatives included; keyed by id because `PLAYER_GUILDID`
    /// is public, so any visible player's guild is nameable ([`unit_guild`]).
    pub identities: QueryCache<u32, Identity>,
    /// Bumped when an identity lands or is evicted, never by an ask; the unit feeds watch it.
    pub identity_generation: u64,
    /// Apart from the roster: `GE_MOTD` sets it alone, and at login vmangos sends that event before
    /// any roster (`CharacterHandler.cpp:558`).
    pub motd: String,
    /// The guild information text (`GuildInfoFrame`), carried only by the roster.
    pub info_text: String,
    /// One rights word per rank id; its length is the guild's real rank count.
    pub rank_rights: Vec<u32>,
    /// The members, in wire order.
    pub members: Vec<GuildRosterMember>,
    pub sort: SortStack,
    /// The selected member as a guid, `0` for none, as `SetGuildRosterSelection 0x4d1820` stores
    /// it (`0xb73128`, at `0x4d186a`/`0x4d1872`): a re-sort keeps the same player selected.
    pub selection: u64,
    /// Member guids in the order last shown, mapping a Lua row index back to its player.
    pub display_order: Vec<u64>,
    /// The held invitation, `(inviter, guild)`; the accept and decline packets do not name it.
    pub pending_invite: Option<(String, String)>,
    pub roster_event: Option<RosterUpdate>,
    /// Real-time seconds before which `GuildRoster()` is swallowed; `0.0` = allowed now.
    pub roster_allowed_at: f64,
    /// The pushed snapshot is stale; the feed rebuilds only when set.
    pub dirty: bool,
    /// `/ginfo` answers for `ui_guild`'s feed, (`ui_guild`), which builds their lines where the VM is: their templates are
    /// not catalog rows (`0x5e6fb0`), so they cannot ride [`crate::ui_action::UiErrorKeys`].
    pub pending_info: Vec<GuildInfo>,
}

impl crate::query_cache::AskOnce for GuildState {
    fn clear_pending(&mut self) {
        self.identities.clear_pending();
    }
}

impl GuildState {
    /// `IsInGuild`: our own `PLAYER_GUILDID` is set, known before any guild packet.
    pub fn in_guild(&self) -> bool {
        self.guild_id != 0
    }

    /// Mirror our descriptor's guild fields; `true` when either moved. Asks for nothing: the
    /// identity cache is lazy and the roster is the FrameXML's to request.
    pub fn mirror_self(&mut self, guild_id: u32, rank_index: u32) -> bool {
        if (self.guild_id, self.rank_index) == (guild_id, rank_index) {
            return false;
        }
        // A first 0 to N is learning our guild, not changing it: at login the MOTD event arrives
        // before our descriptor (`CharacterHandler.cpp:558`), and a wipe here would lose it. Only
        // a move out of a real guild clears the mirror.
        let known = self.guild_id != 0;
        let moved = self.guild_id != guild_id;
        self.guild_id = guild_id;
        self.rank_index = rank_index;
        if moved && known {
            // The old guild's roster goes; the identity cache is keyed by id and stays true.
            self.motd.clear();
            self.info_text.clear();
            self.rank_rights.clear();
            self.members.clear();
            self.selection = 0;
        }
        if moved {
            self.note_roster_update(RosterUpdate::Applied);
        }
        self.dirty = true;
        true
    }

    /// Owe the VM a `GUILD_ROSTER_UPDATE`. [`RosterUpdate::Stale`] wins over `Applied` in one
    /// frame, since a re-request also repaints, and clears the request throttle.
    pub fn note_roster_update(&mut self, kind: RosterUpdate) {
        if kind.arg1() {
            self.roster_allowed_at = 0.0;
        }
        if kind.arg1() || self.roster_event.is_none() {
            self.roster_event = Some(kind);
        }
        self.dirty = true;
    }

    /// Send `CMSG_GUILD_QUERY` for a guild we hold nothing for, a negative counting as held.
    pub fn request_identity(&self, guild_id: u32, commands: &NetCommands) {
        if guild_id != 0 {
            let command = ClientCommand::GuildQuery { guild_id };
            if !commands.supports(&command) {
                return;
            }
            self.identities.get_or_ask(guild_id, || {
                let _ = commands.0.send(command);
            });
        }
    }

    /// A guild's identity if we hold a real one; `None` while in flight and for no such guild.
    /// Our own guild's rank name for `index`, `""` before the identity answers.
    pub fn own_rank_name(&self, index: u32) -> &str {
        self.identity(self.guild_id)
            .map_or("", |i| i.rank_name(index))
    }

    pub fn identity(&self, guild_id: u32) -> Option<&Identity> {
        self.identities.get(guild_id).filter(|i| i.exists())
    }

    /// The read that also asks: `None` on a miss, which sends the query, as the reference's
    /// `GetGuildRosterInfo` answers nil and queries inline (`0x4d1291`).
    pub fn resolve_identity(&self, guild_id: u32, commands: &NetCommands) -> Option<&Identity> {
        self.request_identity(guild_id, commands);
        self.identity(guild_id)
    }

    /// The landed-identity counter the unit feeds watch.
    pub fn identity_generation(&self) -> u64 {
        self.identity_generation
    }

    /// Our guild's five tabard fields for the tabard designer, `-1`s when undesigned as the wire
    /// carries them; a miss sends the query.
    pub fn own_emblem_record(&self, guild_id: u32, commands: &NetCommands) -> Option<[i32; 5]> {
        let e = self.resolve_identity(guild_id, commands)?.emblem;
        Some([
            e.emblem_style,
            e.emblem_color,
            e.border_style,
            e.border_color,
            e.background_color,
        ])
    }

    /// Drop our guild's cached record after a saved emblem (`0x5e715f`), so the next read
    /// re-fetches it and every member in sight re-dresses, as in the reference.
    pub fn evict_own_identity(&mut self) {
        if self.identities.evict(self.guild_id) {
            self.identity_generation = self.identity_generation.wrapping_add(1);
            self.dirty = true;
        }
    }

    /// `SMSG_GUILD_QUERY_RESPONSE`: fill the identity cache, an empty name as a negative.
    pub fn apply_query_response(&mut self, response: GuildQueryResponse) {
        let ours = response.guild_id == self.guild_id;
        self.identities.insert(
            response.guild_id,
            Some(Identity {
                name: response.name,
                rank_names: response.rank_names,
                emblem: GuildEmblem {
                    emblem_style: response.emblem_style,
                    emblem_color: response.emblem_color,
                    border_style: response.border_style,
                    border_color: response.border_color,
                    background_color: response.background_color,
                },
            }),
        );
        self.identity_generation = self.identity_generation.wrapping_add(1);
        self.dirty = true;
        if ours {
            // Our rank names landed: repaint, never re-request, as the reference's cache callback
            // `0x4d1480` does (`0x4d148c`).
            self.note_roster_update(RosterUpdate::Applied);
        }
    }

    /// `SMSG_GUILD_ROSTER`: a complete snapshot, never a delta.
    pub fn apply_roster(&mut self, roster: GuildRoster) {
        self.motd = roster.motd;
        self.info_text = roster.info;
        // Deviation: clamped to ten, because the reference's unbounded loop (`0x4d0bb0`) overruns
        // its ten-slot array on a hostile `rankCount >= 12`.
        self.rank_rights = roster.rank_rights;
        self.rank_rights.truncate(GUILD_RANKS_MAX_COUNT);
        self.members = roster.members;
        self.note_roster_update(RosterUpdate::Applied);
    }

    /// `SMSG_GUILD_EVENT`'s effect on the mirror; its line is [`lines`]'.
    pub fn apply_event(&mut self, notice: &GuildEventNotice) {
        match notice.event {
            guild_event::MOTD => {
                // Stored so `GetGuildRosterMOTD` agrees with the event before the next roster.
                self.motd = notice.params.first().cloned().unwrap_or_default();
                self.dirty = true;
            }
            guild_event::UPDATE_RANK_NAME => {
                // Written into the cached record with no event of its own (`0x560e30`); params are
                // the rank id as text, then the name (vmangos `Guild/Guild.h:133`).
                let rank = notice.params.first().and_then(|p| p.parse::<usize>().ok());
                let name = notice.params.get(1).cloned().unwrap_or_default();
                if let (Some(rank), Some(identity)) = (rank, self.identities.get_mut(self.guild_id))
                {
                    if let Some(slot) = identity.rank_names.get_mut(rank) {
                        *slot = name;
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
        // Only cases `0x00`-`0x0b` carry arg1: not sign-on or sign-off, nor anything past them.
        if notice.event <= guild_event::UPDATE_ROSTER {
            self.note_roster_update(RosterUpdate::Stale);
        }
    }

    /// `SMSG_GUILD_COMMAND_RESULT`'s effect on the mirror: stale only for a successful command
    /// `0x13`/`0x14` or result `0x14` on command `0x05` (`0x5e7792`); the server re-sends the
    /// roster for any real change.
    pub fn apply_command_result(&mut self, result: &GuildCommandResult) {
        use wrathplus_protocol::types::{guild_command, guild_command_error};
        let stale = match result.result {
            guild_command_error::PLAYER_NO_MORE_IN_GUILD => {
                matches!(result.command, guild_command::UNK19 | guild_command::UNK20)
            }
            guild_command_error::UNK20 => result.command == 0x05,
            _ => false,
        };
        if stale {
            self.note_roster_update(RosterUpdate::Stale);
        }
    }

    /// `SMSG_GUILD_INVITE`: hold the invitation, which fires the popup's show edge.
    pub fn apply_invite(&mut self, inviter: String, guild: String) {
        self.pending_invite = Some((inviter, guild));
        self.dirty = true;
    }

    /// The popup's Accept or Decline. Nothing raises a hide edge (`0x48f470` has no callers), but
    /// clearing lets a second invitation fire the show edge again.
    pub fn clear_invite(&mut self) {
        if self.pending_invite.take().is_some() {
            self.dirty = true;
        }
    }

    /// `SortGuildRoster(field)`: re-order and repaint, never re-request ([`sort::SortStack`]).
    pub fn sort_by(&mut self, field: &str) {
        self.sort.select(SortField::parse(field));
        self.note_roster_update(RosterUpdate::Applied);
    }

    /// `SetGuildRosterSelection(index)`: store the row's guid. No event: the stock click handler
    /// repaints itself (`FriendsFrame.lua:706`).
    pub fn select(&mut self, index: u32) {
        let guid = self.guid_at(index).unwrap_or(0);
        if self.selection != guid {
            self.selection = guid;
            self.dirty = true;
        }
    }

    /// The guid at a 1-based display row, over the whole roster, not the show-offline count.
    pub(crate) fn guid_at(&self, index: u32) -> Option<u64> {
        index
            .checked_sub(1)
            .and_then(|i| self.display_order.get(i as usize))
            .copied()
    }

    /// The member at a 1-based display row, mutable because the note verbs write before they send.
    pub fn member_at_mut(&mut self, index: u32) -> Option<&mut GuildRosterMember> {
        let guid = self.guid_at(index)?;
        self.members.iter_mut().find(|m| m.guid == guid)
    }

    /// Our rank's rights word, which every `Can*` predicate tests; `0` (none) before a roster.
    pub fn own_rights(&self) -> u32 {
        self.rank_rights
            .get(self.rank_index as usize)
            .copied()
            .unwrap_or(0)
    }
}

/// Days since logout, fully decomposed as `GetGuildRosterLastOnline 0x4d14a0` does
/// (`0x4d1508`-`0x4d1598`), each step truncated (`_ftol` `0x40a2b0`) and the remainder after
/// months narrowed to f32 (`0x4d1544`). The stock frame shows only the largest non-zero unit, and
/// all zeroes as under an hour (`FriendsFrame.lua:957-978`).
pub fn last_online(days: f32) -> LastOnline {
    if days.is_nan() || days <= 0.0 {
        // NaN, negative, and an online member's absent float: all zeroes.
        return LastOnline::default();
    }
    // f64 for the reference's x87 extended precision; it multiplies by f32 reciprocals (1/365 at
    // `0x80733c`, 1/30 at `0x807334`; 365, 30 and 24 at `0x807338`, `0x807330`, `0x80732c`).
    const PER_YEAR: f32 = 1.0 / 365.0;
    const PER_MONTH: f32 = 1.0 / 30.0;
    let days = f64::from(days);
    let years = (days * f64::from(PER_YEAR)) as u32;
    let rem = days - f64::from(years) * 365.0;
    let months = (rem * f64::from(PER_MONTH)) as u32;
    let rem = (rem - f64::from(months) * 30.0) as f32;
    let day = rem as u32;
    LastOnline {
        years,
        months,
        days: day,
        hours: ((rem - day as f32) * 24.0) as u32,
    }
}

/// `GetGuildInfo(unit)`: the unit's public guild fields joined against the identity cache, asking
/// on a miss. `None` for a guildless unit, a creature or an unanswered query, one nil path in the
/// reference (`0x4c93d7`, `0x4c943c`).
pub fn unit_guild(
    fields: &ObjectFields,
    guild: &mut GuildState,
    commands: &NetCommands,
) -> Option<UnitGuild> {
    let guild_id = fields.player_guild_id();
    let rank_index = fields.player_guild_rank();
    let identity = guild.resolve_identity(guild_id, commands)?;
    Some(UnitGuild {
        name: identity.name.clone(),
        rank_name: identity.rank_name(rank_index).to_string(),
        rank_index,
    })
}

/// A unit's guild name alone: the overhead name stack's guild line (`"\n<%s>"`, `0x860f9c`), shown
/// under `UnitNamePlayerGuild`'s mask bit `0x10` (`0x609085`) and read through `0x5e09f0`.
/// [`unit_guild`] without its clones: [`crate::nameplates::drive_nameplates`] reads it per frame.
pub fn unit_guild_name<'a>(
    fields: &ObjectFields,
    guild: &'a mut GuildState,
    commands: &NetCommands,
) -> Option<&'a str> {
    guild
        .resolve_identity(fields.player_guild_id(), commands)
        .map(|identity| identity.name.as_str())
}

/// A unit's guild crest for the body composite, off its public `PLAYER_GUILDID`, asking on a miss
/// like [`unit_guild`]. `None` keeps a Guild Tabard's own `Tabard_A_05Default` art, as the
/// reference never enters its install (`0x47a610`) for a guildless wearer (`0x560e30` returns NULL
/// at `0x560e3f`), a creature, a query in flight (re-dressed when
/// [`GuildState::identity_generation`] moves; the reference's callback is `0x5e0650`) or an
/// undesigned `-1` crest, which would paint blank.
pub fn unit_guild_emblem(
    fields: &ObjectFields,
    guild: &mut GuildState,
    commands: &NetCommands,
) -> Option<GuildEmblem> {
    guild_emblem(fields.player_guild_id(), guild, commands)
}

/// The same crest for a corpse, off its own `CORPSE_FIELD_GUILD` (`0x5d6edf`) through the same
/// lookup (`0x5d6ec0`), with the same `None` cases.
pub fn corpse_guild_emblem(
    fields: &ObjectFields,
    guild: &mut GuildState,
    commands: &NetCommands,
) -> Option<GuildEmblem> {
    guild_emblem(fields.corpse_guild(), guild, commands)
}

fn guild_emblem(
    guild_id: u32,
    guild: &mut GuildState,
    commands: &NetCommands,
) -> Option<GuildEmblem> {
    let emblem = guild.resolve_identity(guild_id, commands)?.emblem;
    emblem.is_designed().then_some(emblem)
}

/// The guild packet handlers: they update the mirror and queue the message ids [`lines`] names,
/// whose catalog rows pick the surface and the sound; the guild events fire from the feed.
pub mod net {
    use super::*;
    use crate::app_core::errors::UiError;
    use crate::app_core::errors::UiErrorKeys;
    use crate::app_core::interaction::SocialState;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use crate::net::NetHandlerApp;

    /// Register the guild handlers, one per kind, plus the session-end listener.
    pub fn register(app: &mut App) {
        use SessionEventKind as K;
        app.net_handler(K::GuildQueryResponse, on_query_response)
            .net_handler(K::GuildRoster, on_roster)
            .net_handler(K::GuildEvent, on_event)
            .net_handler(K::GuildCommandResult, on_command_result)
            .net_handler(K::GuildInvite, on_invite)
            .net_handler(K::GuildDecline, on_decline)
            .net_handler(K::GuildInfo, on_info)
            .net_handler(K::Disconnected, on_session_end);
    }

    fn on_query_response(In(ev): In<SessionEvent>, mut guild: ResMut<GuildState>) {
        if let SessionEvent::GuildQueryResponse(response) = ev {
            query_response(&mut guild, response);
        }
    }

    fn on_roster(In(ev): In<SessionEvent>, mut guild: ResMut<GuildState>) {
        if let SessionEvent::GuildRoster(r) = ev {
            roster(&mut guild, r);
        }
    }

    /// Reads the friends list, the notify CVar and our name for the sign-on condition ([`event`]).
    fn on_event(
        In(ev): In<SessionEvent>,
        mut guild: ResMut<GuildState>,
        mut errors: ResMut<UiErrorKeys>,
        social: Res<SocialState>,
        notify: Res<GuildMemberNotify>,
        self_guid: Res<crate::app_core::session::SelfGuid>,
        names: Res<crate::names::NameCache>,
    ) {
        if let SessionEvent::GuildEvent(notice) = ev {
            let own_name = self_guid.0.and_then(|g| names.peek(g));
            event(&mut guild, &mut errors, &social, &notify, own_name, notice);
        }
    }

    fn on_command_result(
        In(ev): In<SessionEvent>,
        mut guild: ResMut<GuildState>,
        mut errors: ResMut<UiErrorKeys>,
    ) {
        if let SessionEvent::GuildCommandResult(result) = ev {
            command_result(&mut guild, &mut errors, result);
        }
    }

    fn on_invite(
        In(ev): In<SessionEvent>,
        mut guild: ResMut<GuildState>,
        mut errors: ResMut<UiErrorKeys>,
    ) {
        if let SessionEvent::GuildInvite { inviter, guild: g } = ev {
            invite(&mut guild, &mut errors, inviter, g);
        }
    }

    fn on_decline(In(ev): In<SessionEvent>, mut errors: ResMut<UiErrorKeys>) {
        if let SessionEvent::GuildDecline { name } = ev {
            decline(&mut errors, &name);
        }
    }

    fn on_info(In(ev): In<SessionEvent>, mut guild: ResMut<GuildState>) {
        if let SessionEvent::GuildInfo(i) = ev {
            info(&mut guild, i);
        }
    }

    /// Reset at session end: the next login may be another character. Deviation: the identity
    /// cache goes too, where the reference keeps it in `guildcache.wdb`, because a renamed guild
    /// would otherwise show its old name.
    fn on_session_end(In(_): In<SessionEvent>, mut guild: ResMut<GuildState>) {
        *guild = GuildState::default();
    }

    /// Queue the message ids [`lines`] named.
    fn push_lines(errors: &mut UiErrorKeys, lines: impl IntoIterator<Item = UiError>) {
        errors.0.extend(lines);
    }

    /// `SMSG_GUILD_QUERY_RESPONSE`.
    pub(crate) fn query_response(guild: &mut GuildState, response: GuildQueryResponse) {
        guild.apply_query_response(response);
    }

    /// `SMSG_GUILD_ROSTER`.
    pub(crate) fn roster(guild: &mut GuildState, roster: GuildRoster) {
        guild.apply_roster(roster);
    }

    /// `SMSG_GUILD_EVENT`. The sign-on/sign-off line's condition, three conjuncts in arms 12/13 of
    /// `0x6d92d0` (`0x6d95be`, `0x6d964c`), each failing to the silent exit `0x6d9901`:
    ///
    /// 1. `guildMemberNotify` is on (`[0xbd0974]+0x30`);
    /// 2. `params[0]` is not our name (`GetOwnName` `0x6b1060`), case-sensitively (`SStrCmp`
    ///    `0x76e760`): AzerothCore sends the sign-on to the signer too (`Guild.cpp:1917,2162`);
    /// 3. the guid is not a friend (`0x6b3920`, not the ignore check), whose
    ///    `SMSG_FRIEND_STATUS` prints the same line ungated.
    pub(crate) fn event(
        guild: &mut GuildState,
        errors: &mut UiErrorKeys,
        social: &SocialState,
        notify: &GuildMemberNotify,
        own_name: Option<&str>,
        notice: GuildEventNotice,
    ) {
        let announce = announce_signon(social, notify, own_name, &notice);
        guild.apply_event(&notice);
        push_lines(errors, lines::event_line(&notice, announce));
    }

    /// The sign-on/sign-off line's three conjuncts ([`event`]) as one predicate; no own name, no
    /// subject name or no guid on the wire is the silent exit.
    pub fn announce_signon(
        social: &SocialState,
        notify: &GuildMemberNotify,
        own_name: Option<&str>,
        notice: &GuildEventNotice,
    ) -> bool {
        match (own_name, notice.params.first(), notice.guid) {
            (Some(me), Some(name), Some(guid)) => notify.0 && name != me && !social.is_friend(guid),
            _ => false,
        }
    }

    /// `SMSG_GUILD_COMMAND_RESULT`.
    pub(crate) fn command_result(
        guild: &mut GuildState,
        errors: &mut UiErrorKeys,
        result: GuildCommandResult,
    ) {
        guild.apply_command_result(&result);
        push_lines(errors, lines::command_line(&result));
    }

    /// `SMSG_GUILD_INVITE`: the popup's show edge and the notice line printed beside it.
    pub(crate) fn invite(
        guild: &mut GuildState,
        errors: &mut UiErrorKeys,
        inviter: String,
        guild_name: String,
    ) {
        push_lines(errors, [lines::invite_line(&inviter, &guild_name)]);
        guild.apply_invite(inviter, guild_name);
    }

    /// `SMSG_GUILD_DECLINE`: a line, no state.
    pub(crate) fn decline(errors: &mut UiErrorKeys, name: &str) {
        push_lines(errors, [lines::decline_line(name)]);
    }

    /// `SMSG_GUILD_INFO`, the `/ginfo` answer: parked for the feed, which builds its two lines
    /// ([`GuildState::pending_info`]).
    pub(crate) fn info(guild: &mut GuildState, info: GuildInfo) {
        guild.pending_info.push(info);
    }
}
