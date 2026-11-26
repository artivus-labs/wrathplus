//! The calendar's dates and months: today off the game clock (`0xd37f98`), the shown month
//! (`0xacff8c`/`0xacff90`) in its offset and absolute forms, and the calendar's fixed limits.

use mlua::{Lua, Value};

use super::super::date::{abs_month, days_in_month, weekday};
use super::{gs, i, int_arg, many, read, s, set, usage, write, State, WrathCalendarDate};

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

fn date4(d: &WrathCalendarDate) -> Vec<Value> {
    vec![i(d.weekday), i(d.month), i(d.day), i(d.year)]
}

fn month_values(y: i32, m: u8) -> Vec<Value> {
    vec![i(m), i(y), i(days_in_month(y, m)), i(weekday(y, m, 1))]
}

/// The absolute month `CalendarGetAbsMonth`/`CalendarSetAbsMonth` name; the year defaults to the
/// shown one (`0x5b9b00`).
fn abs(st: &State, month: i64, year: Option<i64>) -> (i32, u8) {
    let year = year.unwrap_or(i64::from(st.shown().0));
    abs_month(st.today().year, month, year)
}

/// `CalendarGetMaxCreateDate`'s day (`0x5b7a80`): the first of the month thirteen months on, less a
/// day, so the last day of this month next year; a year past 2031 is held there, weekday kept.
fn max_create(t: WrathCalendarDate) -> WrathCalendarDate {
    let m0 = i32::from(t.month) - 1 + 13;
    let first = WrathCalendarDate::new(t.year + m0 / 12, (m0 % 12) as u8 + 1, 1, 23, 59);
    let last = WrathCalendarDate::from_seconds(first.seconds() - 86_400);
    WrathCalendarDate {
        year: last.year.min(2031),
        ..last
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // CalendarGetDate (`0x5b8160`): weekday, month, day, year of the game clock.
    set(lua, "CalendarGetDate", |lua, _| {
        let t = read(lua, State::today);
        many(date4(&t))
    })?;
    // CalendarGetMonth([offset]) (`0x5b99f0`): month, year, days, weekday of the 1st.
    set(lua, "CalendarGetMonth", |lua, args| {
        let offset = int_arg(lua, &args, 0).unwrap_or(0) as i32;
        let (y, m) = read(lua, |st| st.month(offset));
        many(month_values(y, m))
    })?;
    // CalendarGetAbsMonth(month[, year]) (`0x5b9ac0`).
    set(lua, "CalendarGetAbsMonth", |lua, args| {
        let Some(m) = int_arg(lua, &args, 0) else {
            return usage("Usage: CalendarGetAbsMonth(month[, year])");
        };
        let y = int_arg(lua, &args, 1);
        let (y, m) = read(lua, |st| abs(st, m, y));
        many(month_values(y, m))
    })?;
    // CalendarSetMonth(offset) (`0x5bcc00`) and CalendarSetAbsMonth(month[, year]) (`0x5c43b0`)
    // store the shown month; neither fires an event.
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
        write(lua, |st, _| st.shown = Some(abs(st, m, y)));
        many(vec![])
    })?;
    // CalendarGetMinDate (`0x5b81f0`): 24 November 2004.
    set(lua, "CalendarGetMinDate", |_, _| {
        many(date4(&WrathCalendarDate::new(2004, 11, 24, 0, 0)))
    })?;
    // CalendarGetMinHistoryDate (`0x5b8350`): today less 14 days (`0x76c280(-14, 0)`).
    set(lua, "CalendarGetMinHistoryDate", |lua, _| {
        let t = read(lua, State::today);
        let midnight = WrathCalendarDate::new(t.year, t.month, t.day, 0, 0).seconds();
        many(date4(&WrathCalendarDate::from_seconds(
            midnight - 14 * 86_400,
        )))
    })?;
    set(lua, "CalendarGetMaxCreateDate", |lua, _| {
        let t = read(lua, State::today);
        many(date4(&max_create(t)))
    })?;
    // CalendarGetMaxDate (`0x5b82a0`): 1 January 2031 less a day, 23:59.
    set(lua, "CalendarGetMaxDate", |_, _| {
        many(date4(&WrathCalendarDate::new(2030, 12, 31, 23, 59)))
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
    })
}
