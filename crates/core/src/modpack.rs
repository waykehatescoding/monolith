use crate::content_sources::{curseforge_api_key, curseforge_files};
use crate::events::{emit, Event};
use crate::modrinth::fetch_modrinth_versions;
use crate::modrinth_installs::download_file;
use crate::safety::sanitize_download_file_name;
use crate::server_transfer::safe_join;
use crate::types::{ServerCreationProgress, ServerCreationStage};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const MRPACK_INDEX: &str = "modrinth.index.json";
const CURSEFORGE_MANIFEST: &str = "manifest.json";
const NO_KEY: &str = "CurseForge modpacks need your own CurseForge API key. Add it in Settings, or use a Modrinth .mrpack file instead.";

#[derive(Debug, Clone, PartialEq)]
pub struct ModpackInfo {
    pub name: String,
    pub version: String,
    pub framework: String,
    pub game_version: String,
    pub loader_version: Option<String>,
    pub curseforge: bool,
    pub files: usize,
}

#[derive(Deserialize)]
struct MrpackIndex {
    name: String,
    #[serde(default, rename = "versionId")]
    version_id: String,
    #[serde(default)]
    files: Vec<MrpackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Deserialize)]
struct MrpackFile {
    path: String,
    #[serde(default)]
    hashes: HashMap<String, String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    downloads: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeManifest {
    name: String,
    #[serde(default)]
    version: String,
    minecraft: CurseForgeMinecraft,
    #[serde(default)]
    files: Vec<CurseForgeFile>,
    overrides: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CurseForgeLoader>,
}

#[derive(Deserialize)]
struct CurseForgeLoader {
    id: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeFile {
    #[serde(rename = "fileID")]
    file_id: u64,
    #[serde(default = "yes")]
    required: bool,
}

fn yes() -> bool {
    true
}

enum Pack {
    Modrinth(MrpackIndex),
    CurseForge(CurseForgeManifest),
}

fn framework_name(loader: &str) -> Option<&'static str> {
    match loader {
        "fabric-loader" | "fabric" => Some("Fabric"),
        "quilt-loader" | "quilt" => Some("Quilt"),
        "forge" => Some("Forge"),
        "neoforge" => Some("NeoForge"),
        _ => None,
    }
}

fn mrpack_info(index: &MrpackIndex) -> Result<ModpackInfo, String> {
    let game_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or("The modpack does not say which Minecraft version it needs.")?;
    let loader = ["neoforge", "forge", "quilt-loader", "fabric-loader"]
        .into_iter()
        .find_map(|key| Some((framework_name(key)?, index.dependencies.get(key)?)));
    Ok(ModpackInfo {
        name: index.name.clone(),
        version: index.version_id.clone(),
        framework: loader.map_or("Vanilla", |(name, _)| name).to_string(),
        game_version,
        loader_version: loader.map(|(_, version)| version.clone()),
        curseforge: false,
        files: server_files(index).count(),
    })
}

fn server_files(index: &MrpackIndex) -> impl Iterator<Item = &MrpackFile> {
    index
        .files
        .iter()
        .filter(|f| f.env.get("server").map(String::as_str) != Some("unsupported"))
}

fn curseforge_info(manifest: &CurseForgeManifest) -> ModpackInfo {
    let loader = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .or(manifest.minecraft.mod_loaders.first())
        .and_then(|l| l.id.split_once('-'))
        .and_then(|(name, version)| Some((framework_name(name)?, version.to_string())));
    ModpackInfo {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        framework: loader
            .as_ref()
            .map_or("Vanilla", |(name, _)| name)
            .to_string(),
        game_version: manifest.minecraft.version.clone(),
        loader_version: loader.map(|(_, version)| version),
        curseforge: true,
        files: manifest.files.iter().filter(|f| f.required).count(),
    }
}

fn read_text(archive: &mut ZipArchive<File>, name: &str) -> Option<Result<String, String>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut raw = String::new();
    Some(
        entry
            .read_to_string(&mut raw)
            .map(|_| raw)
            .map_err(|e| e.to_string()),
    )
}

fn parse_pack(raw: Option<Result<String, String>>, curseforge: bool) -> Result<Pack, String> {
    let raw = raw.ok_or("This file is not a Modrinth or CurseForge modpack.")??;
    let invalid = |e: serde_json::Error| format!("The modpack manifest is invalid: {e}");
    if curseforge {
        serde_json::from_str(&raw)
            .map(Pack::CurseForge)
            .map_err(invalid)
    } else {
        serde_json::from_str(&raw)
            .map(Pack::Modrinth)
            .map_err(invalid)
    }
}

fn read_pack(path: &Path) -> Result<Pack, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|_| "This file is not a modpack archive.")?;
    match read_text(&mut archive, MRPACK_INDEX) {
        Some(raw) => parse_pack(Some(raw), false),
        None => parse_pack(read_text(&mut archive, CURSEFORGE_MANIFEST), true),
    }
}

async fn load_pack(path: PathBuf) -> Result<Pack, String> {
    tokio::task::spawn_blocking(move || read_pack(&path))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn inspect_modpack(path: PathBuf) -> Result<ModpackInfo, String> {
    match load_pack(path).await? {
        Pack::Modrinth(index) => mrpack_info(&index),
        Pack::CurseForge(manifest) => {
            if curseforge_api_key().await.is_none() {
                return Err(NO_KEY.into());
            }
            Ok(curseforge_info(&manifest))
        }
    }
}

pub async fn download_modrinth_modpack(project_id: String) -> Result<PathBuf, String> {
    let versions = fetch_modrinth_versions(&project_id, None, None).await?;
    let version = versions
        .iter()
        .find(|v| v["version_type"] == "release")
        .or(versions.first())
        .ok_or("This modpack has no versions to download.")?;
    let files = version["files"].as_array().cloned().unwrap_or_default();
    let file = files
        .iter()
        .find(|f| f["primary"] == true)
        .or(files.first())
        .ok_or("This modpack has no downloadable file.")?;
    let name = sanitize_download_file_name(file["filename"].as_str().unwrap_or_default())?;
    let dest = std::env::temp_dir().join("monolith-modpacks").join(name);
    download_file(
        file["url"].as_str().unwrap_or_default(),
        &dest,
        file.get("hashes"),
    )
    .await?;
    Ok(dest)
}

fn progress(done: usize, total: usize) {
    emit(Event::CreationProgress(ServerCreationProgress {
        stage: ServerCreationStage::Downloading,
        message: format!("Downloading modpack files, {done} of {total}"),
        percent: if total == 0 {
            100.0
        } else {
            (done as f64 / total as f64 * 100.0).round()
        },
        bytes_downloaded: None,
        bytes_total: None,
    }));
}

fn extract_overrides(pack: &Path, server: &Path, folders: &[&str]) -> Result<(), String> {
    let file = File::open(pack).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    for folder in folders {
        let prefix = format!("{folder}/");
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
            let name = entry.name().replace('\\', "/");
            let Some(relative) = name.strip_prefix(&prefix) else {
                continue;
            };
            if entry.is_dir() || relative.is_empty() {
                continue;
            }
            let Some(dest) = safe_join(server, relative) else {
                continue;
            };
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(&dest).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn install_mrpack(server: &Path, index: MrpackIndex) -> Result<Vec<String>, String> {
    let files: Vec<&MrpackFile> = server_files(&index).collect();
    for (done, file) in files.iter().enumerate() {
        progress(done, files.len());
        let dest = safe_join(server, &file.path)
            .ok_or_else(|| format!("The modpack has an unsafe file path: {}", file.path))?;
        let hashes = json!({
            "sha1": file.hashes.get("sha1"),
            "sha512": file.hashes.get("sha512"),
        });
        if !file.hashes.contains_key("sha1") && !file.hashes.contains_key("sha512") {
            return Err(format!("{} has no checksum to verify", file.path));
        }
        let url = file
            .downloads
            .first()
            .ok_or_else(|| format!("{} has no download link", file.path))?;
        download_file(url, &dest, Some(&hashes))
            .await
            .map_err(|e| format!("{}: {e}", file.path))?;
    }
    progress(files.len(), files.len());
    Ok(Vec::new())
}

async fn install_curseforge(
    server: &Path,
    manifest: &CurseForgeManifest,
) -> Result<Vec<String>, String> {
    let ids: Vec<u64> = manifest
        .files
        .iter()
        .filter(|f| f.required)
        .map(|f| f.file_id)
        .collect();
    let resolved = curseforge_files(&ids).await?;
    let mut manual = Vec::new();
    for (done, version) in resolved.iter().enumerate() {
        progress(done, resolved.len());
        let Some(file) = version["files"].get(0) else {
            manual.push(version["name"].as_str().unwrap_or_default().to_string());
            continue;
        };
        let name = sanitize_download_file_name(file["filename"].as_str().unwrap_or_default())?;
        download_file(
            file["url"].as_str().unwrap_or_default(),
            &server.join("mods").join(&name),
            file.get("hashes"),
        )
        .await
        .map_err(|e| format!("{name}: {e}"))?;
    }
    progress(resolved.len(), resolved.len());
    Ok(manual)
}

pub async fn install_modpack(
    server_path: String,
    pack_path: PathBuf,
) -> Result<Vec<String>, String> {
    let server = PathBuf::from(server_path);
    let (manual, folders) = match load_pack(pack_path.clone()).await? {
        Pack::Modrinth(index) => (
            install_mrpack(&server, index).await?,
            vec!["overrides".to_string(), "server-overrides".to_string()],
        ),
        Pack::CurseForge(manifest) => {
            if curseforge_api_key().await.is_none() {
                return Err(NO_KEY.into());
            }
            let folder = manifest
                .overrides
                .clone()
                .unwrap_or_else(|| "overrides".into());
            (install_curseforge(&server, &manifest).await?, vec![folder])
        }
    };
    tokio::task::spawn_blocking(move || {
        let folders: Vec<&str> = folders.iter().map(String::as_str).collect();
        extract_overrides(&pack_path, &server, &folders)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(manual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_mrpack_loader_and_server_files() {
        let index: MrpackIndex = serde_json::from_value(json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "1.2.0",
            "name": "Cozy Pack",
            "files": [
                { "path": "mods/a.jar", "hashes": { "sha1": "a" }, "env": { "client": "required", "server": "required" }, "downloads": ["https://cdn.modrinth.com/a.jar"], "fileSize": 1 },
                { "path": "mods/b.jar", "hashes": { "sha1": "b" }, "env": { "client": "required", "server": "unsupported" }, "downloads": ["https://cdn.modrinth.com/b.jar"], "fileSize": 1 },
                { "path": "mods/c.jar", "hashes": { "sha1": "c" }, "downloads": ["https://cdn.modrinth.com/c.jar"], "fileSize": 1 }
            ],
            "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.11" }
        }))
        .unwrap();
        let info = mrpack_info(&index).unwrap();
        assert_eq!(info.framework, "Fabric");
        assert_eq!(info.game_version, "1.20.1");
        assert_eq!(info.loader_version.as_deref(), Some("0.15.11"));
        assert_eq!(info.files, 2);
        assert!(server_files(&index).all(|f| f.path != "mods/b.jar"));
    }

    #[test]
    fn mrpack_without_loader_is_vanilla_and_needs_minecraft() {
        let vanilla: MrpackIndex =
            serde_json::from_value(json!({ "name": "V", "dependencies": { "minecraft": "1.21" } }))
                .unwrap();
        assert_eq!(mrpack_info(&vanilla).unwrap().framework, "Vanilla");
        let broken: MrpackIndex =
            serde_json::from_value(json!({ "name": "B", "dependencies": { "forge": "47.2.0" } }))
                .unwrap();
        assert!(mrpack_info(&broken).is_err());
    }

    #[test]
    fn reads_curseforge_manifest() {
        let manifest: CurseForgeManifest = serde_json::from_value(json!({
            "minecraft": {
                "version": "1.20.1",
                "modLoaders": [{ "id": "neoforge-47.1.106", "primary": true }]
            },
            "manifestType": "minecraftModpack",
            "name": "Big Pack",
            "version": "3.0",
            "files": [
                { "projectID": 1, "fileID": 10, "required": true },
                { "projectID": 2, "fileID": 20, "required": false }
            ],
            "overrides": "overrides"
        }))
        .unwrap();
        let info = curseforge_info(&manifest);
        assert_eq!(info.framework, "NeoForge");
        assert_eq!(info.loader_version.as_deref(), Some("47.1.106"));
        assert_eq!(info.files, 1);
        assert!(info.curseforge);
    }

    #[test]
    fn extracts_overrides_but_not_client_overrides() {
        let root = std::env::temp_dir().join(format!("monolith-pack-{}", uuid::Uuid::new_v4()));
        let server = root.join("server");
        std::fs::create_dir_all(&server).unwrap();
        let pack = root.join("pack.mrpack");
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("modrinth.index.json", "{}"),
            ("overrides/config/a.toml", "base"),
            ("server-overrides/config/a.toml", "server"),
            ("client-overrides/options.txt", "client"),
            ("overrides/../escape.txt", "nope"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        extract_overrides(&pack, &server, &["overrides", "server-overrides"]).unwrap();
        assert_eq!(
            std::fs::read_to_string(server.join("config/a.toml")).unwrap(),
            "server"
        );
        assert!(!server.join("options.txt").exists());
        assert!(!root.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
