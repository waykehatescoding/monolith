use chrono::{Duration, NaiveDateTime, NaiveTime};

pub const WARNING_SECS: [i64; 3] = [300, 60, 10];

pub fn parse_times(raw: &[String]) -> Result<Vec<String>, String> {
    let mut times = Vec::new();
    for entry in raw {
        let time = NaiveTime::parse_from_str(entry.trim(), "%H:%M")
            .map_err(|_| format!("Invalid restart time \"{entry}\", use HH:MM"))?;
        times.push(time);
    }
    times.sort();
    times.dedup();
    Ok(times
        .iter()
        .map(|t| t.format("%H:%M").to_string())
        .collect())
}

pub fn seconds_until_next(now: NaiveDateTime, times: &[String]) -> Option<i64> {
    times
        .iter()
        .filter_map(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
        .map(|t| {
            let today = now.date().and_time(t);
            let next = if today > now {
                today
            } else {
                today + Duration::days(1)
            };
            (next - now).num_seconds()
        })
        .min()
}

pub fn crossed_warning(previous: i64, current: i64) -> Option<i64> {
    WARNING_SECS
        .into_iter()
        .find(|w| previous > *w && current <= *w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 1)
            .and_then(|d| d.and_hms_opt(h, m, s))
            .unwrap_or_default()
    }

    #[test]
    fn parses_sorts_and_dedupes() {
        let raw = vec!["18:30".into(), "4:05".into(), "18:30".into()];
        assert_eq!(parse_times(&raw), Ok(vec!["04:05".into(), "18:30".into()]));
        assert!(parse_times(&["25:00".to_string()]).is_err());
        assert!(parse_times(&["noon".to_string()]).is_err());
    }

    #[test]
    fn computes_next_run_across_midnight() {
        let times = vec!["04:00".to_string(), "18:00".to_string()];
        assert_eq!(seconds_until_next(at(17, 0, 0), &times), Some(3600));
        assert_eq!(seconds_until_next(at(18, 0, 0), &times), Some(10 * 3600));
        assert_eq!(seconds_until_next(at(23, 0, 0), &times), Some(5 * 3600));
        assert_eq!(seconds_until_next(at(1, 0, 0), &[]), None);
    }

    #[test]
    fn warns_when_crossing_thresholds() {
        assert_eq!(crossed_warning(305, 298), Some(300));
        assert_eq!(crossed_warning(70, 55), Some(60));
        assert_eq!(crossed_warning(12, 6), Some(10));
        assert_eq!(crossed_warning(200, 190), None);
    }
}
