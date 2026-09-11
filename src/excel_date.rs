//! Excel 1900 date system helpers (including the fictitious 1900-02-29).

/// Excel's practical maximum date serial (9999-12-31 in the 1900 date system).
pub const MAX_EXCEL_SERIAL: f64 = 2958465.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl CivilDate {
    pub fn cmp_date(self, other: Self) -> std::cmp::Ordering {
        (self.year, self.month, self.day).cmp(&(other.year, other.month, other.day))
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn days_in_year_adjusted(year: i32) -> i32 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

pub fn ymd_to_serial(year: i32, month: u32, day: u32) -> Result<f64, ()> {
    if year == 1900 && month == 2 && day == 29 {
        return Ok(60.0);
    }
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(());
    }
    let dim = days_in_month(year, month);
    if day < 1 || day > dim {
        return Err(());
    }

    let mut serial = i64::from(day);
    for m in 1..month {
        serial += i64::from(days_in_month(year, m));
    }
    for y in 1900..year {
        serial += i64::from(days_in_year_adjusted(y));
    }
    // Excel's fictitious leap day for dates after 1900-02-28.
    if year > 1900 || month > 2 {
        serial += 1;
    }
    Ok(serial as f64)
}

pub fn serial_to_ymd(serial: f64) -> Result<CivilDate, ()> {
    if !serial.is_finite() || !(1.0..=MAX_EXCEL_SERIAL).contains(&serial) {
        return Err(());
    }

    let mut n = serial.floor() as i64;
    if n == 60 {
        return Ok(CivilDate {
            year: 1900,
            month: 2,
            day: 29,
        });
    }
    if n > 60 {
        n -= 1;
    }

    let mut year = 1900;
    loop {
        let diy = i64::from(days_in_year_adjusted(year));
        if n > diy {
            n -= diy;
            year += 1;
            if year > 9999 {
                return Err(());
            }
        } else {
            break;
        }
    }

    let mut month = 1u32;
    loop {
        let dim = i64::from(days_in_month(year, month));
        if n > dim {
            n -= dim;
            month += 1;
            if month > 12 {
                return Err(());
            }
        } else {
            break;
        }
    }

    Ok(CivilDate {
        year,
        month,
        day: n as u32,
    })
}

/// Excel `DATE(year, month, day)` with month/day rollover.
pub fn excel_date(mut year: i32, mut month: i32, day: i32) -> Result<f64, ()> {
    if (0..1900).contains(&year) {
        year += 1900;
    }
    if !(1900..=9999).contains(&year) {
        return Err(());
    }

    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }
    if !(1900..=9999).contains(&year) {
        return Err(());
    }

    let base = ymd_to_serial(year, month as u32, 1)?;
    let serial = base + f64::from(day - 1);
    if !(1.0..=MAX_EXCEL_SERIAL).contains(&serial) {
        return Err(());
    }
    // Normalize through round-trip so overflow lands on a valid civil date / serial.
    let civil = serial_to_ymd(serial)?;
    ymd_to_serial(civil.year, civil.month, civil.day)
}

/// Strict date parsing for `DATEVALUE` (invalid calendar dates are rejected).
pub fn datevalue(text: &str) -> Result<f64, ()> {
    let text = text.trim();
    let (year, month, day) = if let Some(parts) = parse_japanese_date(text) {
        parts
    } else {
        parse_slash_or_dash_date(text)?
    };

    if !(1..=12).contains(&month) {
        return Err(());
    }
    ymd_to_serial(year, month, day)
}

fn parse_slash_or_dash_date(text: &str) -> Result<(i32, u32, u32), ()> {
    let sep = if text.contains('-') {
        '-'
    } else if text.contains('/') {
        '/'
    } else if text.contains('.') {
        '.'
    } else {
        return Err(());
    };

    let parts: Vec<&str> = text.split(sep).map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return Err(());
    }

    let a: i32 = parts[0].parse().map_err(|_| ())?;
    let b: u32 = parts[1].parse().map_err(|_| ())?;
    let c: i32 = parts[2].parse().map_err(|_| ())?;

    if parts[0].len() >= 4 {
        // Y/M/D or Y-M-D
        Ok((a, b, c as u32))
    } else if parts[2].len() == 4 {
        // D/M/Y when day is unambiguous (>12); otherwise Excel US default M/D/Y.
        if a > 12 {
            Ok((c, b, a as u32)) // D/M/Y
        } else {
            Ok((c, a as u32, b)) // M/D/Y (including ambiguous cases)
        }
    } else {
        Err(())
    }
}

/// Parses `YYYY年M月D日` and wareki forms such as `令和6年4月1日`.
fn parse_japanese_date(text: &str) -> Option<(i32, u32, u32)> {
    let text = text.trim();
    if !(text.contains('年') && text.contains('月') && text.contains('日')) {
        return None;
    }

    let (era_offset, rest) = if let Some(rest) = text.strip_prefix("令和") {
        (Some(2018), rest)
    } else if let Some(rest) = text.strip_prefix("平成") {
        (Some(1988), rest)
    } else if let Some(rest) = text.strip_prefix("昭和") {
        (Some(1925), rest)
    } else {
        (None, text)
    };

    let year_split = rest.split_once('年')?;
    let month_split = year_split.1.split_once('月')?;
    let day_part = month_split.1.strip_suffix('日')?;

    let year_number: i32 = if year_split.0 == "元" {
        1
    } else {
        year_split.0.parse().ok()?
    };
    let month: u32 = month_split.0.parse().ok()?;
    let day: u32 = day_part.parse().ok()?;

    let year = match era_offset {
        Some(offset) => offset + year_number,
        None => year_number,
    };

    Some((year, month, day))
}

pub fn datedif(start: CivilDate, end: CivilDate, unit: &str) -> Result<f64, DatedifError> {
    if start.cmp_date(end).is_gt() {
        return Err(DatedifError::Num);
    }

    match unit.trim().to_ascii_uppercase().as_str() {
        "Y" => {
            let mut years = end.year - start.year;
            if (end.month, end.day) < (start.month, start.day) {
                years -= 1;
            }
            Ok(f64::from(years))
        }
        "M" => {
            let mut months =
                (end.year - start.year) * 12 + (end.month as i32 - start.month as i32);
            if end.day < start.day {
                months -= 1;
            }
            Ok(f64::from(months))
        }
        "D" => {
            let start_serial = ymd_to_serial(start.year, start.month, start.day)
                .map_err(|_| DatedifError::Value)?;
            let end_serial = ymd_to_serial(end.year, end.month, end.day)
                .map_err(|_| DatedifError::Value)?;
            Ok(end_serial - start_serial)
        }
        "YM" => {
            let mut months = end.month as i32 - start.month as i32;
            if end.day < start.day {
                months -= 1;
            }
            if months < 0 {
                months += 12;
            }
            Ok(f64::from(months))
        }
        "YD" => {
            let mut end_year = start.year;
            let end_day = clamp_day(end_year, end.month, end.day);
            let mut adjusted = CivilDate {
                year: end_year,
                month: end.month,
                day: end_day,
            };
            if adjusted.cmp_date(start).is_lt() {
                end_year += 1;
                let end_day = clamp_day(end_year, end.month, end.day);
                adjusted = CivilDate {
                    year: end_year,
                    month: end.month,
                    day: end_day,
                };
            }
            let start_serial = ymd_to_serial(start.year, start.month, start.day)
                .map_err(|_| DatedifError::Value)?;
            let end_serial = ymd_to_serial(adjusted.year, adjusted.month, adjusted.day)
                .map_err(|_| DatedifError::Value)?;
            Ok(end_serial - start_serial)
        }
        "MD" => Ok(datedif_md(start, end)),
        _ => Err(DatedifError::Value),
    }
}

/// PhpSpreadsheet / Excel-compatible `MD` (can be negative in known edge cases).
fn datedif_md(start: CivilDate, end: CivilDate) -> f64 {
    if end.day >= start.day {
        f64::from(end.day - start.day)
    } else {
        // Match PhpSpreadsheet: move end back by `end.day` days (last day of previous month),
        // then `end.day + previousMonthLength - start.day`.
        let (prev_year, prev_month) = if end.month == 1 {
            (end.year - 1, 12)
        } else {
            (end.year, end.month - 1)
        };
        let adjust_days = days_in_month(prev_year, prev_month);
        f64::from(end.day as i32 + adjust_days as i32 - start.day as i32)
    }
}

fn clamp_day(year: i32, month: u32, day: u32) -> u32 {
    day.min(days_in_month(year, month))
}

#[derive(Debug, PartialEq, Eq)]
pub enum DatedifError {
    Num,
    Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_known_serials() {
        assert_eq!(ymd_to_serial(1900, 1, 1).unwrap(), 1.0);
        assert_eq!(ymd_to_serial(1900, 2, 28).unwrap(), 59.0);
        assert_eq!(ymd_to_serial(1900, 2, 29).unwrap(), 60.0);
        assert_eq!(ymd_to_serial(1900, 3, 1).unwrap(), 61.0);
        assert_eq!(ymd_to_serial(2008, 1, 1).unwrap(), 39448.0);
        assert_eq!(ymd_to_serial(9999, 12, 31).unwrap(), MAX_EXCEL_SERIAL);
    }

    #[test]
    fn serial_round_trip_skips_fake_leap_via_lookup() {
        assert_eq!(
            serial_to_ymd(60.0).unwrap(),
            CivilDate {
                year: 1900,
                month: 2,
                day: 29
            }
        );
        assert_eq!(
            serial_to_ymd(61.0).unwrap(),
            CivilDate {
                year: 1900,
                month: 3,
                day: 1
            }
        );
        assert_eq!(
            serial_to_ymd(39448.0).unwrap(),
            CivilDate {
                year: 2008,
                month: 1,
                day: 1
            }
        );
    }

    #[test]
    fn serial_to_ymd_rejects_non_finite_and_out_of_range() {
        assert!(serial_to_ymd(f64::INFINITY).is_err());
        assert!(serial_to_ymd(f64::NAN).is_err());
        assert!(serial_to_ymd(0.0).is_err());
        assert!(serial_to_ymd(MAX_EXCEL_SERIAL + 1.0).is_err());
    }

    #[test]
    fn datevalue_rejects_invalid_calendar_dates() {
        assert!(datevalue("2024-01-32").is_err());
        assert!(datevalue("2024-02-30").is_err());
        assert_eq!(datevalue("2008/1/1").unwrap(), 39448.0);
    }

    #[test]
    fn datevalue_supports_dmy_when_unambiguous_and_japanese() {
        assert_eq!(
            datevalue("15/1/2008").unwrap(),
            ymd_to_serial(2008, 1, 15).unwrap()
        );
        assert_eq!(
            datevalue("1/15/2008").unwrap(),
            ymd_to_serial(2008, 1, 15).unwrap()
        );
        assert_eq!(
            datevalue("2008年1月1日").unwrap(),
            ymd_to_serial(2008, 1, 1).unwrap()
        );
        assert_eq!(
            datevalue("令和6年4月1日").unwrap(),
            ymd_to_serial(2024, 4, 1).unwrap()
        );
        assert_eq!(
            datevalue("平成元年1月8日").unwrap(),
            ymd_to_serial(1989, 1, 8).unwrap()
        );
    }

    #[test]
    fn datedif_microsoft_examples() {
        let start = CivilDate {
            year: 2001,
            month: 1,
            day: 1,
        };
        let end = CivilDate {
            year: 2003,
            month: 1,
            day: 1,
        };
        assert_eq!(datedif(start, end, "Y").unwrap(), 2.0);

        let start = CivilDate {
            year: 2001,
            month: 6,
            day: 1,
        };
        let end = CivilDate {
            year: 2002,
            month: 8,
            day: 15,
        };
        assert_eq!(datedif(start, end, "D").unwrap(), 440.0);
        assert_eq!(datedif(start, end, "YD").unwrap(), 75.0);
    }

    #[test]
    fn datedif_md_matches_phpspreadsheet() {
        let start = CivilDate {
            year: 2007,
            month: 1,
            day: 1,
        };
        let end = CivilDate {
            year: 2007,
            month: 1,
            day: 31,
        };
        assert_eq!(datedif(start, end, "MD").unwrap(), 30.0);

        // end.day < start.day: uses previous month of *end* (PhpSpreadsheet algorithm).
        let start = CivilDate {
            year: 2007,
            month: 1,
            day: 20,
        };
        let end = CivilDate {
            year: 2008,
            month: 3,
            day: 5,
        };
        // 5 + days_in_feb(2008)=29 - 20 = 14
        assert_eq!(datedif(start, end, "MD").unwrap(), 14.0);

        // Known Excel/PhpSpreadsheet quirk: result can be negative.
        let start = CivilDate {
            year: 2007,
            month: 1,
            day: 31,
        };
        let end = CivilDate {
            year: 2007,
            month: 3,
            day: 1,
        };
        // 1 + 28 - 31 = -2
        assert_eq!(datedif(start, end, "MD").unwrap(), -2.0);
    }
}
