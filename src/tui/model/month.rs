//! Calendar month math: civil-date arithmetic (no chrono dependency) and
//! Monday-first month grids for the calendar screen.

/// A simple civil date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    /// Today in UTC.
    pub fn today() -> Date {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        from_days(secs as i64 / 86400)
    }

    /// Parse the date part of an ISO 8601 string ("YYYY-MM-DD...").
    pub fn from_iso(s: &str) -> Option<Date> {
        let s = s.get(..10)?;
        let mut parts = s.split('-');
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.parse().ok()?;
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// "YYYY-MM-DD".
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Weekday, Monday = 0 … Sunday = 6.
    pub fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday (weekday 3)
        (to_days(*self).rem_euclid(7) as u32 + 3) % 7
    }

    pub fn add_days(&self, delta: i64) -> Date {
        from_days(to_days(*self) + delta)
    }

    /// Move by whole months, clamping the day to the target month's length.
    pub fn add_months(&self, delta: i32) -> Date {
        let total = self.year * 12 + (self.month as i32 - 1) + delta;
        let year = total.div_euclid(12);
        let month = (total.rem_euclid(12) + 1) as u32;
        let day = self.day.min(days_in_month(year, month));
        Date { year, month, day }
    }
}

/// Days since 1970-01-01 (Howard Hinnant's civil algorithm).
fn to_days(d: Date) -> i64 {
    let y = if d.month <= 2 {
        d.year as i64 - 1
    } else {
        d.year as i64
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (d.month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d.day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn from_days(days: i64) -> Date {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y } as i32;
    Date { year, month, day }
}

pub fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

/// A Monday-first month grid: each week is 7 optional day numbers.
pub fn month_grid(year: i32, month: u32) -> Vec<[Option<u32>; 7]> {
    let first_weekday = Date {
        year,
        month,
        day: 1,
    }
    .weekday();
    let days = days_in_month(year, month);

    let mut weeks = Vec::new();
    let mut week = [None; 7];
    let mut col = first_weekday as usize;
    for day in 1..=days {
        week[col] = Some(day);
        col += 1;
        if col == 7 {
            weeks.push(week);
            week = [None; 7];
            col = 0;
        }
    }
    if col > 0 {
        weeks.push(week);
    }
    weeks
}

/// English month name.
pub fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_weekdays() {
        // 2026-07-13 is a Monday
        let d = Date {
            year: 2026,
            month: 7,
            day: 13,
        };
        assert_eq!(d.weekday(), 0);
        // 1970-01-01 was a Thursday
        let epoch = Date {
            year: 1970,
            month: 1,
            day: 1,
        };
        assert_eq!(epoch.weekday(), 3);
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }

    #[test]
    fn grid_july_2026() {
        // July 2026 starts on a Wednesday (col 2), has 31 days
        let g = month_grid(2026, 7);
        assert_eq!(g[0][2], Some(1));
        assert_eq!(g[0][0], None);
        assert_eq!(g[1][0], Some(6));
        // 13th in the 3rd row (index 2), Monday column
        assert_eq!(g[2][0], Some(13));
        let last = g.last().unwrap();
        assert!(last.contains(&Some(31)));
    }

    #[test]
    fn add_days_crosses_month_and_year() {
        let d = Date {
            year: 2026,
            month: 12,
            day: 31,
        };
        assert_eq!(
            d.add_days(1),
            Date {
                year: 2027,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            d.add_days(-31),
            Date {
                year: 2026,
                month: 11,
                day: 30
            }
        );
    }

    #[test]
    fn add_months_clamps_day() {
        let d = Date {
            year: 2026,
            month: 1,
            day: 31,
        };
        assert_eq!(
            d.add_months(1),
            Date {
                year: 2026,
                month: 2,
                day: 28
            }
        );
        assert_eq!(
            d.add_months(-1),
            Date {
                year: 2025,
                month: 12,
                day: 31
            }
        );
        assert_eq!(d.add_months(12).year, 2027);
    }

    #[test]
    fn iso_round_trip() {
        let d = Date::from_iso("2026-07-13T09:00:00").unwrap();
        assert_eq!(d.iso(), "2026-07-13");
        assert!(Date::from_iso("2026-13-01").is_none());
        assert!(Date::from_iso("2026-02-30").is_none());
        assert!(Date::from_iso("garbage").is_none());
    }
}
