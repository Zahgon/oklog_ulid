//! Minimal, faithful port of the `time.Time` surface that `ulid` uses.
//!
//! Represented as an instant relative to the Unix epoch (`sec` + `nsec`), which
//! is what every ULID code path actually observes. Go's *zero* `time.Time` is
//! January 1 of year 1, i.e. Unix second -62135596800; that value is
//! load-bearing, because `MustNewDefault(time.Time{})` must overflow into
//! `ErrBigTime`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Nanoseconds in a millisecond (Go's `time.Millisecond`).
pub const MILLISECOND: i64 = 1_000_000;
/// Nanoseconds in a second (Go's `time.Second`).
pub const SECOND: i64 = 1_000_000_000;

/// Unix second corresponding to Go's zero `time.Time` (Jan 1, year 1, UTC).
pub const ZERO_TIME_UNIX_SEC: i64 = -62135596800;

/// Port of Go's `time.Time`, reduced to an absolute instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Time {
    sec: i64,
    nsec: i32, // always normalised into [0, 1e9)
}

impl Time {
    /// Port of `time.Unix(sec, nsec)`, including Go's normalisation of an
    /// out-of-range `nsec`.
    pub fn unix_new(mut sec: i64, mut nsec: i64) -> Time {
        if !(0..SECOND).contains(&nsec) {
            let n = nsec.div_euclid(SECOND);
            sec += n;
            nsec -= n * SECOND;
        }
        Time {
            sec,
            nsec: nsec as i32,
        }
    }

    /// Go's zero value `time.Time{}`.
    pub fn zero() -> Time {
        Time {
            sec: ZERO_TIME_UNIX_SEC,
            nsec: 0,
        }
    }

    /// Port of `time.Now()`.
    pub fn now() -> Time {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => Time {
                sec: d.as_secs() as i64,
                nsec: d.subsec_nanos() as i32,
            },
            Err(e) => {
                let d = e.duration();
                Time::unix_new(-(d.as_secs() as i64), -(d.subsec_nanos() as i64))
            }
        }
    }

    /// Port of `(time.Time).Unix()`.
    pub fn unix(&self) -> i64 {
        self.sec
    }

    /// Port of `(time.Time).Nanosecond()`.
    pub fn nanosecond(&self) -> i32 {
        self.nsec
    }

    /// Port of `(time.Time).UnixNano()`.
    pub fn unix_nano(&self) -> i64 {
        self.sec.wrapping_mul(SECOND).wrapping_add(self.nsec as i64)
    }

    /// Port of `(time.Time).Truncate(d)`.
    ///
    /// Go truncates relative to the zero time (year 1). For every divisor used
    /// here the offset between year 1 and the Unix epoch is an exact multiple,
    /// so truncating relative to the epoch is equivalent.
    pub fn truncate(&self, d: i64) -> Time {
        if d <= 0 {
            return *self;
        }
        let total = (self.sec as i128) * (SECOND as i128) + self.nsec as i128;
        let r = total.rem_euclid(d as i128);
        let truncated = total - r;
        let sec = truncated.div_euclid(SECOND as i128) as i64;
        let nsec = truncated.rem_euclid(SECOND as i128) as i32;
        Time { sec, nsec }
    }

    /// Port of `(time.Time).Sub(u)`, returning nanoseconds.
    pub fn sub(&self, u: Time) -> i64 {
        let a = (self.sec as i128) * (SECOND as i128) + self.nsec as i128;
        let b = (u.sec as i128) * (SECOND as i128) + u.nsec as i128;
        (a - b) as i64
    }

    /// Port of `(time.Time).Add(d)`, where `d` is in nanoseconds.
    pub fn add(&self, d: i64) -> Time {
        Time::unix_new(self.sec, self.nsec as i64 + d)
    }
}

// --------------------------------------------------------------------------
// Calendar conversion + the two layouts the CLI supports
// --------------------------------------------------------------------------

/// Broken-down calendar time, equivalent to the fields Go's formatter reads.
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub nsec: i32,
    pub weekday: u32, // 0 = Sunday
    pub offset: i32,  // seconds east of UTC
    pub zone: String,
}

/// Howard Hinnant's `days_from_civil` — the inverse of [`civil_from_days`].
///
/// Lives outside the platform-gated `tz` module so it can be unit-tested on
/// every platform: the Windows tz path derives its UTC offset from this, and
/// that arithmetic should not go unverified just because the surrounding FFI
/// can only run on Windows.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Seconds since the Unix epoch for a broken-down calendar time, interpreted
/// without any zone adjustment.
pub fn civil_to_unix(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> i64 {
    days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec
}

/// UTC offset in seconds, derived by differencing a local and a UTC
/// broken-down time for the same instant. This is how the Windows path
/// recovers `tm_gmtoff`, which the Windows CRT does not provide.
///
/// Each tuple is `(year, month, day, hour, min, sec)`.
pub fn offset_between(
    local: (i64, i64, i64, i64, i64, i64),
    utc: (i64, i64, i64, i64, i64, i64),
) -> i32 {
    let l = civil_to_unix(local.0, local.1, local.2, local.3, local.4, local.5);
    let u = civil_to_unix(utc.0, utc.1, utc.2, utc.3, utc.4, utc.5);
    (l - u) as i32
}

/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

impl Time {
    /// Convert to UTC calendar fields.
    pub fn utc_civil(&self) -> Civil {
        self.civil_with_offset(0, "UTC".to_string())
    }

    fn civil_with_offset(&self, offset: i32, zone: String) -> Civil {
        let local = self.sec + offset as i64;
        let days = local.div_euclid(86400);
        let rem = local.rem_euclid(86400);
        let (year, month, day) = civil_from_days(days);
        // 1970-01-01 was a Thursday (4).
        let weekday = (days.rem_euclid(7) + 4).rem_euclid(7) as u32;
        Civil {
            year,
            month,
            day,
            hour: (rem / 3600) as u32,
            min: ((rem % 3600) / 60) as u32,
            sec: (rem % 60) as u32,
            nsec: self.nsec,
            weekday,
            offset,
            zone,
        }
    }

    /// Convert to local-time calendar fields, using the platform's tz database
    /// via `localtime_r` (gives both the UTC offset and the zone abbreviation,
    /// matching Go's `MST` layout element).
    pub fn local_civil(&self) -> Civil {
        match local_offset_and_zone(self.sec) {
            Some((off, zone)) => self.civil_with_offset(off, zone),
            None => self.utc_civil(),
        }
    }
}

// --- platform tz lookup -----------------------------------------------------
//
// Returns (UTC offset in seconds, zone abbreviation) at the given instant.
// Only the CLI's `--local` flag depends on this; everything else is UTC.

#[cfg(unix)]
mod tz {
    #[repr(C)]
    struct CTm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        // `char` signedness is platform-dependent (i8 on x86_64, u8 on aarch64),
        // so use the target-correct alias rather than hardcoding i8.
        tm_zone: *const core::ffi::c_char,
    }

    extern "C" {
        // `time_t` is 64-bit on every target this crate supports.
        fn localtime_r(timep: *const i64, result: *mut CTm) -> *mut CTm;
    }

    /// Query the platform tz database via `localtime_r`, which supplies both
    /// the offset (`tm_gmtoff`) and the abbreviation (`tm_zone`).
    pub fn offset_and_zone(sec: i64) -> Option<(i32, String)> {
        unsafe {
            let mut tm = CTm {
                tm_sec: 0,
                tm_min: 0,
                tm_hour: 0,
                tm_mday: 0,
                tm_mon: 0,
                tm_year: 0,
                tm_wday: 0,
                tm_yday: 0,
                tm_isdst: 0,
                tm_gmtoff: 0,
                tm_zone: std::ptr::null(),
            };
            let t: i64 = sec;
            if localtime_r(&t as *const i64, &mut tm as *mut CTm).is_null() {
                return None;
            }
            let zone = if tm.tm_zone.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(tm.tm_zone)
                    .to_string_lossy()
                    .into_owned()
            };
            Some((tm.tm_gmtoff as i32, zone))
        }
    }
}

#[cfg(windows)]
mod tz {
    // The Windows CRT has no `tm_gmtoff`/`tm_zone`, so the offset is recovered
    // by differencing local and UTC broken-down time, and the abbreviation is
    // read from the CRT's `tzname`.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct CTm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
    }

    extern "C" {
        fn _localtime64_s(tm: *mut CTm, time: *const i64) -> i32;
        fn _gmtime64_s(tm: *mut CTm, time: *const i64) -> i32;
        fn _tzset();
        fn _get_tzname(ret: *mut usize, buf: *mut u8, len: usize, index: i32) -> i32;
    }

    pub fn offset_and_zone(sec: i64) -> Option<(i32, String)> {
        unsafe {
            _tzset();
            let mut lt = CTm::default();
            let mut gt = CTm::default();
            if _localtime64_s(&mut lt, &sec) != 0 || _gmtime64_s(&mut gt, &sec) != 0 {
                return None;
            }
            // Reuse the shared, platform-independently tested arithmetic.
            let offset = super::offset_between(
                (
                    lt.tm_year as i64 + 1900,
                    lt.tm_mon as i64 + 1,
                    lt.tm_mday as i64,
                    lt.tm_hour as i64,
                    lt.tm_min as i64,
                    lt.tm_sec as i64,
                ),
                (
                    gt.tm_year as i64 + 1900,
                    gt.tm_mon as i64 + 1,
                    gt.tm_mday as i64,
                    gt.tm_hour as i64,
                    gt.tm_min as i64,
                    gt.tm_sec as i64,
                ),
            );

            // index 0 = standard name, 1 = daylight name
            let index = if lt.tm_isdst > 0 { 1 } else { 0 };
            let mut buf = [0u8; 64];
            let mut needed: usize = 0;
            let zone = if _get_tzname(&mut needed, buf.as_mut_ptr(), buf.len(), index) == 0 {
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..end]).into_owned()
            } else {
                String::new()
            };
            Some((offset, zone))
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod tz {
    /// No tz database available; callers fall back to UTC.
    pub fn offset_and_zone(_sec: i64) -> Option<(i32, String)> {
        None
    }
}

use tz::offset_and_zone as local_offset_and_zone;

/// Go layout: `"Mon Jan 02 15:04:05.999 MST 2006"` (fractional zeros trimmed).
pub const LAYOUT_DEFAULT_MS: &str = "Mon Jan 02 15:04:05.999 MST 2006";
/// Go layout: `"2006-01-02T15:04:05.000Z07:00"` (fixed 3-digit fraction).
pub const LAYOUT_RFC3339_MS: &str = "2006-01-02T15:04:05.000Z07:00";

/// Formats a `Civil` using the two reference layouts the CLI supports.
pub fn format_civil(c: &Civil, layout: &str) -> String {
    let ms = c.nsec / 1_000_000;
    match layout {
        LAYOUT_DEFAULT_MS => {
            // `.999` drops trailing zeros, and the whole fraction if zero.
            let mut frac = String::new();
            if ms != 0 {
                let s = format!("{:03}", ms);
                let trimmed = s.trim_end_matches('0');
                if !trimmed.is_empty() {
                    frac = format!(".{}", trimmed);
                }
            }
            format!(
                "{} {} {:02} {:02}:{:02}:{:02}{} {} {}",
                DAY_NAMES[c.weekday as usize],
                MONTH_NAMES[(c.month - 1) as usize],
                c.day,
                c.hour,
                c.min,
                c.sec,
                frac,
                c.zone,
                c.year
            )
        }
        LAYOUT_RFC3339_MS => {
            let zone = if c.offset == 0 {
                "Z".to_string()
            } else {
                let a = c.offset.abs();
                format!(
                    "{}{:02}:{:02}",
                    if c.offset < 0 { '-' } else { '+' },
                    a / 3600,
                    (a % 3600) / 60
                )
            };
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{}",
                c.year, c.month, c.day, c.hour, c.min, c.sec, ms, zone
            )
        }
        _ => String::new(),
    }
}
