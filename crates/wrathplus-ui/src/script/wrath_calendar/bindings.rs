//! The 96 build-12340 calendar globals. Addresses are the bound C functions
//! (`/tmp/azb-app-mig/ghidra/cal1.log`); shapes follow their pushes.

use mlua::{Lua, MultiValue, Value};

use super::date::{days_in_month, weekday};
use super::{
    Open, Opened, State, WrathCalendarDate, WrathCalendarEntry, WrathCalendarFields,
    WrathCalendarInvitee, WrathCalendarKind as K, WrathCalendarRequest as R, CRITERIA,
};
use crate::script::{Model, ScriptValue};

/// The calendar's last year (`0x5b82a0`'s constant, `0x5b7a80`'s clamp at 31 past 2000).
const MAX_YEAR: i32 = 2031;

/// `CALENDAR_STATUS_*` by wire status (`0x5f1000`).
const STATUS_KEYS: [&str; 9] = [
    "CALENDAR_STATUS_INVITED",
    "CALENDAR_STATUS_ACCEPTED",
    "CALENDAR_STATUS_DECLINED",
    "CALENDAR_STATUS_CONFIRMED",
    "CALENDAR_STATUS_OUT",
    "CALENDAR_STATUS_STANDBY",
    "CALENDAR_STATUS_SIGNEDUP",
    "CALENDAR_STATUS_NOT_SIGNEDUP",
    "CALENDAR_STATUS_TENTATIVE",
];
const TYPE_KEYS: [&str; 5] = [
    "CALENDAR_TYPE_RAID",
    "CALENDAR_TYPE_DUNGEON",
    "CALENDAR_TYPE_PVP",
    "CALENDAR_TYPE_MEETING",
    "CALENDAR_TYPE_OTHER",
];
const REPEAT_KEYS: [&str; 4] = [
    "CALENDAR_REPEAT_NEVER",
    "CALENDAR_REPEAT_WEEKLY",
    "CALENDAR_REPEAT_BIWEEKLY",
    "CALENDAR_REPEAT_MONTHLY",
];
const MONTH_KEYS: [&str; 12] = [
    "MONTH_JANUARY",
    "MONTH_FEBRUARY",
    "MONTH_MARCH",
    "MONTH_APRIL",
    "MONTH_MAY",
    "MONTH_JUNE",
    "MONTH_JULY",
    "MONTH_AUGUST",
    "MONTH_SEPTEMBER",
    "MONTH_OCTOBER",
    "MONTH_NOVEMBER",
    "MONTH_DECEMBER",
];
const WEEKDAY_KEYS: [&str; 7] = [
    "WEEKDAY_SUNDAY",
    "WEEKDAY_MONDAY",
    "WEEKDAY_TUESDAY",
    "WEEKDAY_WEDNESDAY",
    "WEEKDAY_THURSDAY",
    "WEEKDAY_FRIDAY",
    "WEEKDAY_SATURDAY",
];
/// Labelled (`0x5f26f0` untraced): the statuses a moderator may set, by invite kind.
const NORMAL_STATUSES: [u8; 7] = [0, 1, 2, 3, 4, 5, 8];
const SIGNUP_STATUSES: [u8; 6] = [6, 3, 4, 5, 8, 7];
/// Wire statuses.
const INVITED: u8 = 0;
const ACCEPTED: u8 = 1;
const DECLINED: u8 = 2;
const TENTATIVE: u8 = 8;
/// `CALENDAR_INVITESTATUS_NOT_SIGNEDUP` and `CALENDAR_INVITETYPE_SIGNUP`, a guild event the
/// player has not answered (Labelled: `0x9f0df0`/`0x9e8ce8` read as these).
const LUA_NOT_SIGNEDUP: i64 = 8;
const LUA_SIGNUP: i64 = 2;
/// `CALENDAR_FLAG_INVITES_LOCKED`, and the auto-approve bit `CalendarGetEventInfo` reads.
const LOCKED: u32 = 0x10;
const AUTO_APPROVE: u32 = 0x20;
const GUILD_EVENT: u32 = 0x400;
const ANNOUNCEMENT: u32 = 0x40;
/// `GR_RIGHT_CREATE_GUILD_EVENT`.
const RIGHT_GUILD_EVENT: u32 = 0x10_0000;

type Rv = mlua::Result<MultiValue>;

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn num(lua: &Lua, args: &MultiValue, i: usize) -> Option<f64> {
    lua.coerce_number(arg(args, i)).ok().flatten()
}

/// A rounded number argument.
fn int_arg(lua: &Lua, args: &MultiValue, i: usize) -> Option<i64> {
    num(lua, args, i).map(|n| n.round() as i64)
}

fn text_arg(lua: &Lua, args: &MultiValue, i: usize) -> Option<String> {
    lua.coerce_string(arg(args, i))
        .ok()
        .flatten()
        .map(|s| s.to_string_lossy())
}

fn i(n: impl Into<i64>) -> Value {
    Value::Integer(n.into())
}

fn f(n: impl Into<f64>) -> Value {
    Value::Number(n.into())
}

fn s(lua: &Lua, text: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(text)?))
}

fn b(v: bool) -> Value {
    if v {
        Value::Integer(1)
    } else {
        Value::Nil
    }
}

fn many(values: Vec<Value>) -> Rv {
    Ok(MultiValue::from_iter(values))
}

fn usage(text: &'static str) -> Rv {
    Err(mlua::Error::runtime(text))
}

/// A global string, else its key.
fn gs(lua: &Lua, key: &str) -> String {
    lua.globals()
        .get::<Option<String>>(key)
        .ok()
        .flatten()
        .unwrap_or_else(|| key.to_string())
}

fn read<T>(lua: &Lua, f: impl FnOnce(&State) -> T) -> T {
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    f(&model.wrath_calendar)
}

fn write<T>(lua: &Lua, f: impl FnOnce(&mut State, f64) -> T) -> T {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let now = model.now;
    f(&mut model.wrath_calendar, now)
}

fn fire(lua: &Lua, event: &str, args: Vec<ScriptValue>) {
    crate::script::tick::fire_event_into(lua, event, args);
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> Rv + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn date4(d: &WrathCalendarDate) -> Vec<Value> {
    vec![i(d.weekday), i(d.month), i(d.day), i(d.year)]
}

fn mod_status(rank: u8) -> &'static str {
    match rank {
        1 => "MODERATOR",
        2 => "CREATOR",
        _ => "",
    }
}

/// The 15 `CalendarGetDayEvent` values for a row (`0x5c1070`).
fn day_event(lua: &Lua, row: &WrathCalendarEntry) -> Rv {
    let raid = row.raid.as_ref();
    let holiday = row.kind == K::Holiday;
    let (status, rank) = row.invite.unwrap_or((u8::MAX, 0));
    let invite_status = if row.invite.is_some() {
        i(status + 1)
    } else if row.kind == K::GuildEvent {
        i(LUA_NOT_SIGNEDUP)
    } else {
        i(0)
    };
    many(vec![
        s(lua, &row.title)?,
        i(row.date.hour),
        i(row.date.minute),
        s(lua, row.kind.token())?,
        s(lua, if holiday { row.sequence.2 } else { "" })?,
        i(row.event_type + 1),
        s(lua, &row.texture)?,
        s(lua, mod_status(rank))?,
        invite_status,
        s(lua, row.invited_by.as_deref().unwrap_or(""))?,
        i(raid.map_or(0, |r| r.difficulty + 1)),
        i(if row.kind == K::GuildEvent && row.invite.is_none() {
            LUA_SIGNUP
        } else {
            1
        }),
        i(if holiday { row.sequence.0 } else { 1 }),
        i(if holiday { row.sequence.1 } else { 0 }),
        s(
            lua,
            &raid.map_or(String::new(), |r| difficulty_name(lua, &r.difficulty_key)),
        )?,
    ])
}

fn difficulty_name(lua: &Lua, key: &str) -> String {
    if key.is_empty() {
        String::new()
    } else {
        gs(lua, key)
    }
}

/// `0x5b9700`: the 1-based place of a dungeon in its type's texture list, 0 for none.
fn texture_index(st: &State, event_type: u8, dungeon: i32) -> i64 {
    let list = match event_type {
        0 => &st.view.textures[0],
        1 => &st.view.textures[1],
        _ => return 0,
    };
    list.iter()
        .position(|t| t.dungeon == dungeon)
        .map_or(0, |p| p as i64 + 1)
}

/// The day row the three Lua arguments name.
fn row_at(lua: &Lua, args: &MultiValue) -> Option<WrathCalendarEntry> {
    let (o, d, n) = (
        int_arg(lua, args, 0)?,
        int_arg(lua, args, 1)?,
        int_arg(lua, args, 2)?,
    );
    read(lua, |st| st.row(st.slot(o as i32, d, n)?))
}

fn has3(lua: &Lua, args: &MultiValue) -> bool {
    (0..3).all(|k| num(lua, args, k).is_some())
}

/// Edit the open event's fields; a row or nothing open ignores it.
fn edit(lua: &Lua, change: impl FnOnce(&mut WrathCalendarFields)) {
    write(lua, |st, _| {
        if let Some(open) = st
            .open
            .as_mut()
            .filter(|o| !matches!(o.what, Opened::Row(_)))
        {
            change(&mut open.fields);
            open.changed = true;
        }
    });
}

/// The open event's ids and the player's invite.
fn open_ids(st: &State) -> Option<(u64, u64)> {
    match st.open.as_ref()?.what {
        Opened::Event {
            event_id,
            invite_id,
        } => Some((event_id, invite_id)),
        Opened::Draft => Some((0, 0)),
        Opened::Row(_) => None,
    }
}

fn context_row(st: &State) -> Option<WrathCalendarEntry> {
    st.row(st.context?)
}

fn invite_at(st: &State, index: Option<i64>) -> Option<WrathCalendarInvitee> {
    let k = usize::try_from(index? - 1).ok()?;
    st.invitees().into_iter().nth(k)
}

/// An invite gesture on the open event, the player's own invite as the owner.
fn invite_request(
    lua: &Lua,
    args: &MultiValue,
    make: impl FnOnce(u64, &WrathCalendarInvitee, u64) -> R,
) {
    let index = int_arg(lua, args, 0);
    write(lua, |st, _| {
        let Some((event_id, _)) = open_ids(st) else {
            return;
        };
        let Some(target) = invite_at(st, index) else {
            return;
        };
        let owner = st.my_invitee().map_or(0, |m| m.invite_id);
        let request = make(event_id, &target, owner);
        st.push(request);
    });
}

fn rsvp_open(lua: &Lua, status: u8) {
    write(lua, |st, _| {
        if let Some(Opened::Event {
            event_id,
            invite_id,
        }) = st.open.as_ref().map(|o| o.what.clone())
        {
            let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
            st.push(R::Rsvp {
                event_id,
                invite_id,
                status,
            });
        }
    });
}

fn rsvp_context(lua: &Lua, status: u8) {
    write(lua, |st, _| {
        if let Some(row) = context_row(st).filter(|r| r.event_id != 0) {
            st.push(R::Rsvp {
                event_id: row.event_id,
                invite_id: row.invite_id,
                status,
            });
        }
    });
}

fn month_values(y: i32, m: u8) -> Vec<Value> {
    vec![i(m), i(y), i(days_in_month(y, m)), i(weekday(y, m, 1))]
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // ── dates and months ──────────────────────────────────────────────────────────────────────
    // CalendarGetDate (`0x5b8160`): weekday, month, day, year of the server's today.
    set(lua, "CalendarGetDate", |lua, _| {
        let t = read(lua, State::today);
        many(date4(&t))
    })?;
    // CalendarGetMonth([offset]) (`0x5b99f0`): month, year, days, weekday of the 1st.
    set(lua, "CalendarGetMonth", |lua, args| {
        let offset = int_arg(lua, &args, 0).unwrap_or(0) as i32;
        let v = read(lua, |st| {
            let (y, m) = st.month(offset);
            month_values(y, m)
        });
        many(v)
    })?;
    set(lua, "CalendarGetAbsMonth", |lua, args| {
        let (Some(m), y) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: CalendarGetAbsMonth(month[, year])");
        };
        let v = read(lua, |st| {
            let y = y.map_or(st.shown().0, |y| y as i32);
            let (y, m) = super::date::add_months(y, 1, m as i32 - 1);
            month_values(y, m)
        });
        many(v)
    })?;
    // CalendarSetMonth(offset) (`0x5bcc00`) and CalendarSetAbsMonth(month[, year]) (`0x5c43b0`).
    set(lua, "CalendarSetMonth", |lua, args| {
        let Some(offset) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarSetMonth(offsetMonths)");
        };
        write(lua, |st, _| st.shown = Some(st.month(offset as i32)));
        many(vec![])
    })?;
    set(lua, "CalendarSetAbsMonth", |lua, args| {
        let Some(m) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarSetAbsMonth(month[, year])");
        };
        let y = int_arg(lua, &args, 1);
        write(lua, |st, _| {
            let y = y.map_or(st.shown().0, |y| y as i32);
            st.shown = Some(super::date::add_months(y, 1, m as i32 - 1));
        });
        many(vec![])
    })?;
    // CalendarGetMinDate (`0x5b81f0`): 24 November 2004. Labelled: the history floor is the same,
    // where `0x5b8350` steps a copy of today back 14 days and the decompile pushes today itself.
    for name in ["CalendarGetMinDate", "CalendarGetMinHistoryDate"] {
        set(lua, name, |_, _| {
            many(date4(&WrathCalendarDate::new(2004, 11, 24, 0, 0)))
        })?;
    }
    // CalendarGetMaxCreateDate (`0x5b8410` → `0x5b7a80`): the first of the month thirteen months on
    // from today, 23:59, its year at most 2031.
    set(lua, "CalendarGetMaxCreateDate", |lua, _| {
        let t = read(lua, State::today);
        let (y, m) = super::date::add_months(t.year, t.month, 13);
        many(date4(&WrathCalendarDate::new(
            y.min(MAX_YEAR),
            m,
            1,
            23,
            59,
        )))
    })?;
    // CalendarGetMaxDate (`0x5b82a0`): 1 January 2031, a constant.
    set(lua, "CalendarGetMaxDate", |_, _| {
        many(date4(&WrathCalendarDate::new(MAX_YEAR, 1, 1, 23, 59)))
    })?;
    set(lua, "CalendarGetMonthNames", |lua, _| {
        many(
            MONTH_KEYS
                .iter()
                .map(|k| s(lua, &gs(lua, k)))
                .collect::<mlua::Result<_>>()?,
        )
    })?;
    set(lua, "CalendarGetWeekdayNames", |lua, _| {
        many(
            WEEKDAY_KEYS
                .iter()
                .map(|k| s(lua, &gs(lua, k)))
                .collect::<mlua::Result<_>>()?,
        )
    })?;

    // ── the day lists ─────────────────────────────────────────────────────────────────────────
    set(lua, "CalendarGetNumDayEvents", |lua, args| {
        let (Some(o), Some(d)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: CalendarGetNumDayEvents([-1,0,1], monthDay)");
        };
        let n = read(lua, |st| {
            st.slot(o as i32, d, 1)
                .map_or(0, |(y, m, d, _)| st.day(y, m, d).len())
        });
        many(vec![i(n as i64)])
    })?;
    set(lua, "CalendarGetDayEvent", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarGetDayEvent([-1.0.1], monthDay, index)");
        }
        let row = row_at(lua, &args);
        match row {
            Some(row) => day_event(lua, &row),
            None => {
                let e = s(lua, "")?;
                many(vec![
                    Value::Nil,
                    i(0),
                    i(0),
                    e.clone(),
                    e.clone(),
                    i(0),
                    e.clone(),
                    e.clone(),
                    i(0),
                    e.clone(),
                    i(0),
                    i(0),
                    i(0),
                    i(0),
                    e,
                ])
            }
        }
    })?;
    set(lua, "CalendarGetDayEventSequenceInfo", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarGetDayEventSequenceInfo([-1.0.1], monthDay, index)");
        }
        match row_at(lua, &args).filter(|r| r.kind == K::Holiday) {
            Some(r) => many(vec![
                i(r.sequence.0),
                i(r.sequence.1),
                s(lua, r.sequence.2)?,
            ]),
            None => many(vec![i(0), i(0), s(lua, "")?]),
        }
    })?;
    // CalendarGetFirstPendingInvite (`0x5b9e10`): the day's first unanswered invite, 0 for none.
    set(lua, "CalendarGetFirstPendingInvite", |lua, args| {
        let (Some(o), Some(d)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: CalendarGetDay([-1,0,1], monthDay)");
        };
        let n = read(lua, |st| {
            st.slot(o as i32, d, 1).map_or(0, |(y, m, d, _)| {
                st.day(y, m, d)
                    .iter()
                    .position(|r| r.invite.is_some_and(|(s, _)| s == INVITED))
                    .map_or(0, |p| p as i64 + 1)
            })
        });
        many(vec![i(n)])
    })?;
    // CalendarGetHolidayInfo (`0x5c1480`): name, description, texture of a holiday row.
    set(lua, "CalendarGetHolidayInfo", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarGetHolidayInfo([-1,0,1], monthDay, index)");
        }
        match row_at(lua, &args).filter(|r| r.kind == K::Holiday) {
            Some(r) => many(vec![
                s(lua, &r.title)?,
                s(lua, &r.holiday_description)?,
                s(lua, &r.texture)?,
            ]),
            None => many(vec![Value::Nil, Value::Nil, Value::Nil]),
        }
    })?;
    // CalendarGetRaidInfo (`0x5ba000`): name, type, raid id, hour, minute, difficulty, its name.
    set(lua, "CalendarGetRaidInfo", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarGetRaidInfo([-1,0,1], monthDay, index)");
        }
        match row_at(lua, &args).and_then(|r| Some((r.raid.clone()?, r))) {
            Some((raid, r)) => many(vec![
                s(lua, &r.title)?,
                s(lua, r.kind.token())?,
                f(raid.instance as f64),
                i(r.date.hour),
                i(r.date.minute),
                i(raid.difficulty + 1),
                s(lua, &difficulty_name(lua, &raid.difficulty_key))?,
            ]),
            None => {
                let e = s(lua, "")?;
                many(vec![Value::Nil, e.clone(), i(0), i(0), i(0), i(0), e])
            }
        }
    })?;
    set(lua, "CalendarGetNumPendingInvites", |lua, _| {
        let n = read(lua, |st| st.view.pending_invites);
        many(vec![i(n)])
    })?;
    set(lua, "CalendarIsActionPending", |lua, _| {
        let p = read(lua, |st| st.action_pending);
        many(vec![Value::Boolean(p)])
    })?;
    // OpenCalendar(): `CMSG_CALENDAR_GET_CALENDAR`.
    set(lua, "OpenCalendar", |lua, _| {
        write(lua, |st, _| st.push(R::GetCalendar));
        many(vec![])
    })?;
    set(lua, "CanEditGuildEvent", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let can = model.guild.in_guild && model.guild.rights & RIGHT_GUILD_EVENT != 0;
        many(vec![b(can)])
    })?;

    // ── opening, selecting and reading an event ───────────────────────────────────────────────
    // CalendarOpenEvent (`0x5b9eb0` → `0x5b9310`): a player event asks the server; a holiday or
    // raid row opens at once.
    set(lua, "CalendarOpenEvent", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarOpenEvent([-1,0,1], monthDay, index)");
        }
        let (o, d, n) = (
            int_arg(lua, &args, 0).unwrap_or(0),
            int_arg(lua, &args, 1).unwrap_or(0),
            int_arg(lua, &args, 2).unwrap_or(0),
        );
        let opened = write(lua, |st, _| {
            let slot = st.slot(o as i32, d, n)?;
            let row = st.row(slot)?;
            st.selected = Some(slot);
            st.selected_invite = None;
            if matches!(
                row.kind,
                K::Holiday | K::RaidLockout | K::RaidReset | K::System
            ) {
                let kind = row.kind;
                st.open = Some(Open {
                    what: Opened::Row(Box::new(row)),
                    fields: WrathCalendarFields::default(),
                    synced: None,
                    changed: false,
                });
                Some(kind)
            } else {
                st.open = Some(Open {
                    what: Opened::Event {
                        event_id: row.event_id,
                        invite_id: row.invite_id,
                    },
                    fields: WrathCalendarFields::default(),
                    synced: None,
                    changed: false,
                });
                st.push(R::GetEvent {
                    event_id: row.event_id,
                });
                st.sync_open();
                None
            }
        });
        if let Some(kind) = opened {
            fire(
                lua,
                "CALENDAR_OPEN_EVENT",
                vec![ScriptValue::Str(kind.token().into())],
            );
        }
        many(vec![])
    })?;
    set(lua, "CalendarCloseEvent", |lua, _| {
        write(lua, |st, _| {
            if st.open.take().is_some() {
                st.push(R::CloseEvent);
            }
            st.selected = None;
            st.selected_invite = None;
        });
        many(vec![])
    })?;
    set(lua, "CalendarGetEventIndex", |lua, _| {
        let v = read(lua, |st| match st.selected {
            Some((y, m, d, n)) => (st.offset_of(y, m), d as i64, n as i64 + 1),
            None => (0, 0, 0),
        });
        many(vec![i(v.0), i(v.1), i(v.2)])
    })?;
    // CalendarGetEventInfo (`0x5bd8a0`): 25 values of the open event.
    set(lua, "CalendarGetEventInfo", |lua, _| {
        let info = read(lua, |st| {
            let open = st.open.as_ref()?;
            if matches!(open.what, Opened::Row(_)) {
                return None;
            }
            let event = st.event();
            let me = st.my_invitee();
            let fields = open.fields.clone();
            let creator = event
                .and_then(|e| e.creator_name.clone())
                .unwrap_or_default();
            let tex = texture_index(st, fields.event_type, fields.dungeon);
            let (status, kind) = match &me {
                Some(m) => (i64::from(m.status) + 1, i64::from(m.signup) + 1),
                None if fields.flags & GUILD_EVENT != 0 => (LUA_NOT_SIGNEDUP, LUA_SIGNUP),
                None => (0, 0),
            };
            let pending = me.as_ref().is_some_and(|m| m.status == INVITED);
            Some((fields, creator, tex, pending, status, kind))
        });
        let Some((fields, creator, tex, pending, status, kind)) = info else {
            let mut v = vec![Value::Nil, Value::Nil, s(lua, "")?];
            v.extend((0..16).map(|_| i(0)));
            v.extend([Value::Nil, Value::Nil, Value::Nil, i(0), i(0), s(lua, "")?]);
            return many(v);
        };
        let d = fields.date;
        let l = fields.lockout;
        many(vec![
            s(lua, &fields.title)?,
            s(lua, &fields.description)?,
            s(lua, &creator)?,
            i(fields.event_type + 1),
            i(fields.repeat + 1),
            i(fields.max_invites),
            i(tex),
            i(d.weekday),
            i(d.month),
            i(d.day),
            i(d.year),
            i(d.hour),
            i(d.minute),
            i(l.weekday),
            i(l.month),
            i(l.day),
            i(l.year),
            i(l.hour),
            i(l.minute),
            b(fields.flags & LOCKED != 0),
            b(fields.flags & AUTO_APPROVE != 0),
            b(pending),
            i(status),
            i(kind),
            s(lua, K::of_flags(fields.flags).token())?,
        ])
    })?;
    set(lua, "CalendarEventGetCalendarType", |lua, _| {
        let t = read(lua, |st| match &st.open.as_ref()?.what {
            Opened::Row(r) => Some(r.kind.token()),
            _ => Some(K::of_flags(st.open.as_ref()?.fields.flags).token()),
        });
        match t {
            Some(t) => many(vec![s(lua, t)?]),
            None => many(vec![Value::Nil]),
        }
    })?;
    set(lua, "CalendarEventCanEdit", |lua, _| {
        many(vec![Value::Boolean(read(lua, State::can_edit))])
    })?;
    set(lua, "CalendarEventIsModerator", |lua, _| {
        let m = read(lua, |st| {
            st.my_invitee().is_some_and(|m| m.rank >= 1) || st.can_edit()
        });
        many(vec![Value::Boolean(m)])
    })?;
    // Only the creator names moderators.
    set(lua, "CalendarEventCanModerate", |lua, _| {
        let m = read(lua, |st| {
            matches!(st.open.as_ref().map(|o| &o.what), Some(Opened::Draft))
                || st.event().is_some_and(|e| e.creator == st.view.me)
        });
        many(vec![Value::Boolean(m)])
    })?;
    set(lua, "CalendarEventHaveSettingsChanged", |lua, _| {
        let c = read(lua, |st| st.open.as_ref().is_some_and(|o| o.changed));
        many(vec![Value::Boolean(c)])
    })?;
    set(lua, "CalendarEventHasPendingInvite", |lua, _| {
        let p = read(lua, |st| {
            st.my_invitee().is_some_and(|m| m.status == INVITED)
        });
        many(vec![Value::Boolean(p)])
    })?;
    // CalendarCanAddEvent (`0x5b8ba0`) and CalendarCanSendInvite (`0x5b8b30`): nothing pending
    // and, for a player, 5 s since the last add or 2 s since the last invite.
    set(lua, "CalendarCanAddEvent", |lua, _| {
        let can = write(lua, |st, now| {
            !st.action_pending && (st.view.gm || st.last_add.is_none_or(|t| now - t >= 5.0))
        });
        many(vec![Value::Boolean(can)])
    })?;
    set(lua, "CalendarCanSendInvite", |lua, _| {
        let can = write(lua, |st, now| {
            !st.action_pending && (st.view.gm || st.last_invite.is_none_or(|t| now - t >= 2.0))
        });
        many(vec![Value::Boolean(can)])
    })?;

    // ── new events and their fields ───────────────────────────────────────────────────────────
    for (name, flags) in [
        ("CalendarNewEvent", 0),
        ("CalendarNewGuildEvent", GUILD_EVENT),
        ("CalendarNewGuildAnnouncement", ANNOUNCEMENT),
    ] {
        set(lua, name, move |lua, _| {
            new_draft(lua, flags, None);
            many(vec![])
        })?;
    }
    set(lua, "CalendarNewArenaTeamEvent", |lua, args| {
        let Some(slot) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarNewArenaTeamEvent(index)");
        };
        new_draft(lua, 0, u8::try_from(slot).ok());
        many(vec![])
    })?;
    set(lua, "CalendarEventSetTitle", |lua, args| {
        let t = text_arg(lua, &args, 0).unwrap_or_default();
        edit(lua, |f| f.title = t);
        many(vec![])
    })?;
    set(lua, "CalendarEventSetDescription", |lua, args| {
        let t = text_arg(lua, &args, 0).unwrap_or_default();
        edit(lua, |f| f.description = t);
        many(vec![])
    })?;
    set(lua, "CalendarEventSetType", |lua, args| {
        if let Some(t) = int_arg(lua, &args, 0).filter(|t| (1..=5).contains(t)) {
            edit(lua, |f| {
                f.event_type = (t - 1) as u8;
                f.dungeon = -1;
            });
        }
        many(vec![])
    })?;
    set(lua, "CalendarEventSetRepeatOption", |lua, args| {
        if let Some(r) = int_arg(lua, &args, 0).filter(|r| (1..=4).contains(r)) {
            edit(lua, |f| f.repeat = (r - 1) as u8);
        }
        many(vec![])
    })?;
    set(lua, "CalendarEventSetSize", |lua, args| {
        if let Some(n) = int_arg(lua, &args, 0) {
            edit(lua, |f| f.max_invites = n.max(0) as u32);
        }
        many(vec![])
    })?;
    set(lua, "CalendarEventSetTextureID", |lua, args| {
        let index = int_arg(lua, &args, 0);
        let dungeons = read(lua, |st| st.view.textures.clone());
        edit(lua, |f| {
            let list = match f.event_type {
                0 => &dungeons[0],
                1 => &dungeons[1],
                _ => return,
            };
            f.dungeon = index
                .and_then(|k| usize::try_from(k - 1).ok())
                .and_then(|k| list.get(k))
                .map_or(-1, |t| t.dungeon);
        });
        many(vec![])
    })?;
    for (name, lockout) in [
        ("CalendarEventSetDate", false),
        ("CalendarEventSetLockoutDate", true),
    ] {
        set(lua, name, move |lua, args| {
            let (Some(m), Some(d), Some(y)) = (
                int_arg(lua, &args, 0),
                int_arg(lua, &args, 1),
                int_arg(lua, &args, 2),
            ) else {
                return many(vec![]);
            };
            edit(lua, |f| {
                let slot = if lockout { &mut f.lockout } else { &mut f.date };
                *slot = WrathCalendarDate::new(y as i32, m as u8, d as u8, slot.hour, slot.minute);
            });
            many(vec![])
        })?;
    }
    for (name, lockout) in [
        ("CalendarEventSetTime", false),
        ("CalendarEventSetLockoutTime", true),
    ] {
        set(lua, name, move |lua, args| {
            let (Some(h), Some(m)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
                return many(vec![]);
            };
            edit(lua, |f| {
                let slot = if lockout { &mut f.lockout } else { &mut f.date };
                slot.hour = h.clamp(0, 23) as u8;
                slot.minute = m.clamp(0, 59) as u8;
            });
            many(vec![])
        })?;
    }
    for (name, bit, on) in [
        ("CalendarEventSetLocked", LOCKED, true),
        ("CalendarEventClearLocked", LOCKED, false),
        ("CalendarEventSetAutoApprove", AUTO_APPROVE, true),
        ("CalendarEventClearAutoApprove", AUTO_APPROVE, false),
    ] {
        set(lua, name, move |lua, _| {
            edit(lua, |f| {
                if on {
                    f.flags |= bit;
                } else {
                    f.flags &= !bit;
                }
            });
            many(vec![])
        })?;
    }
    // CalendarAddEvent and CalendarUpdateEvent send the edited fields; both set the pending bit.
    set(lua, "CalendarAddEvent", |lua, _| {
        write(lua, |st, now| {
            let Some(open) = st.open.as_ref().filter(|o| o.what == Opened::Draft) else {
                return;
            };
            let fields = open.fields.clone();
            st.push(R::AddEvent(fields));
            st.last_add = Some(now);
            st.action_pending = true;
        });
        many(vec![])
    })?;
    set(lua, "CalendarUpdateEvent", |lua, _| {
        write(lua, |st, _| {
            let Some(Open {
                what:
                    Opened::Event {
                        event_id,
                        invite_id,
                    },
                fields,
                ..
            }) = st.open.clone()
            else {
                return;
            };
            let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
            st.push(R::UpdateEvent {
                event_id,
                invite_id,
                fields,
            });
            st.action_pending = true;
            if let Some(open) = st.open.as_mut() {
                open.changed = false;
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarRemoveEvent", |lua, _| {
        write(lua, |st, _| {
            if let Some((event_id, invite_id)) = open_ids(st).filter(|(e, _)| *e != 0) {
                let flags = st.open.as_ref().map_or(0, |o| o.fields.flags);
                let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
                st.push(R::RemoveEvent {
                    event_id,
                    invite_id,
                    flags,
                });
                st.action_pending = true;
            }
        });
        many(vec![])
    })?;

    // ── the option lists ──────────────────────────────────────────────────────────────────────
    set(lua, "CalendarEventGetTypes", |lua, _| {
        many(
            TYPE_KEYS
                .iter()
                .map(|k| s(lua, &gs(lua, k)))
                .collect::<mlua::Result<_>>()?,
        )
    })?;
    set(lua, "CalendarEventGetRepeatOptions", |lua, _| {
        many(
            REPEAT_KEYS
                .iter()
                .map(|k| s(lua, &gs(lua, k)))
                .collect::<mlua::Result<_>>()?,
        )
    })?;
    // CalendarEventGetTextures(eventType) (`0x5bb970`): title, texture, expansion and, for a
    // raid, the difficulty's name, per row of the type's `LFGDungeons` list.
    set(lua, "CalendarEventGetTextures", |lua, args| {
        let Some(t) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventGetTextures(eventType)");
        };
        let list = read(lua, |st| match t {
            1 => st.view.textures[0].clone(),
            2 => st.view.textures[1].clone(),
            _ => Vec::new(),
        });
        let mut v = Vec::with_capacity(list.len() * 4);
        for row in list {
            v.push(s(lua, &row.title)?);
            v.push(s(lua, &row.texture)?);
            v.push(i(row.expansion));
            let name = row
                .difficulty_key
                .filter(|_| t == 1)
                .map(|k| difficulty_name(lua, &k))
                .unwrap_or_default();
            v.push(s(lua, &name)?);
        }
        many(v)
    })?;
    // CalendarEventGetStatusOptions(index) (`0x5bb130`): (status, name) pairs a moderator may
    // set; anyone else is refused with `CALENDAR_ERROR_PERMISSIONS`.
    set(lua, "CalendarEventGetStatusOptions", |lua, args| {
        let Some(index) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventGetStatusOptions(index)");
        };
        let (open, can, invitee) = read(lua, |st| {
            (st.open.is_some(), st.can_edit(), invite_at(st, Some(index)))
        });
        if !open {
            return many(vec![]);
        }
        if !can {
            let text = gs(lua, "CALENDAR_ERROR_PERMISSIONS");
            fire(lua, "CALENDAR_UPDATE_ERROR", vec![ScriptValue::Str(text)]);
            return many(vec![]);
        }
        let statuses: &[u8] = if invitee.is_some_and(|i| i.signup) {
            &SIGNUP_STATUSES
        } else {
            &NORMAL_STATUSES
        };
        let mut v = Vec::new();
        for &st in statuses {
            v.push(i(st + 1));
            v.push(s(lua, &gs(lua, STATUS_KEYS[usize::from(st)]))?);
        }
        many(v)
    })?;

    // ── the invite list ───────────────────────────────────────────────────────────────────────
    set(lua, "CalendarEventGetNumInvites", |lua, _| {
        let n = read(lua, |st| st.event().map_or(0, |e| e.invitees.len()));
        many(vec![i(n as i64)])
    })?;
    // CalendarEventGetInvite(index) (`0x5ba220`): name, level, class name and file, status,
    // moderator status, mine, invite type, notes.
    set(lua, "CalendarEventGetInvite", |lua, args| {
        let Some(index) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventGetInvite(index)");
        };
        let (row, me) = read(lua, |st| (invite_at(st, Some(index)), st.view.me));
        let Some(r) = row else {
            return many(vec![
                Value::Nil,
                i(0),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Boolean(false),
                i(0),
                Value::Nil,
            ]);
        };
        let opt = |lua: &Lua, v: &Option<String>| -> mlua::Result<Value> {
            Ok(match v {
                Some(v) => s(lua, v)?,
                None => s(lua, "")?,
            })
        };
        many(vec![
            match &r.name {
                Some(n) => s(lua, n)?,
                None => Value::Nil,
            },
            i(r.level),
            opt(lua, &r.class_name)?,
            opt(lua, &r.class_file)?,
            i(r.status + 1),
            s(lua, mod_status(r.rank))?,
            Value::Boolean(r.guid == me),
            i(i64::from(r.signup) + 1),
            s(lua, &r.notes)?,
        ])
    })?;
    set(lua, "CalendarEventGetInviteResponseTime", |lua, args| {
        let index = int_arg(lua, &args, 0);
        let t = read(lua, |st| invite_at(st, index).and_then(|r| r.status_time));
        match t {
            Some(t) => many(vec![
                i(t.weekday),
                i(t.month),
                i(t.day),
                i(t.year),
                i(t.hour),
                i(t.minute),
            ]),
            None => many((0..6).map(|_| i(0)).collect()),
        }
    })?;
    // CalendarEventInvite(name) (`0x5bcf00` → `0x5bc360`): a draft's invite is a pre-invite.
    set(lua, "CalendarEventInvite", |lua, args| {
        let Some(name) = text_arg(lua, &args, 0).filter(|n| !n.is_empty()) else {
            return many(vec![]);
        };
        write(lua, |st, now| {
            let Some((event_id, invite_id)) = open_ids(st) else {
                return;
            };
            let guild_event = st
                .open
                .as_ref()
                .is_some_and(|o| o.fields.flags & GUILD_EVENT != 0);
            let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
            st.push(R::Invite {
                event_id,
                invite_id,
                name,
                pre_invite: event_id == 0,
                guild_event,
            });
            st.last_invite = Some(now);
        });
        many(vec![])
    })?;
    set(lua, "CalendarEventRemoveInvite", |lua, args| {
        invite_request(lua, &args, |event_id, t, owner| R::RemoveInvite {
            event_id,
            invitee: t.guid,
            invite_id: t.invite_id,
            owner_invite_id: owner,
        });
        many(vec![])
    })?;
    // CalendarEventSetStatus(index, status) (`0x5b8730` → `0x5b7d60`).
    set(lua, "CalendarEventSetStatus", |lua, args| {
        let Some(status) = int_arg(lua, &args, 1).filter(|s| (1..=9).contains(s)) else {
            return usage("Usage: CalendarEventSetStatus(index, status)");
        };
        invite_request(lua, &args, |event_id, t, owner| R::SetStatus {
            event_id,
            invitee: t.guid,
            invite_id: t.invite_id,
            owner_invite_id: owner,
            status: (status - 1) as u8,
        });
        many(vec![])
    })?;
    for (name, rank) in [
        ("CalendarEventSetModerator", 1u8),
        ("CalendarEventClearModerator", 0u8),
    ] {
        set(lua, name, move |lua, args| {
            invite_request(lua, &args, |event_id, t, owner| R::SetModerator {
                event_id,
                invitee: t.guid,
                invite_id: t.invite_id,
                owner_invite_id: owner,
                rank,
            });
            many(vec![])
        })?;
    }
    set(lua, "CalendarEventSelectInvite", |lua, args| {
        let index = int_arg(lua, &args, 0);
        write(lua, |st, _| {
            st.selected_invite = index.and_then(|k| usize::try_from(k - 1).ok());
        });
        many(vec![])
    })?;
    set(lua, "CalendarEventGetSelectedInvite", |lua, _| {
        let k = read(lua, |st| st.selected_invite.map_or(0, |k| k as i64 + 1));
        many(vec![i(k)])
    })?;
    // CalendarEventSortInvites("criteria", reverse) (`0x5b8570`): a change fires
    // `CALENDAR_UPDATE_INVITE_LIST` (event `0x23e`); an unknown criterion sorts by status.
    set(lua, "CalendarEventSortInvites", |lua, args| {
        let Some(criterion) = text_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventSortInvites(\"criteria\", reverse)");
        };
        let reverse = !matches!(arg(&args, 1), Value::Nil | Value::Boolean(false));
        let index = CRITERIA.iter().position(|c| *c == criterion).unwrap_or(3);
        let changed = write(lua, |st, _| {
            let changed = st.open.is_some() && st.sort() != (index, reverse);
            st.sort = Some((index, reverse));
            changed
        });
        if changed {
            fire(lua, "CALENDAR_UPDATE_INVITE_LIST", vec![]);
        }
        many(vec![])
    })?;
    set(lua, "CalendarEventGetInviteSortCriterion", |lua, _| {
        let (c, r) = read(lua, State::sort);
        many(vec![s(lua, CRITERIA[c])?, Value::Boolean(r)])
    })?;
    // CalendarMassInviteGuild(minLevel, maxLevel, minRank) (`0x5bccb0` → `0x5bc160`).
    set(lua, "CalendarMassInviteGuild", |lua, args| {
        let (Some(lo), Some(hi), Some(rank)) = (
            int_arg(lua, &args, 0),
            int_arg(lua, &args, 1),
            int_arg(lua, &args, 2),
        ) else {
            return usage("Usage: CalendarMassInviteGuild(minLevel, maxLevel, minRank)");
        };
        write(lua, |st, now| {
            st.push(R::MassInviteGuild {
                min_level: lo.max(0) as u32,
                max_level: hi.max(0) as u32,
                min_rank: (rank - 1).max(0) as u32,
            });
            st.last_invite = Some(now);
        });
        many(vec![])
    })?;
    set(lua, "CalendarMassInviteArenaTeam", |lua, args| {
        if let Some(slot) = int_arg(lua, &args, 0).and_then(|s| u8::try_from(s).ok()) {
            write(lua, |st, now| {
                st.push(R::MassInviteArenaTeam { slot });
                st.last_invite = Some(now);
            });
        }
        many(vec![])
    })?;
    // CalendarDefaultGuildFilter (`0x5bcda0`): the level cap twice, then the guild's rank count.
    set(lua, "CalendarDefaultGuildFilter", |lua, _| {
        let level = read(lua, |st| st.view.level_cap);
        let ranks = lua
            .app_data_ref::<Model>()
            .expect("model app_data")
            .guild
            .ranks
            .len();
        many(vec![i(level), i(level), i(ranks as i64)])
    })?;
    // The player's answer to the open event.
    for (name, status) in [
        ("CalendarEventAvailable", ACCEPTED),
        ("CalendarEventDecline", DECLINED),
        ("CalendarEventTentative", TENTATIVE),
    ] {
        set(lua, name, move |lua, _| {
            rsvp_open(lua, status);
            many(vec![])
        })?;
    }
    set(lua, "CalendarEventSignUp", |lua, _| {
        write(lua, |st, _| {
            if let Some((event_id, _)) = open_ids(st).filter(|(e, _)| *e != 0) {
                st.push(R::SignUp {
                    event_id,
                    tentative: false,
                });
            }
        });
        many(vec![])
    })?;

    // ── the context menu's event ──────────────────────────────────────────────────────────────
    set(lua, "CalendarContextSelectEvent", |lua, args| {
        let (o, d, n) = (
            int_arg(lua, &args, 0),
            int_arg(lua, &args, 1),
            int_arg(lua, &args, 2),
        );
        write(lua, |st, _| {
            st.context = match (o, d, n) {
                (Some(o), Some(d), Some(n)) => st.slot(o as i32, d, n),
                _ => None,
            };
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextDeselectEvent", |lua, _| {
        write(lua, |st, _| st.context = None);
        many(vec![])
    })?;
    set(lua, "CalendarContextGetEventIndex", |lua, _| {
        let v = read(lua, |st| match st.context {
            Some((y, m, d, n)) => (st.offset_of(y, m), d as i64, n as i64 + 1),
            None => (0, 0, 0),
        });
        many(vec![i(v.0), i(v.1), i(v.2)])
    })?;
    set(
        lua,
        "CalendarContextEventGetCalendarType",
        |lua, _| match read(lua, context_row) {
            Some(r) => many(vec![s(lua, r.kind.token())?]),
            None => many(vec![Value::Nil]),
        },
    )?;
    set(lua, "CalendarContextEventCanEdit", |lua, _| {
        let can = read(lua, |st| {
            context_row(st).is_some_and(|r| {
                matches!(r.kind, K::Player | K::GuildEvent | K::GuildAnnouncement)
                    && (r.creator == st.view.me || r.invite.is_some_and(|(_, rank)| rank >= 1))
            })
        });
        many(vec![Value::Boolean(can)])
    })?;
    set(lua, "CalendarContextEventCanComplain", |lua, _| {
        let can = read(lua, |st| {
            context_row(st).is_some_and(|r| {
                r.kind == K::Player
                    && r.creator != st.view.me
                    && r.invite.is_some_and(|(s, _)| s == INVITED)
            })
        });
        many(vec![Value::Boolean(can)])
    })?;
    set(lua, "CalendarContextEventComplain", |lua, _| {
        write(lua, |st, _| {
            if let Some(r) = context_row(st).filter(|r| r.event_id != 0) {
                st.push(R::Complain {
                    event_id: r.event_id,
                    guid: r.creator,
                });
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextEventClipboard", |lua, _| {
        let has = read(lua, |st| st.clipboard.is_some());
        many(vec![Value::Boolean(has)])
    })?;
    set(lua, "CalendarContextEventCopy", |lua, _| {
        write(lua, |st, _| {
            if let Some(r) = context_row(st).filter(|r| r.event_id != 0) {
                st.clipboard = Some((r.event_id, r.invite_id, r.date));
            }
        });
        many(vec![])
    })?;
    // CalendarContextEventPaste(offset, day) (`0x5bd460` → `0x5bc870`): a copy on that day at
    // the source's time.
    set(lua, "CalendarContextEventPaste", |lua, args| {
        let (Some(o), Some(d)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: Script_CalendarContextEventPaste([-1.0.1], monthDay)");
        };
        write(lua, |st, _| {
            let Some((event_id, invite_id, at)) = st.clipboard else {
                return;
            };
            let Some((y, m, day, _)) = st.slot(o as i32, d, 1) else {
                return;
            };
            st.push(R::CopyEvent {
                event_id,
                invite_id,
                date: WrathCalendarDate::new(y, m, day, at.hour, at.minute),
            });
            st.action_pending = true;
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextEventRemove", |lua, _| {
        write(lua, |st, _| {
            if let Some(r) = context_row(st).filter(|r| r.event_id != 0) {
                st.push(R::RemoveEvent {
                    event_id: r.event_id,
                    invite_id: r.invite_id,
                    flags: r.flags,
                });
                st.action_pending = true;
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextEventSignUp", |lua, _| {
        write(lua, |st, _| {
            if let Some(r) = context_row(st).filter(|r| r.event_id != 0) {
                st.push(R::SignUp {
                    event_id: r.event_id,
                    tentative: false,
                });
            }
        });
        many(vec![])
    })?;
    for (name, status) in [
        ("CalendarContextInviteAvailable", ACCEPTED),
        ("CalendarContextInviteDecline", DECLINED),
        ("CalendarContextInviteTentative", TENTATIVE),
    ] {
        set(lua, name, move |lua, _| {
            rsvp_context(lua, status);
            many(vec![])
        })?;
    }
    set(lua, "CalendarContextInviteRemove", |lua, _| {
        write(lua, |st, _| {
            if let Some(r) = context_row(st).filter(|r| r.event_id != 0) {
                let me = st.view.me;
                st.push(R::RemoveInvite {
                    event_id: r.event_id,
                    invitee: me,
                    invite_id: r.invite_id,
                    owner_invite_id: r.invite_id,
                });
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextInviteIsPending", |lua, _| {
        let p = read(lua, |st| {
            context_row(st).is_some_and(|r| r.invite.is_some_and(|(s, _)| s == INVITED))
        });
        many(vec![Value::Boolean(p)])
    })?;
    set(lua, "CalendarContextInviteStatus", |lua, _| {
        let v = read(lua, |st| {
            context_row(st).map_or(0, |r| match r.invite {
                Some((s, _)) => i64::from(s) + 1,
                None if r.kind == K::GuildEvent => LUA_NOT_SIGNEDUP,
                None => 0,
            })
        });
        many(vec![i(v)])
    })?;
    set(lua, "CalendarContextInviteType", |lua, _| {
        let v = read(lua, |st| {
            context_row(st).map_or(0, |r| {
                if r.kind == K::GuildEvent && r.invite.is_none() {
                    LUA_SIGNUP
                } else {
                    1
                }
            })
        });
        many(vec![i(v)])
    })?;
    set(lua, "CalendarContextInviteModeratorStatus", |lua, _| {
        let rank = read(lua, |st| {
            context_row(st).and_then(|r| r.invite).map_or(0, |(_, r)| r)
        });
        many(vec![s(lua, mod_status(rank))?])
    })?;
    Ok(())
}

/// A `CalendarNew*` draft: today at the current hour, type Other, never repeating.
fn new_draft(lua: &Lua, flags: u32, arena_slot: Option<u8>) {
    write(lua, |st, _| {
        let today = st.today();
        let date = WrathCalendarDate::new(today.year, today.month, today.day, today.hour, 0);
        st.open = Some(Open {
            what: Opened::Draft,
            fields: WrathCalendarFields {
                event_type: 4,
                dungeon: -1,
                date,
                lockout: date,
                flags,
                ..Default::default()
            },
            synced: None,
            changed: false,
        });
        st.selected_invite = None;
        st.push(R::NewDraft { flags, arena_slot });
    });
}
