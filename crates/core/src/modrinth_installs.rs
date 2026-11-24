use crate::content_sources::versions;
use crate::modrinth::get_modrinth_project;
use crate::safety::{is_path_within, sanitize_download_file_name};
use crate::server_store;
use crate::types::{
    ContentSource, ModrinthInstallEntry, ModrinthInstallRequest, ModrinthInstallResult,
    ModrinthProjectType, ModrinthUpdateResult, SuccessResult,
};
use futures_util::{future::BoxFuture, StreamExt};
use once_cell::sync::Lazy;
use serde_json::Value;
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const META_DIR_NAME: &str = ".monolith";
const MANIFEST_NAME: &str = "modrinth.json";

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
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
        .unwrap_or_default()
});

fn manifest_path(server_path: &Path) -> PathBuf {
    server_path.join(META_DIR_NAME).join(MANIFEST_NAME)
}

async fn read_manifest(server_path: &Path) -> Vec<ModrinthInstallEntry> {
    match tokio::fs::read_to_string(manifest_path(server_path)).await {
        Ok(content) => serde_json::from_str::<Vec<Value>>(&content)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

async fn write_manifest(
    server_path: &Path,
    entries: &[ModrinthInstallEntry],
) -> Result<(), String> {
    let path = manifest_path(server_path);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    tokio::fs::write(path, json)
        .await
        .map_err(|e| e.to_string())
}

fn install_folder(project_type: ModrinthProjectType) -> &'static str {
    match project_type {
        ModrinthProjectType::Plugin => "plugins",
        ModrinthProjectType::Mod | ModrinthProjectType::Modpack => "mods",
    }
}

fn is_entry(entry: &ModrinthInstallEntry, source: ContentSource, project_id: &str) -> bool {
    entry.source == source && entry.project_id == project_id
}

fn disabled_name(file_name: &str) -> String {
    format!("{file_name}.disabled")
}

async fn remove_installed(install_dir: &Path, file_name: &str) -> Result<(), String> {
    let (_, path) = safe_install_path(install_dir, file_name)?;
    let (_, disabled) = safe_install_path(install_dir, &disabled_name(file_name))?;
    let _ = tokio::fs::remove_file(path).await;
    let _ = tokio::fs::remove_file(disabled).await;
    Ok(())
}

fn safe_install_path(install_dir: &Path, file_name: &str) -> Result<(String, PathBuf), String> {
    let safe_name = sanitize_download_file_name(file_name)?;
    let file_path = install_dir.join(&safe_name);
    if !is_path_within(&file_path, install_dir) || file_path == install_dir {
        return Err("Invalid install filename".into());
    }
    Ok((safe_name, file_path))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn primary_file(version: &Value) -> Option<&Value> {
    let files = version.get("files")?.as_array()?;
    files
        .iter()
        .find(|f| f.get("primary").and_then(Value::as_bool).unwrap_or(false))
        .or_else(|| files.first())
}

async fn latest_version(
    source: ContentSource,
    project_id: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Value, String> {
    versions(source, project_id, loader, game_version)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "No compatible versions found".to_string())
}

async fn version_by_id(
    source: ContentSource,
    project_id: &str,
    version_id: &str,
) -> Result<Value, String> {
    versions(source, project_id, None, None)
        .await?
        .into_iter()
        .find(|v| str_field(v, "id") == Some(version_id))
        .ok_or_else(|| format!("Selected version was not found on {}", source.label()))
}

fn hash_matches(actual: &str, expected: Option<&str>) -> bool {
    match expected
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
    {
        Some(expected) => actual.to_lowercase() == expected,
        None => true,
    }
}

pub(crate) async fn download_file(
    url: &str,
    file_path: &Path,
    hashes: Option<&Value>,
) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("Downloads must use HTTPS".into());
    }

    let res = CLIENT.get(parsed).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Download failed: {}", res.status().as_u16()));
    }

    let expected_sha1 = hashes.and_then(|h| str_field(h, "sha1"));
    let expected_sha512 = hashes.and_then(|h| str_field(h, "sha512"));
    let expected_sha256 = hashes.and_then(|h| str_field(h, "sha256"));
    let result = stream_to_file(
        res,
        file_path,
        expected_sha1,
        expected_sha512,
        expected_sha256,
    )
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(file_path).await;
    }
    result
}

async fn stream_to_file(
    res: reqwest::Response,
    file_path: &Path,
    expected_sha1: Option<&str>,
    expected_sha512: Option<&str>,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    if let Some(dir) = file_path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::File::create(file_path)
        .await
        .map_err(|e| e.to_string())?;
    let mut sha1 = expected_sha1.map(|_| Sha1::new());
    let mut sha512 = expected_sha512.map(|_| Sha512::new());
    let mut sha256 = expected_sha256.map(|_| Sha256::new());

    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if let Some(h) = sha1.as_mut() {
            h.update(&chunk);
        }
        if let Some(h) = sha512.as_mut() {
            h.update(&chunk);
        }
        if let Some(h) = sha256.as_mut() {
            h.update(&chunk);
        }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    let sha1_hex = sha1.map(|h| hex::encode(h.finalize())).unwrap_or_default();
    let sha512_hex = sha512
        .map(|h| hex::encode(h.finalize()))
        .unwrap_or_default();
    let sha256_hex = sha256
        .map(|h| hex::encode(h.finalize()))
        .unwrap_or_default();
    if !hash_matches(&sha1_hex, expected_sha1)
        || !hash_matches(&sha512_hex, expected_sha512)
        || !hash_matches(&sha256_hex, expected_sha256)
    {
        return Err("Download integrity check failed".into());
    }
    Ok(())
}

fn install_version<'a>(
    server_path: &'a Path,
    request: ModrinthInstallRequest,
    version: Value,
    entries: &'a mut Vec<ModrinthInstallEntry>,
    visited: &'a mut HashSet<String>,
) -> BoxFuture<'a, Result<ModrinthInstallEntry, String>> {
    Box::pin(async move {
        let source = request.source;
        if visited.contains(&request.project_id) {
            return match entries
                .iter()
                .find(|e| is_entry(e, source, &request.project_id))
            {
                Some(existing) => Ok(existing.clone()),
                None => Err(format!(
                    "Circular dependency detected for {}",
                    request.project_id
                )),
            };
        }
        visited.insert(request.project_id.clone());

        let dependencies: Vec<Value> = version
            .get("dependencies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for dependency in dependencies {
            if str_field(&dependency, "dependency_type") != Some("required") {
                continue;
            }
            let Some(dep_project_id) =
                str_field(&dependency, "project_id").filter(|s| !s.is_empty())
            else {
                continue;
            };
            if entries.iter().any(|e| is_entry(e, source, dep_project_id)) {
                continue;
            }

            let dep_version_id = str_field(&dependency, "version_id").filter(|s| !s.is_empty());
            let dep_version = match dep_version_id {
                Some(id) => version_by_id(source, dep_project_id, id).await?,
                None => {
                    latest_version(
                        source,
                        dep_project_id,
                        request.loader.as_deref(),
                        request.game_version.as_deref(),
                    )
                    .await?
                }
            };
            let dep_project = match source {
                ContentSource::Modrinth => {
                    get_modrinth_project(dep_project_id.to_string()).await.ok()
                }
                _ => None,
            };

            let dep_request = ModrinthInstallRequest {
                source,
                project_id: dep_project_id.to_string(),
                project_type: request.project_type,
                loader: request.loader.clone(),
                game_version: request.game_version.clone(),
                version_id: dep_version_id.map(str::to_string),
                title: Some(
                    dep_project
                        .as_ref()
                        .map(|p| p.title.clone())
                        .or_else(|| str_field(&dependency, "title").map(String::from))
                        .unwrap_or_else(|| dep_project_id.to_string()),
                ),
                slug: dep_project.as_ref().map(|p| p.slug.clone()),
                icon_url: dep_project.as_ref().and_then(|p| p.icon_url.clone()),
            };
            install_version(server_path, dep_request, dep_version, entries, visited).await?;
        }

        let file =
            primary_file(&version).ok_or_else(|| "No downloadable file available".to_string())?;

        let install_dir = server_path.join(install_folder(request.project_type));
        tokio::fs::create_dir_all(&install_dir)
            .await
            .map_err(|e| e.to_string())?;

        let existing = entries
            .iter()
            .find(|e| is_entry(e, source, &request.project_id))
            .cloned();
        if let Some(existing) = &existing {
            remove_installed(&install_dir, &existing.file_name).await?;
        }

        let file_name = str_field(file, "filename").unwrap_or_default();
        let (safe_name, file_path) = safe_install_path(&install_dir, file_name)?;
        let url = str_field(file, "url").unwrap_or_default();
        download_file(url, &file_path, file.get("hashes")).await?;
        let disabled = existing.is_some_and(|e| e.disabled);
        if disabled {
            let (_, target) = safe_install_path(&install_dir, &disabled_name(&safe_name))?;
            tokio::fs::rename(&file_path, target)
                .await
                .map_err(|e| e.to_string())?;
        }

        let entry = ModrinthInstallEntry {
            source,
            disabled,
            project_id: request.project_id.clone(),
            version_id: str_field(&version, "id").unwrap_or_default().to_string(),
            file_name: safe_name,
            title: request
                .title
                .clone()
                .unwrap_or_else(|| request.project_id.clone()),
            slug: request.slug.clone(),
            icon_url: request.icon_url.clone(),
            project_type: request.project_type,
            loader: request.loader.clone(),
            game_version: request.game_version.clone(),
            installed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };

        match entries
            .iter()
            .position(|e| is_entry(e, source, &request.project_id))
        {
            Some(index) => entries[index] = entry.clone(),
            None => entries.push(entry.clone()),
        }
        Ok(entry)
    })
}

pub async fn list_installs_at(
    server_path: &Path,
    project_type: ModrinthProjectType,
) -> Vec<ModrinthInstallEntry> {
    let mut filtered = Vec::new();
    for entry in read_manifest(server_path).await {
        if entry.project_type != project_type {
            continue;
        }
        let Ok(name) = sanitize_download_file_name(&entry.file_name) else {
            continue;
        };
        let dir = server_path.join(install_folder(entry.project_type));
        if tokio::fs::metadata(dir.join(&name)).await.is_ok() {
            filtered.push(ModrinthInstallEntry {
                disabled: false,
                ..entry
            });
        } else if tokio::fs::metadata(dir.join(disabled_name(&name)))
            .await
            .is_ok()
        {
            filtered.push(ModrinthInstallEntry {
                disabled: true,
                ..entry
            });
        }
    }
    filtered
}

async fn try_install(
    server_path: &Path,
    request: ModrinthInstallRequest,
) -> Result<(ModrinthInstallEntry, Vec<String>), String> {
    let source = request.source;
    let version = match request.version_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => version_by_id(source, &request.project_id, id).await?,
        None => {
            latest_version(
                source,
                &request.project_id,
                request.loader.as_deref(),
                request.game_version.as_deref(),
            )
            .await?
        }
    };
    let mut entries = read_manifest(server_path).await;
    let before = entries.len();
    let project_id = request.project_id.clone();
    let entry = install_version(
        server_path,
        request,
        version,
        &mut entries,
        &mut HashSet::new(),
    )
    .await?;
    write_manifest(server_path, &entries).await?;
    let dependencies = entries[before.min(entries.len())..]
        .iter()
        .filter(|e| !is_entry(e, source, &project_id))
        .map(|e| e.title.clone())
        .collect();
    Ok((entry, dependencies))
}

pub async fn install_at(
    server_path: &Path,
    request: ModrinthInstallRequest,
) -> ModrinthInstallResult {
    match try_install(server_path, request).await {
        Ok((entry, dependencies)) => ModrinthInstallResult {
            success: true,
            entry: Some(entry),
            dependencies,
            error: None,
        },
        Err(error) => ModrinthInstallResult {
            success: false,
            error: Some(error),
            ..Default::default()
        },
    }
}

async fn try_update(
    server_path: &Path,
    request: ModrinthInstallRequest,
) -> Result<ModrinthUpdateResult, String> {
    let mut entries = read_manifest(server_path).await;
    let Some(existing) = entries
        .iter()
        .find(|e| is_entry(e, request.source, &request.project_id))
        .cloned()
    else {
        return Ok(ModrinthUpdateResult {
            success: false,
            updated: false,
            entry: None,
            error: Some("Not installed".into()),
        });
    };

    let loader = request.loader.clone().or_else(|| existing.loader.clone());
    let game_version = request
        .game_version
        .clone()
        .or_else(|| existing.game_version.clone());
    let version = latest_version(
        request.source,
        &request.project_id,
        loader.as_deref(),
        game_version.as_deref(),
    )
    .await?;

    if str_field(&version, "id") == Some(existing.version_id.as_str()) {
        return Ok(ModrinthUpdateResult {
            success: true,
            updated: false,
            entry: Some(existing),
            error: None,
        });
    }

    let install_request = ModrinthInstallRequest {
        project_type: existing.project_type,
        title: Some(existing.title.clone()),
        slug: existing.slug.clone(),
        icon_url: existing.icon_url.clone(),
        ..request
    };
    let entry = install_version(
        server_path,
        install_request,
        version,
        &mut entries,
        &mut HashSet::new(),
    )
    .await?;
    write_manifest(server_path, &entries).await?;
    Ok(ModrinthUpdateResult {
        success: true,
        updated: true,
        entry: Some(entry),
        error: None,
    })
}

pub async fn update_at(
    server_path: &Path,
    request: ModrinthInstallRequest,
) -> ModrinthUpdateResult {
    match try_update(server_path, request).await {
        Ok(result) => result,
        Err(error) => ModrinthUpdateResult {
            success: false,
            updated: false,
            entry: None,
            error: Some(error),
        },
    }
}

async fn try_remove(
    server_path: &Path,
    source: ContentSource,
    project_id: &str,
) -> Result<(), String> {
    let entries = read_manifest(server_path).await;
    let Some(entry) = entries.iter().find(|e| is_entry(e, source, project_id)) else {
        return Ok(());
    };
    let install_dir = server_path.join(install_folder(entry.project_type));
    remove_installed(&install_dir, &entry.file_name).await?;

    let remaining: Vec<_> = entries
        .into_iter()
        .filter(|e| !is_entry(e, source, project_id))
        .collect();
    write_manifest(server_path, &remaining).await
}

pub async fn remove_at(
    server_path: &Path,
    source: ContentSource,
    project_id: &str,
) -> SuccessResult {
    match try_remove(server_path, source, project_id).await {
        Ok(()) => SuccessResult::ok(),
        Err(error) => SuccessResult::err(error),
    }
}

async fn server_path_of(server_id: String) -> Option<PathBuf> {
    server_store::get_server(server_id)
        .await
        .map(|s| PathBuf::from(s.server_path))
}

pub async fn list_modrinth_installs(
    server_id: String,
    project_type: ModrinthProjectType,
) -> Vec<ModrinthInstallEntry> {
    match server_path_of(server_id).await {
        Some(path) => list_installs_at(&path, project_type).await,
        None => Vec::new(),
    }
}

pub async fn install_modrinth_project(
    server_id: String,
    request: ModrinthInstallRequest,
) -> ModrinthInstallResult {
    match server_path_of(server_id).await {
        Some(path) => install_at(&path, request).await,
        None => ModrinthInstallResult {
            success: false,
            error: Some("Server not found".into()),
            ..Default::default()
        },
    }
}

pub async fn update_modrinth_install(
    server_id: String,
    request: ModrinthInstallRequest,
) -> ModrinthUpdateResult {
    match server_path_of(server_id).await {
        Some(path) => update_at(&path, request).await,
        None => ModrinthUpdateResult {
            success: false,
            updated: false,
            entry: None,
            error: Some("Server not found".into()),
        },
    }
}

pub async fn remove_modrinth_install(
    server_id: String,
    source: ContentSource,
    project_id: String,
) -> SuccessResult {
    match server_path_of(server_id).await {
        Some(path) => remove_at(&path, source, &project_id).await,
        None => SuccessResult::err("Server not found"),
    }
}

async fn try_set_enabled(
    server_path: &Path,
    source: ContentSource,
    project_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let entries = read_manifest(server_path).await;
    let entry = entries
        .iter()
        .find(|e| is_entry(e, source, project_id))
        .ok_or("Not installed")?;
    let install_dir = server_path.join(install_folder(entry.project_type));
    let (_, active) = safe_install_path(&install_dir, &entry.file_name)?;
    let (_, disabled) = safe_install_path(&install_dir, &disabled_name(&entry.file_name))?;
    let (from, to) = if enabled {
        (disabled, active)
    } else {
        (active, disabled)
    };
    if tokio::fs::metadata(&to).await.is_ok() {
        return Ok(());
    }
    tokio::fs::rename(from, to).await.map_err(|e| e.to_string())
}

pub async fn set_enabled_at(
    server_path: &Path,
    source: ContentSource,
    project_id: &str,
    enabled: bool,
) -> SuccessResult {
    match try_set_enabled(server_path, source, project_id, enabled).await {
        Ok(()) => SuccessResult::ok(),
        Err(error) => SuccessResult::err(error),
    }
}

pub async fn set_install_enabled(
    server_id: String,
    source: ContentSource,
    project_id: String,
    enabled: bool,
) -> SuccessResult {
    match server_path_of(server_id).await {
        Some(path) => set_enabled_at(&path, source, &project_id, enabled).await,
        None => SuccessResult::err("Server not found"),
    }
}

fn quick_verdict(
    entry: &ModrinthInstallEntry,
    target: Option<(ModrinthProjectType, &str)>,
) -> Option<bool> {
    match target {
        None => Some(false),
        Some((project_type, _)) if project_type != entry.project_type => Some(false),
        _ if entry.source == ContentSource::Spigot => Some(true),
        _ => None,
    }
}

pub async fn incompatible_installs(
    server_id: String,
    framework: String,
    game_version: String,
) -> Result<Vec<String>, String> {
    let path = server_path_of(server_id).await.ok_or("Server not found")?;
    let target = crate::content_sources::loader_for(&framework);
    let mut incompatible = Vec::new();
    for project_type in [ModrinthProjectType::Plugin, ModrinthProjectType::Mod] {
        for entry in list_installs_at(&path, project_type).await {
            let compatible = match quick_verdict(&entry, target) {
                Some(verdict) => verdict,
                None => !versions(
                    entry.source,
                    &entry.project_id,
                    target.map(|(_, loader)| loader),
                    Some(&game_version),
                )
                .await?
                .is_empty(),
            };
            if !compatible {
                incompatible.push(entry.title);
            }
        }
    }
    Ok(incompatible)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: ContentSource, project_id: &str, file_name: &str) -> ModrinthInstallEntry {
        ModrinthInstallEntry {
            source,
            disabled: false,
            project_id: project_id.into(),
            version_id: "v1".into(),
            file_name: file_name.into(),
            title: project_id.into(),
            slug: None,
            icon_url: None,
            project_type: ModrinthProjectType::Plugin,
            loader: None,
            game_version: None,
            installed_at: String::new(),
        }
    }

    #[tokio::test]
    async fn disables_and_enables_without_losing_track() {
        let dir = std::env::temp_dir().join(format!("monolith-toggle-{}", uuid::Uuid::new_v4()));
        let plugins = dir.join("plugins");
        tokio::fs::create_dir_all(&plugins).await.unwrap();
        tokio::fs::write(plugins.join("a.jar"), b"PK")
            .await
            .unwrap();
        write_manifest(
            &dir,
            &[
                entry(ContentSource::Modrinth, "a", "a.jar"),
                entry(ContentSource::Hangar, "a", "missing.jar"),
            ],
        )
        .await
        .unwrap();

        assert!(
            set_enabled_at(&dir, ContentSource::Modrinth, "a", false)
                .await
                .success
        );
        assert!(plugins.join("a.jar.disabled").exists());
        assert!(!plugins.join("a.jar").exists());
        let listed = list_installs_at(&dir, ModrinthProjectType::Plugin).await;
        assert_eq!(listed.len(), 1);
        assert!(listed[0].disabled);

        assert!(
            set_enabled_at(&dir, ContentSource::Modrinth, "a", true)
                .await
                .success
        );
        assert!(plugins.join("a.jar").exists());
        assert!(!list_installs_at(&dir, ModrinthProjectType::Plugin).await[0].disabled);

        assert!(
            set_enabled_at(&dir, ContentSource::Modrinth, "a", false)
                .await
                .success
        );
        assert!(remove_at(&dir, ContentSource::Modrinth, "a").await.success);
        assert!(!plugins.join("a.jar.disabled").exists());
        assert_eq!(read_manifest(&dir).await.len(), 1);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn quick_verdicts_skip_the_network_when_possible() {
        let plugin = entry(ContentSource::Modrinth, "a", "a.jar");
        assert_eq!(quick_verdict(&plugin, None), Some(false));
        assert_eq!(
            quick_verdict(&plugin, Some((ModrinthProjectType::Mod, "fabric"))),
            Some(false)
        );
        assert_eq!(
            quick_verdict(&plugin, Some((ModrinthProjectType::Plugin, "paper"))),
            None
        );
        let spigot = entry(ContentSource::Spigot, "1", "b.jar");
        assert_eq!(
            quick_verdict(&spigot, Some((ModrinthProjectType::Plugin, "paper"))),
            Some(true)
        );
    }

    #[test]
    fn old_manifests_default_to_modrinth() {
        let parsed: ModrinthInstallEntry = serde_json::from_value(serde_json::json!({
            "projectId": "abc", "versionId": "v", "fileName": "a.jar", "title": "A",
            "projectType": "plugin", "installedAt": ""
        }))
        .unwrap();
        assert_eq!(parsed.source, ContentSource::Modrinth);
        assert!(!parsed.disabled);
    }
}
