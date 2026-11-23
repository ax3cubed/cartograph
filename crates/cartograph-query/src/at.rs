//! `at=`: `now`, a date (`YYYY-MM`, `YYYY-MM-DD` or RFC 3339) or a commit sha, and the UTC
//! calendar arithmetic it needs (no time crate for two functions).

use crate::QueryError;

/// A point on the timeline, before it is resolved to a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    Now,
    /// Seconds since the Unix epoch. Resolves to the last step whose corrected time is at or
    /// before it.
    Time(i64),
    /// A full sha or a prefix of at least 4 hex digits, lowercase.
    Sha(String),
}

impl At {
    pub fn parse(s: &str) -> Result<Self, QueryError> {
        let s = s.trim();
        if s.is_empty() || s == "now" {
            return Ok(Self::Now);
        }
        if (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(Self::Sha(s.to_ascii_lowercase()));
        }
        parse_instant(s).map(Self::Time).ok_or_else(|| {
            QueryError::bad(
                "at isn't a date, a commit or now",
                "at takes now, YYYY-MM, YYYY-MM-DD, an RFC 3339 time or a commit sha",
                "for example at=2024-03-01 or at=now",
            )
        })
    }
}

/// `YYYY-MM` and `YYYY-MM-DD` are midnight UTC at their start; RFC 3339 needs an offset.
pub fn parse_instant(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let d = b.get(r)?;
        d.iter()
            .all(u8::is_ascii_digit)
            .then(|| d.iter().fold(0i64, |n, c| n * 10 + i64::from(c - b'0')))
    };
    let year = num(0..4)?;
    if b.get(4) != Some(&b'-') {
        return None;
    }
    let month = num(5..7)?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if b.len() == 7 {
        return Some(days_from_civil(year, month, 1) * 86_400);
    }
    if b.get(7) != Some(&b'-') {
        return None;
    }
    let day = num(8..10)?;
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let date = days_from_civil(year, month, day) * 86_400;
    if b.len() == 10 {
        return Some(date);
    }
    if !matches!(b.get(10), Some(b'T' | b't' | b' '))
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
    {
        return None;
    }
    let (h, m, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if h > 23 || m > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if b.len() == i + 1 => 0,
        sign @ (b'+' | b'-') if b.len() == i + 6 && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let o = oh * 3600 + om * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    Some(date + h * 3600 + m * 60 + sec - offset)
}

/// RFC 3339 in UTC, to the second.
pub fn rfc3339(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's algorithms for the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_to_utc_seconds() {
        assert_eq!(parse_instant("1970-01"), Some(0));
        assert_eq!(parse_instant("2024-03-01"), Some(1_709_251_200));
        assert_eq!(parse_instant("2024-03-01T12:00:00Z"), Some(1_709_294_400));
        assert_eq!(
            parse_instant("2024-03-01T12:00:00.250+02:00"),
            Some(1_709_287_200)
        );
        assert_eq!(parse_instant("2024-02-29"), Some(1_709_164_800));
        for bad in [
            "2023-02-29",
            "2024-13",
            "2024-1-01",
            "2024-03-01T12:00:00",
            "2024-03-01T25:00:00Z",
            "march",
        ] {
            assert_eq!(parse_instant(bad), None, "{bad}");
        }
    }

    #[test]
    fn rfc3339_round_trips() {
        for t in [0, 1_709_294_400, 951_782_400, -86_400, 4_102_444_799] {
            assert_eq!(parse_instant(&rfc3339(t)), Some(t), "{t}");
        }
        assert_eq!(rfc3339(1_709_294_400), "2024-03-01T12:00:00Z");
    }

    #[test]
    fn at_reads_now_dates_and_shas() {
        assert_eq!(At::parse("now").unwrap(), At::Now);
        assert_eq!(At::parse("").unwrap(), At::Now);
        assert_eq!(At::parse("ABCD12").unwrap(), At::Sha("abcd12".into()));
        assert_eq!(At::parse("2024-03").unwrap(), At::Time(1_709_251_200));
        assert!(At::parse("abc").is_err());
        assert!(At::parse("HEAD~1").is_err());
        assert!(At::parse("'; DROP TABLE link; --").is_err());
    }
}
