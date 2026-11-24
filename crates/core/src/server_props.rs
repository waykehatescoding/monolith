use crate::server_runner::update_player_capacity_string;
use crate::server_store;
use crate::types::{ServerProperty, SuccessResult};
use futures_util::StreamExt;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

fn to_result(res: std::io::Result<()>) -> SuccessResult {
    match res {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e.to_string()),
    }
}

pub async fn read_properties(server_path: &Path) -> Vec<ServerProperty> {
    let Ok(content) = tokio::fs::read_to_string(server_path.join("server.properties")).await else {
        return Vec::new();
    };
    let mut properties = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            properties.push(ServerProperty {
                key: String::new(),
                value: String::new(),
                comment: Some(trimmed.to_string()),
            });
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            properties.push(ServerProperty {
                key: key.to_string(),
                value: value.to_string(),
                comment: None,
            });
        }
    }
    properties
}

pub async fn write_properties(server_path: &Path, properties: &[ServerProperty]) -> SuccessResult {
    let lines: Vec<String> = properties
        .iter()
        .map(|p| match p.comment.as_deref() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => format!("{}={}", p.key, p.value),
        })
        .collect();
    let content = format!("{}\n", lines.join("\n"));
    to_result(tokio::fs::write(server_path.join("server.properties"), content).await)
}

pub const WHITELIST: &str = "whitelist.json";
pub const BANNED_PLAYERS: &str = "banned-players.json";
pub const OPS: &str = "ops.json";

const PROFILE_API: &str = "https://api.mojang.com/users/profiles/minecraft/";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(
        default,
        rename = "bypassesPlayerLimit",
        skip_serializing_if = "Option::is_none"
    )]
    pub bypasses_player_limit: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ListEntry {
    pub fn player(name: impl Into<String>, uuid: impl Into<String>) -> Self {
        Self {
            uuid: Some(uuid.into()),
            name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or_default()
    }
}

pub async fn read_list(path: &Path) -> Vec<ListEntry> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub async fn write_list(path: &Path, entries: &[ListEntry]) -> SuccessResult {
    match serde_json::to_string_pretty(entries) {
        Ok(text) => to_result(tokio::fs::write(path, text).await),
        Err(e) => SuccessResult::err(e.to_string()),
    }
}

const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S %z";

pub fn minecraft_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.format(TIME_FORMAT).to_string()
}

pub fn parse_minecraft_time(text: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_str(text, TIME_FORMAT).ok()
}

pub fn offline_uuid(name: &str) -> String {
    let hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}")).into();
    uuid::Builder::from_md5_bytes(hash)
        .into_uuid()
        .hyphenated()
        .to_string()
}

#[derive(Deserialize)]
struct Profile {
    id: String,
    name: String,
}

async fn lookup(client: &reqwest::Client, name: &str) -> Result<ListEntry, String> {
    let response = client
        .get(format!("{PROFILE_API}{name}"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    match response.status().as_u16() {
        200 => {
            let profile: Profile = response.json().await.map_err(|e| e.to_string())?;
            let uuid = uuid::Uuid::parse_str(&profile.id)
                .map_err(|_| "Mojang returned an invalid id".to_string())?;
            Ok(ListEntry::player(
                profile.name,
                uuid.hyphenated().to_string(),
            ))
        }
        204 | 404 => Err("no such Minecraft account".into()),
        429 => Err("too many lookups, try again in a minute".into()),
        code => Err(format!("account lookup failed with HTTP {code}")),
    }
}

async fn online_mode(id: String) -> bool {
    !get_server_properties(id)
        .await
        .iter()
        .any(|p| p.key == "online-mode" && p.value.trim() == "false")
}

pub async fn resolve_players(
    id: String,
    names: Vec<String>,
) -> Vec<(String, Result<ListEntry, String>)> {
    if !online_mode(id).await {
        return names
            .into_iter()
            .map(|name| {
                let entry = ListEntry::player(name.clone(), offline_uuid(&name));
                (name, Ok(entry))
            })
            .collect();
    }
    let client = reqwest::Client::new();
    futures_util::stream::iter(names)
        .map(|name| {
            let client = &client;
            async move {
                let result = lookup(client, &name).await;
                (name, result)
            }
        })
        .buffered(4)
        .collect()
        .await
}

pub async fn get_server_properties(id: String) -> Vec<ServerProperty> {
    match server_store::get_server(id).await {
        Some(server) => read_properties(Path::new(&server.server_path)).await,
        None => Vec::new(),
    }
}

pub async fn save_server_properties(id: String, properties: Vec<ServerProperty>) -> SuccessResult {
    let Some(server) = server_store::get_server(id.clone()).await else {
        return SuccessResult::err("Server not found");
    };
    let result = write_properties(Path::new(&server.server_path), &properties).await;
    if result.success {
        if let Some(max) = properties.iter().find(|p| p.key == "max-players") {
            let players = update_player_capacity_string(Some(&server.players), &max.value);
            server_store::update_server_status(&id, server.status, Some(&players)).await;
        }
    }
    result
}

pub async fn get_list(id: String, file: &str) -> Vec<ListEntry> {
    match server_store::get_server(id).await {
        Some(server) => read_list(&Path::new(&server.server_path).join(file)).await,
        None => Vec::new(),
    }
}

pub async fn save_list(id: String, file: &str, entries: Vec<ListEntry>) -> SuccessResult {
    match server_store::get_server(id).await {
        Some(server) => write_list(&Path::new(&server.server_path).join(file), &entries).await,
        None => SuccessResult::err("Server not found"),
    }
}

pub async fn accept_eula(server_id: String) -> SuccessResult {
    let mut servers = server_store::load_server_list().await;
    let Some(server) = servers.iter_mut().find(|s| s.id == server_id) else {
        return SuccessResult::err("Server not found");
    };
    let eula = Path::new(&server.server_path).join("eula.txt");
    if let Err(e) = tokio::fs::write(eula, "eula=true\r\n").await {
        return SuccessResult::err(e.to_string());
    }
    server.eula_accepted = Some(true);
    match server_store::save_server_list(&servers).await {
        Ok(()) => SuccessResult::ok(),
        Err(e) => SuccessResult::err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        assert_eq!(
            offline_uuid("Notch"),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn minecraft_time_round_trips() {
        let time = chrono::DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let text = minecraft_time(time);
        assert_eq!(text, "2026-01-01 12:00:00 +0000");
        assert_eq!(parse_minecraft_time(&text).unwrap(), time);
        assert!(parse_minecraft_time("forever").is_none());
    }

    #[test]
    fn list_entry_keeps_unknown_fields() {
        let text =
            r#"[{"uuid":"u","name":"Steve","level":4,"bypassesPlayerLimit":false,"custom":1}]"#;
        let entries: Vec<ListEntry> = serde_json::from_str(text).unwrap();
        assert_eq!(entries[0].name(), "Steve");
        assert_eq!(entries[0].level, Some(4));
        assert_eq!(entries[0].bypasses_player_limit, Some(false));
        let back: Value = serde_json::to_value(&entries).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(text).unwrap());
    }
}
