use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use flate2::read::GzDecoder;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

use crate::server_runner::re;
use crate::types::{ConsoleLine, ConsoleLineType};

const UPLOAD_URL: &str = "https://api.mclo.gs/1/log";
const MAX_UPLOAD_LINES: usize = 25_000;
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;

static CODES: Lazy<Regex> = Lazy::new(|| re(r"\x1b\[[0-9;]*[A-Za-z]|§[0-9a-fk-orx]"));

pub fn plain_text(text: &str) -> String {
    CODES.replace_all(text, "").into_owned()
}

#[derive(Debug, Clone)]
pub struct LogFile {
    pub name: String,
    pub path: PathBuf,
    pub modified: DateTime<Local>,
    pub size: u64,
    pub crash_report: bool,
}

pub async fn list_logs(server_path: PathBuf) -> Vec<LogFile> {
    tokio::task::spawn_blocking(move || {
        let mut files = list_dir(&server_path.join("crash-reports"), &[".txt"], true);
        files.extend(list_dir(
            &server_path.join("logs"),
            &[".log", ".log.gz"],
            false,
        ));
        files
    })
    .await
    .unwrap_or_default()
}

fn list_dir(dir: &Path, extensions: &[&str], crash_report: bool) -> Vec<LogFile> {
    let mut files: Vec<LogFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().ok()?;
            if !meta.is_file() || !extensions.iter().any(|ext| name.ends_with(ext)) {
                return None;
            }
            Some(LogFile {
                name,
                path: entry.path(),
                modified: meta.modified().ok()?.into(),
                size: meta.len(),
                crash_report,
            })
        })
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.modified));
    files
}

pub async fn read_log(path: PathBuf) -> Result<Vec<ConsoleLine>, String> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        let result = if path.extension().is_some_and(|ext| ext == "gz") {
            GzDecoder::new(file)
                .take(MAX_READ_BYTES)
                .read_to_end(&mut bytes)
        } else {
            file.take(MAX_READ_BYTES).read_to_end(&mut bytes)
        };
        result.map_err(|e| e.to_string())?;
        Ok(to_lines(&String::from_utf8_lossy(&bytes)))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn to_lines(text: &str) -> Vec<ConsoleLine> {
    text.lines()
        .map(|line| ConsoleLine {
            timestamp: String::new(),
            text: line.to_string(),
            kind: ConsoleLineType::Stdout,
        })
        .collect()
}

#[derive(Deserialize)]
struct UploadReply {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub async fn upload_log(server_path: PathBuf, console: Vec<String>) -> Result<String, String> {
    let content = match tokio::fs::read(server_path.join("logs").join("latest.log")).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => console.join("\n"),
    };
    let lines: Vec<&str> = content.lines().collect();
    let content = plain_text(&lines[lines.len().saturating_sub(MAX_UPLOAD_LINES)..].join("\n"));
    if content.trim().is_empty() {
        return Err("There is no log to upload yet.".into());
    }
    let reply: UploadReply = reqwest::Client::new()
        .post(UPLOAD_URL)
        .form(&[("content", content)])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    match reply {
        UploadReply { url: Some(url), .. } => Ok(url),
        UploadReply { error, .. } => {
            Err(error.unwrap_or_else(|| "mclo.gs rejected the log.".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(plain_text("\x1b[33m§cHello§r world\x1b[0m"), "Hello world");
    }

    #[test]
    fn lists_and_reads_crash_reports() {
        let root = std::env::temp_dir().join(format!("monolith-logs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("crash-reports")).unwrap();
        std::fs::write(root.join("crash-reports/crash-1.txt"), "boom\nat foo").unwrap();
        std::fs::write(root.join("crash-reports/notes.md"), "x").unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let files = runtime.block_on(list_logs(root.clone()));
        assert_eq!(files.len(), 1);
        assert!(files[0].crash_report);
        let lines = runtime.block_on(read_log(files[0].path.clone())).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "at foo");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_and_reads_compressed_logs() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("monolith-logs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs/latest.log"), "a\nb\n").unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"old\nlines\nhere").unwrap();
        std::fs::write(root.join("logs/2024-01-01-1.log.gz"), gz.finish().unwrap()).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let files = runtime.block_on(list_logs(root.clone()));
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| !file.crash_report));
        let old = runtime
            .block_on(read_log(root.join("logs/2024-01-01-1.log.gz")))
            .unwrap();
        assert_eq!(old.len(), 3);
        assert_eq!(old[2].text, "here");
        std::fs::remove_dir_all(root).unwrap();
    }
}
