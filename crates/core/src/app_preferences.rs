use std::path::PathBuf;

use crate::paths;
use crate::types::{AppPreferences, AppPreferencesUpdate};

fn preferences_path() -> PathBuf {
    paths::user_data_dir().join("preferences.json")
}

fn normalize(value: &serde_json::Value) -> AppPreferences {
    AppPreferences {
        ask_before_close: value
            .get("askBeforeClose")
            .and_then(|v| v.as_bool())
            .unwrap_or(AppPreferences::default().ask_before_close),
    }
}

fn merge(current: &AppPreferences, updates: &AppPreferencesUpdate) -> AppPreferences {
    AppPreferences {
        ask_before_close: updates.ask_before_close.unwrap_or(current.ask_before_close),
    }
}

async fn read_preferences() -> AppPreferences {
    match tokio::fs::read_to_string(preferences_path()).await {
        Ok(raw) => serde_json::from_str(&raw)
            .map(|v| normalize(&v))
            .unwrap_or_default(),
        Err(_) => AppPreferences::default(),
    }
}

pub async fn get_app_preferences() -> AppPreferences {
    read_preferences().await
}

pub async fn update_app_preferences(
    updates: AppPreferencesUpdate,
) -> Result<AppPreferences, String> {
    let path = preferences_path();
    let next = merge(&read_preferences().await, &updates);

    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| e.to_string())?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_invalid_values_to_defaults() {
        assert!(normalize(&json!({})).ask_before_close);
        assert!(normalize(&json!(null)).ask_before_close);
        assert!(normalize(&json!({ "askBeforeClose": "no" })).ask_before_close);
        assert!(!normalize(&json!({ "askBeforeClose": false })).ask_before_close);
    }

    #[test]
    fn merges_updates_over_current() {
        let current = AppPreferences {
            ask_before_close: true,
        };
        assert!(
            !merge(
                &current,
                &AppPreferencesUpdate {
                    ask_before_close: Some(false)
                }
            )
            .ask_before_close
        );
        assert!(merge(&current, &AppPreferencesUpdate::default()).ask_before_close);
    }
}
