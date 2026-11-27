//! The day list's context menu: each verb names its row by `([-1,0,1], monthDay, index)` and only
//! without those falls back to the context event `CalendarContextSelectEvent` picked (`0xc207e8`);
//! with neither it raises its usage line (`0x5baf7a` and its siblings).

use mlua::{Lua, MultiValue, Value};

use super::sends::creatable;
use super::{
    context_row, guild_event_right, has3, i, int_arg, many, pend_if, read, row_at, s, set,
    update_error, usage, write, WrathCalendarDate, WrathCalendarEntry, ACCEPTED, DECLINED,
    GUILD_EVENT, LUA_NOT_SIGNEDUP, LUA_SIGNUP, R, TENTATIVE,
};

/// The row the arguments name (a miss is `None`), else the context event, else the usage line.
fn target(
    lua: &Lua,
    args: &MultiValue,
    line: &'static str,
) -> mlua::Result<Option<WrathCalendarEntry>> {
    if has3(lua, args) {
        return Ok(row_at(lua, args));
    }
    match read(lua, context_row) {
        Some(row) => Ok(Some(row)),
        None => Err(mlua::Error::runtime(line)),
    }
}

/// A guild event the row is a sign-up for (`0x5b80b0`).
fn signup(r: &WrathCalendarEntry) -> bool {
    r.flags & GUILD_EVENT != 0 && r.bits & 8 != 0
}

/// The copy and remove gate (`0x5bd2d2`): the guild right for a guild event or announcement,
/// else the moderator or creator bits.
fn may_edit(r: &WrathCalendarEntry, right: bool) -> bool {
    if r.flags & 0x440 != 0 {
        right
    } else {
        r.bits & 6 != 0
    }
}

/// `0x5bc9d0`: a player event (no system, holiday, lockout, reset, announcement or guild flag,
/// `0x6cc`) someone else created. Labelled: the complaint system's switch (`[0xbceffc]`) and the
/// creator's place on the contact list (`0x6b3920`) are not held here.
fn may_complain(r: &WrathCalendarEntry, me: u64) -> bool {
    r.flags & 0x6cc == 0 && r.creator != me
}

/// A context request on a row with an event id, nothing while pending, then the pending bit
/// (`0x5b991e`, `0x5bac99`, `0x5bae49`, `0x5bc5a0`, `0x5bc697`, `0x5bc787`).
fn send(
    lua: &Lua,
    row: Option<WrathCalendarEntry>,
    make: impl FnOnce(&WrathCalendarEntry, u64) -> Option<R>,
) {
    let sent = write(lua, |st, _| {
        let Some(request) = row
            .filter(|r| r.event_id != 0 && !st.action_pending)
            .and_then(|r| make(&r, st.view.me))
        else {
            return false;
        };
        st.push(request);
        true
    });
    pend_if(lua, sent);
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // CalendarContextSelectEvent (`0x5ba5b0` → `0x5b97d0`): a row that exists becomes the context
    // event; any other keeps the last.
    set(lua, "CalendarContextSelectEvent", |lua, args| {
        if !has3(lua, &args) {
            return usage("Usage: CalendarContextSelectEvent([-1.0.1], monthDay, index)");
        }
        let (o, d, n) = (
            int_arg(lua, &args, 0).unwrap_or(0),
            int_arg(lua, &args, 1).unwrap_or(0),
            int_arg(lua, &args, 2).unwrap_or(0),
        );
        write(lua, |st, _| {
            if let Some(slot) = st
                .slot(o as i32, d, n)
                .filter(|&slot| st.row(slot).is_some())
            {
                st.context = Some(slot);
            }
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextDeselectEvent", |lua, _| {
        write(lua, |st, _| st.context = None);
        many(vec![])
    })?;
    // CalendarContextGetEventIndex (`0x5ba680` → `0x5b9000`): zeros outside the shown -1..1.
    set(lua, "CalendarContextGetEventIndex", |lua, _| {
        let v = read(lua, |st| {
            let (y, m, d, n) = st.context.filter(|&slot| st.row(slot).is_some())?;
            let offset = st.offset_of(y, m);
            (-1..=1).contains(&offset).then_some((offset, d, n))
        });
        let (o, d, n) = v.map_or((0, 0, 0), |(o, d, n)| (o, i64::from(d), n as i64 + 1));
        many(vec![i(o), i(d), i(n)])
    })?;
    // CalendarContextEventGetCalendarType (`0x5bafc0`): `0x5b7fd0`'s type, nil for a miss.
    set(lua, "CalendarContextEventGetCalendarType", |lua, args| {
        let line = "Usage: CalendarContextEventGetCalendarType([-1.0.1], monthDay, index)";
        match target(lua, &args, line)? {
            Some(r) => many(vec![s(lua, r.kind.token())?]),
            None => many(vec![Value::Nil]),
        }
    })?;
    // CalendarContextEventCanEdit (`0x5baeb0`).
    set(lua, "CalendarContextEventCanEdit", |lua, args| {
        let line = "Usage: CalendarContextEventCanEdit([-1.0.1], monthDay, index)";
        let right = guild_event_right(lua);
        let can = target(lua, &args, line)?.is_some_and(|r| may_edit(&r, right));
        many(vec![Value::Boolean(can)])
    })?;
    // CalendarContextEventCanComplain (`0x5bd4f0`): never while an action is pending.
    set(lua, "CalendarContextEventCanComplain", |lua, args| {
        let line = "Usage: CalendarContextEventCanComplain([-1.0.1], monthDay, index)";
        let row = target(lua, &args, line)?;
        let can = read(lua, |st| {
            !st.action_pending && row.is_some_and(|r| may_complain(&r, st.view.me))
        });
        many(vec![Value::Boolean(can)])
    })?;
    // CalendarContextEventComplain (`0x5bd600` → `0x5bca50`). Labelled: 12340 also ignores the
    // creator (`0x6b3750`) or, already ignoring, removes the invite; here only the complaint goes.
    set(lua, "CalendarContextEventComplain", |lua, args| {
        let line = "Usage: CalendarContextEventComplain([-1.0.1], monthDay, index)";
        let row = target(lua, &args, line)?;
        send(lua, row, |r, me| {
            may_complain(r, me).then_some(R::Complain {
                event_id: r.event_id,
                guid: r.creator,
            })
        });
        many(vec![])
    })?;
    set(lua, "CalendarContextEventClipboard", |lua, _| {
        let has = read(lua, |st| st.clipboard.is_some());
        many(vec![Value::Boolean(has)])
    })?;
    // CalendarContextEventCopy (`0x5bd340` → `0x5bc7c0`): refused, it empties the clipboard.
    set(lua, "CalendarContextEventCopy", |lua, args| {
        let line = "Usage: Script_CalendarContextEventCopy([-1.0.1], monthDay, index)";
        let right = guild_event_right(lua);
        let Some(r) = target(lua, &args, line)? else {
            return many(vec![]);
        };
        let copied = may_edit(&r, right);
        write(lua, |st, _| {
            st.clipboard = copied.then_some((r.event_id, r.invite_id, r.date));
        });
        if !copied {
            update_error(lua, "CALENDAR_ERROR_PERMISSIONS");
        }
        many(vec![])
    })?;
    // CalendarContextEventPaste(offset, day) (`0x5bd460` → `0x5bc870`): nothing while pending or
    // with an empty clipboard; a player pastes once in 5 s of the last add (`0x5bc8be`); a
    // creatable day (`0x5b7c00`) gets a copy at the source's time.
    set(lua, "CalendarContextEventPaste", |lua, args| {
        let (Some(o), Some(d)) = (int_arg(lua, &args, 0), int_arg(lua, &args, 1)) else {
            return usage("Usage: Script_CalendarContextEventPaste([-1.0.1], monthDay)");
        };
        let outcome = write(lua, |st, now| {
            let (event_id, invite_id, at) = st.clipboard.filter(|_| !st.action_pending)?;
            if !st.view.gm && st.last_add.is_some_and(|t| now - t < 5.0) {
                return Some(false);
            }
            let (y, m, day, _) = st.slot(o as i32, d, 1)?;
            let date = WrathCalendarDate::new(y, m, day, at.hour, at.minute);
            if !creatable(st, date) {
                return None;
            }
            st.push(R::CopyEvent {
                event_id,
                invite_id,
                date,
            });
            st.last_add = Some(now);
            Some(true)
        });
        match outcome {
            Some(false) => update_error(lua, "CALENDAR_ERROR_EVENT_THROTTLED"),
            sent => pend_if(lua, sent.is_some()),
        }
        many(vec![])
    })?;
    // CalendarContextEventRemove (`0x5bd220` → `0x5bc6d0`): the copy gate, else
    // CALENDAR_ERROR_PERMISSIONS.
    set(lua, "CalendarContextEventRemove", |lua, args| {
        let line = "Usage: Script_CalendarContextEventRemove([-1.0.1], monthDay, index)";
        let right = guild_event_right(lua);
        let Some(r) = target(lua, &args, line)? else {
            return many(vec![]);
        };
        if !may_edit(&r, right) {
            update_error(lua, "CALENDAR_ERROR_PERMISSIONS");
            return many(vec![]);
        }
        send(lua, Some(r), |r, _| {
            Some(R::RemoveEvent {
                event_id: r.event_id,
                invite_id: r.invite_id,
                flags: r.flags,
            })
        });
        many(vec![])
    })?;
    // CalendarContextEventSignUp (`0x5bd130` → `0x5bc5e0(0)`): a guild event not yet answered
    // (status 7, `0x5bc60d`).
    set(lua, "CalendarContextEventSignUp", |lua, args| {
        let line = "Usage: CalendarContextEventSignUp([-1.0.1], monthDay, index)";
        let row = target(lua, &args, line)?;
        send(lua, row, |r, _| {
            (r.flags & GUILD_EVENT != 0)
                .then(|| sign_up(r, false))
                .flatten()
        });
        many(vec![])
    })?;
    // CalendarContextInviteAvailable / Decline (`0x5bab50`, `0x5bad00`): any row but a guild
    // event's sign-up, with the row's invite.
    for (name, status, line) in [
        (
            "CalendarContextInviteAvailable",
            ACCEPTED,
            "Usage: CalendarContextInviteAvailable([-1.0.1], monthDay, index)",
        ),
        (
            "CalendarContextInviteDecline",
            DECLINED,
            "Usage: CalendarContextInviteDecline([-1.0.1], monthDay, index)",
        ),
    ] {
        set(lua, name, move |lua, args| {
            let row = target(lua, &args, line)?;
            send(lua, row, |r, _| answer(r, status));
            many(vec![])
        })?;
    }
    // CalendarContextInviteTentative (`0x5bcf30`): a guild event's sign-up signs up tentatively
    // (`0x5bc5e0(1)`), any other row answers tentative (`0x5b9860(8)`).
    set(lua, "CalendarContextInviteTentative", |lua, args| {
        let line = "Usage: Script_CalendarContextInviteTentative([-1.0.1], monthDay, index)";
        let row = target(lua, &args, line)?;
        send(lua, row, |r, _| {
            if signup(r) {
                sign_up(r, true)
            } else {
                answer(r, TENTATIVE)
            }
        });
        many(vec![])
    })?;
    // CalendarContextInviteRemove (`0x5bd040` → `0x5bc4d0`): the player's own invite to a player
    // or guild event (flags `0x401`).
    set(lua, "CalendarContextInviteRemove", |lua, args| {
        let line = "Usage: CalendarContextInviteRemove([-1.0.1], monthDay, index)";
        let row = target(lua, &args, line)?;
        send(lua, row, |r, me| {
            (r.flags & 0x401 != 0).then_some(R::RemoveInvite {
                event_id: r.event_id,
                invitee: me,
                invite_id: r.invite_id,
                owner_invite_id: r.invite_id,
            })
        });
        many(vec![])
    })?;
    // CalendarContextInviteIsPending (`0x5ba700`): the pending bit. Deviation: 12340 answers
    // false from the context event (`0x5ba7c0` jumps past it); here it reads it, as CanEdit does.
    set(lua, "CalendarContextInviteIsPending", |lua, args| {
        let line = "Usage: CalendarContextInviteIsPending([-1.0.1], monthDay, index)";
        let p = target(lua, &args, line)?.is_some_and(|r| r.bits & 1 != 0);
        many(vec![Value::Boolean(p)])
    })?;
    // CalendarContextInviteStatus (`0x5ba910`): `+0xa8 + 1`, 0 for a miss.
    set(lua, "CalendarContextInviteStatus", |lua, args| {
        let line = "Usage: CalendarContextInviteStatus([-1.0.1], monthDay, index)";
        let v = target(lua, &args, line)?.map_or(0, |r| match r.invite {
            Some((s, _)) => i64::from(s) + 1,
            None if r.flags & GUILD_EVENT != 0 => LUA_NOT_SIGNEDUP,
            None => 1,
        });
        many(vec![i(v)])
    })?;
    // CalendarContextInviteType (`0x5baa20`): SIGNUP for a guild event's sign-up, else 1; 0 for a
    // miss.
    set(lua, "CalendarContextInviteType", |lua, args| {
        let line = "Usage: CalendarContextInviteType([-1.0.1], monthDay, index)";
        let v = target(lua, &args, line)?.map_or(0, |r| if signup(&r) { LUA_SIGNUP } else { 1 });
        many(vec![i(v)])
    })?;
    // CalendarContextInviteModeratorStatus (`0x5ba7f0`): CREATOR or MODERATOR by the bits, else
    // nil. Deviation as for IsPending: 12340 answers nil from the context event.
    set(lua, "CalendarContextInviteModeratorStatus", |lua, args| {
        let line = "Usage: CalendarContextInviteModeratorStatus([-1.0.1], monthDay, index)";
        let bits = target(lua, &args, line)?.map_or(0, |r| r.bits);
        if bits & 4 != 0 {
            many(vec![s(lua, "CREATOR")?])
        } else if bits & 2 != 0 {
            many(vec![s(lua, "MODERATOR")?])
        } else {
            many(vec![Value::Nil])
        }
    })?;
    Ok(())
}

/// `0x5b9860`: an answer for any row but a guild event's sign-up.
fn answer(r: &WrathCalendarEntry, status: u8) -> Option<R> {
    (!signup(r)).then_some(R::Rsvp {
        event_id: r.event_id,
        invite_id: r.invite_id,
        status,
    })
}

/// `0x5bc5e0`: a sign-up, on a guild event only while unanswered (`+0xa8` 7).
fn sign_up(r: &WrathCalendarEntry, tentative: bool) -> Option<R> {
    let open = r.flags & GUILD_EVENT == 0 || r.invite.is_none();
    open.then_some(R::SignUp {
        event_id: r.event_id,
        tentative,
    })
}
