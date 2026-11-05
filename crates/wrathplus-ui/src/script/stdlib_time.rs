//! `time()` and `date()`, the stdlib layer's clock globals.

use mlua::Lua;

/// `time([t])` and `date([format [, when]])`: build 12340's engine globals (`0x817650`,
/// `0x817450`), Lua's `os.time`/`os.date` hoisted. `time()` is wall-clock epoch seconds, as addons
/// persist it across sessions.
///
/// Deviation: both read UTC where the reference uses local time, because the tree has no timezone
/// database; so `time(date("*t", t))` is `t` again.
pub(super) fn install_time(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "time",
        lua.create_function(|lua, args: mlua::MultiValue| {
            let t = match args.front() {
                None | Some(mlua::Value::Nil) => return Ok(Some(crate::civil::unix_seconds())),
                Some(mlua::Value::Table(t)) => t,
                Some(v) => {
                    return Err(mlua::Error::runtime(format!(
                        "bad argument #1 to 'time' (table expected, got {})",
                        super::strings::typename(v)
                    )))
                }
            };
            // `getfield` in `os.time`'s order, sec, min, hour, day, month, year: an `int` by
            // `lua_tointeger`'s `fistp` (`0x84e0a4`), else the default, and day, month and year
            // have none (`0xa445d8`).
            let field = |key: &str, default: Option<i64>| -> mlua::Result<i64> {
                match lua.coerce_number(t.get::<mlua::Value>(key)?)? {
                    Some(n) => Ok(fistp_i32(n).into()),
                    None => default.ok_or_else(|| {
                        mlua::Error::runtime(format!("field '{key}' missing in date table"))
                    }),
                }
            };
            let (sec, min, hour) = (
                field("sec", Some(0))?,
                field("min", Some(0))?,
                field("hour", Some(12))?,
            );
            let (day, month, year) = (
                field("day", None)?,
                field("month", None)?,
                field("year", None)?,
            );
            // `mktime` (`0x88e39c`) carries out-of-range fields over and fails outside its 32-bit
            // `time_t` (`0x817880` loads the answer as a dword), so `time` answers nil there. From
            // `i32` fields none of this `i64` arithmetic can overflow.
            let months = year * 12 + month - 1;
            let days = crate::civil::days_from_civil(
                months.div_euclid(12),
                months.rem_euclid(12) as u32 + 1,
                1,
            ) + day
                - 1;
            let secs = days * 86_400 + hour * 3600 + min * 60 + sec;
            Ok((0..=i64::from(i32::MAX)).contains(&secs).then_some(secs))
        })?,
    )?;
    g.set(
        "date",
        lua.create_function(|lua, (fmt, when): (Option<String>, Option<i64>)| {
            // Bare `date()` is `%c`, as in Lua.
            let fmt = fmt.unwrap_or_else(|| "%c".to_string());
            let secs = when.unwrap_or_else(crate::civil::unix_seconds);
            // A leading `!` selects UTC, which every time here already is, so it is only consumed.
            let body = fmt.strip_prefix('!').unwrap_or(&fmt);
            // `*t` answers a table, as in Lua: an addon that loops until its `wday` matches
            // (`Accountant_WeekStart`) spins forever on anything else.
            if body == "*t" {
                let c = crate::civil::from_unix(secs);
                let t = lua.create_table()?;
                t.set("year", c.year)?;
                t.set("month", c.month)?;
                t.set("day", c.day)?;
                t.set("hour", c.hour)?;
                t.set("min", c.min)?;
                t.set("sec", c.sec)?;
                // Lua counts weekdays 1..7 from Sunday; ours is the 0-based index into `DAYS`.
                t.set("wday", c.weekday + 1)?;
                // Already 1-based, as in Lua.
                t.set("yday", c.yearday)?;
                // UTC has no daylight saving.
                t.set("isdst", false)?;
                return Ok(mlua::Value::Table(t));
            }
            Ok(mlua::Value::String(
                lua.create_string(format_epoch(secs, body))?,
            ))
        })?,
    )?;
    Ok(())
}

/// x87 `fistp` to a dword under the default rounding: nearest, ties to even, and the integer
/// indefinite `i32::MIN` for a NaN or a value out of range.
fn fistp_i32(n: f64) -> i32 {
    let r = n.round_ties_even();
    if (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&r) {
        r as i32
    } else {
        i32::MIN
    }
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The `strftime` subset addons use; an unknown specifier is emitted verbatim.
fn format_epoch(secs: i64, fmt: &str) -> String {
    let crate::civil::Civil {
        year,
        month,
        day,
        hour,
        min,
        sec,
        weekday: wday,
        yearday: yday,
    } = crate::civil::from_unix(secs);
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        let hour12 = match hour % 12 {
            0 => 12,
            h => h,
        };
        match spec {
            'a' => out.push_str(&DAYS[wday as usize][..3]),
            'A' => out.push_str(DAYS[wday as usize]),
            'b' | 'h' => out.push_str(&MONTHS[(month - 1) as usize][..3]),
            'B' => out.push_str(MONTHS[(month - 1) as usize]),
            'c' => out.push_str(&format!(
                "{} {} {:2} {:02}:{:02}:{:02} {}",
                &DAYS[wday as usize][..3],
                &MONTHS[(month - 1) as usize][..3],
                day,
                hour,
                min,
                sec,
                year
            )),
            'd' => out.push_str(&format!("{day:02}")),
            'H' => out.push_str(&format!("{hour:02}")),
            'I' => out.push_str(&format!("{hour12:02}")),
            'j' => out.push_str(&format!("{yday:03}")),
            'm' => out.push_str(&format!("{month:02}")),
            'M' => out.push_str(&format!("{min:02}")),
            'p' => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            'S' => out.push_str(&format!("{sec:02}")),
            'w' => out.push_str(&format!("{wday}")),
            'x' => out.push_str(&format!("{month:02}/{day:02}/{:02}", year.rem_euclid(100))),
            'X' => out.push_str(&format!("{hour:02}:{min:02}:{sec:02}")),
            'y' => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            'Y' => out.push_str(&format!("{year}")),
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod date_table_tests {
    use crate::script::UiScript;

    /// `Accountant.lua:364-375` steps back a day until `date("*t", ct).wday` matches, which a
    /// string return never does.
    #[test]
    fn date_star_t_answers_a_table_that_walks_with_its_argument() {
        let s = UiScript::new().unwrap();
        // A Wednesday at 00:00 UTC: wday 4, counting from Sunday = 1.
        let t: i64 = 1_786_492_800;
        assert_eq!(
            s.eval::<(i64, i64, i64, i64, bool)>(&format!(
                "local d = date(\"*t\", {t}) return d.year, d.month, d.day, d.wday, d.isdst"
            ))
            .unwrap(),
            (2026, 8, 12, 4, false)
        );
        assert_eq!(
            s.eval::<(i64, i64, i64, i64)>(&format!(
                "local d = date(\"*t\", {t} + 3661) return d.hour, d.min, d.sec, d.yday"
            ))
            .unwrap(),
            (1, 1, 1, 224)
        );

        // Accountant's loop: one step back from Wednesday reaches Tuesday.
        assert_eq!(
            s.eval::<i64>(&format!(
                "local ct = {t} local n = 0 \
                 while date(\"*t\", ct).wday ~= 3 and n < 100 do ct = ct - 86400 n = n + 1 end \
                 return n"
            ))
            .unwrap(),
            1,
            "Wednesday(4) back to Tuesday(3) is exactly one step; a constant wday would spin"
        );

        // `!` is consumed, in the table form and the string form.
        assert_eq!(
            s.eval::<i64>(&format!("return date(\"!*t\", {t}).wday"))
                .unwrap(),
            4
        );
        assert!(
            !s.eval::<String>(&format!("return date(\"!%A\", {t})"))
                .unwrap()
                .contains('!'),
            "the UTC flag is consumed, not printed"
        );
        assert_eq!(
            s.eval::<String>(&format!("return date(\"%A\", {t})"))
                .unwrap(),
            "Wednesday"
        );
    }
}
