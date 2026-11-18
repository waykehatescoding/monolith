use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::events::Event;
use crate::types::{
    ChangelogEntry, DownloadUpdateProgress, DownloadUpdateResult, SuccessResult, UpdateCheckResult,
};

const RELEASES_API: &str = "https://api.github.com/repos/waykehatescoding/monolith/releases/latest";
const RELEASES_URL: &str = "https://github.com/waykehatescoding/monolith/releases";
const USER_AGENT: &str = concat!("Monolith/", env!("CARGO_PKG_VERSION"));

static VERIFIED_INSTALLER: Lazy<std::sync::Mutex<Option<(PathBuf, String)>>> =
    Lazy::new(|| std::sync::Mutex::new(None));
static VERSION_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"\d+(?:\.\d+)*(?:[-+][0-9A-Za-z.-]+)?").ok());
static DIGEST_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r"(?i)^sha256:([0-9a-f]{64})$").ok());

fn net_err(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

#[derive(Debug, Default, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GitHubRelease {
    tag_name: Option<String>,
    name: Option<String>,
    html_url: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

impl GitHubRelease {
    fn installer(&self) -> Option<&GitHubAsset> {
        self.assets
            .iter()
            .find(|a| a.name.starts_with("Monolith-Setup-") && a.name.ends_with(".exe"))
    }
}

async fn fetch_latest() -> Result<GitHubRelease, String> {
    let response = reqwest::Client::new()
        .get(RELEASES_API)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(net_err)?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch latest release: HTTP {}",
            response.status().as_u16()
        ));
    }
    response.json().await.map_err(net_err)
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.is_empty())
}

fn normalize_version(version: &str) -> String {
    let normalized = version.trim().trim_start_matches(['v', 'V']);
    VERSION_RE
        .as_ref()
        .and_then(|re| re.find(normalized))
        .map(|m| m.as_str())
        .unwrap_or(normalized)
        .to_string()
}

fn version_parts(version: &str) -> Vec<u64> {
    let normalized = normalize_version(version);
    let core = normalized.split(['+', '-']).next().unwrap_or("");
    core.split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

pub fn compare_versions(v1: &str, v2: &str) -> i32 {
    let a = version_parts(v1);
    let b = version_parts(v2);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x > y {
            return 1;
        }
        if x < y {
            return -1;
        }
    }
    0
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn build_changelog_entry(release: &GitHubRelease, latest_version: &str) -> Option<ChangelogEntry> {
    let notes = release.body.as_deref().map(str::trim).unwrap_or("");
    if notes.is_empty() {
        return None;
    }
    Some(ChangelogEntry {
        version: latest_version.to_string(),
        date: release
            .published_at
            .as_deref()
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(10).collect())
            .unwrap_or_default(),
        title: non_empty(&release.name)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Monolith {latest_version}")),
        changes: notes
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
    })
}

fn build_update_check_result(release: &GitHubRelease, current: &str) -> UpdateCheckResult {
    let latest_version = normalize_version(
        non_empty(&release.tag_name)
            .or_else(|| non_empty(&release.name))
            .unwrap_or(current),
    );
    let release_notes = release.body.as_deref().map(|b| b.trim().to_string());
    let update_available = compare_versions(&latest_version, current) > 0;
    let changelog = if update_available {
        build_changelog_entry(release, &latest_version).map(|e| vec![e])
    } else {
        None
    };
    UpdateCheckResult {
        update_available,
        latest_version,
        current_version: current.to_string(),
        release_url: non_empty(&release.html_url)
            .unwrap_or(RELEASES_URL)
            .to_string(),
        release_name: release.name.clone(),
        published_at: release.published_at.clone(),
        release_notes,
        changelog,
        error: None,
    }
}

fn failed_check(current: &str, error: String) -> UpdateCheckResult {
    UpdateCheckResult {
        update_available: false,
        latest_version: current.to_string(),
        current_version: current.to_string(),
        release_url: RELEASES_URL.to_string(),
        error: Some(error),
        ..Default::default()
    }
}

pub async fn check_for_updates() -> UpdateCheckResult {
    let current = current_version();
    match fetch_latest().await {
        Ok(release) => build_update_check_result(&release, &current),
        Err(e) => failed_check(&current, e),
    }
}

fn sha256_hex_of(digest: Option<&str>) -> Option<String> {
    DIGEST_RE
        .as_ref()?
        .captures(digest?)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_lowercase())
}

fn download_failed(error: impl Into<String>) -> DownloadUpdateResult {
    DownloadUpdateResult {
        success: false,
        file_path: None,
        error: Some(error.into()),
    }
}

async fn download_installer() -> Result<(PathBuf, String), String> {
    let release = fetch_latest().await?;
    let Some(asset) = release.installer() else {
        return Err("No installer available for the latest release".to_string());
    };
    let Some(expected_hash) = sha256_hex_of(asset.digest.as_deref()) else {
        return Err(
            "Latest release has no verifiable checksum; download it manually from GitHub"
                .to_string(),
        );
    };

    let response = reqwest::Client::new()
        .get(&asset.browser_download_url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(net_err)?;
    if !response.status().is_success() {
        return Err(format!(
            "Download failed: HTTP {}",
            response.status().as_u16()
        ));
    }

    let file_name = Path::new(&asset.name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Monolith-Setup.exe".to_string());
    let dest = crate::paths::temp_dir().join(file_name);
    let bytes_total = response.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut bytes_downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    let streamed: Result<(), String> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(net_err)?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            hasher.update(&chunk);
            bytes_downloaded += chunk.len() as u64;
            let percent = if bytes_total > 0 {
                (bytes_downloaded as f64 / bytes_total as f64 * 100.0).round()
            } else {
                0.0
            };
            crate::events::emit(Event::UpdateDownloadProgress(DownloadUpdateProgress {
                percent,
                bytes_downloaded,
                bytes_total,
            }));
        }
        file.flush().await.map_err(|e| e.to_string())
    }
    .await;
    drop(file);
    streamed?;

    if hex::encode(hasher.finalize()) != expected_hash {
        let _ = tokio::fs::remove_file(&dest).await;
        return Err("Downloaded installer failed integrity check and was discarded".to_string());
    }
    Ok((dest, expected_hash))
}

pub async fn download_update() -> DownloadUpdateResult {
    if let Ok(mut guard) = VERIFIED_INSTALLER.lock() {
        *guard = None;
    }
    match download_installer().await {
        Ok((path, hash)) => {
            if let Ok(mut guard) = VERIFIED_INSTALLER.lock() {
                *guard = Some((path.clone(), hash));
            }
            DownloadUpdateResult {
                success: true,
                file_path: Some(path.to_string_lossy().to_string()),
                error: None,
            }
        }
        Err(e) => download_failed(e),
    }
}

pub async fn install_update() -> SuccessResult {
    let path = VERIFIED_INSTALLER.lock().ok().and_then(|g| g.clone());
    let Some((path, expected_hash)) = path else {
        return SuccessResult::err("No verified installer to run. Download it first.");
    };
    let current_hash = tokio::fs::read(&path)
        .await
        .map(|bytes| hex::encode(Sha256::digest(&bytes)));
    if current_hash.ok().as_deref() != Some(expected_hash.as_str()) {
        let _ = tokio::fs::remove_file(&path).await;
        return SuccessResult::err("Installer changed after verification and was discarded");
    }

    let mut command = Command::new(&path);
    command
        .arg("--update")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    if let Err(e) = command.spawn() {
        return SuccessResult::err(e.to_string());
    }

    tokio::spawn(async {
        crate::lifecycle::shutdown().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::process::exit(0);
    });
    SuccessResult::ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_version_tags() {
        assert_eq!(normalize_version("v1.3.0"), "1.3.0");
        assert_eq!(normalize_version("  release-2.0.1  "), "2.0.1");
    }

    #[test]
    fn compares_versions_with_prerelease_suffixes() {
        assert_eq!(compare_versions("1.3.0", "1.2.9"), 1);
        assert_eq!(compare_versions("1.2.0", "1.2.0"), 0);
        assert_eq!(compare_versions("1.2.0-beta.1", "1.2.0"), 0);
    }

    #[test]
    fn maps_newer_release_to_update_result() {
        let result = build_update_check_result(
            &GitHubRelease {
                tag_name: Some("v1.3.0".into()),
                name: Some("Monolith 1.3.0".into()),
                html_url: Some(
                    "https://github.com/waykehatescoding/monolith/releases/tag/v1.3.0".into(),
                ),
                published_at: Some("2026-06-13T10:00:00Z".into()),
                body: Some("- Added safer shutdown\n- Improved update checks".into()),
                ..Default::default()
            },
            "1.2.0",
        );
        assert!(result.update_available);
        assert_eq!(result.latest_version, "1.3.0");
        assert_eq!(result.current_version, "1.2.0");
        assert_eq!(result.release_name.as_deref(), Some("Monolith 1.3.0"));
        assert_eq!(result.published_at.as_deref(), Some("2026-06-13T10:00:00Z"));
        assert_eq!(
            result.release_url,
            "https://github.com/waykehatescoding/monolith/releases/tag/v1.3.0"
        );
        assert_eq!(
            result.release_notes.as_deref(),
            Some("- Added safer shutdown\n- Improved update checks")
        );
        let changelog = result.changelog.unwrap_or_default();
        assert_eq!(changelog[0].date, "2026-06-13");
        assert_eq!(changelog[0].changes.len(), 2);
    }

    #[test]
    fn current_release_is_not_an_update() {
        let result = build_update_check_result(
            &GitHubRelease {
                tag_name: Some("v1.2.0".into()),
                html_url: Some(
                    "https://github.com/waykehatescoding/monolith/releases/tag/v1.2.0".into(),
                ),
                ..Default::default()
            },
            "1.2.0",
        );
        assert!(!result.update_available);
        assert_eq!(result.latest_version, "1.2.0");
        assert_eq!(result.current_version, "1.2.0");
        assert!(result.changelog.is_none());
    }

    #[test]
    fn picks_installer_asset() {
        let release: GitHubRelease = serde_json::from_str(
            r#"{"assets":[{"name":"Monolith-1.0.0.zip","browser_download_url":"u1"},{"name":"Monolith-Setup-1.0.0.exe","browser_download_url":"u2","digest":"sha256:x"}]}"#,
        )
        .unwrap_or_default();
        assert_eq!(
            release.installer().map(|a| a.browser_download_url.as_str()),
            Some("u2")
        );
    }

    #[test]
    fn parses_sha256_digest() {
        let hex = "a".repeat(64);
        assert_eq!(sha256_hex_of(Some(&format!("sha256:{hex}"))), Some(hex));
        assert_eq!(sha256_hex_of(Some("sha1:abc")), None);
    }
}
