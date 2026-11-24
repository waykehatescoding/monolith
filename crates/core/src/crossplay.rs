use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config_files::{yaml_get, yaml_set};
use crate::modrinth_installs::{install_at, remove_at};
use crate::server_create::{download_file, FrameworkHashes};
use crate::types::{ContentSource, ModrinthInstallRequest, ModrinthProjectType, ServerRecord};

const API: &str = "https://download.geysermc.org/v2/projects";
const USER_AGENT: &str = "Monolith (Minecraft server manager)";
const GEYSER_MODRINTH: &str = "wKkoqHrH";
const FLOODGATE_MODRINTH: &str = "bWrNNfkb";
const FABRIC_API_MODRINTH: &str = "P7dR8mSH";
pub const DEFAULT_BEDROCK_PORT: u16 = 19132;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crossplay {
    pub supported: bool,
    pub installed: bool,
    pub bedrock_port: u16,
    pub needs_restart: bool,
}

fn platform(framework: &str) -> Option<&'static str> {
    match framework {
        "Paper" | "Purpur" => Some("spigot"),
        "Fabric" => Some("fabric"),
        "Velocity" => Some("velocity"),
        _ => None,
    }
}

fn content_dir(server: &Path, platform: &str) -> PathBuf {
    server.join(if platform == "fabric" {
        "mods"
    } else {
        "plugins"
    })
}

fn config_path(server: &Path, platform: &str) -> PathBuf {
    match platform {
        "fabric" => server.join("config").join("Geyser-Fabric"),
        "velocity" => server.join("plugins").join("geyser"),
        _ => server.join("plugins").join("Geyser-Spigot"),
    }
    .join("config.yml")
}

async fn find_jars(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return found;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with(prefix) && name.ends_with(".jar") {
            found.push(entry.path());
        }
    }
    found
}

pub fn apply_floodgate_auth(config: &str) -> Option<String> {
    ["java.auth-type", "remote.auth-type"]
        .iter()
        .find(|key| yaml_get(config, key).is_some())
        .filter(|key| yaml_get(config, key).as_deref() != Some("floodgate"))
        .and_then(|key| yaml_set(config, key, "floodgate"))
}

pub async fn status(server: &ServerRecord) -> Crossplay {
    let Some(platform) = platform(&server.framework) else {
        return Crossplay::default();
    };
    let path = Path::new(&server.server_path);
    let installed = !find_jars(&content_dir(path, platform), "geyser")
        .await
        .is_empty();
    let mut result = Crossplay {
        supported: true,
        installed,
        bedrock_port: DEFAULT_BEDROCK_PORT,
        needs_restart: false,
    };
    if !installed {
        return result;
    }
    let config = config_path(path, platform);
    let Ok(text) = tokio::fs::read_to_string(&config).await else {
        result.needs_restart = true;
        return result;
    };
    if let Some(port) = yaml_get(&text, "bedrock.port").and_then(|p| p.trim().parse().ok()) {
        result.bedrock_port = port;
    }
    if let Some(updated) = apply_floodgate_auth(&text) {
        result.needs_restart = tokio::fs::write(&config, updated).await.is_ok();
    }
    result
}

pub async fn bedrock_port(server: &ServerRecord) -> Option<u16> {
    let status = status(server).await;
    status.installed.then_some(status.bedrock_port)
}

async fn latest(project: &str, platform: &str) -> Result<(String, String, String), String> {
    let info: Value = reqwest::Client::new()
        .get(format!("{API}/{project}/versions/latest/builds/latest"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let download = &info["downloads"][platform];
    let (Some(version), Some(build), Some(name), Some(sha256)) = (
        info["version"].as_str(),
        info["build"].as_i64(),
        download["name"].as_str(),
        download["sha256"].as_str(),
    ) else {
        return Err(format!("GeyserMC has no {project} download for {platform}"));
    };
    Ok((
        format!("{API}/{project}/versions/{version}/builds/{build}/downloads/{platform}"),
        crate::safety::sanitize_download_file_name(name)?,
        sha256.to_string(),
    ))
}

async fn install_from_geyser(dir: &Path, project: &str, platform: &str) -> Result<(), String> {
    let (url, name, sha256) = latest(project, platform).await?;
    for old in find_jars(dir, &project.to_lowercase()).await {
        let _ = tokio::fs::remove_file(old).await;
    }
    let hashes = FrameworkHashes {
        sha1: None,
        sha256: Some(sha256),
    };
    download_file(&url, &dir.join(name), |_, _| {}, Some(&hashes)).await
}

async fn install_from_modrinth(server: &ServerRecord, project_id: &str) -> Result<(), String> {
    let result = install_at(
        Path::new(&server.server_path),
        ModrinthInstallRequest {
            source: ContentSource::Modrinth,
            project_id: project_id.to_string(),
            project_type: ModrinthProjectType::Mod,
            loader: Some("fabric".into()),
            game_version: Some(server.version.clone()),
            version_id: None,
            title: None,
            slug: None,
            icon_url: None,
        },
    )
    .await;
    match result.success {
        true => Ok(()),
        false => Err(result.error.unwrap_or_else(|| "Install failed".into())),
    }
}

pub async fn enable(server: ServerRecord) -> Result<(), String> {
    let platform = platform(&server.framework).ok_or_else(|| {
        format!(
            "Crossplay needs Paper, Purpur, Fabric or Velocity. This server runs {}.",
            server.framework
        )
    })?;
    let dir = content_dir(Path::new(&server.server_path), platform);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    if platform == "fabric" {
        for project in [FABRIC_API_MODRINTH, GEYSER_MODRINTH, FLOODGATE_MODRINTH] {
            install_from_modrinth(&server, project).await?;
        }
        return Ok(());
    }
    install_from_geyser(&dir, "geyser", platform).await?;
    install_from_geyser(&dir, "floodgate", platform).await
}

pub async fn disable(server: ServerRecord) -> Result<(), String> {
    let platform = platform(&server.framework).ok_or("Crossplay is not supported here")?;
    let path = Path::new(&server.server_path);
    if platform == "fabric" {
        for project in [GEYSER_MODRINTH, FLOODGATE_MODRINTH] {
            let removed = remove_at(path, ContentSource::Modrinth, project).await;
            if !removed.success {
                return Err(removed.error.unwrap_or_default());
            }
        }
    }
    let dir = content_dir(path, platform);
    for prefix in ["geyser", "floodgate"] {
        for jar in find_jars(&dir, prefix).await {
            tokio::fs::remove_file(jar)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_auth_type_to_floodgate() {
        let legacy = "bedrock:\n  port: 19132\nremote:\n  address: auto\n  auth-type: online\n";
        let fixed = apply_floodgate_auth(legacy).unwrap();
        assert!(fixed.contains("auth-type: floodgate"));
        assert_eq!(apply_floodgate_auth(&fixed), None);
        let current = "java:\n  auth-type: online\n";
        assert_eq!(
            apply_floodgate_auth(current).as_deref(),
            Some("java:\n  auth-type: floodgate\n")
        );
        assert_eq!(apply_floodgate_auth("bedrock:\n  port: 19132\n"), None);
    }

    #[test]
    fn maps_frameworks_to_geyser_platforms() {
        assert_eq!(platform("Purpur"), Some("spigot"));
        assert_eq!(platform("Fabric"), Some("fabric"));
        assert_eq!(platform("Forge"), None);
    }
}
