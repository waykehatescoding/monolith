use std::time::Duration;

const MAX_ATTEMPTS: usize = 3;
const WINDOW_SECS: u64 = 10 * 60;
const BASE_DELAY_SECS: u64 = 5;

pub fn next_delay(attempts: &mut Vec<u64>, now_secs: u64) -> Option<Duration> {
    attempts.retain(|t| now_secs.saturating_sub(*t) < WINDOW_SECS);
    if attempts.len() >= MAX_ATTEMPTS {
        return None;
    }
    let delay = BASE_DELAY_SECS << attempts.len();
    attempts.push(now_secs);
    Some(Duration::from_secs(delay))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_then_gives_up() {
        let mut attempts = Vec::new();
        assert_eq!(next_delay(&mut attempts, 0), Some(Duration::from_secs(5)));
        assert_eq!(next_delay(&mut attempts, 10), Some(Duration::from_secs(10)));
        assert_eq!(next_delay(&mut attempts, 30), Some(Duration::from_secs(20)));
        assert_eq!(next_delay(&mut attempts, 60), None);
    }

    #[test]
    fn old_attempts_expire() {
        let mut attempts = vec![0, 10, 30];
        assert_eq!(next_delay(&mut attempts, 700), Some(Duration::from_secs(5)));
        assert_eq!(attempts, vec![700]);
    }
}
