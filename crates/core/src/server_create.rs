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
use std::path::{Path, PathBuf};
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
const QUILT_META_URL: &str = "https://meta.quiltmc.org/v3";
const FORGE_PROMOS_URL: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_VERSIONS_URL: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const INSTALLER_JAR: &str = "installer.jar";

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
    pub install_args: Vec<String>,
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
        ..Default::default()
    })
}

fn purpur_download(version: &str) -> FrameworkDownload {
    FrameworkDownload {
        url: format!("{}/{}/latest/download", PURPUR_API_BASE, version),
        filename: format!("purpur-{}.jar", version),
        jar_file: "server.jar".into(),
        hashes: None,
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
    })
}

fn installer_download(url: String, install_args: &[&str]) -> FrameworkDownload {
    FrameworkDownload {
        url,
        filename: INSTALLER_JAR.into(),
        jar_file: INSTALLER_JAR.into(),
        hashes: None,
        install_args: install_args.iter().map(|a| a.to_string()).collect(),
    }
}

async fn forge_download(version: &str) -> Result<FrameworkDownload, String> {
    let data = fetch_value(
        FORGE_PROMOS_URL,
        None,
        false,
        "Failed to fetch Forge versions".into(),
    )
    .await?;
    let promos = &data["promos"];
    let forge = promos[format!("{version}-recommended").as_str()]
        .as_str()
        .or_else(|| promos[format!("{version}-latest").as_str()].as_str())
        .ok_or_else(|| format!("No Forge build found for {version}"))?;
    let full = format!("{version}-{forge}");
    Ok(installer_download(
        format!("{FORGE_MAVEN}/{full}/forge-{full}-installer.jar"),
        &["--installServer"],
    ))
}

fn is_prerelease(v: &str) -> bool {
    v.contains("beta") || v.contains("alpha")
}

fn neoforge_mc_version(neoforge: &str) -> Option<String> {
    let mut parts = neoforge.split(['.', '-']);
    let mut next = || parts.next()?.parse::<u32>().ok();
    let (major, minor, patch) = (next()?, next()?, next()?);
    if major < 20 {
        None
    } else if major >= 25 {
        Some(if patch == 0 {
            format!("{major}.{minor}")
        } else {
            format!("{major}.{minor}.{patch}")
        })
    } else if minor == 0 {
        Some(format!("1.{major}"))
    } else {
        Some(format!("1.{major}.{minor}"))
    }
}

async fn neoforge_versions() -> Result<Vec<String>, String> {
    let data = fetch_value(
        NEOFORGE_VERSIONS_URL,
        None,
        false,
        "Failed to fetch NeoForge versions".into(),
    )
    .await?;
    data["versions"]
        .as_array()
        .map(|a| {
            a.iter()
                .rev()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| MALFORMED.to_string())
}

async fn neoforge_download(version: &str) -> Result<FrameworkDownload, String> {
    let all = neoforge_versions().await?;
    let matching: Vec<&String> = all
        .iter()
        .filter(|v| neoforge_mc_version(v).as_deref() == Some(version))
        .collect();
    let neoforge = matching
        .iter()
        .find(|v| !is_prerelease(v))
        .or(matching.first())
        .ok_or_else(|| format!("No NeoForge build found for {version}"))?;
    Ok(installer_download(
        format!("{NEOFORGE_MAVEN}/{neoforge}/neoforge-{neoforge}-installer.jar"),
        &["--installServer"],
    ))
}

async fn quilt_download(version: &str) -> Result<FrameworkDownload, String> {
    let fail = || "Failed to fetch Quilt versions".to_string();
    let loaders = fetch_value(
        &format!("{QUILT_META_URL}/versions/loader"),
        None,
        false,
        fail(),
    )
    .await?;
    let loader = loaders
        .as_array()
        .and_then(|a| {
            a.iter()
                .filter_map(|v| v["version"].as_str())
                .find(|v| !v.contains('-'))
        })
        .ok_or_else(|| "No stable Quilt loader found".to_string())?;
    let installers = fetch_value(
        &format!("{QUILT_META_URL}/versions/installer"),
        None,
        false,
        fail(),
    )
    .await?;
    let url = installers[0]["url"]
        .as_str()
        .ok_or_else(|| "No Quilt installer found".to_string())?;
    Ok(installer_download(
        url.to_string(),
        &[
            "install",
            "server",
            version,
            loader,
            "--download-server",
            "--install-dir=.",
        ],
    ))
}

fn find_launch_target(dir: &Path) -> Option<String> {
    let root_jar = std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let launcher = ["forge-", "neoforge-", "quilt-server-launch"]
            .iter()
            .any(|p| name.starts_with(p));
        (launcher && name.ends_with(".jar") && !name.contains("installer")).then_some(name)
    });
    let args_name = if cfg!(windows) {
        "win_args.txt"
    } else {
        "unix_args.txt"
    };
    root_jar.or_else(|| {
        walkdir::WalkDir::new(dir.join("libraries"))
            .into_iter()
            .flatten()
            .find(|e| e.file_name() == args_name)
            .and_then(|e| e.path().strip_prefix(dir).ok().map(Path::to_path_buf))
            .map(|p| format!("@{}", p.to_string_lossy().replace('\\', "/")))
    })
}

pub async fn run_installer(
    info: &FrameworkDownload,
    dir: &Path,
    java: Result<PathBuf, String>,
) -> Result<String, String> {
    if info.install_args.is_empty() {
        return Ok(info.jar_file.clone());
    }
    let mut cmd = tokio::process::Command::new(java?);
    cmd.arg("-jar")
        .arg(&info.jar_file)
        .args(&info.install_args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let status = cmd.status().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "The installer failed (see {}.log in the server folder)",
            info.jar_file
        ));
    }
    let _ = tokio::fs::remove_file(dir.join(&info.jar_file)).await;
    let _ = tokio::fs::remove_file(dir.join(format!("{}.log", info.jar_file))).await;
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || find_launch_target(&dir))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| "The installer finished but no server launcher was found".to_string())
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
        "Quilt" => quilt_download(version).await,
        "Forge" => forge_download(version).await,
        "NeoForge" => neoforge_download(version).await,
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
        "Fabric" | "Quilt" => {
            let base = if framework == "Fabric" {
                FABRIC_META_URL
            } else {
                QUILT_META_URL
            };
            let data = fetch_value(&format!("{}/versions/game", base), None, false, fail).await?;
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
        "Forge" => {
            let data = fetch_value(FORGE_PROMOS_URL, None, false, fail).await?;
            let promos = data["promos"]
                .as_object()
                .ok_or_else(|| MALFORMED.to_string())?;
            let mut ids: Vec<&str> = promos
                .keys()
                .filter_map(|k| k.strip_suffix("-latest"))
                .filter(|id| id.split('.').nth(1).and_then(|m| m.parse::<u32>().ok()) >= Some(12))
                .collect();
            ids.sort_by(|a, b| crate::update_checker::compare_versions(b, a).cmp(&0));
            Ok(ids
                .into_iter()
                .map(|id| version(id, promos.contains_key(&format!("{id}-recommended"))))
                .collect())
        }
        "NeoForge" => {
            let mut out: Vec<MinecraftVersion> = Vec::new();
            for v in neoforge_versions().await? {
                let Some(id) = neoforge_mc_version(&v) else {
                    continue;
                };
                let stable = !is_prerelease(&v);
                match out.iter_mut().find(|m| m.id == id) {
                    Some(m) => m.stable |= stable,
                    None => out.push(version(&id, stable)),
                }
            }
            Ok(out)
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

pub fn is_args_file(jar_file: &str) -> bool {
    jar_file.starts_with("@libraries/")
        && !jar_file.contains("..")
        && !jar_file.contains(['"', '&', '|', '%', '^', '<', '>', '\r', '\n', ' '])
}

pub fn generate_start_bat(ram_mb: u32, jar_file: &str) -> String {
    let target = if is_args_file(jar_file) {
        jar_file.to_string()
    } else {
        let jar = sanitize_download_file_name(jar_file).unwrap_or_else(|_| "server.jar".into());
        format!("-jar \"{jar}\"")
    };
    let ram = if ram_mb == 0 {
        1024
    } else {
        ram_mb.clamp(256, 65536)
    };
    let initial = ((ram as f64 * 0.4).floor() as u32).min(ram).max(256);
    format!(
        "@echo off\r\njava -Xms{initial}M -Xmx{ram}M -XX:+DisableExplicitGC -XX:+ParallelRefProcEnabled -XX:+PerfDisableSharedMem -XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1HeapRegionSize=8M -XX:G1HeapWastePercent=5 -XX:G1MaxNewSizePercent=40 -XX:G1MixedGCCountTarget=4 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1NewSizePercent=30 -XX:G1RSetUpdatingPauseTimePercent=5 -XX:G1ReservePercent=20 -XX:InitiatingHeapOccupancyPercent=15 -XX:MaxGCPauseMillis=200 -XX:MaxTenuringThreshold=1 -XX:SurvivorRatio=32 -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true {target} nogui\r\npause\r\n"
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
    let java = ensure_java_installed(
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

    if !info.install_args.is_empty() {
        progress(
            ServerCreationStage::WritingFiles,
            format!("Running the {} installer...", params.framework),
            88.0,
        );
    }
    let launch = run_installer(&info, &final_dir, java).await?;

    progress(
        ServerCreationStage::WritingFiles,
        "Creating start.bat...",
        89.0,
    );
    tokio::fs::write(
        final_dir.join("start.bat"),
        generate_start_bat(clamp_ram(params.ram_mb), &launch),
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
        jar_file: Some(launch),
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

#[cfg(test)]
mod modded_tests {
    use super::*;

    #[test]
    fn maps_neoforge_versions() {
        assert_eq!(neoforge_mc_version("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_mc_version("21.0.167").as_deref(), Some("1.21"));
        assert_eq!(
            neoforge_mc_version("20.4.237-beta").as_deref(),
            Some("1.20.4")
        );
        assert_eq!(
            neoforge_mc_version("26.1.0.5-beta").as_deref(),
            Some("26.1")
        );
        assert_eq!(neoforge_mc_version("0.25w14craftmine.3-beta"), None);
    }

    #[test]
    fn start_bat_uses_args_file() {
        let bat = generate_start_bat(
            4096,
            "@libraries/net/neoforged/neoforge/21.1.77/win_args.txt",
        );
        assert!(bat.contains(" @libraries/net/neoforged/neoforge/21.1.77/win_args.txt nogui"));
        assert!(generate_start_bat(4096, "@libraries/x & calc").contains("-jar \"server.jar\""));
    }
}
