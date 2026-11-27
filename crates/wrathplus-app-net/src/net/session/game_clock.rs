//! The game clock's date (`0xd37f98`, what `CalendarGetDate` reads): the `SMSG_LOGIN_SETTIMESPEED`
//! date, rolled a day at each game midnight as the minute tick `0x76d740` does (`0x76c280(1, 0)`).

use wrathplus_ui::script::WrathCalendarDate;

use crate::net::GameTime;

impl GameTime {
    /// The game clock's date and time now.
    pub fn date_now(&self) -> WrathCalendarDate {
        self.date_after(self.received.elapsed().as_secs_f64() * f64::from(self.timescale))
    }

    /// The date and time `minutes` game minutes after the login packet.
    fn date_after(&self, minutes: f64) -> WrathCalendarDate {
        let (y, m, d) = self.date();
        let login = WrathCalendarDate::new(2000 + y as i32, m as u8, d as u8, 0, 0);
        let since_midnight = (f64::from(self.base_minute) + minutes).floor() as i64;
        WrathCalendarDate::from_seconds(login.seconds() + since_midnight * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 23:58 on 31 December 2026: two game minutes on, the clock reads 1 January 2027, 00:00.
    #[test]
    fn the_date_rolls_at_game_midnight() {
        let clock = GameTime::new(23, 58, 26 * 372 + 12 * 31 + 31, 1.0);
        let at = |minutes| {
            let t = clock.date_after(minutes);
            (t.year, t.month, t.day, t.hour, t.minute, t.weekday)
        };
        assert_eq!(at(1.5), (2026, 12, 31, 23, 59, 5));
        assert_eq!(at(2.0), (2027, 1, 1, 0, 0, 6));
        assert_eq!(at(2.0 + 3.0 * 1440.0), (2027, 1, 4, 0, 0, 2));
    }
}
