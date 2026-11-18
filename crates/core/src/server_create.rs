use crate::events::{emit, Event};
use crate::java_manager::{ensure_java_installed, get_required_java_version, JavaInstallStage};
use crate::paths::servers_dir;
use crate::safety::sanitize_download_file_name;
use crate::server_framework::install_analytics_plugin;
use crate::server_store::{
    clamp_ram, load_server_list, sanitize_server_name, save_server_list, validate_server_path,
};
use crate::types::{
    CreateServerParams, CreateServerResult, MinecraftVersion, ServerCreationProgress,
    ServerCreationStage, ServerRecord, ServerRuntimeStatus,
};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde_json::Value;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const PAPER_API_BASE: &str = "https://fill.papermc.io/v3/projects/paper";
const PAPER_USER_AGENT: &str = "Monolith/1.2.0 (Minecraft server manager; https://github.com)";
const PURPUR_API_BASE: &str = "https://api.purpurmc.org/v2/purpur";
const VANILLA_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MALFORMED: &str = "Unexpected response from version API";
const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";

static VERSION_CACHE: Lazy<Mutex<HashMap<String, Vec<MinecraftVersion>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Default)]
pub struct FrameworkHashes {
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FrameworkDownload {
    pub url: String,
    pub filename: String,
    pub jar_file: String,
    pub hashes: Option<FrameworkHashes>,
}

fn is_release_version(id: &str) -> bool {
    let digits = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() > 1 && parts.iter().all(|p| digits(p))
}

async fn fetch_value(
    url: &str,
    user_agent: Option<&str>,
    accept_json: bool,
    fail_msg: String,
) -> Result<Value, String> {
    let mut req = reqwest::Client::new().get(url);
    if let Some(ua) = user_agent {
        req = req.header("User-Agent", ua);
    }
    if accept_json {
        req = req.header("Accept", "application/json");
    }
    let res = req.send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("{}: {}", fail_msg, res.status().as_u16()));
    }
    res.json::<Value>().await.map_err(|e| e.to_string())
}

async fn paper_download(version: &str) -> Result<FrameworkDownload, String> {
    let url = format!("{}/versions/{}/builds", PAPER_API_BASE, version);
    let data = fetch_value(
        &url,
        Some(PAPER_USER_AGENT),
        true,
        format!("Failed to fetch Paper builds for {}", version),
    )
    .await?;

    let builds: Vec<&Value> = data
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|b| b["downloads"]["server:default"].is_object())
                .collect()
        })
        .unwrap_or_default();
    let stable: Vec<&Value> = builds
        .iter()
        .copied()
        .filter(|b| b["channel"] == "STABLE")
        .collect();
    let pool = if stable.is_empty() { &builds } else { &stable };
    let latest = pool
        .iter()
        .rev()
        .max_by_key(|b| b["id"].as_i64().unwrap_or(i64::MIN))
        .ok_or_else(|| format!("No builds found for Paper {}", version))?;

    let download = &latest["downloads"]["server:default"];
    let url = download["url"]
        .as_str()
        .ok_or_else(|| format!("No server download found for Paper {}", version))?;
    let hashes = download["checksums"]["sha256"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| FrameworkHashes {
            sha1: None,
            sha256: Some(s.to_string()),
        });

    Ok(FrameworkDownload {
        url: url.to_string(),
        filename: download["name"].as_str().unwrap_or_default().to_string(),
        jar_file: "server.jar".into(),
        hashes,
    })
}

fn purpur_download(version: &str) -> FrameworkDownload {
    FrameworkDownload {
        url: format!("{}/{}/latest/download", PURPUR_API_BASE, version),
        filename: format!("purpur-{}.jar", version),
        jar_file: "server.jar".into(),
        hashes: None,
    }
}

async fn vanilla_download(version: &str) -> Result<FrameworkDownload, String> {
    let manifest = fetch_value(
        VANILLA_MANIFEST_URL,
        None,
        false,
        "Failed to fetch Minecraft version manifest".into(),
    )
    .await?;
    let entry_url = manifest["versions"]
        .as_array()
        .and_then(|v| v.iter().find(|e| e["id"] == version))
        .and_then(|e| e["url"].as_str())
        .ok_or_else(|| format!("Vanilla version {} not found in manifest", version))?
        .to_string();

    let data = fetch_value(
        &entry_url,
        None,
        false,
        format!("Failed to fetch version details for {}", version),
    )
    .await?;
    let server = &data["downloads"]["server"];
    if !server.is_object() {
        return Err(format!(
            "No server download available for Vanilla {}",
            version
        ));
    }

    Ok(FrameworkDownload {
        url: server["url"].as_str().unwrap_or_default().to_string(),
        filename: format!("server-{}.jar", version),
        jar_file: "server.jar".into(),
        hashes: Some(FrameworkHashes {
            sha1: server["sha1"].as_str().map(str::to_string),
            sha256: None,
        }),
    })
}

async fn latest_stable(kind: &str) -> Result<String, String> {
    let data = fetch_value(
        &format!("{}/versions/{}", FABRIC_META_URL, kind),
        None,
        false,
        format!("Failed to fetch Fabric {} versions", kind),
    )
    .await?;
    data.as_array()
        .and_then(|a| a.iter().find(|v| v["stable"] == true))
        .and_then(|v| v["version"].as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("No stable Fabric {} found", kind))
}

async fn fabric_download(version: &str) -> Result<FrameworkDownload, String> {
    let loader = latest_stable("loader").await?;
    let installer = latest_stable("installer").await?;
    let url = format!(
        "{}/versions/loader/{}/{}/{}/server/jar",
        FABRIC_META_URL, version, loader, installer
    );
    let name = format!(
        "fabric-server-mc.{}-loader.{}-launcher.{}.jar",
        version, loader, installer
    );
    Ok(FrameworkDownload {
        url,
        filename: name.clone(),
        jar_file: name,
        hashes: None,
    })
}

pub async fn resolve_framework_download(
    framework: &str,
    version: &str,
) -> Result<FrameworkDownload, String> {
    match framework {
        "Paper" => paper_download(version).await,
        "Purpur" => Ok(purpur_download(version)),
        "Vanilla" => vanilla_download(version).await,
        "Fabric" => fabric_download(version).await,
        _ => Err(format!("Unsupported framework: {}", framework)),
    }
}

fn version(id: &str, stable: bool) -> MinecraftVersion {
    MinecraftVersion {
        id: id.to_string(),
        stable,
    }
}

async fn fetch_version_list(framework: &str) -> Result<Vec<MinecraftVersion>, String> {
    let fail = format!("Failed to fetch {} versions", framework);
    match framework {
        "Paper" => {
            let data = fetch_value(PAPER_API_BASE, Some(PAPER_USER_AGENT), false, fail).await?;
            data["versions"]
                .as_object()
                .map(|m| {
                    m.values()
                        .filter_map(|v| v.as_array())
                        .flatten()
                        .filter_map(|v| v.as_str())
                        .map(|id| version(id, is_release_version(id)))
                        .collect()
                })
                .ok_or_else(|| MALFORMED.to_string())
        }
        "Purpur" => {
            let data = fetch_value(PURPUR_API_BASE, None, false, fail).await?;
            data["versions"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .rev()
                        .filter_map(|v| v.as_str())
                        .map(|id| version(id, is_release_version(id)))
                        .collect()
                })
                .ok_or_else(|| MALFORMED.to_string())
        }
        "Fabric" => {
            let data = fetch_value(
                &format!("{}/versions/game", FABRIC_META_URL),
                None,
                false,
                fail,
            )
            .await?;
            data.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| Some(version(v["version"].as_str()?, v["stable"] == true)))
                        .collect()
                })
                .ok_or_else(|| MALFORMED.to_string())
        }
        "Vanilla" => {
            let data = fetch_value(VANILLA_MANIFEST_URL, None, false, fail).await?;
            data["versions"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| {
                            let kind = v["type"].as_str()?;
                            (kind == "release" || kind == "snapshot").then(|| {
                                version(v["id"].as_str().unwrap_or_default(), kind == "release")
                            })
                        })
                        .collect()
                })
                .ok_or_else(|| MALFORMED.to_string())
        }
        _ => Err(format!("Unsupported framework: {}", framework)),
    }
}

fn net_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Download timed out".to_string()
    } else {
        e.to_string()
    }
}

pub async fn download_file(
    url: &str,
    dest_path: &Path,
    on_progress: impl Fn(u64, u64) + Send + Sync,
    hashes: Option<&FrameworkHashes>,
) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" {
        return Err("Downloads must use HTTPS".into());
    }

    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.url().scheme() != "https" {
            attempt.error("Downloads must use HTTPS")
        } else if attempt.previous().len() >= 10 {
            attempt.error("Too many redirects")
        } else {
            attempt.follow()
        }
    });
    let client = reqwest::Client::builder()
        .redirect(policy)
        .read_timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(url).send().await.map_err(net_error)?;
    if res.status().as_u16() != 200 {
        return Err(format!(
            "Download failed with status {}",
            res.status().as_u16()
        ));
    }

    let total = res.content_length().unwrap_or(0);
    let mut sha1 = hashes.and_then(|h| h.sha1.as_ref()).map(|_| Sha1::new());
    let mut sha256 = hashes
        .and_then(|h| h.sha256.as_ref())
        .map(|_| Sha256::new());

    let written = async {
        let mut file = tokio::fs::File::create(dest_path)
            .await
            .map_err(|e| e.to_string())?;
        let mut downloaded = 0u64;
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(net_error)?;
            downloaded += chunk.len() as u64;
            if let Some(h) = sha1.as_mut() {
                h.update(&chunk);
            }
            if let Some(h) = sha256.as_mut() {
                h.update(&chunk);
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            on_progress(downloaded, total);
        }
        file.flush().await.map_err(|e| e.to_string())
    }
    .await;

    if let Err(err) = written {
        let _ = tokio::fs::remove_file(dest_path).await;
        return Err(err);
    }

    let mut mismatches = Vec::new();
    if let (Some(expected), Some(h)) = (hashes.and_then(|h| h.sha1.as_ref()), sha1) {
        if hex::encode(h.finalize()) != expected.to_lowercase() {
            mismatches.push("sha1");
        }
    }
    if let (Some(expected), Some(h)) = (hashes.and_then(|h| h.sha256.as_ref()), sha256) {
        if hex::encode(h.finalize()) != expected.to_lowercase() {
            mismatches.push("sha256");
        }
    }
    if !mismatches.is_empty() {
        let _ = tokio::fs::remove_file(dest_path).await;
        return Err(format!(
            "Download integrity check failed ({})",
            mismatches.join(", ")
        ));
    }

    Ok(())
}

pub fn generate_start_bat(ram_mb: u32, jar_file: &str) -> String {
    let jar = sanitize_download_file_name(jar_file).unwrap_or_else(|_| "server.jar".into());
    let ram = if ram_mb == 0 {
        1024
    } else {
        ram_mb.clamp(256, 65536)
    };
    let initial = ((ram as f64 * 0.4).floor() as u32).min(ram).max(256);
    format!(
        "@echo off\r\njava -Xms{initial}M -Xmx{ram}M -XX:+DisableExplicitGC -XX:+ParallelRefProcEnabled -XX:+PerfDisableSharedMem -XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1HeapRegionSize=8M -XX:G1HeapWastePercent=5 -XX:G1MaxNewSizePercent=40 -XX:G1MixedGCCountTarget=4 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1NewSizePercent=30 -XX:G1RSetUpdatingPauseTimePercent=5 -XX:G1ReservePercent=20 -XX:InitiatingHeapOccupancyPercent=15 -XX:MaxGCPauseMillis=200 -XX:MaxTenuringThreshold=1 -XX:SurvivorRatio=32 -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true -jar \"{jar}\" nogui\r\npause\r\n"
    )
}

fn progress(stage: ServerCreationStage, message: impl Into<String>, percent: f64) {
    emit(Event::CreationProgress(ServerCreationProgress {
        stage,
        message: message.into(),
        percent,
        bytes_downloaded: None,
        bytes_total: None,
    }));
}

async fn create_server_inner(params: &CreateServerParams) -> Result<ServerRecord, String> {
    let folder_name = sanitize_server_name(&params.name);
    let server_dir = servers_dir().join(&folder_name);

    let mut final_dir = server_dir;
    let mut suffix = 1;
    while tokio::fs::try_exists(&final_dir).await.unwrap_or(false) {
        final_dir = servers_dir().join(format!("{}-{}", folder_name, suffix));
        suffix += 1;
    }

    progress(
        ServerCreationStage::CreatingFolder,
        "Creating server folder...",
        5.0,
    );
    tokio::fs::create_dir_all(&final_dir)
        .await
        .map_err(|e| e.to_string())?;

    progress(
        ServerCreationStage::Downloading,
        format!(
            "Resolving {} {} download...",
            params.framework, params.version
        ),
        8.0,
    );
    let info = resolve_framework_download(&params.framework, &params.version).await?;

    progress(
        ServerCreationStage::SetupJava,
        "Checking required Java version...",
        12.0,
    );
    let java_version = get_required_java_version(&params.version);
    let _ = ensure_java_installed(
        java_version,
        Some(Box::new(move |stage, percent, downloaded, total| {
            let message = if stage == JavaInstallStage::Extracting {
                format!("Extracting Java {}...", java_version)
            } else {
                format!("Downloading Java {}... {}%", java_version, percent.round())
            };
            emit(Event::CreationProgress(ServerCreationProgress {
                stage: ServerCreationStage::SetupJava,
                message,
                percent: 12.0 + percent * 0.08,
                bytes_downloaded: downloaded,
                bytes_total: total,
            }));
        })),
    )
    .await;

    progress(
        ServerCreationStage::Downloading,
        format!("Downloading {} {}...", params.framework, params.version),
        20.0,
    );
    let dest_filename = sanitize_download_file_name(&info.jar_file)?;
    let jar_path = final_dir.join(&dest_filename);
    let label = format!("{} {}", params.framework, params.version);
    download_file(
        &info.url,
        &jar_path,
        |downloaded, total| {
            let downloaded_mb = downloaded as f64 / 1024.0 / 1024.0;
            let (message, percent) = if total > 0 {
                let pct = downloaded as f64 / total as f64 * 100.0;
                let total_mb = total as f64 / 1024.0 / 1024.0;
                (
                    format!(
                        "Downloading {}... {}% ({:.1} / {:.1} MB)",
                        label,
                        pct.round(),
                        downloaded_mb,
                        total_mb
                    ),
                    (20.0 + pct * 0.7).round(),
                )
            } else {
                let estimated = (downloaded as f64 / (50.0 * 1024.0 * 1024.0) * 100.0).min(99.0);
                (
                    format!("Downloading {}... {:.1} MB", label, downloaded_mb),
                    (20.0 + estimated * 0.7).round(),
                )
            };
            emit(Event::CreationProgress(ServerCreationProgress {
                stage: ServerCreationStage::Downloading,
                message,
                percent,
                bytes_downloaded: Some(downloaded),
                bytes_total: Some(total),
            }));
        },
        info.hashes.as_ref(),
    )
    .await?;

    progress(
        ServerCreationStage::WritingFiles,
        "Creating start.bat...",
        85.0,
    );
    tokio::fs::write(
        final_dir.join("start.bat"),
        generate_start_bat(clamp_ram(params.ram_mb), &dest_filename),
    )
    .await
    .map_err(|e| e.to_string())?;

    let plugin_compatible = params.framework == "Paper" || params.framework == "Purpur";
    let analytics_enabled = params.enable_analytics.unwrap_or(false) && plugin_compatible;
    if analytics_enabled {
        progress(
            ServerCreationStage::WritingFiles,
            "Installing MonolithAnalytics plugin...",
            90.0,
        );
        install_analytics_plugin(&final_dir).await;
    }

    if plugin_compatible {
        progress(
            ServerCreationStage::WritingFiles,
            "Installing default plugins...",
            91.0,
        );
        crate::default_plugins::install_default_plugins(
            &final_dir,
            &params.framework,
            &params.version,
            params.default_plugins.as_deref(),
        )
        .await;
    }

    let record = ServerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name: params.name.clone(),
        framework: params.framework.clone(),
        version: params.version.clone(),
        ram_mb: clamp_ram(params.ram_mb),
        status: ServerRuntimeStatus::Offline,
        players: "0/20".into(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        server_path: final_dir.to_string_lossy().into_owned(),
        eula_accepted: Some(false),
        jar_file: Some(dest_filename),
        analytics_enabled: Some(analytics_enabled),
        ..Default::default()
    };

    progress(
        ServerCreationStage::WritingFiles,
        "Saving server configuration...",
        95.0,
    );
    let mut servers = load_server_list().await;
    servers.push(record.clone());
    save_server_list(&servers).await?;

    progress(
        ServerCreationStage::Done,
        "Server created successfully!",
        100.0,
    );
    Ok(record)
}

pub async fn create_server(params: CreateServerParams) -> CreateServerResult {
    let server_dir = servers_dir().join(sanitize_server_name(&params.name));
    if !validate_server_path(&server_dir.to_string_lossy()) {
        return CreateServerResult {
            success: false,
            server: None,
            error: Some("Invalid server name".into()),
        };
    }
    match create_server_inner(&params).await {
        Ok(server) => CreateServerResult {
            success: true,
            server: Some(server),
            error: None,
        },
        Err(error) => {
            progress(ServerCreationStage::Error, error.clone(), 0.0);
            CreateServerResult {
                success: false,
                server: None,
                error: Some(error),
            }
        }
    }
}

pub async fn get_minecraft_versions(framework: String) -> Result<Vec<MinecraftVersion>, String> {
    if let Ok(cache) = VERSION_CACHE.lock() {
        if let Some(cached) = cache.get(&framework) {
            return Ok(cached.clone());
        }
    }
    let versions = fetch_version_list(&framework).await?;
    if let Ok(mut cache) = VERSION_CACHE.lock() {
        cache.insert(framework, versions.clone());
    }
    Ok(versions)
}
