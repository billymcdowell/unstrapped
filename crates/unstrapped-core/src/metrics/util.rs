//! Small shared helpers: civil-date arithmetic (no chrono), descriptive statistics.

pub const MS_PER_MIN: i64 = 60_000;
pub const MS_PER_DAY: i64 = 86_400_000;

// ---------- dates ----------

/// Days since 1970-01-01 for a proleptic-Gregorian civil date (Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse `YYYY-MM-DD` into a day index (days since the Unix epoch).
pub fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Format a day index as `YYYY-MM-DD`.
pub fn format_day(idx: i64) -> String {
    let (y, m, d) = civil_from_days(idx);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Local day index of a Unix-ms timestamp given the zone offset (minutes east of UTC).
pub fn local_day_index(ts_ms: i64, tz_offset_min: i32) -> i64 {
    (ts_ms + tz_offset_min as i64 * MS_PER_MIN).div_euclid(MS_PER_DAY)
}

/// Local `YYYY-MM-DD` of a timestamp.
pub fn local_day_string(ts_ms: i64, tz_offset_min: i32) -> String {
    format_day(local_day_index(ts_ms, tz_offset_min))
}

/// Unix-ms of local midnight that starts day index `idx`.
pub fn day_start_ms(idx: i64, tz_offset_min: i32) -> i64 {
    idx * MS_PER_DAY - tz_offset_min as i64 * MS_PER_MIN
}

/// Minutes since local midnight, in `[0, 1440)`.
pub fn local_clock_min(ts_ms: i64, tz_offset_min: i32) -> f64 {
    ((ts_ms + tz_offset_min as i64 * MS_PER_MIN).rem_euclid(MS_PER_DAY)) as f64 / MS_PER_MIN as f64
}

/// Monday-based start (day index) of the week containing day index `idx`.
pub fn week_start(idx: i64) -> i64 {
    // 1970-01-01 was a Thursday (Monday = 0 -> 3).
    idx - (idx + 3).rem_euclid(7)
}

// ---------- statistics ----------

pub fn mean(v: &[f64]) -> Option<f64> {
    if v.is_empty() { None } else { Some(v.iter().sum::<f64>() / v.len() as f64) }
}

/// Sample standard deviation (n-1). `None` for fewer than 2 values.
pub fn std_dev(v: &[f64]) -> Option<f64> {
    if v.len() < 2 {
        return None;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    Some((v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt())
}

/// Linear-interpolated percentile `p` in `[0,100]` of an unsorted slice.
pub fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return None;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = p.clamp(0.0, 100.0) / 100.0 * (s.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    Some(s[lo] + (s[hi] - s[lo]) * (rank - lo as f64))
}

pub fn median(v: &[f64]) -> Option<f64> {
    percentile(v, 50.0)
}

pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

pub fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Index of the first element with `ts >= t` in a timestamp-sorted slice (via accessor).
pub fn lower_bound<T>(s: &[T], t: i64, ts: impl Fn(&T) -> i64) -> usize {
    s.partition_point(|x| ts(x) < t)
}

/// Mean absolute deviation (minutes) of clock times around their circular mean on a 1440-min circle.
pub fn circular_mad_min(clock_min: &[f64]) -> Option<f64> {
    if clock_min.is_empty() {
        return None;
    }
    let tau = std::f64::consts::TAU;
    let (mut s, mut c) = (0.0, 0.0);
    for m in clock_min {
        s += (m / 1440.0 * tau).sin();
        c += (m / 1440.0 * tau).cos();
    }
    let mu = s.atan2(c);
    let total: f64 = clock_min
        .iter()
        .map(|m| {
            let mut d = (m / 1440.0 * tau - mu).rem_euclid(tau);
            if d > std::f64::consts::PI {
                d = tau - d;
            }
            d / tau * 1440.0
        })
        .sum();
    Some(total / clock_min.len() as f64)
}

/// Circular mean of clock times (minutes since midnight), in `[0,1440)`.
pub fn circular_mean_min(clock_min: &[f64]) -> Option<f64> {
    if clock_min.is_empty() {
        return None;
    }
    let tau = std::f64::consts::TAU;
    let (mut s, mut c) = (0.0, 0.0);
    for m in clock_min {
        s += (m / 1440.0 * tau).sin();
        c += (m / 1440.0 * tau).cos();
    }
    Some((s.atan2(c) / tau * 1440.0).rem_euclid(1440.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(parse_day("2026-03-10").map(format_day).as_deref(), Some("2026-03-10"));
        assert_eq!(format_day(days_from_civil(2024, 2, 29)), "2024-02-29");
    }

    #[test]
    fn local_day_and_week() {
        // 2026-03-10 23:30 UTC at UTC+2 is already 03-11 local.
        let ts = days_from_civil(2026, 3, 10) * MS_PER_DAY + 23 * 3_600_000 + 30 * 60_000;
        assert_eq!(local_day_string(ts, 120), "2026-03-11");
        assert_eq!(local_day_string(ts, 0), "2026-03-10");
        // 2026-03-09 is a Monday.
        let mon = days_from_civil(2026, 3, 9);
        assert_eq!(week_start(mon), mon);
        assert_eq!(week_start(mon + 6), mon);
        assert_eq!(week_start(mon + 7), mon + 7);
    }

    #[test]
    fn percentiles() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(median(&v), Some(3.0));
        assert_eq!(percentile(&v, 25.0), Some(2.0));
        assert_eq!(percentile(&v, 100.0), Some(5.0));
    }

    #[test]
    fn circular_stats_wrap_midnight() {
        // 23:50 and 00:10 average to midnight, MAD 10 min.
        let m = circular_mean_min(&[1430.0, 10.0]).unwrap();
        assert!(m < 1.0 || m > 1439.0);
        assert!((circular_mad_min(&[1430.0, 10.0]).unwrap() - 10.0).abs() < 1e-6);
    }
}
