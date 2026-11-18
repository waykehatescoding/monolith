use crate::server_store;
use crate::types::{AnalyticsDataResult, DiskUsageResult, SystemInfo};
use std::path::{Path, PathBuf};
use sysinfo::System;

pub async fn calculate_folder_size(dir_path: &Path) -> u64 {
    let dir: PathBuf = dir_path.to_path_buf();
    tokio::task::spawn_blocking(move || folder_size(&dir))
        .await
        .unwrap_or(0)
}

fn folder_size(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return total;
    };
    for entry in entries {
        let Ok(entry) = entry else { break };
        let Ok(file_type) = entry.file_type() else {
            break;
        };
        if file_type.is_dir() {
            total += folder_size(&entry.path());
        } else if file_type.is_file() {
            match std::fs::metadata(entry.path()) {
                Ok(meta) => total += meta.len(),
                Err(_) => break,
            }
        }
    }
    total
}

pub async fn get_server_disk_usage(server_id: String) -> DiskUsageResult {
    let servers = server_store::load_server_list().await;
    match servers.iter().find(|s| s.id == server_id) {
        Some(server) => DiskUsageResult {
            success: true,
            bytes: Some(calculate_folder_size(Path::new(&server.server_path)).await),
            error: None,
        },
        None => DiskUsageResult {
            success: false,
            bytes: None,
            error: Some("Server not found".to_string()),
        },
    }
}

pub async fn get_analytics_data(server_id: String) -> AnalyticsDataResult {
    let servers = server_store::load_server_list().await;
    let Some(server) = servers.iter().find(|s| s.id == server_id) else {
        return failure("Server not found");
    };

    let analytics_path = Path::new(&server.server_path)
        .join("plugins")
        .join("MonolithAnalytics")
        .join("data")
        .join("analytics.json");

    if !analytics_path.exists() {
        return failure("no-data");
    }

    let parsed = tokio::fs::read_to_string(&analytics_path)
        .await
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok());

    match parsed {
        Some(data) => AnalyticsDataResult {
            success: true,
            data: Some(data),
            error: None,
        },
        None => failure("Failed to read analytics data"),
    }
}

fn failure(message: &str) -> AnalyticsDataResult {
    AnalyticsDataResult {
        success: false,
        data: None,
        error: Some(message.to_string()),
    }
}

pub fn max_ram_mb() -> u64 {
    let total_mb = total_memory_mb();
    let eighty_percent = (total_mb as f64 * 0.8).floor() as i64;
    let minus_one_gb = total_mb as i64 - 1024;
    512.max(eighty_percent.min(minus_one_gb)) as u64
}

fn total_memory_mb() -> u64 {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.total_memory() / (1024 * 1024)
}

pub fn get_system_info() -> SystemInfo {
    SystemInfo {
        total_memory_mb: total_memory_mb(),
        max_ram_mb: max_ram_mb(),
    }
}
