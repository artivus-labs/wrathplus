//! The open event's setters (`0x5bb210`..`0x5bb930`): each needs the edit right (`0x5b7d20`: the
//! guild right for an announcement, else the creator or a moderator) and marks the settings
//! changed (`+0x501`) only when a value really changes.

use mlua::{Lua, MultiValue};

use super::sends::edit_right;
use super::{
    guild_event_right, int_arg, many, read, set, text_arg, update_error, usage, write, Rv,
    WrathCalendarDate, WrathCalendarFields, ANNOUNCEMENT, AUTO_APPROVE, GUILD_EVENT, LOCKED,
};

/// Apply `change` to the open event under the edit right; `false` when refused.
fn edit(lua: &Lua, change: impl FnOnce(&mut WrathCalendarFields)) -> bool {
    let right = guild_event_right(lua);
    write(lua, |st, _| {
        if !edit_right(st, right) {
            return false;
        }
        let Some(open) = st.open.as_mut() else {
            return false;
        };
        let before = open.fields.clone();
        change(&mut open.fields);
        open.changed |= open.fields != before;
        true
    })
}

/// `0x7e1830(src, dst, chars, bytes, …)`: at most `chars - 1` code points in `bytes - 1` bytes
/// (`0x7e1890` cuts at the code point that reaches `chars`).
fn cap(text: &str, chars: usize, bytes: usize) -> String {
    let mut out = String::new();
    for c in text.chars().take(chars - 1) {
        if out.len() + c.len_utf8() > bytes - 1 {
            break;
        }
        out.push(c);
    }
    out
}

/// A date or time setter's arguments, or its usage line.
fn numbers<const N: usize>(
    lua: &Lua,
    args: &MultiValue,
    line: &'static str,
) -> Result<[i64; N], Rv> {
    let mut out = [0; N];
    for (k, slot) in out.iter_mut().enumerate() {
        *slot = int_arg(lua, args, k).ok_or_else(|| usage(line))?;
    }
    Ok(out)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // CalendarEventSetTitle / SetDescription (`0x5bb210`, `0x5bb280` → `0x5f1130`, `0x5f1180`): a
    // string only; a text unlike the stored one (`0x76e760` on the raw argument, so an over-long
    // one always differs) goes through `0x7e1830` with 32 code points and 128 bytes for the title
    // (`0x5f1159`), 256 and 1024 for the description, and marks the settings changed. Labelled:
    // the title's flag 1 (`0x7e1640`) is not modelled.
    for (name, chars, bytes, title) in [
        ("CalendarEventSetTitle", 0x20, 0x80, true),
        ("CalendarEventSetDescription", 0x100, 0x400, false),
    ] {
        set(lua, name, move |lua, args| {
            if let Some(t) = text_arg(lua, &args, 0) {
                let right = guild_event_right(lua);
                write(lua, |st, _| {
                    if !edit_right(st, right) {
                        return;
                    }
                    let Some(open) = st.open.as_mut() else {
                        return;
                    };
                    let f = &mut open.fields;
                    let field = if title {
                        &mut f.title
                    } else {
                        &mut f.description
                    };
                    if *field != t {
                        *field = cap(&t, chars, bytes);
                        open.changed = true;
                    }
                });
            }
            many(vec![])
        })?;
    }
    // CalendarEventSetType (`0x5bb2f0` → `0x5f11d0`): types 1..5; the dungeon is left alone.
    set(lua, "CalendarEventSetType", |lua, args| {
        if let Some(t) = int_arg(lua, &args, 0).filter(|t| (1..=5).contains(t)) {
            edit(lua, |f| f.event_type = (t - 1) as u8);
        }
        many(vec![])
    })?;
    // CalendarEventSetRepeatOption (`0x5bb380` → `0x5f1200`): options 1..4.
    set(lua, "CalendarEventSetRepeatOption", |lua, args| {
        if let Some(r) = int_arg(lua, &args, 0).filter(|r| (1..=4).contains(r)) {
            edit(lua, |f| f.repeat = (r - 1) as u8);
        }
        many(vec![])
    })?;
    // CalendarEventSetSize (`0x5bb410` → `0x5f1230`): above 1, and not on a guild event or
    // announcement (`0x5f1233`).
    set(lua, "CalendarEventSetSize", |lua, args| {
        if let Some(n) = int_arg(lua, &args, 0).map(|n| n as u32).filter(|&n| n > 1) {
            edit(lua, |f| {
                if f.flags & (ANNOUNCEMENT | GUILD_EVENT) == 0 {
                    f.max_invites = n;
                }
            });
        }
        many(vec![])
    })?;
    // CalendarEventSetTextureID(textureIndex) (`0x5bb800` → `0x5b9630`): a place in the event
    // type's texture list.
    set(lua, "CalendarEventSetTextureID", |lua, args| {
        let Some(index) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarEventSetTextureID(textureIndex)");
        };
        let dungeons = read(lua, |st| st.view.textures.clone());
        edit(lua, |f| {
            let list = match f.event_type {
                0 => &dungeons[0],
                1 => &dungeons[1],
                _ => return,
            };
            if let Some(t) = usize::try_from(index - 1).ok().and_then(|k| list.get(k)) {
                f.dungeon = t.dungeon;
            }
        });
        many(vec![])
    })?;
    // CalendarEventSetDate / SetLockoutDate(month, day, year) (`0x5bb4a0`, `0x5bb650`): a month
    // 1..12, day 1..32 and year 2000..2031 (`0x76c480`), else, or without the edit right,
    // CALENDAR_ERROR_INVALID_DATE.
    for (name, lockout, line) in [
        (
            "CalendarEventSetDate",
            false,
            "Usage: CalendarEventSetDate(month, day, year)",
        ),
        (
            "CalendarEventSetLockoutDate",
            true,
            "Usage: CalendarEventSetLockoutDate(month, day, year)",
        ),
    ] {
        set(lua, name, move |lua, args| {
            let [m, d, y] = match numbers::<3>(lua, &args, line) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let valid =
                (1..=12).contains(&m) && (1..=32).contains(&d) && (2000..=2031).contains(&y);
            let set = valid
                && edit(lua, |f| {
                    let at = if lockout { &mut f.lockout } else { &mut f.date };
                    *at = WrathCalendarDate::new(y as i32, m as u8, d as u8, at.hour, at.minute);
                });
            if !set {
                update_error(lua, "CALENDAR_ERROR_INVALID_DATE");
            }
            many(vec![])
        })?;
    }
    // CalendarEventSetTime / SetLockoutTime(hour, minute) (`0x5bb5a0`, `0x5bb750`): an hour below 24
    // and minute below 60 (`0x76c3c0`), else CALENDAR_ERROR_INVALID_TIME. The lockout setter's
    // usage line is spelled as 12340 spells it (`0xa18dac`).
    for (name, lockout, line) in [
        (
            "CalendarEventSetTime",
            false,
            "Usage: CalendarEventSetTime(hour, minute)",
        ),
        (
            "CalendarEventSetLockoutTime",
            true,
            "Usage: CalendarEvenSettLockoutTime(hour, minute)",
        ),
    ] {
        set(lua, name, move |lua, args| {
            let [h, m] = match numbers::<2>(lua, &args, line) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let set = (0..24).contains(&h)
                && (0..60).contains(&m)
                && edit(lua, |f| {
                    let at = if lockout { &mut f.lockout } else { &mut f.date };
                    (at.hour, at.minute) = (h as u8, m as u8);
                });
            if !set {
                update_error(lua, "CALENDAR_ERROR_INVALID_TIME");
            }
            many(vec![])
        })?;
    }
    // The lock and auto-approve flags (`0x5f13b0` sets, `0x5f13e0` clears, each only on a change).
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
    Ok(())
}
