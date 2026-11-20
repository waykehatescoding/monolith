use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use sysinfo::Networks;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::events::{emit, Event};
use crate::paths::{home_dir, ngrok_dir, user_data_dir};
use crate::server_runner::re;
use crate::types::{
    InstallNgrokResult, NgrokAuthtokenValidation, NgrokInstallProgress, NgrokStatus,
    NgrokTunnelInfo, StartNgrokResult, SuccessResult,
};

struct Tunnel {
    id: u64,
    kill: Option<oneshot::Sender<oneshot::Sender<()>>>,
    public_url: String,
}

static TUNNELS: Lazy<std::sync::Mutex<HashMap<String, Tunnel>>> = Lazy::new(Default::default);
static INSTALLING: AtomicBool = AtomicBool::new(false);
static NEXT_TUNNEL_ID: AtomicU64 = AtomicU64::new(1);
static TOKEN_RE: Lazy<Regex> = Lazy::new(|| re(r"^[a-zA-Z0-9_\-]+$"));
static URL_RE: Lazy<Regex> = Lazy::new(|| re(r"url=(tcp://\S+)"));
static ESTABLISHED_RE: Lazy<Regex> = Lazy::new(|| re(r"Tunnel established at (tcp://\S+)"));
static AUTHTOKEN_RE: Lazy<Regex> = Lazy::new(|| re(r"(?:auth_token|authtoken):\s*(.+)"));
static ERR_RE: Lazy<Regex> = Lazy::new(|| re(r#"lvl=(?:eror|crit)\b.*\berr="((?:[^"\\]|\\.)*)""#));

fn lock_tunnels() -> std::sync::MutexGuard<'static, HashMap<String, Tunnel>> {
    TUNNELS.lock().unwrap_or_else(|e| e.into_inner())
}

fn config_dir() -> PathBuf {
    ngrok_dir().join("config")
}

fn config_path() -> PathBuf {
    config_dir().join("ngrok.yml")
}

fn ngrok_path() -> PathBuf {
    ngrok_dir().join(if cfg!(windows) { "ngrok.exe" } else { "ngrok" })
}

fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn arch_name() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn download_url() -> Result<&'static str, String> {
    match (platform_name(), arch_name()) {
        ("win32", "x64") => {
            Ok("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-windows-amd64.zip")
        }
        ("darwin", "x64") => {
            Ok("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-darwin-amd64.zip")
        }
        ("darwin", "arm64") => {
            Ok("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-darwin-arm64.zip")
        }
        ("linux", "x64") => {
            Ok("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-linux-amd64.zip")
        }
        ("linux", "arm64") => {
            Ok("https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-linux-arm64.zip")
        }
        (platform, arch) => Err(format!("Unsupported platform/arch: {platform}/{arch}")),
    }
}

fn ngrok_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.kill_on_drop(true);
    cmd
}

async fn kill_tunnel(mut tunnel: Tunnel) {
    if let Some(kill) = tunnel.kill.take() {
        kill_with_ack(kill).await;
    }
}

async fn kill_with_ack(kill: oneshot::Sender<oneshot::Sender<()>>) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if kill.send(ack_tx).is_ok() {
        let _ = ack_rx.await;
    }
}

fn spawn_error(program: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("spawn {} ENOENT", program.display())
    } else {
        error.to_string()
    }
}

async fn is_ngrok_installed() -> bool {
    tokio::fs::metadata(ngrok_path()).await.is_ok()
}

pub fn is_installing() -> bool {
    INSTALLING.load(Ordering::SeqCst)
}

pub async fn stop_all_tunnels() {
    let tunnels: Vec<Tunnel> = lock_tunnels().drain().map(|(_, t)| t).collect();
    for tunnel in tunnels {
        kill_tunnel(tunnel).await;
    }
}

async fn download_file(url: &str, dest: &Path, on_progress: impl Fn(u64)) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" {
        return Err("Downloads must use HTTPS".to_string());
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("Downloads must use HTTPS")
            } else if attempt.previous().len() >= 10 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() || response.status().as_u16() != 200 {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }

    let total = response.content_length().unwrap_or(0);
    let result: Result<(), String> = async {
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| e.to_string())?;
        let mut downloaded: u64 = 0;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;
            if total > 0 {
                on_progress(((downloaded as f64 / total as f64) * 100.0).round() as u64);
            }
        }
        file.flush().await.map_err(|e| e.to_string())
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut targets = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        match entry.enclosed_name() {
            Some(name) => targets.push(dest_dir.join(name)),
            None => return Err(format!("Unsafe zip entry: {}", entry.name())),
        }
    }

    for (i, target) in targets.iter().enumerate() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Extraction failed: {e}"))?;
        if entry.is_dir() {
            std::fs::create_dir_all(target).map_err(|e| format!("Extraction failed: {e}"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Extraction failed: {e}"))?;
        }
        let mut out =
            std::fs::File::create(target).map_err(|e| format!("Extraction failed: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extraction failed: {e}"))?;
    }
    Ok(())
}

async fn install_inner(on_progress: impl Fn(u64)) -> Result<(), String> {
    let dir = ngrok_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;

    if is_ngrok_installed().await {
        return Ok(());
    }

    let url = download_url()?;
    let zip_path = dir.join("ngrok.zip");

    download_file(url, &zip_path, on_progress).await?;

    let extract_target = dir.clone();
    let extract_source = zip_path.clone();
    tokio::task::spawn_blocking(move || extract_zip(&extract_source, &extract_target))
        .await
        .map_err(|e| e.to_string())??;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(ngrok_path(), std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| e.to_string())?;
    }

    let _ = tokio::fs::remove_file(&zip_path).await;
    tokio::fs::create_dir_all(config_dir())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn install(on_progress: impl Fn(u64)) -> InstallNgrokResult {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return SuccessResult::err("Installation already in progress");
    }
    let result = install_inner(on_progress).await;
    INSTALLING.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => SuccessResult::ok(),
        Err(error) => SuccessResult::err(error),
    }
}

pub async fn install_ngrok() -> InstallNgrokResult {
    install(|percent| {
        emit(Event::NgrokInstallProgress(NgrokInstallProgress {
            percent: percent as f64,
        }))
    })
    .await
}

async fn run_add_authtoken(token: &str, config: &Path) -> Result<(), String> {
    let output = ngrok_command(&ngrok_path())
        .args(["config", "add-authtoken", token, "--config"])
        .arg(config)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| spawn_error(&ngrok_path(), e))?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!(
            "Exit code: {}",
            output
                .status
                .code()
                .map_or("null".to_string(), |c| c.to_string())
        )
    })
}

async fn ensure_installed() -> Result<(), String> {
    if is_ngrok_installed().await {
        return Ok(());
    }
    let result = install(|_| {}).await;
    if result.success {
        Ok(())
    } else {
        Err(format!(
            "Failed to install ngrok: {}",
            result.error.unwrap_or_else(|| "Unknown error".to_string())
        ))
    }
}

pub async fn configure_ngrok_authtoken(authtoken: String) -> SuccessResult {
    if authtoken.is_empty() || !TOKEN_RE.is_match(authtoken.trim()) {
        return SuccessResult::err("Invalid authtoken format");
    }
    if let Err(error) = ensure_installed().await {
        return SuccessResult::err(error);
    }
    if let Err(error) = tokio::fs::create_dir_all(config_dir()).await {
        return SuccessResult::err(error.to_string());
    }
    match run_add_authtoken(&authtoken, &config_path()).await {
        Ok(()) => SuccessResult::ok(),
        Err(error) => SuccessResult::err(error),
    }
}

pub async fn is_ngrok_authtoken_configured() -> bool {
    match tokio::fs::read_to_string(config_path()).await {
        Ok(content) => content.contains("auth_token:") || content.contains("authtoken:"),
        Err(_) => false,
    }
}

async fn fetch_tunnel_url() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let response = client
        .get("http://127.0.0.1:4040/api/tunnels")
        .send()
        .await
        .ok()?;
    let json: Value = response.json().await.ok()?;
    json.get("tunnels")?
        .as_array()?
        .first()?
        .get("public_url")?
        .as_str()
        .map(str::to_string)
}

fn ngrok_error(line: &str) -> Option<String> {
    if let Some(rest) = line.trim().strip_prefix("ERROR:") {
        return Some(rest.trim().to_string()).filter(|s| !s.is_empty());
    }
    ERR_RE.captures(line).map(|c| {
        c[1].replace("\\r", "")
            .replace("\\n", " ")
            .replace("\\\"", "\"")
    })
}

fn capture_url(line: &str, slot: &std::sync::Mutex<String>) {
    let found = ESTABLISHED_RE
        .captures(line)
        .or_else(|| URL_RE.captures(line))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    if let Some(url) = found {
        if let Ok(mut guard) = slot.lock() {
            *guard = url;
        }
    }
}

pub async fn start_ngrok(server_id: String, port: u16) -> StartNgrokResult {
    match start_tunnel(&server_id, port).await {
        Ok(public_url) => {
            emit(Event::NgrokUrlChanged(NgrokTunnelInfo {
                server_id,
                public_url: public_url.clone(),
                port,
                protocol: "tcp".to_string(),
            }));
            StartNgrokResult {
                success: true,
                public_url: Some(public_url),
                error: None,
            }
        }
        Err(error) => {
            emit(Event::NgrokFailed {
                server_id,
                error: error.clone(),
            });
            StartNgrokResult {
                success: false,
                public_url: None,
                error: Some(error),
            }
        }
    }
}

async fn start_tunnel(server_id: &str, port: u16) -> Result<String, String> {
    if port < 1 {
        return Err("Invalid port number".to_string());
    }

    if let Some(existing) = lock_tunnels().get(server_id) {
        return Ok(existing.public_url.clone());
    }

    if !is_ngrok_installed().await {
        return Err("Ngrok is not installed".to_string());
    }

    let config = config_path();
    if tokio::fs::metadata(&config).await.is_err() {
        tokio::fs::create_dir_all(config_dir())
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&config, "version: 2\n")
            .await
            .map_err(|e| e.to_string())?;
    }

    let mut child = ngrok_command(&ngrok_path())
        .args(["tcp", &port.to_string(), "--log=stdout", "--config"])
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(&ngrok_path(), e))?;

    let found_url = Arc::new(std::sync::Mutex::new(String::new()));
    let last_error = Arc::new(std::sync::Mutex::new(String::new()));

    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn AsyncRead + Send + Unpin>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn AsyncRead + Send + Unpin>),
    ]
    .into_iter()
    .flatten()
    {
        let url_slot = Arc::clone(&found_url);
        let error_slot = Arc::clone(&last_error);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                capture_url(&line, &url_slot);
                if let (Some(error), Ok(mut slot)) = (ngrok_error(&line), error_slot.lock()) {
                    *slot = error;
                }
            }
        });
    }

    let id = NEXT_TUNNEL_ID.fetch_add(1, Ordering::SeqCst);
    let (kill_tx, mut kill_rx) = oneshot::channel::<oneshot::Sender<()>>();
    let watcher_server_id = server_id.to_string();
    tokio::spawn(async move {
        let mut ack = None;
        tokio::select! {
            _ = child.wait() => {}
            signal = &mut kill_rx => {
                let _ = child.kill().await;
                ack = signal.ok();
            }
        }
        {
            let mut tunnels = lock_tunnels();
            if tunnels.get(&watcher_server_id).is_some_and(|t| t.id == id) {
                tunnels.remove(&watcher_server_id);
            }
        }
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
    });

    let read_url = || found_url.lock().map(|g| g.clone()).unwrap_or_default();
    let mut public_url = String::new();
    for _ in 0..50 {
        if !public_url.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        public_url = read_url();
        if !public_url.is_empty() {
            break;
        }
        if kill_tx.is_closed() {
            break;
        }
        if let Some(url) = fetch_tunnel_url().await {
            public_url = url;
            break;
        }
    }

    if public_url.is_empty() {
        kill_with_ack(kill_tx).await;
        let error = last_error.lock().map(|g| g.clone()).unwrap_or_default();
        return Err(if error.is_empty() {
            "ngrok did not create a public address in time. Check your authtoken in Settings > Sharing.".to_string()
        } else {
            format!("ngrok: {error}")
        });
    }

    let clean_url = public_url
        .strip_prefix("tcp://")
        .unwrap_or(&public_url)
        .to_string();
    lock_tunnels().insert(
        server_id.to_string(),
        Tunnel {
            id,
            kill: Some(kill_tx),
            public_url: clean_url.clone(),
        },
    );
    Ok(clean_url)
}

pub async fn stop_ngrok(server_id: String) -> SuccessResult {
    let tunnel = lock_tunnels().remove(&server_id);
    if let Some(tunnel) = tunnel {
        kill_tunnel(tunnel).await;
    }
    SuccessResult::ok()
}

pub async fn get_ngrok_status(server_id: String) -> NgrokStatus {
    let installed = is_ngrok_installed().await;
    let public_url = lock_tunnels().get(&server_id).map(|t| t.public_url.clone());
    NgrokStatus {
        installed,
        installing: is_installing(),
        tunnel_active: public_url.is_some(),
        public_url,
        error: None,
    }
}

pub async fn get_local_ip() -> String {
    tokio::task::spawn_blocking(local_ip)
        .await
        .unwrap_or_else(|_| "localhost".to_string())
}

fn local_ip() -> String {
    let networks = Networks::new_with_refreshed_list();
    let ipv4_of = |name: &str| -> Option<String> {
        let (_, data) = networks.iter().find(|(n, _)| n.as_str() == name)?;
        data.ip_networks().iter().find_map(|net| match net.addr {
            IpAddr::V4(addr) if !addr.is_loopback() => Some(addr.to_string()),
            _ => None,
        })
    };

    for name in ["Ethernet", "Wi-Fi", "en0", "eth0", "wlan0"] {
        if let Some(ip) = ipv4_of(name) {
            return ip;
        }
    }
    let mut names: Vec<&String> = networks.keys().collect();
    names.sort();
    for name in names {
        if let Some(ip) = ipv4_of(name) {
            return ip;
        }
    }
    "localhost".to_string()
}

pub async fn validate_ngrok_authtoken(authtoken: String) -> NgrokAuthtokenValidation {
    let invalid = |error: &str| NgrokAuthtokenValidation {
        valid: false,
        error: Some(error.to_string()),
    };

    if authtoken.trim().is_empty() {
        return invalid("Authtoken is required");
    }
    let token = authtoken.trim();
    if !TOKEN_RE.is_match(token) {
        return invalid("Invalid authtoken format. Only alphanumeric characters, underscores and hyphens are allowed.");
    }
    if token.len() < 20 {
        return invalid("Authtoken is too short. Please check your token at dashboard.ngrok.com");
    }
    if let Err(error) = ensure_installed().await {
        return invalid(&error);
    }

    let temp_config = ngrok_dir().join("temp_validate.yml");
    let result = run_add_authtoken(token, &temp_config).await;
    let _ = tokio::fs::remove_file(&temp_config).await;

    match result {
        Ok(()) => NgrokAuthtokenValidation {
            valid: true,
            error: None,
        },
        Err(message) => {
            let lower = message.to_lowercase();
            let error = if [
                "invalid",
                "not found",
                "unauthorized",
                "authentication failed",
            ]
            .iter()
            .any(|p| lower.contains(p))
            {
                "Invalid authtoken. Please check your token at dashboard.ngrok.com".to_string()
            } else if ["network", "connection", "timeout"]
                .iter()
                .any(|p| lower.contains(p))
            {
                "Network error. Please check your internet connection.".to_string()
            } else {
                message
            };
            NgrokAuthtokenValidation {
                valid: false,
                error: Some(error),
            }
        }
    }
}

pub async fn get_ngrok_authtoken_censored() -> Option<String> {
    let candidates = [
        config_path(),
        user_data_dir().join("ngrok").join("ngrok.yml"),
        home_dir().join(".ngrok2").join("ngrok.yml"),
    ];

    for path in candidates {
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        if let Some(caps) = AUTHTOKEN_RE.captures(&content) {
            let token = caps.get(1).map_or("", |m| m.as_str()).trim();
            let chars: Vec<char> = token.chars().collect();
            if chars.len() > 7 {
                let tail: String = chars[chars.len() - 7..].iter().collect();
                return Some(format!("****_****_****_{tail}"));
            }
            return Some(format!("****_{token}"));
        }
    }
    None
}

fn settings_path() -> PathBuf {
    user_data_dir().join("ngrok_settings.json")
}

pub async fn is_ngrok_enabled() -> bool {
    let Ok(content) = tokio::fs::read_to_string(settings_path()).await else {
        return false;
    };
    serde_json::from_str::<Value>(&content)
        .ok()
        .and_then(|v| v.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false)
}

pub async fn set_ngrok_enabled(enabled: bool) -> Result<(), String> {
    tokio::fs::write(
        settings_path(),
        serde_json::json!({ "enabled": enabled }).to_string(),
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn remove_ngrok_authtoken() -> SuccessResult {
    match tokio::fs::remove_file(config_path()).await {
        Ok(()) => SuccessResult::ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SuccessResult::err(format!(
            "ENOENT: no such file or directory, unlink '{}'",
            config_path().display()
        )),
        Err(error) => SuccessResult::err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ngrok_errors() {
        assert_eq!(
            ngrok_error("ERROR:  authentication failed: Your account is suspended."),
            Some("authentication failed: Your account is suspended.".into())
        );
        assert_eq!(
            ngrok_error(r#"t=1 lvl=eror msg="session closing" obj=tunnels.session err="TCP tunnels are only available to verified accounts.\r\n\r\nERR_NGROK_8013""#)
                .as_deref()
                .map(|e| e.contains("ERR_NGROK_8013")),
            Some(true)
        );
        assert_eq!(
            ngrok_error(r#"t=1 lvl=info msg="started tunnel" url=tcp://0.tcp.ngrok.io:1"#),
            None
        );
    }
}
