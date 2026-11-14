//! RFC 3339 UTC timestamps with milliseconds, `std` only (for `sent_at`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `2026-10-18T14:02:11.004Z` for the given instant.
pub fn rfc3339_utc_ms(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let secs = d.as_secs();
    let ms = d.subsec_millis();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, day) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Now, as [`rfc3339_utc_ms`].
pub fn now_rfc3339() -> String {
    rfc3339_utc_ms(SystemTime::now())
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01 → y/m/d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339_utc_ms(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        // 2026-10-18T14:02:11.004Z
        let t = UNIX_EPOCH + Duration::from_millis(1_792_332_131_004);
        assert_eq!(rfc3339_utc_ms(t), "2026-10-18T14:02:11.004Z");
        // leap day
        let t = UNIX_EPOCH + Duration::from_secs(1_709_164_800);
        assert_eq!(rfc3339_utc_ms(t), "2024-02-29T00:00:00.000Z");
        assert!(now_rfc3339().ends_with('Z'));
    }
}
