//! The 96 build-12340 calendar globals. Addresses are the bound C functions
//! (`/tmp/azb-app-mig/ghidra/cal1.log`); shapes follow their pushes.

use mlua::{Lua, MultiValue, Value};

mod context;
mod dates;
mod sends;
mod setters;

use super::{
    Open, Opened, State, WrathCalendarDate, WrathCalendarEntry, WrathCalendarFields,
    WrathCalendarInvitee, WrathCalendarKind as K, WrathCalendarRequest as R, CRITERIA,
};
use crate::script::{Model, ScriptValue};

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
/// `CALENDAR_FLAG_ALL_ALLOWED`, a player event's flag.
const PLAYER: u32 = 0x1;
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

/// A number argument as the calendar verbs read it, `lua_tonumber` then truncated (`0x88b9c0`).
fn int_arg(lua: &Lua, args: &MultiValue, i: usize) -> Option<i64> {
    num(lua, args, i).map(|n| i64::from(crate::script::binding_abi::ftol(n)))
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

/// `0x5b7e20`: the action-pending bit, announced as `CALENDAR_ACTION_PENDING` (event `0x247`).
fn pend_if(lua: &Lua, on: bool) {
    if on {
        write(lua, |st, _| st.action_pending = true);
        fire(
            lua,
            "CALENDAR_ACTION_PENDING",
            vec![ScriptValue::Bool(true)],
        );
    }
}

/// `0x5b9960`: `CALENDAR_UPDATE_ERROR` with the key's line, then the pending bit cleared and
/// announced (`0x5b99d0`).
fn update_error(lua: &Lua, key: &str) {
    update_error_with(lua, key, "");
}

/// [`update_error`] with the line's `%s` or `%d` filled (`0x5b9960`'s argument).
fn update_error_with(lua: &Lua, key: &str, arg: &str) {
    let line = gs(lua, key);
    let line = match ["%s", "%d"].iter().find(|p| line.contains(**p)) {
        Some(p) if !arg.is_empty() => line.replacen(p, arg, 1),
        _ => line,
    };
    fire(lua, "CALENDAR_UPDATE_ERROR", vec![ScriptValue::Str(line)]);
    write(lua, |st, _| st.action_pending = false);
    fire(
        lua,
        "CALENDAR_ACTION_PENDING",
        vec![ScriptValue::Bool(false)],
    );
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> Rv + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn mod_status(rank: u8) -> &'static str {
    match rank {
        1 => "MODERATOR",
        2 => "CREATOR",
        _ => "",
    }
}

/// `0x5b8040`: a holiday day's place in a holiday of more than one day, else "".
fn sequence_type(row: &WrathCalendarEntry) -> &'static str {
    match row.sequence {
        (_, total) if row.kind != K::Holiday || total <= 1 => "",
        (0, _) => "START",
        (k, total) if k + 1 == total => "END",
        _ => "ONGOING",
    }
}

/// The 15 `CalendarGetDayEvent` values for a row (`0x5c1070`).
fn day_event(lua: &Lua, row: &WrathCalendarEntry) -> Rv {
    let raid = row.raid.as_ref();
    let guild = row.flags & GUILD_EVENT != 0;
    // `+0xa8 + 1` for every row: 0 (INVITED) where no invite set it (`0x5c125a`).
    let invite_status = match row.invite {
        Some((status, _)) => i(status + 1),
        None if guild => i(LUA_NOT_SIGNEDUP),
        None => i(1),
    };
    // `0x5b8080` and `0x5b80b0` read the invite bits, not the rank.
    let rank = if row.bits & 4 != 0 {
        2
    } else {
        row.bits >> 1 & 1
    };
    // `+0xa0 + 1`: holiday, lockout and reset rows are built with type -1 (`0x5b784d`, `0x5c2652`).
    let event_type = match row.kind {
        K::Holiday | K::RaidLockout | K::RaidReset => 0,
        _ => i64::from(row.event_type) + 1,
    };
    many(vec![
        s(lua, &row.title)?,
        i(row.date.hour),
        i(row.date.minute),
        s(lua, row.kind.token())?,
        s(lua, sequence_type(row))?,
        i(event_type),
        s(lua, &row.texture)?,
        s(lua, mod_status(rank))?,
        invite_status,
        s(lua, row.invited_by.as_deref().unwrap_or(""))?,
        i(raid.map_or(0, |r| r.difficulty + 1)),
        i(if guild && row.bits & 8 != 0 {
            LUA_SIGNUP
        } else {
            1
        }),
        i(row.sequence.0 + 1),
        i(row.sequence.1),
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

/// `0x5cbca0`: in a guild whose rank grants `GR_RIGHT_CREATE_GUILD_EVENT`.
fn guild_event_right(lua: &Lua) -> bool {
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    model.guild.in_guild && model.guild.rights & RIGHT_GUILD_EVENT != 0
}

fn context_row(st: &State) -> Option<WrathCalendarEntry> {
    st.row(st.context?)
}

fn invite_at(st: &State, index: Option<i64>) -> Option<WrathCalendarInvitee> {
    let k = usize::try_from(index? - 1).ok()?;
    st.invitees().into_iter().nth(k)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    dates::install(lua)?;

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
        // `0x5b9ca0`: the day's 0-based place `+0xc4`, the count `+0xc8` and `0x5b8040`'s type.
        match row_at(lua, &args) {
            Some(r) => many(vec![
                i(r.sequence.0),
                i(r.sequence.1),
                s(lua, sequence_type(&r))?,
            ]),
            None => many(vec![i(0), i(0), s(lua, "")?]),
        }
    })?;
    // CalendarGetFirstPendingInvite (`0x5b9e10`): the first row with the pending bit (`0x5b92b0`).
    set(lua, "CalendarGetFirstPendingInvite", |lua, args| {
        let (Some(o), Some(d)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: CalendarGetDay([-1,0,1], monthDay)");
        };
        let n = read(lua, |st| {
            st.slot(o as i32, d, 1).map_or(0, |(y, m, d, _)| {
                st.day(y, m, d)
                    .iter()
                    .position(|r| r.bits & 1 != 0)
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
        many(vec![b(guild_event_right(lua))])
    })?;

    // ── opening, selecting and reading an event ───────────────────────────────────────────────
    // CalendarOpenEvent (`0x5b9eb0` → `0x5b9310`): nothing while an action is pending; a player,
    // guild or announcement row (flags `0x543`) asks the server and sets the pending bit, its event
    // and selection arriving with `SMSG_CALENDAR_SEND_EVENT`; any other row is selected at once and
    // opens with its type. Neither replaces the open event before the reply.
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
            if st.action_pending {
                return None;
            }
            let slot = st.slot(o as i32, d, n)?;
            let row = st.row(slot)?;
            if row.flags & 0x543 != 0 {
                st.asked = Some((row.event_id, row.invite_id));
                st.push(R::GetEvent {
                    event_id: row.event_id,
                });
                return Some(None);
            }
            st.selected = Some(slot);
            Some(Some(row.kind))
        });
        match opened {
            Some(Some(kind)) => {
                let kind = ScriptValue::Str(kind.token().into());
                fire(lua, "CALENDAR_OPEN_EVENT", vec![kind]);
            }
            Some(None) => pend_if(lua, true),
            None => {}
        }
        many(vec![])
    })?;
    // CalendarCloseEvent (`0x5b84a0`) only clears the selection; the open event stays.
    set(lua, "CalendarCloseEvent", |lua, _| {
        write(lua, |st, _| st.selected = None);
        many(vec![])
    })?;
    // CalendarGetEventIndex (`0x5b9f80` → `0x5b9000`): zeros unless the row is in the shown -1..1.
    set(lua, "CalendarGetEventIndex", |lua, _| {
        let v = read(lua, |st| {
            let (y, m, d, n) = st.selected.filter(|&slot| st.row(slot).is_some())?;
            let offset = st.offset_of(y, m);
            (-1..=1).contains(&offset).then_some((offset, d, n))
        });
        let (o, d, n) = v.map_or((0, 0, 0), |(o, d, n)| (o, i64::from(d), n as i64 + 1));
        many(vec![i(o), i(d), i(n)])
    })?;
    // CalendarGetEventInfo (`0x5bd8a0`): 25 values of the open event.
    set(lua, "CalendarGetEventInfo", |lua, _| {
        let info = read(lua, |st| {
            let open = st.open.as_ref()?;
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
        let t = read(lua, |st| {
            Some(K::of_flags(st.open.as_ref()?.fields.flags).token())
        });
        match t {
            Some(t) => many(vec![s(lua, t)?]),
            None => many(vec![Value::Nil]),
        }
    })?;
    // CalendarEventCanEdit (`0x5b8a50`): an announcement needs the guild right (`0x5cbca0`).
    set(lua, "CalendarEventCanEdit", |lua, _| {
        let right = guild_event_right(lua);
        let can = read(lua, |st| match st.open.as_ref() {
            Some(o) if o.fields.flags & ANNOUNCEMENT != 0 => right,
            _ => st.can_edit(),
        });
        many(vec![Value::Boolean(can)])
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
    // The draft constructor (`0x5f2a56`) sets 0x1 for a player event, 0x40 or 0x400 for the others.
    for (name, flags) in [
        ("CalendarNewEvent", PLAYER),
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
        new_draft(lua, PLAYER, u8::try_from(slot).ok());
        many(vec![])
    })?;
    setters::install(lua)?;
    sends::install(lua)?;

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
    // set, filtered by `0x5f26f0`: an invitee in the list, statuses 1..8 but not SIGNEDUP or
    // NOT_SIGNEDUP, not ACCEPTED or DECLINED on a guild event, and not the invitee's own; anyone
    // else is refused with `CALENDAR_ERROR_PERMISSIONS`.
    set(lua, "CalendarEventGetStatusOptions", |lua, args| {
        let Some(index) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventGetStatusOptions(index)");
        };
        let (open, can, invitee) = read(lua, |st| {
            let guild = st
                .open
                .as_ref()
                .is_some_and(|o| o.fields.flags & GUILD_EVENT != 0);
            let invitee = invite_at(st, Some(index)).map(|i| (i.status, guild));
            (st.open.is_some(), st.can_edit(), invitee)
        });
        if !open {
            return many(vec![]);
        }
        if !can {
            update_error(lua, "CALENDAR_ERROR_PERMISSIONS");
            return many(vec![]);
        }
        let Some((own, guild)) = invitee else {
            return many(vec![]);
        };
        let shown = |st: &u8| {
            !matches!(*st, 6 | 7) && *st != own && !(guild && matches!(*st, ACCEPTED | DECLINED))
        };
        let mut v = Vec::new();
        for st in (1..=8).filter(shown) {
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
            // `0x5ba2c4`: a name not yet in the cache is "".
            opt(lua, &r.name)?,
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
    // CalendarEventSelectInvite(index) (`0x5b8ae0`): `lua_tointeger` (`0x5b8b0c`); an index within
    // the open event's list stores that invite's id (`0x5f1a00`).
    set(lua, "CalendarEventSelectInvite", |lua, args| {
        let Some(n) = num(lua, &args, 0) else {
            return usage("Usage: CalendarEventSelectInvite(index)");
        };
        let k = crate::script::item_arg::tointeger(n) as i32 as i64 - 1;
        write(lua, |st, _| {
            let id = usize::try_from(k)
                .ok()
                .and_then(|k| st.invitees().get(k).map(|i| i.invite_id));
            if let (Some(open), Some(id)) = (st.open.as_mut(), id) {
                open.selected_invite = id;
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarEventGetSelectedInvite", |lua, _| {
        // `0x5bbb00` → `0x5b8e00`: the first invite in the list with that id, 0 for none.
        let k = read(lua, |st| {
            let id = st.open.as_ref()?.selected_invite;
            st.invitees().iter().position(|i| i.invite_id == id)
        });
        let k = k.map_or(0, |k| k as i64 + 1);
        many(vec![i(k)])
    })?;
    // CalendarEventSortInvites("criteria", reverse) (`0x5b8570`): the name matched without case
    // (`_strnicmp`), else status; the flag defaults true (`0x815500(L, 2, 1)`). With an open event
    // `0x5f1980` makes it the criterion, flips its reverse byte when the flag is set, clears the
    // others' and fires `CALENDAR_UPDATE_INVITE_LIST` (event `0x23e`).
    set(lua, "CalendarEventSortInvites", |lua, args| {
        let Some(criterion) = text_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventSortInvites(\"criteria\", reverse)");
        };
        let flip = crate::script::binding_abi::bool_or_default(args.get(1), true);
        let c = CRITERIA
            .iter()
            .position(|c| c.eq_ignore_ascii_case(&criterion))
            .unwrap_or(3);
        let group = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let party = model.party.members.iter().map(|m| m.guid);
            party
                .chain(model.party.raid.iter().map(|m| m.guid))
                .collect()
        };
        let sorted = write(lua, |st, _| {
            let Some(open) = st.open.as_mut() else {
                return false;
            };
            let kept = open.reverse[c];
            open.reverse = [false; 6];
            open.reverse[c] = kept != flip;
            (open.criterion, open.group) = (c, group);
            true
        });
        if sorted {
            fire(lua, "CALENDAR_UPDATE_INVITE_LIST", vec![]);
        }
        many(vec![])
    })?;
    // CalendarEventGetInviteSortCriterion (`0x5b8690`): `"", false` with no open event.
    set(lua, "CalendarEventGetInviteSortCriterion", |lua, _| {
        let (c, r) = read(lua, |st| {
            let open = st.open.as_ref()?;
            Some((CRITERIA[open.criterion], open.reverse[open.criterion]))
        })
        .unwrap_or(("", false));
        many(vec![s(lua, c)?, Value::Boolean(r)])
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
    context::install(lua)
}

/// A `CalendarNew*` draft: today at the current hour, type Other, never repeating.
fn new_draft(lua: &Lua, flags: u32, arena_slot: Option<u8>) {
    write(lua, |st, _| {
        let today = st.today();
        let date = WrathCalendarDate::new(today.year, today.month, today.day, today.hour, 0);
        // The constructor's size is 100 (`0x5f108b`).
        let fields = WrathCalendarFields {
            event_type: 4,
            dungeon: -1,
            max_invites: 100,
            date,
            lockout: date,
            flags,
            ..Default::default()
        };
        st.open = Some(Open::new(Opened::Draft, fields));
        // The constructor zeroes the selection (`0x5bbede`).
        (st.selected, st.asked) = (None, None);
        st.push(R::NewDraft { flags, arena_slot });
    });
}
