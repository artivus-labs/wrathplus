//! The calendar's Gregorian arithmetic and the wire's packed time (AzerothCore
//! `secsToTimeBitFields`: minute 6 bits, hour 5, weekday 3, day−1 6, month−1 4, year−2000 5).

/// A server-local date and time; `weekday` 1 is Sunday, as `CalendarGetDate` answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct WrathCalendarDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub weekday: u8,
}

/// Days since 1970-01-01 of a civil date (Hinnant's `days_from_civil`).
pub(super) fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// 1 Sunday … 7 Saturday.
pub(super) fn weekday(y: i32, m: u8, d: u8) -> u8 {
    ((days_from_civil(y, m, d) + 4).rem_euclid(7) + 1) as u8
}

pub(super) fn days_in_month(y: i32, m: u8) -> u8 {
    let (ny, nm) = add_months(y, m, 1);
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u8
}

pub(super) fn add_months(y: i32, m: u8, offset: i32) -> (i32, u8) {
    let n = y * 12 + i32::from(m) - 1 + offset;
    (n.div_euclid(12), (n.rem_euclid(12) + 1) as u8)
}

/// `0x5b8ec0`: `offset` months on with C division and remainder, the year then held to 2030
/// (keeping its month) and, at 2004 or before, to November 2004 or later.
pub(super) fn offset_month(y: i32, m: u8, offset: i32) -> (i32, u8) {
    let (mut y, mut m) = (y - 2000 + offset / 12, i32::from(m) - 1 + offset % 12);
    if m >= 12 {
        (y, m) = (y + 1, m - 12);
    } else if m < 0 {
        (y, m) = (y - 1, m + 12);
    }
    if y >= 30 {
        y = 30;
    } else if y <= 4 {
        (y, m) = (4, m.max(10));
    }
    (2000 + y, (m + 1) as u8)
}

/// `0x5b8f70`: month `|(month - 1) % 12|`, the year held to today's ±5, then to 2004..2030.
pub(super) fn abs_month(today_year: i32, month: i64, year: i64) -> (i32, u8) {
    let t = i64::from(today_year);
    let y = year.clamp(t - 5, t + 5).clamp(2004, 2030);
    (y as i32, ((month - 1) % 12).unsigned_abs() as u8 + 1)
}

pub(super) fn to_seconds(y: i32, m: u8, d: u8, hour: u8, minute: u8) -> i64 {
    days_from_civil(y, m, d) * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60
}

impl WrathCalendarDate {
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            weekday: weekday(year, month, day),
        }
    }

    /// From seconds since 1970 in the same zone.
    pub fn from_seconds(t: i64) -> Self {
        let (y, m, d) = civil_from_days(t.div_euclid(86_400));
        let s = t.rem_euclid(86_400);
        Self::new(y, m, d, (s / 3600) as u8, (s % 3600 / 60) as u8)
    }

    pub fn seconds(&self) -> i64 {
        to_seconds(self.year, self.month, self.day, self.hour, self.minute)
    }

    /// The wire's packed time; `None` for the zero word.
    pub fn from_packed(p: u32) -> Option<Self> {
        if p == 0 {
            return None;
        }
        let minute = (p & 0x3f) as u8;
        let hour = (p >> 6 & 0x1f) as u8;
        let day = (p >> 14 & 0x3f) as u8 + 1;
        let month = (p >> 20 & 0xf) as u8 + 1;
        let year = 2000 + (p >> 24 & 0x1f) as i32;
        Some(Self::new(year, month, day, hour, minute))
    }

    pub fn packed(&self) -> u32 {
        let year = (self.year - 2000).clamp(0, 31) as u32;
        year << 24
            | u32::from(self.month.saturating_sub(1)) << 20
            | u32::from(self.day.saturating_sub(1)) << 14
            | u32::from(self.weekday.saturating_sub(1)) << 11
            | u32::from(self.hour) << 6
            | u32::from(self.minute)
    }

    /// The host's clock as UTC, before the first game clock push.
    pub fn system_now() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Self::from_seconds(t)
    }
}
