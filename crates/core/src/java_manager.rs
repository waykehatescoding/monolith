use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use regex::Regex;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::paths::runtimes_dir;
use crate::safety::is_path_within;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaInstallStage {
    Downloading,
    Extracting,
    Discovering,
}

pub type JavaProgress = Box<dyn Fn(JavaInstallStage, f64, Option<u64>, Option<u64>) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct InstalledJava {
    pub path: PathBuf,
    pub version: u32,
}

fn java_exe() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let _ = cmd;
}

pub fn get_required_java_version(mc_version: &str) -> u32 {
    if let Some(c) = Regex::new(r"^(\d{2})w\d{2}[a-z~]")
        .ok()
        .and_then(|re| re.captures(mc_version))
    {
        let year: u32 = c[1].parse().unwrap_or(0);
        return if year >= 24 {
            21
        } else if year >= 21 {
            17
        } else {
            8
        };
    }

    let caps = match Regex::new(r"^(\d+)\.(\d+)(?:\.(\d+))?")
        .ok()
        .and_then(|re| re.captures(mc_version))
    {
        Some(c) => c,
        None => return 8,
    };

    let major: u32 = caps[1].parse().unwrap_or(0);
    let minor: u32 = caps[2].parse().unwrap_or(0);
    let patch: u32 = caps
        .get(3)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);

    if major >= 26 {
        return 25;
    }
    if major != 1 {
        return 8;
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        21
    } else if minor >= 17 {
        17
    } else if minor >= 12 {
        11
    } else {
        8
    }
}

fn platform_string() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "windows"
    }
}

fn arch_string() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    }
}

fn parse_java_version(line: &str) -> Option<u32> {
    let c = Regex::new(r#"version "(\d+)"#).ok()?.captures(line)?;
    let v: u32 = c[1].parse().ok()?;
    if v == 1 {
        if let Some(s) = Regex::new(r#"version "1\.(\d+)"#)
            .ok()
            .and_then(|re| re.captures(line))
        {
            return s[1].parse().ok();
        }
    }
    Some(v)
}

pub async fn get_java_version(java_path: &Path) -> Option<u32> {
    let mut cmd = Command::new(java_path);
    cmd.arg("-version").kill_on_drop(true);
    hide_window(&mut cmd);
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_java_version(stderr.split('\n').next().unwrap_or(""))
}

async fn which_java() -> Vec<PathBuf> {
    let tool = if cfg!(windows) { "where" } else { "which" };
    let mut cmd = Command::new(tool);
    cmd.arg("java");
    hide_window(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    }
}

fn find_java_binaries(dir: &Path, found: &mut Vec<PathBuf>, depth: u32) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            find_java_binaries(&path, found, depth + 1);
        } else if ft.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name == "java" || name == "java.exe")
                && path.to_string_lossy().to_lowercase().contains("bin")
            {
                found.push(path);
            }
        }
    }
}

fn platform_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                let pf = PathBuf::from(pf);
                for sub in [
                    "Java",
                    "Eclipse Foundation",
                    "AdoptOpenJDK",
                    "Amazon Corretto",
                    "BellSoft",
                    "Zulu",
                ] {
                    dirs.push(pf.join(sub));
                }
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            dirs.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("Eclipse Foundation"),
            );
        }
    } else if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        dirs.push(PathBuf::from("/System/Library/Java/JavaVirtualMachines"));
    } else if cfg!(target_os = "linux") {
        dirs.push(PathBuf::from("/usr/lib/jvm"));
        dirs.push(PathBuf::from("/usr/java"));
    }
    dirs
}

fn find_jdk_home(dir: &Path) -> Option<PathBuf> {
    if dir.join("bin").join(java_exe()).exists() {
        return Some(dir.to_path_buf());
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Some(found) = find_jdk_home(&entry.path()) {
                return Some(found);
            }
        }
    }
    None
}

pub async fn discover_installed_javas() -> Vec<InstalledJava> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut push = |p: PathBuf, candidates: &mut Vec<PathBuf>| {
        if seen.insert(p.clone()) {
            candidates.push(p);
        }
    };

    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|v| !v.is_empty()) {
        push(
            PathBuf::from(home).join("bin").join(java_exe()),
            &mut candidates,
        );
    }
    for p in which_java().await {
        push(p, &mut candidates);
    }

    let discovered = tokio::task::spawn_blocking(|| {
        let mut bins = Vec::new();
        for base in platform_search_dirs() {
            if base.exists() {
                find_java_binaries(&base, &mut bins, 0);
            }
        }
        bins
    })
    .await
    .unwrap_or_default();
    for p in discovered {
        push(p, &mut candidates);
    }

    let mut found: Vec<InstalledJava> = Vec::new();
    for path in candidates {
        if let Some(version) = get_java_version(&path).await.filter(|v| *v > 0) {
            found.push(InstalledJava { path, version });
        }
    }

    let runtimes = runtimes_dir();
    if runtimes.exists() {
        let homes = tokio::task::spawn_blocking(move || {
            let mut homes = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&runtimes) {
                for entry in entries.flatten() {
                    if let Some(home) = find_jdk_home(&entry.path()) {
                        homes.push(home);
                    }
                }
            }
            homes
        })
        .await
        .unwrap_or_default();
        for home in homes {
            let bin = home.join("bin").join(java_exe());
            if let Some(version) = get_java_version(&bin).await.filter(|v| *v > 0) {
                found.push(InstalledJava { path: bin, version });
            }
        }
    }

    let mut unique: Vec<InstalledJava> = Vec::new();
    for j in found {
        if let Some(existing) = unique.iter_mut().find(|u| u.path == j.path) {
            *existing = j;
        } else {
            unique.push(j);
        }
    }
    unique.sort_by(|a, b| b.version.cmp(&a.version));
    unique
}

async fn download_file(
    url: &str,
    dest: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" {
        return Err("Downloads must use HTTPS".into());
    }

    let temp = PathBuf::from(format!(
        "{}.{}.tmp",
        dest.display(),
        chrono::Utc::now().timestamp_millis()
    ));

    let result = tokio::time::timeout(DOWNLOAD_TIMEOUT, download_to(url, &temp, on_progress)).await;
    let result = match result {
        Ok(r) => r,
        Err(_) => Err(format!(
            "Download timed out after {} seconds",
            DOWNLOAD_TIMEOUT.as_secs()
        )),
    };

    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(e);
    }

    let _ = tokio::fs::remove_file(dest).await;
    if let Err(e) = tokio::fs::rename(&temp, dest).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(e.to_string());
    }
    Ok(())
}

async fn download_to(
    url: &str,
    temp: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("Downloads must use HTTPS")
            } else if attempt.previous().len() >= 10 {
                attempt.error("Too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| format!("Download failed: {e}"))?;

    let response = client.get(url).send().await.map_err(|e| match e.source() {
        Some(inner) if e.is_redirect() => inner.to_string(),
        _ => format!("Download failed: {e}"),
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} downloading {}", status.as_u16(), url));
    }

    let total = response.content_length().unwrap_or(0);
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_lowercase())
    };
    let expected = header("x-checksum-sha256")
        .or_else(|| header("x-amz-meta-sha256"))
        .unwrap_or_default();

    let mut file = tokio::fs::File::create(temp)
        .await
        .map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    loop {
        let next = tokio::time::timeout(DOWNLOAD_STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| {
                format!(
                    "Download stalled \u{2014} no data received for {} seconds",
                    DOWNLOAD_STALL_TIMEOUT.as_secs()
                )
            })?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| format!("Download failed: {e}"))?;
        downloaded += chunk.len() as u64;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        on_progress(downloaded, total);
    }

    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    if !expected.is_empty() && hex::encode(hasher.finalize()) != expected {
        return Err("Java download integrity check failed".into());
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("ZIP extraction failed: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("ZIP extraction failed: {e}"))?;
    let resolved = std::path::absolute(dest).map_err(|e| format!("ZIP extraction failed: {e}"))?;

    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| format!("ZIP extraction failed: {e}"))?;
        let out = resolved.join(entry.name());
        if !is_path_within(&out, &resolved) {
            return Err(format!(
                "Unsafe ZIP entry in Java runtime archive: {}",
                entry.name()
            ));
        }
    }

    zip.extract(&resolved)
        .map_err(|e| format!("ZIP extraction failed: {e}"))
}

async fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let archive = archive.to_path_buf();
        let dest = dest.to_path_buf();
        return tokio::task::spawn_blocking(move || extract_zip(&archive, &dest))
            .await
            .map_err(|e| e.to_string())?;
    }

    let mut cmd = Command::new("tar");
    cmd.arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .kill_on_drop(true);
    let output = match tokio::time::timeout(EXTRACTION_TIMEOUT, cmd.output()).await {
        Err(_) => {
            return Err(format!(
                "Extraction timed out after {} seconds",
                EXTRACTION_TIMEOUT.as_secs()
            ))
        }
        Ok(Err(e)) => return Err(format!("Failed to run tar: {e}")),
        Ok(Ok(o)) => o,
    };
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        };
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "null".into());
        Err(format!("tar extraction failed with code {code}{detail}"))
    }
}

async fn install_runtime(
    version: u32,
    runtime_path: &Path,
    archive_path: &Path,
    on_progress: &Option<JavaProgress>,
) -> Result<PathBuf, String> {
    let report = |stage, pct, d, t| {
        if let Some(cb) = on_progress {
            cb(stage, pct, d, t);
        }
    };

    if let Some(home) = find_jdk_home_async(runtime_path).await {
        let bin = home.join("bin").join(java_exe());
        if tokio::fs::metadata(&bin).await.is_ok() {
            report(JavaInstallStage::Downloading, 100.0, None, None);
            return Ok(bin);
        }
    }

    report(JavaInstallStage::Downloading, 0.0, Some(0), Some(0));
    let url = format!(
        "https://api.adoptium.net/v3/binary/latest/{version}/ga/{}/{}/jdk/hotspot/normal/eclipse",
        platform_string(),
        arch_string()
    );

    download_file(&url, archive_path, &|downloaded, total| {
        let pct = if total > 0 {
            downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        report(
            JavaInstallStage::Downloading,
            pct,
            Some(downloaded),
            Some(total),
        );
    })
    .await?;

    report(JavaInstallStage::Extracting, 0.0, None, None);
    let _ = tokio::fs::remove_dir_all(runtime_path).await;
    tokio::fs::create_dir_all(runtime_path)
        .await
        .map_err(|e| e.to_string())?;
    extract_archive(archive_path, runtime_path).await?;
    let _ = tokio::fs::remove_file(archive_path).await;

    let home = find_jdk_home_async(runtime_path)
        .await
        .ok_or_else(|| "JDK home not found after extraction".to_string())?;
    let bin = home.join("bin").join(java_exe());
    tokio::fs::metadata(&bin).await.map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| e.to_string())?;
    }

    report(JavaInstallStage::Extracting, 100.0, None, None);
    Ok(bin)
}

async fn find_jdk_home_async(dir: &Path) -> Option<PathBuf> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || find_jdk_home(&dir))
        .await
        .ok()
        .flatten()
}

pub async fn ensure_java_installed(
    version: u32,
    on_progress: Option<JavaProgress>,
) -> Result<PathBuf, String> {
    if let Some(cb) = &on_progress {
        cb(JavaInstallStage::Discovering, 0.0, None, None);
    }
    let installed = discover_installed_javas().await;
    let best = installed.iter().find(|j| {
        if version == 8 {
            j.version == 8
        } else {
            j.version >= version
        }
    });
    if let Some(j) = best {
        if let Some(cb) = &on_progress {
            cb(JavaInstallStage::Discovering, 100.0, None, None);
        }
        return Ok(j.path.clone());
    }

    let runtimes = runtimes_dir();
    let runtime_path = runtimes.join(format!("java-{version}"));
    let ext = if cfg!(windows) { ".zip" } else { ".tar.gz" };
    let archive_path = runtimes.join(format!("java-{version}{ext}"));

    let result = async {
        tokio::fs::create_dir_all(&runtimes)
            .await
            .map_err(|e| e.to_string())?;
        install_runtime(version, &runtime_path, &archive_path, &on_progress).await
    }
    .await;

    match result {
        Ok(bin) => Ok(bin),
        Err(e) => {
            let _ = tokio::fs::remove_file(&archive_path).await;
            let _ = tokio::fs::remove_dir_all(&runtime_path).await;
            Err(format!("Failed to install Java {version}: {e}"))
        }
    }
}

pub fn get_java_home(java_binary_path: &Path) -> PathBuf {
    java_binary_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_version_for_snapshots() {
        assert_eq!(get_required_java_version("24w14a"), 21);
        assert_eq!(get_required_java_version("21w37a"), 17);
        assert_eq!(get_required_java_version("1.21.5-pre1"), 21);
        assert_eq!(get_required_java_version("26.4-snapshot-10"), 25);
        assert_eq!(get_required_java_version("26.3"), 25);
    }

    #[test]
    fn java_version_for_releases() {
        assert_eq!(get_required_java_version("1.20.4"), 17);
        assert_eq!(get_required_java_version("1.20.5"), 21);
        assert_eq!(get_required_java_version("1.16.5"), 11);
        assert_eq!(get_required_java_version("1.8.9"), 8);
        assert_eq!(get_required_java_version("garbage"), 8);
    }

    #[test]
    fn parses_java_version_lines() {
        assert_eq!(
            parse_java_version(r#"openjdk version "21.0.2" 2024-01-16"#),
            Some(21)
        );
        assert_eq!(parse_java_version(r#"java version "1.8.0_202""#), Some(8));
        assert_eq!(parse_java_version("nope"), None);
    }
}
