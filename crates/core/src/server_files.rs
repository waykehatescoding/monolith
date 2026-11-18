use crate::safety::{is_path_within, resolve_safe_server_path, sanitize_download_file_name};
use crate::server_store::load_server_list;
use crate::types::{FileEntry, ReadServerFileResult, SuccessResult};
use chrono::{DateTime, SecondsFormat, Utc};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_FILE_READ_SIZE: u64 = 2 * 1024 * 1024;

async fn server_root(server_id: &str) -> Option<PathBuf> {
    load_server_list()
        .await
        .into_iter()
        .find(|s| s.id == server_id)
        .map(|s| PathBuf::from(s.server_path))
}

fn fail(error: impl ToString) -> SuccessResult {
    SuccessResult::err(&error.to_string())
}

pub async fn list_server_files(server_id: String, relative_path: String) -> Vec<FileEntry> {
    let Some(root) = server_root(&server_id).await else {
        return Vec::new();
    };
    let Ok(target) = resolve_safe_server_path(&root, &relative_path, true) else {
        return Vec::new();
    };
    let Ok(mut dir) = tokio::fs::read_dir(&target).await else {
        return Vec::new();
    };
    let mut result = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        let Ok(meta) = tokio::fs::metadata(entry.path()).await else {
            continue;
        };
        let is_directory = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        let modified_at = meta
            .modified()
            .map(DateTime::<Utc>::from)
            .unwrap_or_default()
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        result.push(FileEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory,
            size: if is_directory { 0 } else { meta.len() },
            modified_at,
        });
    }
    result.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    result
}

pub async fn read_server_file(server_id: String, relative_path: String) -> ReadServerFileResult {
    let err = |e: &str| ReadServerFileResult {
        success: false,
        content: None,
        error: Some(e.to_string()),
    };
    let Some(root) = server_root(&server_id).await else {
        return err("Server not found");
    };
    let Ok(target) = resolve_safe_server_path(&root, &relative_path, false) else {
        return err("Access denied");
    };
    let meta = match tokio::fs::metadata(&target).await {
        Ok(m) => m,
        Err(e) => return err(&e.to_string()),
    };
    if meta.is_dir() {
        return err("Cannot read a directory");
    }
    if meta.len() > MAX_FILE_READ_SIZE {
        return err("File too large to edit (> 2 MB)");
    }
    match tokio::fs::read(&target).await {
        Ok(bytes) => ReadServerFileResult {
            success: true,
            content: Some(String::from_utf8_lossy(&bytes).into_owned()),
            error: None,
        },
        Err(e) => err(&e.to_string()),
    }
}

pub async fn write_server_file(
    server_id: String,
    relative_path: String,
    content: String,
) -> SuccessResult {
    let Some(root) = server_root(&server_id).await else {
        return SuccessResult::err("Server not found");
    };
    let Ok(target) = resolve_safe_server_path(&root, &relative_path, false) else {
        return SuccessResult::err("Access denied");
    };
    if content.len() as u64 > MAX_FILE_READ_SIZE {
        return SuccessResult::err("File too large to edit (> 2 MB)");
    }
    match tokio::fs::write(&target, content).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => fail(e),
    }
}

pub async fn delete_server_file(server_id: String, relative_path: String) -> SuccessResult {
    let Some(root) = server_root(&server_id).await else {
        return SuccessResult::err("Server not found");
    };
    let Ok(target) = resolve_safe_server_path(&root, &relative_path, false) else {
        return SuccessResult::err("Access denied");
    };
    let result = match tokio::fs::symlink_metadata(&target).await {
        Ok(m) if m.is_dir() => tokio::fs::remove_dir_all(&target).await,
        Ok(_) => match tokio::fs::remove_file(&target).await {
            Err(_) => tokio::fs::remove_dir(&target).await,
            ok => ok,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => SuccessResult::ok(),
        Err(e) => fail(e),
    }
}

async fn sibling_target(
    server_id: &str,
    relative_path: &str,
    new_name: &str,
) -> Result<(PathBuf, PathBuf), SuccessResult> {
    let Some(root) = server_root(server_id).await else {
        return Err(SuccessResult::err("Server not found"));
    };
    let Ok(safe_name) = sanitize_download_file_name(new_name) else {
        return Err(SuccessResult::err("Invalid file name"));
    };
    let root = std::path::absolute(&root).unwrap_or(root);
    let Ok(source) = resolve_safe_server_path(&root, relative_path, false) else {
        return Err(SuccessResult::err("Access denied"));
    };
    let dest = source.parent().unwrap_or(&root).join(safe_name);
    if !is_path_within(&dest, &root) || dest == root {
        return Err(SuccessResult::err("Access denied"));
    }
    Ok((source, dest))
}

pub async fn rename_server_file(
    server_id: String,
    relative_path: String,
    new_name: String,
) -> SuccessResult {
    let (source, dest) = match sibling_target(&server_id, &relative_path, &new_name).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match tokio::fs::rename(&source, &dest).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => fail(e),
    }
}

fn copy_recursive(source: &Path, dest: &Path) -> std::io::Result<()> {
    for entry in WalkDir::new(source) {
        let entry = entry.map_err(std::io::Error::other)?;
        let rel = entry
            .path()
            .strip_prefix(source)
            .map_err(std::io::Error::other)?;
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub async fn copy_server_file(
    server_id: String,
    relative_path: String,
    new_name: String,
) -> SuccessResult {
    let (source, dest) = match sibling_target(&server_id, &relative_path, &new_name).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let result = tokio::task::spawn_blocking(move || {
        if let (Ok(a), Ok(b)) = (std::fs::canonicalize(&source), std::fs::canonicalize(&dest)) {
            if a == b {
                return Err(std::io::Error::other("src and dest cannot be the same"));
            }
        }
        if std::fs::metadata(&source)?.is_dir() {
            copy_recursive(&source, &dest)
        } else {
            std::fs::copy(&source, &dest).map(|_| ())
        }
    })
    .await;
    match result {
        Ok(Ok(())) => SuccessResult::ok(),
        Ok(Err(e)) => fail(e),
        Err(e) => fail(e),
    }
}
