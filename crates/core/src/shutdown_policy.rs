use crate::types::AppPreferences;

pub const WORLD_SAVE_INTERVAL_MS: u64 = 5 * 60 * 1000;
pub const SAVE_ALL_FLUSH_COMMAND: &str = "save-all flush";
pub const STOP_COMMAND: &str = "stop";

pub fn should_confirm_close(preferences: &AppPreferences, running_server_count: usize) -> bool {
    preferences.ask_before_close && running_server_count > 0
}

pub fn build_graceful_stop_commands() -> Vec<&'static str> {
    vec![SAVE_ALL_FLUSH_COMMAND, STOP_COMMAND]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirms_close_only_when_enabled_and_servers_running() {
        let on = AppPreferences {
            ask_before_close: true,
        };
        let off = AppPreferences {
            ask_before_close: false,
        };
        assert!(should_confirm_close(&on, 1));
        assert!(!should_confirm_close(&on, 0));
        assert!(!should_confirm_close(&off, 3));
    }

    #[test]
    fn stops_gracefully_after_flushing() {
        assert_eq!(
            build_graceful_stop_commands(),
            vec!["save-all flush", "stop"]
        );
        assert_eq!(WORLD_SAVE_INTERVAL_MS, 300_000);
    }
}
