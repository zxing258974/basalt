//! 日期时间文本的切分与校验:导出 Excel 时判断能不能写成真正的日期,导入时按目标列校验。

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

/// 时分秒。`fraction` 是小数点后的原文(不含点),可能为空。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clock<'a> {
    pub negative: bool,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub fraction: &'a str,
}

fn digits(text: &str, min: usize, max: usize) -> Option<u32> {
    (text.len() >= min && text.len() <= max && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// `YYYY-MM-DD`(也认 `YYYY/MM/DD`,月日可以是一位),返回日期与剩下的部分。
/// `0000-00-00` 这类不存在的日期返回 `Err(剩下的部分)`,由调用方决定认不认。
pub fn split_date(text: &str) -> Option<(Result<NaiveDate, ()>, &str)> {
    let separator = if text.get(4..5) == Some("/") {
        '/'
    } else {
        '-'
    };
    let mut parts = text.splitn(3, separator);
    let year = parts.next()?;
    let month = parts.next()?;
    let tail = parts.next()?;
    let day_len = tail.bytes().take_while(u8::is_ascii_digit).count();
    let (day, rest) = tail.split_at(day_len);
    let (year, month, day) = (
        digits(year, 4, 4)?,
        digits(month, 1, 2)?,
        digits(day, 1, 2)?,
    );
    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day).ok_or(());
    Some((date, rest))
}

/// `[-]H:MM[:SS[.fraction]]`,小时可以多位(MySQL 的 TIME 能到 838 小时)。
pub fn parse_clock(text: &str) -> Option<Clock<'_>> {
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (main, fraction) = match body.split_once('.') {
        Some((main, fraction)) => (main, fraction),
        None => (body, ""),
    };
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || (body.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let mut parts = main.split(':');
    let hour = digits(parts.next()?, 1, 3)?;
    let minute = digits(parts.next()?, 2, 2)?;
    let second = match parts.next() {
        Some(second) => digits(second, 2, 2)?,
        None if fraction.is_empty() => 0,
        None => return None,
    };
    if parts.next().is_some() || minute > 59 || second > 59 {
        return None;
    }
    Some(Clock {
        negative,
        hour,
        minute,
        second,
        fraction,
    })
}

/// 能原样写成 Excel 日期的值:1900-03-01 之后(避开 Excel 1900 年闰年错误)、
/// 不带时区、秒的小数不超过 3 位。返回 (年, 月, 日, 时, 分, 秒, 毫秒)。
pub fn excel_parts(text: &str, with_time: bool) -> Option<(u16, u8, u8, u16, u8, u8, u16)> {
    let (date, rest) = split_date(text)?;
    let date = date.ok()?;
    let earliest = NaiveDate::from_ymd_opt(1900, 3, 1)?;
    if date < earliest || text.get(4..5) != Some("-") {
        return None;
    }
    let (hour, minute, second, milli) = if with_time {
        let clock = parse_clock(rest.strip_prefix(' ')?)?;
        let fraction = clock.fraction.trim_end_matches('0');
        if clock.negative || clock.hour > 23 || fraction.len() > 3 {
            return None;
        }
        let milli = format!("{fraction:0<3}").parse().ok()?;
        (clock.hour, clock.minute, clock.second, milli)
    } else if rest.is_empty() {
        (0, 0, 0, 0)
    } else {
        return None;
    };
    Some((
        u16::try_from(date.year_ce().1).ok()?,
        u8::try_from(date.month0() + 1).ok()?,
        u8::try_from(date.day0() + 1).ok()?,
        u16::try_from(hour).ok()?,
        u8::try_from(minute).ok()?,
        u8::try_from(second).ok()?,
        milli,
    ))
}

/// Excel 里读出来的日期时间写回文本:`YYYY-MM-DD HH:MM:SS`,有毫秒时带上。
pub fn format_datetime(value: NaiveDateTime) -> String {
    let base = value.format("%Y-%m-%d %H:%M:%S").to_string();
    match value.nanosecond() / 1_000_000 {
        0 => base,
        milli => format!("{base}.{}", format!("{milli:03}").trim_end_matches('0')),
    }
}

/// 只要时间部分(Excel 里的纯时间,日期是 1899-12-30 / 31)。
pub fn format_time(value: NaiveTime) -> String {
    let base = value.format("%H:%M:%S").to_string();
    match value.nanosecond() / 1_000_000 {
        0 => base,
        milli => format!("{base}.{}", format!("{milli:03}").trim_end_matches('0')),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_dates_and_flags_impossible_ones() {
        let (date, rest) = split_date("2026-10-20 08:00").unwrap();
        assert_eq!(
            date.unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()
        );
        assert_eq!(rest, " 08:00");
        let (date, rest) = split_date("2026/1/5").unwrap();
        assert_eq!(date.unwrap(), NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
        assert_eq!(rest, "");
        assert_eq!(split_date("0000-00-00").unwrap().0, Err(()));
        assert_eq!(split_date("2026-02-30").unwrap().0, Err(()));
        assert!(split_date("26-10-20").is_none());
        assert!(split_date("2026-10").is_none());
        assert!(split_date("abcd-10-20").is_none());
    }

    #[test]
    fn parses_clocks_with_long_hours_and_fractions() {
        let clock = parse_clock("-838:59:59.5").unwrap();
        assert!(clock.negative);
        assert_eq!((clock.hour, clock.minute, clock.second), (838, 59, 59));
        assert_eq!(clock.fraction, "5");
        assert_eq!(parse_clock("8:05").unwrap().second, 0);
        for bad in [
            "8",
            "8:5",
            "08:60:00",
            "08:00:61",
            "08:00:00.",
            "08:00:00.x",
            "1:2:3:4",
            "8:05.5",
        ] {
            assert!(parse_clock(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn excel_parts_only_for_exact_representable_values() {
        assert_eq!(
            excel_parts("2026-10-20", false),
            Some((2026, 10, 20, 0, 0, 0, 0))
        );
        assert_eq!(
            excel_parts("2026-10-20 08:05:09.12", true),
            Some((2026, 10, 20, 8, 5, 9, 120))
        );
        assert_eq!(
            excel_parts("2026-10-20 08:05:09.120000", true),
            Some((2026, 10, 20, 8, 5, 9, 120))
        );
        for (text, with_time) in [
            ("2026-10-20 08:05:09.123456", true),
            ("2026-10-20 08:05:09+08", true),
            ("1900-02-28", false),
            ("0000-00-00", false),
            ("2026-10-20 00:00:00", false),
            ("2026/10/20", false),
            ("infinity", false),
            ("2026-10-20", true),
        ] {
            assert_eq!(excel_parts(text, with_time), None, "{text}");
        }
    }

    #[test]
    fn formats_excel_values_back_to_text() {
        let value = NaiveDate::from_ymd_opt(2026, 10, 20)
            .unwrap()
            .and_hms_milli_opt(8, 5, 9, 120)
            .unwrap();
        assert_eq!(format_datetime(value), "2026-10-20 08:05:09.12");
        assert_eq!(
            format_datetime(value.with_nanosecond(0).unwrap()),
            "2026-10-20 08:05:09"
        );
        assert_eq!(format_time(value.time()), "08:05:09.12");
    }
}
