use std::collections::HashMap;
use std::time::Duration;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};

use crate::events::{self, Event};
use crate::server_runner::get_server_logs;
use crate::server_store::{get_server, load_server_list, save_server_list};
use crate::types::{ConsoleLineType, ServerRecord, ServerRuntimeStatus, SuccessResult};

const KEY: &str = "discordWebhook";
const ATTEMPTS: usize = 3;
const MAX_WAIT: Duration = Duration::from_secs(30);
const INVALID_URL: &str =
    "Paste a Discord webhook URL. It looks like https://discord.com/api/webhooks/...";

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
});
static PLAYER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[[^\]]*\](?: \[[^\]]*/INFO\])?: ([A-Za-z0-9_]{1,16}) (joined|left) the game$")
        .expect("static regex")
});
static ANSI: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("static regex"));
static CHAT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\]: <[^>]+> ").expect("static regex"));

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebhookConfig {
    pub url: String,
    pub started: bool,
    pub stopped: bool,
    pub crashed: bool,
    pub joined: bool,
    pub left: bool,
    pub backup_finished: bool,
    pub backup_failed: bool,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            started: true,
            stopped: true,
            crashed: true,
            joined: false,
            left: false,
            backup_finished: false,
            backup_failed: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Started,
    Stopped,
    Crashed,
    Joined,
    Left,
    BackupFinished,
    BackupFailed,
}

impl Kind {
    fn wanted(self, config: &WebhookConfig) -> bool {
        match self {
            Kind::Started => config.started,
            Kind::Stopped => config.stopped,
            Kind::Crashed => config.crashed,
            Kind::Joined => config.joined,
            Kind::Left => config.left,
            Kind::BackupFinished => config.backup_finished,
            Kind::BackupFailed => config.backup_failed,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Kind::Started => "Server online",
            Kind::Stopped => "Server stopped",
            Kind::Crashed => "Server crashed",
            Kind::Joined => "Player joined",
            Kind::Left => "Player left",
            Kind::BackupFinished => "Backup finished",
            Kind::BackupFailed => "Backup failed",
        }
    }

    fn color(self) -> u32 {
        match self {
            Kind::Started | Kind::BackupFinished => 0x5fd38d,
            Kind::Joined => 0x5b8def,
            Kind::Stopped | Kind::Left => 0x6b7178,
            Kind::BackupFailed => 0xe8b562,
            Kind::Crashed => 0xef6a5e,
        }
    }
}

pub fn webhook_config(server: &ServerRecord) -> WebhookConfig {
    server
        .extra
        .get(KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

pub fn is_webhook_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return false;
    };
    parsed.scheme() == "https"
        && matches!(parsed.host_str(), Some("discord.com" | "discordapp.com"))
        && parsed.port().is_none()
        && parsed
            .path()
            .strip_prefix("/api/webhooks/")
            .is_some_and(|rest| rest.split('/').filter(|part| !part.is_empty()).count() >= 2)
}

pub async fn save_webhook_config(id: String, mut config: WebhookConfig) -> SuccessResult {
    config.url = config.url.trim().to_string();
    if !config.url.is_empty() && !is_webhook_url(&config.url) {
        return SuccessResult::err(INVALID_URL);
    }
    let mut servers = load_server_list().await;
    let Some(server) = servers.iter_mut().find(|server| server.id == id) else {
        return SuccessResult::err("Server not found");
    };
    match serde_json::to_value(&config) {
        Ok(value) => server.extra.insert(KEY.into(), value),
        Err(e) => return SuccessResult::err(e.to_string()),
    };
    match save_server_list(&servers).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e),
    }
}

pub async fn send_test(url: String, server_name: String) -> SuccessResult {
    if !is_webhook_url(&url) {
        return SuccessResult::err(INVALID_URL);
    }
    let body = payload(
        "Test message",
        "Monolith can post to this channel.",
        0x5fd38d,
        &server_name,
        &now(),
    );
    match post(url.trim(), &body).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e),
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn payload(title: &str, description: &str, color: u32, server: &str, timestamp: &str) -> Value {
    let mut embed = json!({
        "title": title,
        "color": color,
        "author": { "name": server },
        "timestamp": timestamp,
    });
    if !description.is_empty() {
        embed["description"] = Value::String(description.to_string());
    }
    json!({ "username": "Monolith", "embeds": [embed] })
}

fn retry_after(body: &Value) -> Duration {
    let seconds = body["retry_after"].as_f64().unwrap_or(1.0).max(0.0);
    Duration::from_secs_f64(seconds).min(MAX_WAIT)
}

async fn post(url: &str, body: &Value) -> Result<(), String> {
    for _ in 0..ATTEMPTS {
        let response = CLIENT
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Discord could not be reached: {}", e.without_url()))?;
        let status = response.status();
        if status.as_u16() == 429 {
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            tokio::time::sleep(retry_after(&body)).await;
            continue;
        }
        if status.is_success() {
            return Ok(());
        }
        return Err(match status.as_u16() {
            401 | 404 => "Discord does not know this webhook. Check it was not deleted.".into(),
            code => format!("Discord answered with HTTP {code}."),
        });
    }
    Err("Discord is rate limiting this webhook. Try again in a minute.".into())
}

fn transition(previous: Option<ServerRuntimeStatus>, now: ServerRuntimeStatus) -> Option<Kind> {
    use ServerRuntimeStatus::*;
    match (previous, now) {
        (Some(Online), Online) => None,
        (_, Online) => Some(Kind::Started),
        (Some(Online | Stopping | Starting), Offline) => Some(Kind::Stopped),
        (Some(Crashed), Crashed) => None,
        (_, Crashed) => Some(Kind::Crashed),
        _ => None,
    }
}

fn player_change(line: &str) -> Option<(Kind, String)> {
    let line = ANSI.replace_all(line, "");
    let captures = PLAYER.captures(line.trim_end())?;
    let kind = match &captures[2] {
        "joined" => Kind::Joined,
        _ => Kind::Left,
    };
    Some((kind, captures[1].to_string()))
}

fn is_error_line(kind: ConsoleLineType, text: &str) -> bool {
    if CHAT.is_match(text) {
        return false;
    }
    kind == ConsoleLineType::Stderr
        || ["ERROR", "FATAL", "SEVERE", "Exception"]
            .iter()
            .any(|marker| text.contains(marker))
}

async fn crash_summary(server_id: &str) -> String {
    let logs = get_server_logs(server_id.to_string()).await;
    let mut errors: Vec<&str> = logs
        .iter()
        .rev()
        .filter(|line| is_error_line(line.kind, &line.text))
        .take(6)
        .map(|line| line.text.as_str())
        .collect();
    if errors.is_empty() {
        return "The server stopped unexpectedly.".into();
    }
    errors.reverse();
    let mut joined = errors.join("\n");
    if joined.len() > 1500 {
        let mut cut = joined.len() - 1500;
        while !joined.is_char_boundary(cut) {
            cut += 1;
        }
        joined = joined[cut..].to_string();
    }
    format!(
        "The server stopped unexpectedly. Last errors:\n```\n{}\n```",
        joined.replace("```", "'''")
    )
}

async fn notice(
    event: &Event,
    last: &mut HashMap<String, ServerRuntimeStatus>,
) -> Option<(String, Kind, String)> {
    match event {
        Event::ServerStatus(update) => {
            let previous = last.insert(update.server_id.clone(), update.status);
            let kind = transition(previous, update.status)?;
            let description = match kind {
                Kind::Crashed => crash_summary(&update.server_id).await,
                _ => String::new(),
            };
            Some((update.server_id.clone(), kind, description))
        }
        Event::ConsoleOutput { server_id, line } => {
            let (kind, name) = player_change(&line.text)?;
            Some((server_id.clone(), kind, format!("**{name}**")))
        }
        Event::BackupCompleted(done) => Some((
            done.server_id.clone(),
            Kind::BackupFinished,
            done.backup.name.clone(),
        )),
        Event::BackupProgress(progress) => {
            let error = progress.error.as_deref()?;
            (error != "Backup cancelled").then(|| {
                (
                    progress.server_id.clone(),
                    Kind::BackupFailed,
                    error.to_string(),
                )
            })
        }
        _ => None,
    }
}

pub async fn run() {
    let (sender, mut queue) = mpsc::unbounded_channel::<(String, Value)>();
    tokio::spawn(async move {
        while let Some((url, body)) = queue.recv().await {
            let _ = post(&url, &body).await;
        }
    });
    let mut receiver = events::subscribe();
    let mut last = HashMap::new();
    loop {
        let event = match receiver.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let Some((server_id, kind, description)) = notice(&event, &mut last).await else {
            continue;
        };
        let Some(server) = get_server(server_id).await else {
            continue;
        };
        let config = webhook_config(&server);
        if config.url.is_empty() || !kind.wanted(&config) {
            continue;
        }
        let body = payload(
            kind.title(),
            &description,
            kind.color(),
            &server.name,
            &now(),
        );
        let _ = sender.send((config.url, body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ServerRuntimeStatus::*;

    #[test]
    fn accepts_only_discord_webhook_urls() {
        assert!(is_webhook_url(
            "https://discord.com/api/webhooks/123/abcDEF"
        ));
        assert!(is_webhook_url("https://discordapp.com/api/webhooks/1/x"));
        assert!(!is_webhook_url("http://discord.com/api/webhooks/1/x"));
        assert!(!is_webhook_url("https://discord.com/api/webhooks/1"));
        assert!(!is_webhook_url(
            "https://discord.com.evil.io/api/webhooks/1/x"
        ));
        assert!(!is_webhook_url("https://evil.io/api/webhooks/1/x"));
        assert!(!is_webhook_url("https://discord.com/channels/1/2"));
        assert!(!is_webhook_url("not a url"));
    }

    #[test]
    fn payload_has_one_embed_with_server_and_time() {
        let body = payload(
            "Server online",
            "",
            0x5fd38d,
            "Survival",
            "2026-01-01T00:00:00Z",
        );
        let embed = &body["embeds"][0];
        assert_eq!(embed["title"], "Server online");
        assert_eq!(embed["color"], 0x5fd38d);
        assert_eq!(embed["author"]["name"], "Survival");
        assert_eq!(embed["timestamp"], "2026-01-01T00:00:00Z");
        assert!(embed.get("description").is_none());
        let body = payload("Backup failed", "disk full", 1, "S", "t");
        assert_eq!(body["embeds"][0]["description"], "disk full");
    }

    #[test]
    fn retry_after_reads_seconds_and_caps_the_wait() {
        assert_eq!(
            retry_after(&json!({ "retry_after": 0.5 })),
            Duration::from_millis(500)
        );
        assert_eq!(retry_after(&json!({ "retry_after": 600.0 })), MAX_WAIT);
        assert_eq!(retry_after(&Value::Null), Duration::from_secs(1));
    }

    #[test]
    fn status_transitions_map_to_events() {
        assert_eq!(transition(Some(Starting), Online), Some(Kind::Started));
        assert_eq!(transition(Some(Online), Online), None);
        assert_eq!(transition(Some(Stopping), Offline), Some(Kind::Stopped));
        assert_eq!(transition(Some(Offline), Offline), None);
        assert_eq!(transition(None, Offline), None);
        assert_eq!(transition(Some(Online), Crashed), Some(Kind::Crashed));
        assert_eq!(transition(Some(Crashed), Crashed), None);
    }

    #[test]
    fn player_lines_are_parsed_and_chat_is_ignored() {
        assert_eq!(
            player_change("[12:00:00 INFO]: Steve joined the game"),
            Some((Kind::Joined, "Steve".to_string()))
        );
        assert_eq!(
            player_change("[12:00:00] [Server thread/INFO]: Alex_2 left the game"),
            Some((Kind::Left, "Alex_2".to_string()))
        );
        assert_eq!(
            player_change("\x1b[0m[12:00:00 INFO]: Steve joined the game\x1b[0m\r"),
            Some((Kind::Joined, "Steve".to_string()))
        );
        assert_eq!(
            player_change("[12:00:00 INFO]: <Steve> Bob joined the game"),
            None
        );
        assert_eq!(
            player_change("[12:00:00 INFO]: <Bob> x]: Steve joined the game"),
            None
        );
        assert_eq!(
            player_change("[12:00:00 INFO]: Steve joined the game, then left"),
            None
        );
    }

    #[test]
    fn crash_summary_skips_player_chat() {
        assert!(is_error_line(
            ConsoleLineType::Stdout,
            "[12:00:00 ERROR]: Exception in server tick loop"
        ));
        assert!(!is_error_line(
            ConsoleLineType::Stdout,
            "[12:00:00 INFO]: <Bob> fake ERROR Exception"
        ));
        assert!(!is_error_line(
            ConsoleLineType::Stderr,
            "[12:00:00] [Server thread/INFO]: <Bob> hi"
        ));
    }

    #[test]
    fn old_records_get_default_config() {
        let server = ServerRecord::default();
        assert_eq!(webhook_config(&server), WebhookConfig::default());
    }
}
