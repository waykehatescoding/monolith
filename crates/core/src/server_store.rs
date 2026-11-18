use std::path::{Component, Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use sysinfo::System;
use tokio::sync::Mutex;

use crate::paths;
use crate::server_create::generate_start_bat;
use crate::server_framework::{install_analytics_plugin, uninstall_analytics_plugin};
use crate::types::{ServerRecord, ServerRuntimeStatus, ServerSettingsUpdate, SuccessResult};

const MAX_BACKUP_COUNT: usize = 5;

static SAVE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static INVALID_NAME_CHARS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).expect("static regex"));

fn servers_json() -> PathBuf {
    paths::user_data_dir().join("servers.json")
}

fn servers_json_bak() -> PathBuf {
    paths::user_data_dir().join("servers.json.bak")
}

fn servers_json_tmp() -> PathBuf {
    paths::user_data_dir().join("servers.json.tmp")
}

fn numbered_backup(index: usize) -> PathBuf {
    paths::user_data_dir().join(format!("servers.json.bak.{index}"))
}

fn normalize(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn sanitize_server_name(name: &str) -> String {
    let stripped = INVALID_NAME_CHARS.replace_all(name, "");
    let trimmed = stripped.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '\u{feff}');
    let mut units = 0;
    let cut: String = trimmed
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 64
        })
        .collect();
    if cut.is_empty() {
        "server".to_string()
    } else {
        cut
    }
}

pub fn validate_server_path(server_path: &str) -> bool {
    let resolved = normalize(Path::new(server_path));
    resolved.starts_with(normalize(&paths::servers_dir()))
}

fn max_ram_mb() -> u32 {
    let mut sys = System::new();
    sys.refresh_memory();
    let total_mb = sys.total_memory() / (1024 * 1024);
    let eighty_percent = total_mb * 8 / 10;
    let minus_one_gb = total_mb.saturating_sub(1024);
    eighty_percent.min(minus_one_gb).max(512) as u32
}

pub fn clamp_ram(ram_mb: u32) -> u32 {
    ram_mb.min(max_ram_mb()).max(512)
}

fn tolerant_record(item: Value) -> Option<ServerRecord> {
    let Value::Object(mut map) = item else {
        return None;
    };
    if let Some(ram) = map.get("ramMB").and_then(Value::as_f64) {
        map.insert("ramMB".into(), Value::from(ram.max(0.0).round() as u64));
    }
    let base: serde_json::Map<String, Value> = ["id", "name", "serverPath", "framework", "version"]
        .iter()
        .filter_map(|key| map.get(*key).map(|v| (key.to_string(), v.clone())))
        .collect();
    for key in [
        "ramMB",
        "status",
        "players",
        "createdAt",
        "eulaAccepted",
        "jarFile",
        "javaPath",
        "priority",
        "backupConfig",
        "useNgrok",
        "ngrokUrl",
        "analyticsEnabled",
    ] {
        let Some(value) = map.get(key) else {
            continue;
        };
        let mut probe = base.clone();
        probe.insert(key.into(), value.clone());
        if serde_json::from_value::<ServerRecord>(Value::Object(probe)).is_err() {
            map.remove(key);
        }
    }
    serde_json::from_value(Value::Object(map)).ok()
}

fn parse_valid_records(data: &str) -> Option<Vec<ServerRecord>> {
    let Value::Array(items) = serde_json::from_str::<Value>(data).ok()? else {
        return None;
    };
    Some(
        items
            .into_iter()
            .filter(|item| {
                ["id", "name", "serverPath", "framework", "version"]
                    .iter()
                    .all(|key| item.get(key).is_some_and(Value::is_string))
            })
            .filter_map(tolerant_record)
            .collect(),
    )
}

async fn load_numbered_backups() -> Option<Vec<ServerRecord>> {
    for index in 0..MAX_BACKUP_COUNT {
        if let Ok(data) = tokio::fs::read_to_string(numbered_backup(index)).await {
            if let Some(records) = parse_valid_records(&data).filter(|r| !r.is_empty()) {
                return Some(records);
            }
        }
    }
    None
}

async fn load_legacy_backup() -> Option<Vec<ServerRecord>> {
    let data = tokio::fs::read_to_string(servers_json_bak()).await.ok()?;
    parse_valid_records(&data).filter(|r| !r.is_empty())
}

async fn recover_from_orphaned_folders() -> Vec<ServerRecord> {
    let mut recovered = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(paths::servers_dir()).await else {
        return recovered;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let is_dir = entry.file_type().await.is_ok_and(|t| t.is_dir());
        if !is_dir {
            continue;
        }
        let server_path = entry.path();
        let Ok(mut files) = tokio::fs::read_dir(&server_path).await else {
            continue;
        };
        let mut has_jar = false;
        while let Ok(Some(file)) = files.next_entry().await {
            if file.file_name().to_string_lossy().ends_with(".jar") {
                has_jar = true;
                break;
            }
        }
        if !has_jar {
            continue;
        }
        recovered.push(ServerRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: entry.file_name().to_string_lossy().into_owned(),
            framework: "Unknown".into(),
            version: "Unknown".into(),
            ram_mb: 2048,
            status: ServerRuntimeStatus::Offline,
            players: "0/0".into(),
            created_at: chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string(),
            server_path: server_path.to_string_lossy().into_owned(),
            ..Default::default()
        });
    }
    recovered
}

pub async fn load_server_list() -> Vec<ServerRecord> {
    try_load_server_list().await.unwrap_or_default()
}

async fn try_load_server_list() -> Result<Vec<ServerRecord>, String> {
    if let Ok(data) = tokio::fs::read_to_string(servers_json()).await {
        if let Some(records) = parse_valid_records(&data) {
            let mut validated = Vec::with_capacity(records.len());
            for mut server in records {
                if tokio::fs::metadata(&server.server_path).await.is_err() {
                    server.status = ServerRuntimeStatus::Offline;
                }
                validated.push(server);
            }
            return Ok(validated);
        }
    }

    if let Some(records) = load_numbered_backups().await {
        return Ok(records);
    }
    if let Some(records) = load_legacy_backup().await {
        return Ok(records);
    }

    let recovered = recover_from_orphaned_folders().await;
    if !recovered.is_empty() {
        save_server_list(&recovered).await?;
        return Ok(recovered);
    }

    tokio::fs::create_dir_all(paths::user_data_dir())
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(servers_json(), "[]")
        .await
        .map_err(|e| e.to_string())?;
    Ok(Vec::new())
}

async fn rotate_backups() {
    let _ = tokio::fs::copy(servers_json(), numbered_backup(0)).await;

    for index in (0..MAX_BACKUP_COUNT).rev() {
        let current = numbered_backup(index);
        if tokio::fs::metadata(&current).await.is_err() {
            continue;
        }
        if index == MAX_BACKUP_COUNT - 1 {
            let _ = tokio::fs::remove_file(&current).await;
        } else {
            let _ = tokio::fs::copy(&current, numbered_backup(index + 1)).await;
        }
    }

    let _ = tokio::fs::copy(numbered_backup(0), servers_json_bak()).await;
}

async fn save_server_list_internal(servers: &[ServerRecord]) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(servers).map_err(|e| e.to_string())?;
    let tmp = servers_json_tmp();
    let target = servers_json();

    tokio::fs::write(&tmp, &payload)
        .await
        .map_err(|e| e.to_string())?;

    let written = tokio::fs::read_to_string(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    if serde_json::from_str::<Value>(&written).is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err("[ServerManager] Temp file validation failed, aborting save".into());
    }

    let had_existing = tokio::fs::metadata(&target).await.is_ok();
    if had_existing {
        let _ = tokio::fs::copy(&target, servers_json_bak()).await;
    }

    match tokio::fs::rename(&tmp, &target).await {
        Ok(()) => {
            rotate_backups().await;
            Ok(())
        }
        Err(err) => {
            if had_existing {
                let _ = tokio::fs::copy(servers_json_bak(), &target).await;
            }
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(err.to_string())
        }
    }
}

pub async fn save_server_list(servers: &[ServerRecord]) -> Result<(), String> {
    let _guard = SAVE_LOCK.lock().await;
    save_server_list_internal(servers).await
}

fn preserve_known_player_capacity(current: &str, next: &str) -> String {
    if next != "0/20" {
        return next.to_string();
    }
    static CAPACITY: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^\d+\s*/\s*(\d+)$").expect("static regex"));
    match CAPACITY.captures(current).and_then(|c| c.get(1)) {
        Some(max) if max.as_str() != "20" => format!("0/{}", max.as_str()),
        _ => next.to_string(),
    }
}

pub async fn update_server_status(id: &str, status: ServerRuntimeStatus, players: Option<&str>) {
    let mut servers = load_server_list().await;
    if let Some(server) = servers.iter_mut().find(|s| s.id == id) {
        server.status = status;
        if let Some(players) = players {
            server.players = preserve_known_player_capacity(&server.players, players);
        }
        let _ = save_server_list(&servers).await;
    }
}

pub async fn delete_server_record(server_id: &str) -> SuccessResult {
    let servers = match try_load_server_list().await {
        Ok(servers) => servers,
        Err(e) => return SuccessResult::err(e),
    };
    let Some(server) = servers.iter().find(|s| s.id == server_id) else {
        return SuccessResult::err("Server not found");
    };

    if !matches!(
        server.status,
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle
    ) {
        return SuccessResult::err("Stop the server before deleting it");
    }

    let _ = tokio::fs::remove_dir_all(&server.server_path).await;

    let remaining: Vec<ServerRecord> = servers
        .iter()
        .filter(|s| s.id != server_id)
        .cloned()
        .collect();
    match save_server_list(&remaining).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e),
    }
}

pub async fn get_servers() -> Vec<ServerRecord> {
    load_server_list().await
}

pub async fn get_server(id: String) -> Option<ServerRecord> {
    load_server_list().await.into_iter().find(|s| s.id == id)
}

pub async fn delete_server(server_id: String) -> SuccessResult {
    if crate::server_runner::is_running(&server_id) {
        let stopped = crate::server_runner::stop_server(server_id.clone()).await;
        if !stopped.success {
            return stopped;
        }
    } else {
        crate::server_runner::refresh_server_statuses().await;
    }
    delete_server_record(&server_id).await
}

async fn apply_settings(id: &str, settings: ServerSettingsUpdate) -> Result<(), String> {
    let mut servers = try_load_server_list().await?;
    let server = servers
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("Server not found")?;

    if let Some(ram) = settings.ram_mb {
        let safe_ram = clamp_ram(ram);
        server.ram_mb = safe_ram;
        let jar = server.jar_file.as_deref().unwrap_or("server.jar");
        tokio::fs::write(
            Path::new(&server.server_path).join("start.bat"),
            generate_start_bat(safe_ram, jar),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(java_path) = settings.java_path {
        server.java_path = Some(java_path);
    }
    if let Some(backup_config) = settings.backup_config {
        server.backup_config = Some(backup_config);
    }
    if let Some(use_ngrok) = settings.use_ngrok {
        server.use_ngrok = Some(use_ngrok);
    }
    if let Some(ngrok_url) = settings.ngrok_url {
        server.ngrok_url = if ngrok_url.is_empty() {
            None
        } else {
            Some(ngrok_url)
        };
    }
    if let Some(name) = settings.name {
        server.name = name;
    }
    if let Some(auto_restart) = settings.auto_restart {
        server.auto_restart = Some(auto_restart);
    }
    if let Some(times) = settings.restart_times {
        server.restart_times = crate::schedule::parse_times(&times)?;
    }
    if let Some(start_on_launch) = settings.start_on_launch {
        server.start_on_launch = start_on_launch;
    }

    if let Some(enabled) = settings.analytics_enabled {
        let plugin_compatible = server.framework == "Paper" || server.framework == "Purpur";
        if enabled && !plugin_compatible {
            return Err(format!(
                "MonolithAnalytics requires Paper or Purpur (this server runs {})",
                server.framework
            ));
        }
        let server_path = PathBuf::from(&server.server_path);
        if enabled {
            if !install_analytics_plugin(&server_path).await {
                return Err("MonolithAnalytics plugin file could not be found or copied".into());
            }
        } else {
            uninstall_analytics_plugin(&server_path).await;
        }
        server.analytics_enabled = Some(enabled);
    }

    save_server_list(&servers).await
}

pub async fn update_server_settings(id: String, settings: ServerSettingsUpdate) -> SuccessResult {
    match apply_settings(&id, settings).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e),
    }
}
