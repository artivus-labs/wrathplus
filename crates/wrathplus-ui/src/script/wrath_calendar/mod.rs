//! Build-12340 calendar (`Blizzard_Calendar` and `GameTimeFrame`'s callers): the 80 `Calendar*`
//! functions and `OpenCalendar`, over the calendar App reduces from the native receipts
//! ([`WrathCalendarView`]). Evidence: one headless Ghidra round on the pinned Wow.exe over the
//! bindings (`/tmp/azb-app-mig/ghidra/cal1.log`), the AzerothCore calendar handlers for what the
//! server sends, and the stock Lua's calls for the shapes.
//!
//! Local, as natively: the shown month (`0xacff8c`/`0xacff90`), the selected and context events,
//! the clipboard, the open event's edited fields and its settings-changed bit (`+0x501`), the invite
//! sort (`0x5f1980`), the selected invite, the action-pending bit (`0xc207f3`) and the send
//! throttles (an invite 2 s after the last, `0xc207f8`; an add 5 s, `0xc207fc`). Everything the
//! server owns comes from App; gestures leave as [`WrathCalendarRequest`]s.
//!
//! Labelled: the day list's order (`0x5b9260`) is App's (holidays, then by time); a holiday's
//! sequence split, the raid-reset projection and the status-option filter (`0x5f26f0`) are not
//! traced.

mod bindings;
mod date;

use std::sync::Arc;

pub use date::WrathCalendarDate;

/// What a day row is, `0x5b7fd0`'s calendar-type string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WrathCalendarKind {
    #[default]
    Player,
    GuildAnnouncement,
    GuildEvent,
    /// Labelled: AzerothCore never sends a system event.
    System,
    Holiday,
    RaidLockout,
    RaidReset,
    /// "": an event whose flags `0x5b7fd0` names nothing (an event created with flags 0).
    Blank,
}

impl WrathCalendarKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Player => "PLAYER",
            Self::GuildAnnouncement => "GUILD_ANNOUNCEMENT",
            Self::GuildEvent => "GUILD_EVENT",
            Self::System => "SYSTEM",
            Self::Holiday => "HOLIDAY",
            Self::RaidLockout => "RAID_LOCKOUT",
            Self::RaidReset => "RAID_RESET",
            Self::Blank => "",
        }
    }

    /// An event's type by its flags, `0x5b7fd0`'s tests in its order: the player-allowed bit 0x1
    /// first, so a guild flag on a player event still reads PLAYER.
    pub fn of_flags(flags: u32) -> Self {
        const ORDER: [(u32, WrathCalendarKind); 9] = [
            (0x1, WrathCalendarKind::Player),
            (0x40, WrathCalendarKind::GuildAnnouncement),
            (0x400, WrathCalendarKind::GuildEvent),
            (0x4, WrathCalendarKind::System),
            (0x8, WrathCalendarKind::Holiday),
            (0x80, WrathCalendarKind::RaidLockout),
            (0x200, WrathCalendarKind::RaidReset),
            (0x2, WrathCalendarKind::Player),
            (0x100, WrathCalendarKind::Player),
        ];
        ORDER
            .iter()
            .find(|(bit, _)| flags & bit != 0)
            .map_or(Self::Blank, |&(_, kind)| kind)
    }
}

/// One row of a day: an event or invite, a holiday's day, or a raid lockout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarEntry {
    pub kind: WrathCalendarKind,
    /// Server-local.
    pub date: WrathCalendarDate,
    pub title: String,
    pub event_id: u64,
    /// The player's invite, 0 for none (a guild event to sign up for).
    pub invite_id: u64,
    pub creator: u64,
    /// The creator's name once known.
    pub invited_by: Option<String>,
    pub flags: u32,
    /// `CALENDAR_TYPE_*` 0..4 (raid, dungeon, PvP, meeting, other).
    pub event_type: u8,
    pub dungeon: i32,
    /// The holiday texture's base, or the event's `LFGDungeons` texture.
    pub texture: String,
    /// The player's invite: wire status 0..8 and rank 0..2; `None` without an invite.
    pub invite: Option<(u8, u8)>,
    /// `+0x9c`: 1 pending, 2 moderator, 4 creator, 8 sign-up (`0x5c2cd9`).
    pub bits: u8,
    /// A holiday's day: its 0-based place and the holiday's day count (`+0xc4`, `+0xc8`).
    pub sequence: (u32, u32),
    pub holiday_description: String,
    /// A raid row's map, instance id and difficulty, with the difficulty's string key.
    pub raid: Option<WrathCalendarRaid>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarRaid {
    pub map: u32,
    pub instance: u64,
    pub difficulty: u32,
    /// `MapDifficulty`'s difficulty string, a global-string key.
    pub difficulty_key: String,
}

/// A raid map's weekly reset, projected over the shown days: `first + n·period` seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarRaidReset {
    pub map: u32,
    pub name: String,
    /// Server-local seconds since 1970 of one reset.
    pub first: i64,
    pub period: i64,
    pub difficulty_key: String,
}

/// One invitee of the open event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarInvitee {
    pub guid: u64,
    pub name: Option<String>,
    pub level: u8,
    pub class_name: Option<String>,
    pub class_file: Option<String>,
    /// Wire status 0..8.
    pub status: u8,
    /// 0 player, 1 moderator, 2 creator.
    pub rank: u8,
    /// 1 for a guild event's sign-up.
    pub signup: bool,
    pub invite_id: u64,
    pub status_time: Option<WrathCalendarDate>,
    pub notes: String,
}

/// The event the player opened (`SMSG_CALENDAR_SEND_EVENT`), or the draft a `CalendarNew*` began
/// (`event_id` 0, its pre-invites App's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarEvent {
    pub event_id: u64,
    pub creator: u64,
    pub creator_name: Option<String>,
    pub title: String,
    pub description: String,
    pub event_type: u8,
    pub repeat: u8,
    pub max_invites: u32,
    pub dungeon: i32,
    pub flags: u32,
    pub date: WrathCalendarDate,
    /// AzerothCore keeps the lockout as the event's time-zone time.
    pub lockout: WrathCalendarDate,
    pub invitees: Vec<WrathCalendarInvitee>,
    /// Bumped by App whenever the server changes the event's own fields.
    pub revision: u32,
}

/// `CalendarEventGetTextures`' row: an `LFGDungeons` dungeon or raid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarTexture {
    pub dungeon: i32,
    pub title: String,
    pub texture: String,
    pub expansion: i32,
    /// Raids only: the `MapDifficulty` string key.
    pub difficulty_key: Option<String>,
}

/// Everything App publishes, replaced whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarView {
    pub me: u64,
    /// Every listed row, each on its own day, in App's order.
    pub entries: Vec<WrathCalendarEntry>,
    pub raid_resets: Vec<WrathCalendarRaidReset>,
    pub pending_invites: u32,
    pub event: Option<WrathCalendarEvent>,
    /// Raid rows (type 1) then dungeon rows (type 2).
    pub textures: [Vec<WrathCalendarTexture>; 2],
    /// `CalendarDefaultGuildFilter`'s level, the account expansion's cap (`0xa18684`).
    pub level_cap: u32,
    pub gm: bool,
}

/// A gesture App sends or applies, in call order.
#[derive(Clone, Debug, PartialEq)]
pub enum WrathCalendarRequest {
    GetCalendar,
    GetEvent {
        event_id: u64,
    },
    /// `CalendarNew*`: a draft with these flags; an arena event names the team slot.
    NewDraft {
        flags: u32,
        arena_slot: Option<u8>,
    },
    AddEvent(WrathCalendarFields),
    UpdateEvent {
        event_id: u64,
        invite_id: u64,
        fields: WrathCalendarFields,
    },
    RemoveEvent {
        event_id: u64,
        invite_id: u64,
        flags: u32,
    },
    CopyEvent {
        event_id: u64,
        invite_id: u64,
        date: WrathCalendarDate,
    },
    Invite {
        event_id: u64,
        invite_id: u64,
        name: String,
        pre_invite: bool,
        guild_event: bool,
    },
    RemoveInvite {
        event_id: u64,
        invitee: u64,
        invite_id: u64,
        owner_invite_id: u64,
    },
    SetStatus {
        event_id: u64,
        invitee: u64,
        invite_id: u64,
        owner_invite_id: u64,
        status: u8,
    },
    SetModerator {
        event_id: u64,
        invitee: u64,
        invite_id: u64,
        owner_invite_id: u64,
        rank: u8,
    },
    Rsvp {
        event_id: u64,
        invite_id: u64,
        status: u8,
    },
    SignUp {
        event_id: u64,
        tentative: bool,
    },
    Complain {
        event_id: u64,
        guid: u64,
    },
    MassInviteGuild {
        min_level: u32,
        max_level: u32,
        min_rank: u32,
    },
    MassInviteArenaTeam {
        slot: u8,
    },
}

/// An event's own fields, as edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCalendarFields {
    pub title: String,
    pub description: String,
    pub event_type: u8,
    pub repeat: u8,
    pub max_invites: u32,
    pub dungeon: i32,
    pub date: WrathCalendarDate,
    pub lockout: WrathCalendarDate,
    pub flags: u32,
}

impl WrathCalendarFields {
    fn of(e: &WrathCalendarEvent) -> Self {
        Self {
            title: e.title.clone(),
            description: e.description.clone(),
            event_type: e.event_type,
            repeat: e.repeat,
            max_invites: e.max_invites,
            dungeon: e.dungeon,
            date: e.date,
            lockout: e.lockout,
            flags: e.flags,
        }
    }
}

/// A day row's place: absolute year, month, day and 0-based index.
type Slot = (i32, u8, u8, usize);

/// The open event (`0xc207ec`), kept until another arrives or a `CalendarNew*` replaces it.
#[derive(Clone, Debug, PartialEq)]
enum Opened {
    /// A server event, from its `SMSG_CALENDAR_SEND_EVENT`.
    Event { event_id: u64, invite_id: u64 },
    /// A `CalendarNew*` draft.
    Draft,
}

#[derive(Clone, Debug)]
struct Open {
    what: Opened,
    fields: WrathCalendarFields,
    /// The view's revision the fields came from.
    synced: Option<(u64, u32)>,
    changed: bool,
    /// The selected invite's id (`+0x20`), 0 at construction (`0x5f106a`).
    selected_invite: u64,
}

impl Open {
    fn new(what: Opened, fields: WrathCalendarFields) -> Self {
        Self {
            what,
            fields,
            synced: None,
            changed: false,
            selected_invite: 0,
        }
    }
}

/// `CalendarEventSortInvites`' criteria, `0x5b8570` order.
const CRITERIA: [&str; 6] = ["name", "level", "class", "status", "party", "notes"];

#[derive(Default)]
pub(crate) struct State {
    view: Arc<WrathCalendarView>,
    /// The game clock (`0xd37f98`) the host pushes, as `GetGameTime`'s; `None` before the first.
    clock: Option<WrathCalendarDate>,
    shown: Option<(i32, u8)>,
    /// The selection record (`0xc20f78`) `CalendarGetEventIndex` answers.
    selected: Option<Slot>,
    /// The event and invite a `CalendarOpenEvent` asked for, until it arrives.
    asked: Option<(u64, u64)>,
    context: Option<Slot>,
    clipboard: Option<(u64, u64, WrathCalendarDate)>,
    open: Option<Open>,
    /// `None` sorts by status, the criterion an unknown name also falls to.
    sort: Option<(usize, bool)>,
    action_pending: bool,
    last_invite: Option<f64>,
    last_add: Option<f64>,
    requests: Vec<WrathCalendarRequest>,
}

pub(super) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    bindings::install(lua)
}

impl super::UiScript {
    /// Publish App's calendar; an open event whose fields the player has not touched follows the
    /// server's.
    pub fn set_wrath_calendar(&mut self, view: Arc<WrathCalendarView>) {
        let mut model = self.model_mut();
        let state = &mut model.wrath_calendar;
        state.view = view;
        state.arrive();
        state.sync_open();
    }

    /// Push the game clock's date and time, what `CalendarGetDate` (`0x5b8160`) reads.
    pub fn set_game_date(&mut self, now: WrathCalendarDate) {
        self.model_mut().wrath_calendar.clock = Some(now);
    }

    /// The bit `CalendarIsActionPending` answers, announced as `CALENDAR_ACTION_PENDING`; App
    /// clears it after the packets 12340's dispatcher clears it after (`0x5c4037`).
    pub fn set_wrath_calendar_action_pending(&mut self, pending: bool) {
        self.model_mut().wrath_calendar.action_pending = pending;
        let arg = super::ScriptValue::Bool(pending);
        self.fire_event("CALENDAR_ACTION_PENDING", vec![arg]);
    }

    /// The gestures since the last drain, in call order.
    pub fn take_wrath_calendar_requests(&mut self) -> Vec<WrathCalendarRequest> {
        std::mem::take(&mut self.model_mut().wrath_calendar.requests)
    }

    /// The event the player has open: its id, 0 for a draft.
    pub fn wrath_calendar_open_event(&self) -> Option<u64> {
        match &self.model_ref().wrath_calendar.open {
            Some(Open {
                what: Opened::Event { event_id, .. },
                ..
            }) => Some(*event_id),
            Some(Open {
                what: Opened::Draft,
                ..
            }) => Some(0),
            _ => None,
        }
    }
}

impl State {
    fn today(&self) -> WrathCalendarDate {
        self.clock.unwrap_or_else(WrathCalendarDate::system_now)
    }

    fn shown(&self) -> (i32, u8) {
        self.shown.unwrap_or_else(|| {
            let t = self.today();
            (t.year, t.month)
        })
    }

    /// The shown month moved by `offset` months, clamped as `0x5b8ec0` does.
    fn month(&self, offset: i32) -> (i32, u8) {
        let (y, m) = self.shown();
        date::offset_month(y, m, offset)
    }

    /// The rows of a day: App's entries on it, then the raid resets that fall on it, by time.
    fn day(&self, y: i32, m: u8, d: u8) -> Vec<WrathCalendarEntry> {
        let mut rows: Vec<WrathCalendarEntry> = self
            .view
            .entries
            .iter()
            .filter(|e| (e.date.year, e.date.month, e.date.day) == (y, m, d))
            .cloned()
            .collect();
        let start = date::to_seconds(y, m, d, 0, 0);
        for r in &self.view.raid_resets {
            if r.period <= 0 {
                continue;
            }
            let n = (start - r.first).div_euclid(r.period) + 1;
            let mut t = r.first + n * r.period;
            if t - r.period >= start {
                t -= r.period;
            }
            if t >= start && t < start + 86_400 {
                rows.push(WrathCalendarEntry {
                    kind: WrathCalendarKind::RaidReset,
                    date: WrathCalendarDate::from_seconds(t),
                    title: r.name.clone(),
                    raid: Some(WrathCalendarRaid {
                        map: r.map,
                        instance: 0,
                        difficulty: 0,
                        difficulty_key: r.difficulty_key.clone(),
                    }),
                    ..Default::default()
                });
            }
        }
        rows
    }

    fn row(&self, slot: Slot) -> Option<WrathCalendarEntry> {
        self.day(slot.0, slot.1, slot.2).into_iter().nth(slot.3)
    }

    /// A day addressed the reference's way, as `0x5b91a0` accepts it: a month offset -1..1 from the
    /// shown month and a day 1..31 of a date between 24 November 2004 and 31 December 2030.
    fn slot(&self, offset: i32, day: i64, index: i64) -> Option<Slot> {
        let (y, m) = Some(offset)
            .filter(|o| (-1..=1).contains(o))
            .map(|o| self.month(o))?;
        let d = u8::try_from(day).ok().filter(|d| (1..=31).contains(d))?;
        let i = usize::try_from(index - 1).ok()?;
        ((2004, 11, 24) <= (y, m, d) && (y, m, d) <= (2030, 12, 31)).then_some((y, m, d, i))
    }

    /// The offset of `(y, m)` from the shown month.
    fn offset_of(&self, y: i32, m: u8) -> i32 {
        let (sy, sm) = self.shown();
        (y - sy) * 12 + i32::from(m) - i32::from(sm)
    }

    fn event(&self) -> Option<&WrathCalendarEvent> {
        let open = self.open.as_ref()?;
        let event = self.view.event.as_ref()?;
        match open.what {
            Opened::Event { event_id, .. } if event.event_id == event_id => Some(event),
            Opened::Draft if event.event_id == 0 => Some(event),
            _ => None,
        }
    }

    /// The open event's invitees in the chosen sort.
    fn invitees(&self) -> Vec<WrathCalendarInvitee> {
        let mut list = self.event().map(|e| e.invitees.clone()).unwrap_or_default();
        let (criterion, reverse) = self.sort();
        let key = |i: &WrathCalendarInvitee| -> (u32, String) {
            match criterion {
                0 => (0, i.name.clone().unwrap_or_default().to_lowercase()),
                1 => (u32::from(i.level), String::new()),
                2 => (0, i.class_file.clone().unwrap_or_default()),
                5 => (0, i.notes.to_lowercase()),
                // Labelled: party membership is not held here; it sorts as status.
                _ => (u32::from(i.status), String::new()),
            }
        };
        list.sort_by_key(|a| key(a));
        if reverse {
            list.reverse();
        }
        list
    }

    fn my_invitee(&self) -> Option<WrathCalendarInvitee> {
        let me = self.view.me;
        self.event()?
            .invitees
            .iter()
            .find(|i| i.guid == me)
            .cloned()
    }

    /// `0x5f1b30`: the player created or moderates the open event; a draft is theirs.
    fn can_edit(&self) -> bool {
        match self.open.as_ref().map(|o| &o.what) {
            Some(Opened::Draft) => true,
            Some(Opened::Event { .. }) => self.event().is_some_and(|e| {
                e.creator == self.view.me || self.my_invitee().is_some_and(|i| i.rank >= 1)
            }),
            _ => false,
        }
    }

    /// The asked-for event arrived: it is the open event, and its row the selection (`0x5bef85`).
    fn arrive(&mut self) {
        let Some((event_id, invite_id)) = self.asked else {
            return;
        };
        if self
            .view
            .event
            .as_ref()
            .is_none_or(|e| e.event_id != event_id)
        {
            return;
        }
        let what = Opened::Event {
            event_id,
            invite_id,
        };
        self.open = Some(Open::new(what, WrathCalendarFields::default()));
        self.asked = None;
        self.selected = self
            .view
            .entries
            .iter()
            .find(|e| e.event_id == event_id)
            .and_then(|e| {
                let (y, m, d) = (e.date.year, e.date.month, e.date.day);
                let n = self
                    .day(y, m, d)
                    .iter()
                    .position(|r| r.event_id == event_id)?;
                Some((y, m, d, n))
            });
    }

    /// Refresh untouched fields from a newer server copy of the open event.
    fn sync_open(&mut self) {
        let fresh = self
            .event()
            .map(|e| (e.event_id, e.revision, WrathCalendarFields::of(e)));
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if let Some((id, revision, fields)) = fresh {
            if open.synced != Some((id, revision)) && !open.changed {
                open.fields = fields;
                open.synced = Some((id, revision));
            }
        }
    }

    fn sort(&self) -> (usize, bool) {
        self.sort.unwrap_or((3, false))
    }

    fn push(&mut self, request: WrathCalendarRequest) {
        self.requests.push(request);
    }
}

#[cfg(test)]
mod tests;
