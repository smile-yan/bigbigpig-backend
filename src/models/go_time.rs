//! Go-compatible time serialization.
//!
//! Go's `time.Time` marshals as RFC3339Nano (`2006-01-02T15:04:05.999999999Z07:00`):
//! fractional seconds are omitted when zero and trailing zeros are trimmed;
//! a zero offset renders as `Z`. `DATETIME(3)` values therefore become either
//! `...:05+08:00` or `...:05.604+08:00`.
//!
//! The offset is not taken from the process TZ: `server.timezone` pins it so
//! the API output is stable across deployment hosts.

use chrono::{DateTime, NaiveDateTime, Offset, TimeZone};
use chrono_tz::Tz;

/// Serialize a database `DATETIME` in `tz` exactly like Go's `encoding/json`.
pub fn format(value: NaiveDateTime, tz: Tz) -> String {
    let datetime = tz
        .from_local_datetime(&value)
        .earliest()
        // A gap/ambiguity only exists for DST zones; DATETIME has no offset,
        // so reinterpret the wall clock as UTC instead of dropping the value.
        .unwrap_or_else(|| tz.from_utc_datetime(&value));
    format_datetime(&datetime)
}

/// `Option` variant used by rows whose columns are nullable. Go would fail to
/// scan NULL into `time.Time`; the zero time is rendered for completeness.
pub fn format_opt(value: Option<NaiveDateTime>, tz: Tz) -> String {
    match value {
        Some(value) => format(value, tz),
        None => "0001-01-01T00:00:00Z".to_string(),
    }
}

pub fn format_datetime(datetime: &DateTime<Tz>) -> String {
    let mut output = datetime.format("%Y-%m-%dT%H:%M:%S").to_string();

    let nanos = datetime.timestamp_subsec_nanos();
    if nanos != 0 {
        let mut fraction = format!("{nanos:09}");
        while fraction.ends_with('0') {
            fraction.pop();
        }
        output.push('.');
        output.push_str(&fraction);
    }

    if datetime.offset().fix().local_minus_utc() == 0 {
        output.push('Z');
    } else {
        output.push_str(&datetime.format("%:z").to_string());
    }

    output
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn naive(hour: u32, minute: u32, second: u32, nanos: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 1)
            .expect("valid date")
            .and_hms_nano_opt(hour, minute, second, nanos)
            .expect("valid time")
    }

    #[test]
    fn whole_seconds_have_no_fraction() {
        assert_eq!(
            format(naive(10, 0, 0, 0), Tz::Asia__Shanghai),
            "2026-10-01T10:00:00+08:00"
        );
    }

    #[test]
    fn milliseconds_are_trimmed_like_rfc3339_nano() {
        assert_eq!(
            format(naive(10, 0, 0, 600_000_000), Tz::Asia__Shanghai),
            "2026-10-01T10:00:00.6+08:00"
        );
        assert_eq!(
            format(naive(10, 0, 0, 604_000_000), Tz::Asia__Shanghai),
            "2026-10-01T10:00:00.604+08:00"
        );
    }

    #[test]
    fn sub_millisecond_values_are_preserved() {
        assert_eq!(
            format(naive(10, 0, 0, 123_456_789), Tz::Asia__Shanghai),
            "2026-10-01T10:00:00.123456789+08:00"
        );
    }

    #[test]
    fn utc_offset_uses_z() {
        assert_eq!(format(naive(10, 0, 0, 0), Tz::UTC), "2026-10-01T10:00:00Z");
    }

    #[test]
    fn null_is_go_zero_time() {
        assert_eq!(format_opt(None, Tz::Asia__Shanghai), "0001-01-01T00:00:00Z");
    }
}
