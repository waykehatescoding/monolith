use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::events::{emit, Event};
use crate::paths::user_data_dir;
use crate::server_create::{download_file, FrameworkHashes};
use crate::server_runner::re;

const RELEASE_URL: &str = "https://api.github.com/repos/playit-cloud/playit-agent/releases/latest";
const API: &str = "https://api.playit.gg";
const USER_AGENT: &str = "Monolith (Minecraft server manager)";
pub const TUNNELS_URL: &str = "https://playit.gg/account/tunnels";
const PIPE: &str = r"\\.\pipe\monolith-playit";
const KEPT_LINES: usize = 6;

#[derive(Default)]
struct Agent {
    kill: Option<oneshot::Sender<()>>,
    users: HashSet<String>,
    lines: VecDeque<String>,
    tunnels: Vec<PlayitTunnel>,
}

static AGENT: Lazy<Mutex<Agent>> = Lazy::new(Default::default);
static SECRET_RE: Lazy<Regex> = Lazy::new(|| re(r#"(?m)^\s*secret_key\s*=\s*"([0-9a-fA-F]+)""#));

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayitTunnel {
    pub address: String,
    pub protocol: String,
    pub local_port: Option<u16>,
    pub disabled: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayitStatus {
    pub installed: bool,
    pub claimed: bool,
    pub enabled: bool,
    pub running: bool,
    pub lines: Vec<String>,
    pub tunnels: Vec<PlayitTunnel>,
}

fn lock() -> std::sync::MutexGuard<'static, Agent> {
    AGENT.lock().unwrap_or_else(|e| e.into_inner())
}

fn dir() -> PathBuf {
    user_data_dir().join("playit")
}

fn exe_path() -> PathBuf {
    dir().join("playitd.exe")
}

fn secret_path() -> PathBuf {
    dir().join("playit.toml")
}

fn settings_path() -> PathBuf {
    user_data_dir().join("playit_settings.json")
}

fn asset_name() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("playitd-windows-x86_64-signed.exe"),
        ("windows", "x86") => Ok("playitd-windows-x86-signed.exe"),
        (os, arch) => Err(format!(
            "playit.gg is not set up for {os}/{arch} in Monolith"
        )),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

pub async fn status() -> PlayitStatus {
    let enabled = is_enabled().await;
    let agent = lock();
    PlayitStatus {
        installed: exe_path().exists(),
        claimed: secret_path().exists(),
        enabled,
        running: agent.kill.is_some(),
        lines: agent.lines.iter().cloned().collect(),
        tunnels: agent.tunnels.clone(),
    }
}

pub async fn is_enabled() -> bool {
    tokio::fs::read_to_string(settings_path())
        .await
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value["enabled"].as_bool())
        .unwrap_or(false)
}

pub async fn set_enabled(enabled: bool) -> Result<(), String> {
    tokio::fs::write(settings_path(), json!({ "enabled": enabled }).to_string())
        .await
        .map_err(|e| e.to_string())?;
    if !enabled {
        stop().await;
    }
    Ok(())
}

pub async fn install() -> Result<(), String> {
    let wanted = asset_name()?;
    let release: Value = client()
        .get(RELEASE_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"] == wanted))
        .ok_or_else(|| format!("The latest playit release has no {wanted}"))?;
    let url = asset["browser_download_url"]
        .as_str()
        .filter(|url| url.starts_with("https://github.com/playit-cloud/playit-agent/releases/"))
        .ok_or("The playit download link is not from the official repository")?;
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or("The playit release has no checksum to verify")?;
    tokio::fs::create_dir_all(dir())
        .await
        .map_err(|e| e.to_string())?;
    let partial = dir().join("playitd.download");
    let hashes = FrameworkHashes {
        sha1: None,
        sha256: Some(sha256.to_string()),
    };
    download_file(url, &partial, |_, _| {}, Some(&hashes)).await?;
    stop().await;
    tokio::fs::rename(&partial, exe_path())
        .await
        .map_err(|e| e.to_string())
}

pub fn claim_code() -> String {
    hex::encode(&uuid::Uuid::new_v4().as_bytes()[..5])
}

pub fn claim_url(code: &str) -> String {
    format!("https://playit.gg/claim/{code}")
}

async fn call(path: &str, body: Value, secret: Option<&str>) -> Result<Value, String> {
    let mut request = client().post(format!("{API}{path}")).json(&body);
    if let Some(secret) = secret {
        request = request.header("Authorization", format!("Agent-Key {}", secret.trim()));
    }
    let reply: Value = request
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    match reply["status"].as_str() {
        Some("success") => Ok(reply["data"].clone()),
        Some("fail") => Err(reply["data"]
            .as_str()
            .unwrap_or("Request failed")
            .to_string()),
        _ => Err(format!("playit.gg error: {}", reply["data"])),
    }
}

pub async fn wait_for_claim(code: String) -> Result<(), String> {
    let setup = json!({ "code": code, "agent_type": "assignable", "version": "Monolith" });
    let mut accepted = false;
    for _ in 0..300 {
        match call("/claim/setup", setup.clone(), None).await?.as_str() {
            Some("UserAccepted") => {
                accepted = true;
                break;
            }
            Some("UserRejected") => return Err("The link was declined on playit.gg.".into()),
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    if !accepted {
        return Err("Linking timed out. Try again.".into());
    }
    let mut last = String::new();
    for _ in 0..30 {
        match call("/claim/exchange", json!({ "code": code }), None).await {
            Ok(data) => {
                let secret = data["secret_key"]
                    .as_str()
                    .ok_or("No secret key returned")?;
                tokio::fs::create_dir_all(dir())
                    .await
                    .map_err(|e| e.to_string())?;
                return tokio::fs::write(secret_path(), format!("secret_key = \"{secret}\"\n"))
                    .await
                    .map_err(|e| e.to_string());
            }
            Err(error) => last = error,
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(format!("playit.gg did not finish linking: {last}"))
}

pub async fn unlink() -> Result<(), String> {
    stop().await;
    match tokio::fs::remove_file(secret_path()).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

fn read_secret(text: &str) -> Option<String> {
    SECRET_RE.captures(text).map(|c| c[1].to_string())
}

pub fn parse_tunnels(data: &Value) -> Vec<PlayitTunnel> {
    let Some(tunnels) = data["tunnels"].as_array() else {
        return Vec::new();
    };
    tunnels
        .iter()
        .filter_map(|tunnel| {
            let local_port = tunnel["agent_config"]["fields"]
                .as_array()
                .and_then(|fields| fields.iter().find(|f| f["name"] == "local_port"))
                .and_then(|f| f["value"].as_str())
                .and_then(|v| v.parse().ok());
            Some(PlayitTunnel {
                address: tunnel["display_address"].as_str()?.to_string(),
                protocol: tunnel["port_type"].as_str().unwrap_or("tcp").to_string(),
                local_port,
                disabled: tunnel["disabled_reason"].as_str().map(str::to_string),
            })
        })
        .collect()
}

pub async fn tunnels() -> Result<Vec<PlayitTunnel>, String> {
    let text = tokio::fs::read_to_string(secret_path())
        .await
        .map_err(|_| "Link your playit account first")?;
    let secret = read_secret(&text).ok_or("The saved playit secret is unreadable")?;
    let data = call("/v1/agents/rundata", json!({}), Some(&secret)).await?;
    Ok(parse_tunnels(&data))
}

pub fn status_line(line: &str) -> String {
    let line = line.trim();
    let message = match line.find(": ") {
        Some(at)
            if line[..at].contains(" INFO ")
                || line[..at].contains(" WARN ")
                || line[..at].contains(" ERROR ") =>
        {
            &line[at + 2..]
        }
        _ => line,
    };
    message.to_string()
}

fn push_line(line: String) {
    let line = status_line(&line);
    if line.contains("tunnels loaded") || line.contains("tunnel state updated") {
        tokio::spawn(refresh_tunnels());
    }
    let mut agent = lock();
    agent.lines.push_back(line);
    while agent.lines.len() > KEPT_LINES {
        agent.lines.pop_front();
    }
    drop(agent);
    emit(Event::NetworkChanged);
}

async fn refresh_tunnels() {
    match tunnels().await {
        Ok(tunnels) => {
            lock().tunnels = tunnels;
            emit(Event::NetworkChanged);
        }
        Err(error) => push_line(format!("Could not read your tunnels: {error}")),
    }
}

pub async fn acquire(server_id: &str) -> Result<(), String> {
    if !exe_path().exists() {
        return Err("Install playit.gg in Settings, Sharing first".into());
    }
    if !secret_path().exists() {
        return Err("Link your playit.gg account in Settings, Sharing first".into());
    }
    let mut agent = lock();
    agent.users.insert(server_id.to_string());
    if agent.kill.is_some() {
        return Ok(());
    }
    let mut command = Command::new(exe_path());
    command
        .arg("--secret-path")
        .arg(secret_path())
        .args(["--socket-path", PIPE])
        .env("PLAYIT_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
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
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    push_line(line);
                }
            }
        });
    }
    let (kill_tx, kill_rx) = oneshot::channel();
    agent.kill = Some(kill_tx);
    agent.lines.clear();
    agent.tunnels.clear();
    drop(agent);
    tokio::spawn(async move {
        tokio::select! {
            _ = child.wait() => {}
            _ = kill_rx => { let _ = child.kill().await; }
        }
        let mut agent = lock();
        agent.kill = None;
        agent.users.clear();
        agent.tunnels.clear();
        drop(agent);
        emit(Event::NetworkChanged);
    });
    emit(Event::NetworkChanged);
    Ok(())
}

pub async fn release(server_id: &str) {
    let kill = {
        let mut agent = lock();
        if !agent.users.remove(server_id) || !agent.users.is_empty() {
            return;
        }
        agent.kill.take()
    };
    if let Some(kill) = kill {
        let _ = kill.send(());
    }
}

pub async fn stop() {
    let kill = {
        let mut agent = lock();
        agent.users.clear();
        agent.kill.take()
    };
    if let Some(kill) = kill {
        let _ = kill.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tunnels_from_rundata() {
        let data = json!({
            "agent_id": "x",
            "tunnels": [
                {
                    "display_address": "word-sample.gl.joinmc.link",
                    "port_type": "tcp",
                    "agent_config": { "fields": [
                        { "name": "local_ip", "value": "127.0.0.1" },
                        { "name": "local_port", "value": "25565" }
                    ] },
                    "disabled_reason": null
                },
                { "display_address": "147.185.221.1:4321", "port_type": "udp", "agent_config": { "fields": [] }, "disabled_reason": "Account limit" },
                { "port_type": "tcp" }
            ]
        });
        let tunnels = parse_tunnels(&data);
        assert_eq!(tunnels.len(), 2);
        assert_eq!(tunnels[0].local_port, Some(25565));
        assert_eq!(tunnels[1].protocol, "udp");
        assert_eq!(tunnels[1].disabled.as_deref(), Some("Account limit"));
        assert!(parse_tunnels(&json!({})).is_empty());
    }

    #[test]
    fn shortens_log_lines() {
        assert_eq!(
            status_line("2026-11-24T16:38:09.123Z  INFO playitd::daemon: playit connected; tunnels loaded tunnel_count=1"),
            "playit connected; tunnels loaded tunnel_count=1"
        );
        assert_eq!(status_line("plain text: here"), "plain text: here");
    }

    #[test]
    fn reads_secret_file() {
        assert_eq!(
            read_secret("secret_key = \"ab12\"\n").as_deref(),
            Some("ab12")
        );
        assert_eq!(read_secret("nope"), None);
        assert_eq!(claim_code().len(), 10);
    }
}
