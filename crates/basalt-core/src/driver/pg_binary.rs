//! PostgreSQL 二进制格式里几种需要自己解的类型。
//!
//! sqlx 的 rust_decimal 只能装 28 位有效数字,PostgreSQL 的 NUMERIC 远不止;
//! chrono 遇到 infinity、公元前日期也会失败。这里直接按线上格式转成
//! PostgreSQL 自己的文本写法,写回时 `'...'::类型` 能原样解析。

use std::fmt::Write;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};

const MICROS_PER_DAY: i64 = 86_400_000_000;

const NUMERIC_POSITIVE: u16 = 0x0000;
const NUMERIC_NEGATIVE: u16 = 0x4000;
const NUMERIC_NAN: u16 = 0xC000;
const NUMERIC_INFINITY: u16 = 0xD000;
const NUMERIC_NEG_INFINITY: u16 = 0xF000;
/// NUMERIC 每个数位是 0..9999 的万进制。
const NUMERIC_BASE: i16 = 10_000;

/// NUMERIC:ndigits、weight、sign、dscale 四个 i16,后跟 ndigits 个万进制数位。
pub(crate) fn numeric_to_string(bytes: &[u8]) -> Option<String> {
    let word = |at: usize| -> Option<[u8; 2]> { bytes.get(at..at + 2)?.try_into().ok() };
    let ndigits = usize::try_from(i16::from_be_bytes(word(0)?)).ok()?;
    let weight = i32::from(i16::from_be_bytes(word(2)?));
    let sign = u16::from_be_bytes(word(4)?);
    let dscale = usize::from(u16::from_be_bytes(word(6)?));
    match sign {
        NUMERIC_NAN => return Some("NaN".into()),
        NUMERIC_INFINITY => return Some("Infinity".into()),
        NUMERIC_NEG_INFINITY => return Some("-Infinity".into()),
        NUMERIC_POSITIVE | NUMERIC_NEGATIVE => {}
        _ => return None,
    }
    if bytes.len() != 8 + ndigits * 2 {
        return None;
    }
    let digits = (0..ndigits)
        .map(|k| i16::from_be_bytes(word(8 + k * 2).unwrap_or_default()))
        .collect::<Vec<_>>();
    if digits.iter().any(|d| !(0..NUMERIC_BASE).contains(d)) {
        return None;
    }
    // 第 k 个数位的权是 10000^(weight - k),范围外的位都是 0。
    let digit = |k: i32| {
        usize::try_from(k)
            .ok()
            .and_then(|k| digits.get(k))
            .copied()
            .unwrap_or(0)
    };

    let mut out = String::new();
    if sign == NUMERIC_NEGATIVE {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    }
    for k in 0..=weight {
        let _ = if k == 0 {
            write!(out, "{}", digit(k))
        } else {
            write!(out, "{:04}", digit(k))
        };
    }
    if dscale > 0 {
        let mut fraction = String::with_capacity(dscale + 4);
        let mut k = weight + 1;
        while fraction.len() < dscale {
            let _ = write!(fraction, "{:04}", digit(k));
            k += 1;
        }
        fraction.truncate(dscale);
        out.push('.');
        out.push_str(&fraction);
    }
    Some(out)
}

/// DATE:相对 2000-01-01 的天数。
pub(crate) fn date_to_string(days: i32) -> Option<String> {
    match days {
        i32::MAX => return Some("infinity".into()),
        i32::MIN => return Some("-infinity".into()),
        _ => {}
    }
    let date = pg_epoch().checked_add_signed(Duration::days(days.into()))?;
    let (text, bc) = format_date(date);
    Some(if bc { format!("{text} BC") } else { text })
}

/// TIME:当天零点起的微秒数,可以是 24:00:00。
pub(crate) fn time_to_string(micros: i64) -> String {
    let micros = micros.max(0);
    let seconds = micros / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}{}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        fraction_suffix(micros % 1_000_000)
    )
}

/// TIMESTAMP / TIMESTAMPTZ:相对 2000-01-01 00:00:00 的微秒数。
/// `offset_seconds` 为 `Some` 时先换算到该时区再带上偏移量输出。
pub(crate) fn timestamp_to_string(micros: i64, offset_seconds: Option<i32>) -> Option<String> {
    match micros {
        i64::MAX => return Some("infinity".into()),
        i64::MIN => return Some("-infinity".into()),
        _ => {}
    }
    let offset = offset_seconds.unwrap_or(0);
    let local = micros.checked_add(i64::from(offset) * 1_000_000)?;
    let date = pg_epoch().checked_add_signed(Duration::days(local.div_euclid(MICROS_PER_DAY)))?;
    let (date_text, bc) = format_date(date);
    let mut out = format!(
        "{date_text} {}",
        time_to_string(local.rem_euclid(MICROS_PER_DAY))
    );
    if offset_seconds.is_some() {
        out.push_str(&format_offset(offset));
    }
    if bc {
        out.push_str(" BC");
    }
    Some(out)
}

/// 2000-01-01 起的微秒数 → UTC 时间,给换算本地时区用。超出 chrono 范围时为 `None`。
pub(crate) fn timestamp_to_utc(micros: i64) -> Option<NaiveDateTime> {
    pg_epoch()
        .and_hms_opt(0, 0, 0)?
        .checked_add_signed(Duration::microseconds(micros))
}

/// 公元前的年份按 PostgreSQL 的写法:年份取正,末尾加 BC(chrono 的第 0 年是公元前 1 年)。
fn format_date(date: NaiveDate) -> (String, bool) {
    let year = date.year();
    let bc = year <= 0;
    let shown = if bc { 1 - year } else { year };
    (
        format!("{shown:04}-{:02}-{:02}", date.month(), date.day()),
        bc,
    )
}

fn fraction_suffix(micros: i64) -> String {
    if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}").trim_end_matches('0').to_owned()
    }
}

/// `+08:00`;历史上带秒的地方时偏移(如 1901 年前的 +08:05:43)保留秒,否则写回会差几十秒。
fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.unsigned_abs();
    let mut out = format!("{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60);
    if !abs.is_multiple_of(60) {
        let _ = write!(out, ":{:02}", abs % 60);
    }
    out
}

fn pg_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).expect("2000-01-01 是合法日期")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按线上格式拼一个 NUMERIC。
    fn numeric(weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&i16::try_from(digits.len()).unwrap().to_be_bytes());
        out.extend_from_slice(&weight.to_be_bytes());
        out.extend_from_slice(&sign.to_be_bytes());
        out.extend_from_slice(&dscale.to_be_bytes());
        for digit in digits {
            out.extend_from_slice(&digit.to_be_bytes());
        }
        out
    }

    #[test]
    fn numeric_keeps_scale_and_every_digit() {
        // 12345.678 = [1][2345].[6780],weight 1,dscale 3
        assert_eq!(
            numeric_to_string(&numeric(1, 0, 3, &[1, 2345, 6780])).as_deref(),
            Some("12345.678")
        );
        // -0.0001
        assert_eq!(
            numeric_to_string(&numeric(-1, 0x4000, 4, &[1])).as_deref(),
            Some("-0.0001")
        );
        // 12.50 的尾零来自 dscale
        assert_eq!(
            numeric_to_string(&numeric(0, 0, 2, &[12, 5000])).as_deref(),
            Some("12.50")
        );
        // 1e32:只存一个数位,其余是 weight 补的零
        assert_eq!(
            numeric_to_string(&numeric(8, 0, 0, &[1])).as_deref(),
            Some("100000000000000000000000000000000")
        );
        // 超过 rust_decimal 的 28 位
        assert_eq!(
            numeric_to_string(&numeric(
                0,
                0,
                40,
                &[
                    3, 1415, 9265, 3589, 7932, 3846, 2643, 3832, 7950, 2884, 1971
                ]
            ))
            .as_deref(),
            Some("3.1415926535897932384626433832795028841971")
        );
        assert_eq!(
            numeric_to_string(&numeric(0, 0, 0, &[])).as_deref(),
            Some("0")
        );
        assert_eq!(
            numeric_to_string(&numeric(0, 0, 2, &[])).as_deref(),
            Some("0.00")
        );
    }

    #[test]
    fn numeric_special_values_and_garbage() {
        assert_eq!(
            numeric_to_string(&numeric(0, 0xC000, 0, &[])).as_deref(),
            Some("NaN")
        );
        assert_eq!(
            numeric_to_string(&numeric(0, 0xD000, 0, &[])).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            numeric_to_string(&numeric(0, 0xF000, 0, &[])).as_deref(),
            Some("-Infinity")
        );
        assert_eq!(numeric_to_string(&[0, 1]), None, "太短");
        assert_eq!(
            numeric_to_string(&numeric(0, 0, 0, &[10_000])),
            None,
            "数位越界"
        );
    }

    #[test]
    fn dates_cover_infinity_and_bc() {
        assert_eq!(date_to_string(0).as_deref(), Some("2000-01-01"));
        assert_eq!(date_to_string(9789).as_deref(), Some("2026-10-20"));
        assert_eq!(date_to_string(i32::MAX).as_deref(), Some("infinity"));
        assert_eq!(date_to_string(i32::MIN).as_deref(), Some("-infinity"));
        // 公元前 1 年在 chrono 里是第 0 年。
        let days = (NaiveDate::from_ymd_opt(0, 3, 15).unwrap() - pg_epoch()).num_days();
        assert_eq!(
            date_to_string(i32::try_from(days).unwrap()).as_deref(),
            Some("0001-03-15 BC")
        );
    }

    #[test]
    fn times_print_fractions_only_when_present() {
        assert_eq!(time_to_string(0), "00:00:00");
        assert_eq!(time_to_string(45_296_500_000), "12:34:56.5");
        assert_eq!(time_to_string(45_296_000_001), "12:34:56.000001");
        assert_eq!(time_to_string(MICROS_PER_DAY), "24:00:00");
    }

    #[test]
    fn timestamps_with_and_without_zone() {
        let micros = (NaiveDate::from_ymd_opt(2026, 10, 20).unwrap() - pg_epoch()).num_days()
            * MICROS_PER_DAY
            + 3_600_000_000;
        assert_eq!(
            timestamp_to_string(micros, None).as_deref(),
            Some("2026-10-20 01:00:00")
        );
        assert_eq!(
            timestamp_to_string(micros + 120, Some(8 * 3600)).as_deref(),
            Some("2026-10-20 09:00:00.00012+08:00")
        );
        assert_eq!(
            timestamp_to_string(micros, Some(-(5 * 3600 + 30 * 60))).as_deref(),
            Some("2026-10-19 19:30:00-05:30")
        );
        assert_eq!(
            timestamp_to_string(i64::MAX, Some(0)).as_deref(),
            Some("infinity")
        );
        assert_eq!(
            timestamp_to_string(i64::MIN, None).as_deref(),
            Some("-infinity")
        );
        assert_eq!(
            timestamp_to_string(-MICROS_PER_DAY, None).as_deref(),
            Some("1999-12-31 00:00:00")
        );
    }
}
