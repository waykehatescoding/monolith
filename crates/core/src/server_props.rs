use crate::server_runner::update_player_capacity_string;
use crate::server_store;
use crate::types::{ServerProperty, SuccessResult};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

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

async fn read_names(path: &Path) -> Vec<String> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<Value>>(&content) {
        Ok(entries) if entries.iter().all(Value::is_object) => entries
            .iter()
            .filter_map(|e| e.get("name").and_then(Value::as_str).map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Serialize)]
struct WhitelistEntry<'a> {
    uuid: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct BanEntry<'a> {
    uuid: &'a str,
    name: &'a str,
    created: &'a str,
    source: &'a str,
    expires: &'a str,
    reason: &'a str,
}

async fn write_json<T: Serialize>(path: &Path, data: Vec<T>) -> SuccessResult {
    match serde_json::to_string_pretty(&data) {
        Ok(text) => to_result(tokio::fs::write(path, text).await),
        Err(e) => SuccessResult::err(e.to_string()),
    }
}

pub async fn read_whitelist(server_path: &Path) -> Vec<String> {
    read_names(&server_path.join("whitelist.json")).await
}

pub async fn write_whitelist(server_path: &Path, players: &[String]) -> SuccessResult {
    let data = players
        .iter()
        .map(|name| WhitelistEntry { uuid: "", name })
        .collect();
    write_json(&server_path.join("whitelist.json"), data).await
}

pub async fn read_banlist(server_path: &Path) -> Vec<String> {
    read_names(&server_path.join("banned-players.json")).await
}

pub async fn write_banlist(server_path: &Path, players: &[String]) -> SuccessResult {
    let created = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let data = players
        .iter()
        .map(|name| BanEntry {
            uuid: "",
            name,
            created: &created,
            source: "Monolith",
            expires: "forever",
            reason: "Banned by operator",
        })
        .collect();
    write_json(&server_path.join("banned-players.json"), data).await
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

pub async fn get_whitelist(id: String) -> Vec<String> {
    match server_store::get_server(id).await {
        Some(server) => read_whitelist(Path::new(&server.server_path)).await,
        None => Vec::new(),
    }
}

pub async fn save_whitelist(id: String, players: Vec<String>) -> SuccessResult {
    match server_store::get_server(id).await {
        Some(server) => write_whitelist(Path::new(&server.server_path), &players).await,
        None => SuccessResult::err("Server not found"),
    }
}

pub async fn get_banlist(id: String) -> Vec<String> {
    match server_store::get_server(id).await {
        Some(server) => read_banlist(Path::new(&server.server_path)).await,
        None => Vec::new(),
    }
}

pub async fn save_banlist(id: String, players: Vec<String>) -> SuccessResult {
    match server_store::get_server(id).await {
        Some(server) => write_banlist(Path::new(&server.server_path), &players).await,
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
