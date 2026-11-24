use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::events::{self, Event};
use crate::safety::{is_path_within, sanitize_download_file_name};
use crate::server_runner;
use crate::server_store;
use crate::types::{
    BackupCompletedEvent, BackupConfig, BackupEntry, BackupProgressEvent, BackupStage,
    BackupStatus, BackupType, CancelBackupResult, CreateBackupResult, ServerSettingsUpdate,
    SuccessResult,
};

const BACKUP_DIR_NAME: &str = "backups";

static ACTIVE: Lazy<Mutex<HashMap<String, (Arc<AtomicBool>, bool)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static STATUSES: Lazy<Mutex<HashMap<String, BackupStatus>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn iso(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn send_progress(
    server_id: &str,
    notify: bool,
    percent: f64,
    stage: Option<&str>,
    processed: Option<u64>,
    total: Option<u64>,
) {
    if notify {
        events::emit(Event::BackupProgress(BackupProgressEvent {
            server_id: server_id.to_string(),
            percent,
            stage: stage.map(str::to_string),
            processed_files: processed,
            total_files: total,
            error: None,
        }));
    }
    if let Ok(mut statuses) = STATUSES.lock() {
        if let Some(status) = statuses.get_mut(server_id) {
            status.percent = percent;
            match stage {
                Some("calculating") => status.stage = BackupStage::Calculating,
                Some("archiving") => status.stage = BackupStage::Archiving,
                Some("complete") => status.stage = BackupStage::Complete,
                _ => {}
            }
        }
    }
}

fn send_error(server_id: &str, notify: bool, error: &str) {
    if !notify {
        return;
    }
    events::emit(Event::BackupProgress(BackupProgressEvent {
        server_id: server_id.to_string(),
        percent: -1.0,
        error: Some(error.to_string()),
        ..Default::default()
    }));
}

fn is_in_progress(server_id: &str) -> bool {
    ACTIVE
        .lock()
        .map(|a| a.contains_key(server_id))
        .unwrap_or(false)
}

fn release(server_id: &str) {
    if let Ok(mut active) = ACTIVE.lock() {
        active.remove(server_id);
    }
    if let Ok(mut statuses) = STATUSES.lock() {
        statuses.remove(server_id);
    }
}

fn run_archive(
    server_path: &Path,
    backups_dir: &Path,
    zip_path: &Path,
    server_id: &str,
    notify: bool,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    let walker = WalkDir::new(server_path)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| !entry.path().starts_with(backups_dir));
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("[BACKUP] Unreadable path during walk: {e}");
                continue;
            }
        };
        let Ok(meta) = entry.metadata() else {
            eprintln!(
                "[BACKUP] Unreadable file during walk: {}",
                entry.path().display()
            );
            continue;
        };
        if !meta.is_dir() {
            files.push((entry.path().to_path_buf(), meta.len()));
        }
    }
    let total_size: u64 = files.iter().map(|(_, size)| size).sum();
    let total_files = files.len() as u64;

    let output = File::create(zip_path).map_err(|e| format!("Output stream error: {e}"))?;
    let mut writer = ZipWriter::new(output);
    let base_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));

    send_progress(server_id, notify, 5.0, Some("archiving"), None, None);

    let mut processed: u64 = 0;
    let mut archived: u64 = 0;
    let mut last_update = 0.0_f64;
    let mut buffer = vec![0u8; 64 * 1024];

    for (path, size) in &files {
        if cancel.load(Ordering::Relaxed) {
            return Err("Backup cancelled".to_string());
        }
        let Ok(relative) = path.strip_prefix(server_path) else {
            continue;
        };
        let name = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let mut input = match File::open(path) {
            Ok(input) => input,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("[BACKUP] File vanished, skipping: {}", path.display());
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };

        writer
            .start_file(name, base_options.large_file(*size >= 0xFFFF_FFFF))
            .map_err(|e| e.to_string())?;
        archived += 1;

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Backup cancelled".to_string());
            }
            let read = match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(e.to_string()),
            };
            writer
                .write_all(&buffer[..read])
                .map_err(|e| e.to_string())?;
            processed += read as u64;

            let percent = if total_size > 0 {
                ((processed as f64 / total_size as f64) * 95.0)
                    .round()
                    .min(95.0)
            } else {
                0.0
            };
            if percent - last_update >= 2.0 || percent >= 95.0 {
                last_update = percent;
                send_progress(
                    server_id,
                    notify,
                    percent,
                    Some("archiving"),
                    Some(archived),
                    Some(total_files),
                );
            }
        }
    }

    let mut output = writer.finish().map_err(|e| e.to_string())?;
    output.flush().map_err(|e| e.to_string())?;
    drop(output);

    send_progress(server_id, notify, 100.0, Some("complete"), None, None);
    fs::metadata(zip_path)
        .map(|m| m.len())
        .map_err(|e| e.to_string())
}

pub async fn create_backup_of_type(
    server_id: &str,
    name: Option<&str>,
    kind: BackupType,
) -> CreateBackupResult {
    create_backup_inner(server_id, name, kind, false).await
}

async fn create_backup_inner(
    server_id: &str,
    name: Option<&str>,
    kind: BackupType,
    notify: bool,
) -> CreateBackupResult {
    let failure = |error: String| CreateBackupResult {
        success: false,
        error: Some(error),
        ..Default::default()
    };

    if is_in_progress(server_id) {
        return failure("A backup is already in progress for this server".to_string());
    }

    let Some(server) = server_store::get_server(server_id.to_string()).await else {
        return failure("Server not found".to_string());
    };

    let server_path = PathBuf::from(&server.server_path);
    let backups_dir = server_path.join(BACKUP_DIR_NAME);
    let prepared = {
        let server_path = server_path.clone();
        let backups_dir = backups_dir.clone();
        tokio::task::spawn_blocking(move || {
            if server_path.as_os_str().is_empty() || !server_path.exists() {
                return None;
            }
            Some(fs::create_dir_all(&backups_dir))
        })
        .await
    };
    match prepared {
        Ok(Some(Ok(()))) => {}
        Ok(Some(Err(e))) => {
            send_error(server_id, notify, &e.to_string());
            return failure(e.to_string());
        }
        Ok(None) => return failure("Server path is invalid or missing".to_string()),
        Err(e) => return failure(e.to_string()),
    }

    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string();
    let trimmed = name.map(str::trim).filter(|n| !n.is_empty());
    let type_str = match kind {
        BackupType::Manual => "manual",
        BackupType::Auto => "auto",
    };
    let display_name = match trimmed {
        Some(n) => n.to_string(),
        None if kind == BackupType::Auto => "Automatic Backup".to_string(),
        None => "Manual Backup".to_string(),
    };
    let safe_name = match (trimmed, name) {
        (Some(_), Some(raw)) => raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_string()
                } else {
                    "_".repeat(c.len_utf16())
                }
            })
            .collect::<String>(),
        _ => type_str.to_string(),
    };
    let filename = format!("backup-{timestamp}-{safe_name}.zip");
    let zip_path = backups_dir.join(&filename);

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut active) = ACTIVE.lock() else {
            return failure("Backup state unavailable".to_string());
        };
        if active.contains_key(server_id) {
            return failure("A backup is already in progress for this server".to_string());
        }
        active.insert(server_id.to_string(), (cancel.clone(), notify));
    }
    if let Ok(mut statuses) = STATUSES.lock() {
        statuses.insert(
            server_id.to_string(),
            BackupStatus {
                server_id: server_id.to_string(),
                in_progress: true,
                percent: 0.0,
                stage: BackupStage::Calculating,
                error: None,
            },
        );
    }
    send_progress(server_id, notify, 0.0, Some("calculating"), None, None);

    let joined = {
        let server_id = server_id.to_string();
        let server_path = server_path.clone();
        let backups_dir = backups_dir.clone();
        let zip_path = zip_path.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            run_archive(
                &server_path,
                &backups_dir,
                &zip_path,
                &server_id,
                notify,
                &cancel,
            )
        })
        .await
    };
    let outcome = joined.unwrap_or_else(|e| Err(format!("Backup worker exited: {e}")));

    match outcome {
        Ok(size) => {
            if let Some(config) = &server.backup_config {
                prune(&backups_dir, config.keep_last, config.keep_days);
            }
            let backup = BackupEntry {
                name: display_name,
                filename,
                path: zip_path.to_string_lossy().into_owned(),
                size,
                created_at: iso(Utc::now()),
                kind,
            };
            if notify {
                events::emit(Event::BackupCompleted(BackupCompletedEvent {
                    server_id: server_id.to_string(),
                    backup: backup.clone(),
                }));
            }
            release(server_id);
            CreateBackupResult {
                success: true,
                error: None,
                backup: Some(backup),
                started: Some(true),
            }
        }
        Err(error) => {
            if !cancel.load(Ordering::Relaxed) {
                send_error(server_id, notify, &error);
            }
            release(server_id);
            let _ = tokio::fs::remove_file(&zip_path).await;
            failure(error)
        }
    }
}

pub async fn check_and_run_auto_backups() {
    for server in server_store::load_server_list().await {
        let Some(config) = server.backup_config.clone() else {
            continue;
        };
        if !config.enabled {
            continue;
        }
        let last = match config.last_backup_at.as_deref() {
            Some(s) => match DateTime::parse_from_rfc3339(s) {
                Ok(d) => d.timestamp_millis(),
                Err(_) => continue,
            },
            None => 0,
        };
        let interval_ms = config.interval_hours * 60.0 * 60.0 * 1000.0;
        let elapsed = (Utc::now().timestamp_millis() - last) as f64;
        if elapsed <= interval_ms || is_in_progress(&server.id) {
            continue;
        }

        let result = create_backup_inner(&server.id, None, BackupType::Auto, true).await;
        if !result.success {
            continue;
        }
        let current = server_store::get_server(server.id.clone())
            .await
            .and_then(|s| s.backup_config)
            .unwrap_or(config);
        server_store::update_server_settings(
            server.id.clone(),
            ServerSettingsUpdate {
                backup_config: Some(BackupConfig {
                    last_backup_at: Some(iso(Utc::now())),
                    ..current
                }),
                ..Default::default()
            },
        )
        .await;
    }
}

pub async fn create_backup(server_id: String, name: Option<String>) -> CreateBackupResult {
    create_backup_inner(&server_id, name.as_deref(), BackupType::Manual, true).await
}

fn parse_backup(path: &Path, file: &str) -> Option<BackupEntry> {
    let meta = fs::metadata(path).ok()?;
    let created: DateTime<Utc> = meta.created().or_else(|_| meta.modified()).ok()?.into();

    let (kind, name) = if file.contains("-auto.zip") {
        (BackupType::Auto, "Automatic Backup".to_string())
    } else if let Some(rest) = file.strip_prefix("backup-") {
        let start = rest
            .get(20..)
            .and_then(|s| s.find('-'))
            .map(|i| i + 21)
            .unwrap_or(0);
        (
            BackupType::Manual,
            rest.get(start..).unwrap_or(rest).replacen(".zip", "", 1),
        )
    } else {
        (BackupType::Manual, file.to_string())
    };

    Some(BackupEntry {
        name: name.replace('_', " "),
        filename: file.to_string(),
        path: path.to_string_lossy().into_owned(),
        size: meta.len(),
        created_at: iso(created),
        kind,
    })
}

fn expired(
    backups: &[BackupEntry],
    keep_last: Option<u32>,
    keep_days: Option<u32>,
    now: DateTime<Utc>,
) -> Vec<String> {
    let keep_last = keep_last.filter(|n| *n > 0).map(|n| n as usize);
    let cutoff = keep_days
        .filter(|d| *d > 0)
        .map(|d| now - chrono::Duration::days(i64::from(d)));
    let mut autos: Vec<&BackupEntry> = backups
        .iter()
        .filter(|b| b.kind == BackupType::Auto)
        .collect();
    autos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    autos
        .into_iter()
        .enumerate()
        .filter(|(index, backup)| {
            keep_last.is_some_and(|n| *index >= n)
                || cutoff.is_some_and(|cutoff| {
                    DateTime::parse_from_rfc3339(&backup.created_at)
                        .is_ok_and(|created| created < cutoff)
                })
        })
        .map(|(_, backup)| backup.filename.clone())
        .collect()
}

fn prune(dir: &Path, keep_last: Option<u32>, keep_days: Option<u32>) {
    for filename in expired(&list_backups(dir), keep_last, keep_days, Utc::now()) {
        if let Err(e) = fs::remove_file(dir.join(&filename)) {
            eprintln!("[BACKUP] Could not remove old backup {filename}: {e}");
        }
    }
}

pub async fn get_backups(server_id: String) -> Vec<BackupEntry> {
    let Some(server) = server_store::get_server(server_id).await else {
        return Vec::new();
    };
    list_backups(&Path::new(&server.server_path).join(BACKUP_DIR_NAME))
}

fn list_backups(backups_dir: &Path) -> Vec<BackupEntry> {
    let Ok(read) = fs::read_dir(backups_dir) else {
        return Vec::new();
    };

    let mut backups: Vec<BackupEntry> = read
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            if !file.ends_with(".zip") {
                return None;
            }
            parse_backup(&entry.path(), &file)
        })
        .collect();
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    backups
}

fn resolve_backup_file(server_path: &str, filename: &str) -> Result<(PathBuf, PathBuf), String> {
    let safe = sanitize_download_file_name(filename).map_err(|_| "Invalid filename".to_string())?;
    let backups_dir = Path::new(server_path).join(BACKUP_DIR_NAME);
    let file_path = backups_dir.join(safe);
    if !is_path_within(&file_path, &backups_dir) || file_path == backups_dir {
        return Err("Access denied".to_string());
    }
    Ok((backups_dir, file_path))
}

pub async fn delete_backup(server_id: String, filename: String) -> SuccessResult {
    let Some(server) = server_store::get_server(server_id).await else {
        return SuccessResult::err("Server not found");
    };
    let (_, file_path) = match resolve_backup_file(&server.server_path, &filename) {
        Ok(v) => v,
        Err(e) => return SuccessResult::err(&e),
    };
    if !file_path.exists() {
        return SuccessResult::err("File not found");
    }
    match tokio::fs::remove_file(&file_path).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e.to_string()),
    }
}

fn entry_escapes(entry_name: &str) -> bool {
    let path = Path::new(entry_name);
    if path.has_root() {
        return true;
    }
    let mut depth: i64 = 0;
    for component in path.components() {
        match component {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::Prefix(_) | Component::RootDir => return true,
            Component::CurDir => {}
        }
        if depth < 0 {
            return true;
        }
    }
    false
}

fn validate_entries(zip_path: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
        if entry_escapes(&entry.name().replace('\\', "/")) {
            return Err(format!("Malicious zip entry detected: {}", entry.name()));
        }
    }
    Ok(())
}

fn force_remove(path: &Path) -> std::io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_dir() {
        return fs::remove_dir_all(path);
    }
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    let mut permissions = meta.permissions();
    permissions.set_readonly(false);
    let _ = fs::set_permissions(path, permissions);
    fs::remove_file(path).or_else(|e| {
        if meta.file_type().is_symlink() {
            fs::remove_dir(path)
        } else {
            Err(e)
        }
    })
}

fn restore_from_zip(
    zip_path: &Path,
    server_path: &Path,
    backups_dir: &Path,
    staging: &Path,
) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    archive.extract(staging).map_err(|e| e.to_string())?;

    for entry in fs::read_dir(server_path)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let full = entry.path();
        if full == backups_dir {
            continue;
        }
        force_remove(&full).map_err(|e| e.to_string())?;
    }
    for entry in fs::read_dir(staging).map_err(|e| e.to_string())?.flatten() {
        fs::rename(entry.path(), server_path.join(entry.file_name())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn restore_backup(server_id: String, filename: String) -> SuccessResult {
    let Some(server) = server_store::get_server(server_id.clone()).await else {
        return SuccessResult::err("Server not found");
    };
    if server_runner::is_running(&server_id) {
        return SuccessResult::err("Stop the server before restoring a backup");
    }
    let (backups_dir, zip_path) = match resolve_backup_file(&server.server_path, &filename) {
        Ok(v) => v,
        Err(e) => return SuccessResult::err(&e),
    };
    if !zip_path.exists() {
        return SuccessResult::err("Backup file not found");
    }

    let server_path = PathBuf::from(&server.server_path);
    let base = server_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = server_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let staging = parent.join(format!(".{base}-restore-{}", Utc::now().timestamp_millis()));

    let validated = {
        let zip_path = zip_path.clone();
        tokio::task::spawn_blocking(move || validate_entries(&zip_path)).await
    };
    match validated {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return SuccessResult::err(&e),
        Err(e) => return SuccessResult::err(e.to_string()),
    }

    let stamp = Utc::now().format("%Y-%m-%d %H:%M").to_string();
    let safety = create_backup_of_type(
        &server_id,
        Some(&format!("Before restore {stamp}")),
        BackupType::Manual,
    )
    .await;
    if !safety.success {
        let reason = safety.error.unwrap_or_else(|| "Unknown error".to_string());
        return SuccessResult::err(format!(
            "Could not create the pre-restore safety backup: {reason}"
        ));
    }

    let work = {
        let staging = staging.clone();
        tokio::task::spawn_blocking(move || {
            restore_from_zip(&zip_path, &server_path, &backups_dir, &staging)
        })
        .await
    };
    let _ = tokio::fs::remove_dir_all(&staging).await;

    match work {
        Ok(Ok(())) => SuccessResult::ok(),
        Ok(Err(e)) => SuccessResult::err(&e),
        Err(e) => SuccessResult::err(e.to_string()),
    }
}

pub async fn cancel_backup(server_id: String) -> CancelBackupResult {
    let flag = ACTIVE.lock().ok().and_then(|a| a.get(&server_id).cloned());
    match flag {
        Some((flag, notify)) => {
            flag.store(true, Ordering::Relaxed);
            send_error(&server_id, notify, "Backup cancelled");
            CancelBackupResult { success: true }
        }
        None => CancelBackupResult { success: false },
    }
}

pub async fn get_backup_status(server_id: String) -> Option<BackupStatus> {
    STATUSES
        .lock()
        .ok()
        .and_then(|s| s.get(&server_id).cloned())
}

pub async fn is_backup_in_progress(server_id: String) -> bool {
    is_in_progress(&server_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(filename: &str, days_ago: i64, kind: BackupType) -> BackupEntry {
        BackupEntry {
            name: filename.to_string(),
            filename: filename.to_string(),
            path: String::new(),
            size: 0,
            created_at: iso(Utc::now() - chrono::Duration::days(days_ago)),
            kind,
        }
    }

    #[test]
    fn retention_keeps_the_newest_automatic_backups() {
        let backups = vec![
            entry("a", 3, BackupType::Auto),
            entry("b", 1, BackupType::Auto),
            entry("c", 2, BackupType::Auto),
            entry("m", 9, BackupType::Manual),
        ];
        assert_eq!(expired(&backups, Some(2), None, Utc::now()), vec!["a"]);
    }

    #[test]
    fn retention_drops_automatic_backups_older_than_the_limit() {
        let backups = vec![
            entry("old", 10, BackupType::Auto),
            entry("new", 1, BackupType::Auto),
            entry("manual", 30, BackupType::Manual),
        ];
        assert_eq!(expired(&backups, None, Some(7), Utc::now()), vec!["old"]);
    }

    #[test]
    fn retention_without_limits_keeps_everything() {
        let backups = vec![entry("a", 100, BackupType::Auto)];
        assert!(expired(&backups, None, None, Utc::now()).is_empty());
        assert!(expired(&backups, Some(0), Some(0), Utc::now()).is_empty());
    }
}
