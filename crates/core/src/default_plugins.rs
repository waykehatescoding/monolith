use crate::paths;
use crate::safety::is_path_within;
use crate::types::{
    AddDefaultPluginResult, AddManualDefaultPluginsResult, DefaultPlugin,
    DefaultPluginInstallSummary, DefaultPluginSource, ModrinthInstallRequest, ModrinthProjectType,
    SuccessResult, SyncDefaultPluginsResult,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn default_plugins_file() -> PathBuf {
    paths::user_data_dir().join("default-plugins.json")
}

fn manual_plugins_dir() -> PathBuf {
    paths::user_data_dir().join("default-plugins")
}

fn raw_key(entry: &Value) -> Option<&str> {
    let field = if entry.get("source").and_then(Value::as_str) == Some("modrinth") {
        "projectId"
    } else {
        "fileName"
    };
    entry.get(field).and_then(Value::as_str)
}

fn plugin_key(plugin: &DefaultPlugin) -> &str {
    match &plugin.source {
        DefaultPluginSource::Modrinth { project_id, .. } => project_id,
        DefaultPluginSource::Manual { file_name, .. } => file_name,
    }
}

fn clean_group_name(group: Option<&str>) -> Option<String> {
    let mut units = 0;
    let trimmed: String = group?
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 32
        })
        .collect();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn save_raw(plugins: &[Value]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(plugins).map_err(std::io::Error::other)?;
    tokio::fs::write(default_plugins_file(), json).await
}

fn to_raw(plugin: &DefaultPlugin) -> Value {
    serde_json::to_value(plugin).unwrap_or(Value::Null)
}

fn migrate_entry(entry: Value) -> (Value, bool) {
    let legacy = entry.as_object().is_some_and(|o| {
        !o.contains_key("source") && o.get("projectId").is_some_and(Value::is_string)
    });
    if !legacy {
        return (entry, false);
    }
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let Some(project_id) = text("projectId") else {
        return (entry, false);
    };
    let plugin = DefaultPlugin {
        source: DefaultPluginSource::Modrinth {
            title: text("title").unwrap_or_else(|| project_id.clone()),
            project_id,
            slug: text("slug"),
            icon_url: text("iconUrl"),
            added_at: text("addedAt").unwrap_or_else(now_iso),
        },
        group: None,
    };
    (to_raw(&plugin), true)
}

async fn list_raw() -> Vec<Value> {
    let Ok(content) = tokio::fs::read_to_string(default_plugins_file()).await else {
        return Vec::new();
    };
    let Ok(Value::Array(raw)) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    let mut migrated = false;
    let entries: Vec<Value> = raw
        .into_iter()
        .map(|entry| {
            let (entry, was_migrated) = migrate_entry(entry);
            migrated |= was_migrated;
            entry
        })
        .collect();
    if migrated {
        let _ = save_raw(&entries).await;
    }
    entries
}

async fn list_plugins() -> Vec<DefaultPlugin> {
    list_raw()
        .await
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect()
}

fn resolve_manual_plugin_path(file_name: &str) -> Option<PathBuf> {
    if !file_name.to_lowercase().ends_with(".jar") {
        return None;
    }
    let dir = manual_plugins_dir();
    let target = dir.join(file_name);
    let plain_name = target.file_name().and_then(|n| n.to_str()) == Some(file_name);
    if !is_path_within(&target, &dir) || !plain_name {
        return None;
    }
    Some(target)
}

pub async fn install_default_plugins(
    server_path: &Path,
    framework: &str,
    game_version: &str,
    only: Option<&[String]>,
) -> DefaultPluginInstallSummary {
    let mut summary = DefaultPluginInstallSummary::default();
    if framework != "Paper" && framework != "Purpur" {
        return summary;
    }
    let plugins: Vec<DefaultPlugin> = list_plugins()
        .await
        .into_iter()
        .filter(|p| only.is_none_or(|keys| keys.iter().any(|k| k == plugin_key(p))))
        .collect();

    for plugin in plugins {
        match plugin.source {
            DefaultPluginSource::Manual { file_name, .. } => {
                if copy_manual_plugin(server_path, &file_name).await {
                    summary.installed += 1;
                } else {
                    summary.failed.push(file_name);
                }
            }
            DefaultPluginSource::Modrinth {
                project_id,
                title,
                slug,
                icon_url,
                ..
            } => {
                let request = ModrinthInstallRequest {
                    project_id,
                    project_type: ModrinthProjectType::Plugin,
                    loader: Some("paper".into()),
                    game_version: Some(game_version.to_string()),
                    version_id: None,
                    title: Some(title.clone()),
                    slug,
                    icon_url,
                };
                if crate::modrinth_installs::install_at(server_path, request)
                    .await
                    .success
                {
                    summary.installed += 1;
                } else {
                    summary.failed.push(title);
                }
            }
        }
    }
    summary
}

async fn copy_manual_plugin(server_path: &Path, file_name: &str) -> bool {
    let Some(src) = resolve_manual_plugin_path(file_name) else {
        return false;
    };
    let plugins_dir = server_path.join("plugins");
    if tokio::fs::create_dir_all(&plugins_dir).await.is_err() {
        return false;
    }
    let dest = plugins_dir.join(file_name);
    if !is_path_within(&dest, &plugins_dir) {
        return false;
    }
    tokio::fs::copy(src, dest).await.is_ok()
}

pub async fn list_default_plugins() -> Vec<DefaultPlugin> {
    list_plugins().await
}

pub async fn add_default_plugin(entry: Value) -> Result<AddDefaultPluginResult, String> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let project_id = text("projectId").unwrap_or_default();
    let title = text("title").unwrap_or_default();
    if project_id.is_empty() || title.is_empty() {
        return Ok(AddDefaultPluginResult {
            success: false,
            added: None,
            error: Some("Invalid plugin".into()),
        });
    }
    let mut plugins = list_raw().await;
    let exists = plugins.iter().any(|p| {
        p.get("source").and_then(Value::as_str) == Some("modrinth")
            && p.get("projectId").and_then(Value::as_str) == Some(project_id.as_str())
    });
    if exists {
        return Ok(AddDefaultPluginResult {
            success: false,
            added: None,
            error: Some("Already added".into()),
        });
    }
    let added = DefaultPlugin {
        source: DefaultPluginSource::Modrinth {
            project_id,
            title,
            slug: text("slug"),
            icon_url: text("iconUrl"),
            added_at: now_iso(),
        },
        group: clean_group_name(text("group").as_deref()),
    };
    plugins.push(to_raw(&added));
    save_raw(&plugins).await.map_err(|e| e.to_string())?;
    Ok(AddDefaultPluginResult {
        success: true,
        added: Some(added),
        error: None,
    })
}

pub async fn add_manual_default_plugins(
    sources: Vec<PathBuf>,
    group: Option<String>,
) -> Result<AddManualDefaultPluginsResult, String> {
    let no_files = || AddManualDefaultPluginsResult {
        success: false,
        added: Vec::new(),
        error: Some("No files selected".into()),
    };
    if sources.is_empty() {
        return Ok(no_files());
    }
    let added = store_manual_plugins(&sources, group.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(AddManualDefaultPluginsResult {
        success: true,
        added,
        error: None,
    })
}

async fn store_manual_plugins(
    sources: &[PathBuf],
    group: Option<&str>,
) -> std::io::Result<Vec<DefaultPlugin>> {
    let dir = manual_plugins_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let mut plugins = list_raw().await;
    let mut added = Vec::new();

    for src in sources {
        let base = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (stem, ext) = match base.rfind('.') {
            Some(i) if i > 0 => (&base[..i], &base[i..]),
            _ => (base.as_str(), ""),
        };
        let mut dest_name = base.clone();
        let mut suffix = 1;
        loop {
            let on_disk = tokio::fs::try_exists(dir.join(&dest_name))
                .await
                .unwrap_or(false);
            let tracked = plugins.iter().any(|p| {
                p.get("source").and_then(Value::as_str) == Some("manual")
                    && p.get("fileName").and_then(Value::as_str) == Some(dest_name.as_str())
            });
            if !on_disk && !tracked {
                break;
            }
            dest_name = format!("{stem} ({suffix}){ext}");
            suffix += 1;
        }

        let dest = dir.join(&dest_name);
        tokio::fs::copy(src, &dest).await?;
        let meta = tokio::fs::metadata(&dest).await?;
        let added_at = meta
            .modified()
            .map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })
            .unwrap_or_else(|_| now_iso());
        let entry = DefaultPlugin {
            source: DefaultPluginSource::Manual {
                file_name: dest_name,
                size_bytes: meta.len(),
                added_at,
            },
            group: clean_group_name(group),
        };
        plugins.push(to_raw(&entry));
        added.push(entry);
    }

    save_raw(&plugins).await?;
    Ok(added)
}

pub async fn remove_default_plugin(key: String) -> Result<SuccessResult, String> {
    let mut plugins = list_raw().await;
    let Some(index) = plugins
        .iter()
        .position(|p| raw_key(p) == Some(key.as_str()))
    else {
        return Ok(SuccessResult::ok());
    };
    let target = plugins.remove(index);
    save_raw(&plugins).await.map_err(|e| e.to_string())?;
    if target.get("source").and_then(Value::as_str) == Some("manual") {
        if let Some(path) = target
            .get("fileName")
            .and_then(Value::as_str)
            .and_then(resolve_manual_plugin_path)
        {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
    Ok(SuccessResult::ok())
}

pub async fn set_default_plugin_group(
    key: String,
    group: Option<String>,
) -> Result<SuccessResult, String> {
    let mut plugins = list_raw().await;
    let Some(target) = plugins
        .iter_mut()
        .find(|p| raw_key(p) == Some(key.as_str()))
    else {
        return Ok(SuccessResult::err("Plugin not found"));
    };
    if let Some(object) = target.as_object_mut() {
        match clean_group_name(group.as_deref()) {
            Some(name) => object.insert("group".into(), Value::String(name)),
            None => object.remove("group"),
        };
    }
    save_raw(&plugins).await.map_err(|e| e.to_string())?;
    Ok(SuccessResult::ok())
}

pub async fn sync_default_plugins(server_id: String) -> SyncDefaultPluginsResult {
    let failure = |error: &str| SyncDefaultPluginsResult {
        success: false,
        installed: 0,
        failed: Vec::new(),
        error: Some(error.into()),
    };
    let Some(server) = crate::server_store::get_server(server_id).await else {
        return failure("Server not found");
    };
    let handle = tokio::spawn(async move {
        install_default_plugins(
            Path::new(&server.server_path),
            &server.framework,
            &server.version,
            None,
        )
        .await
    });
    match handle.await {
        Ok(summary) => SyncDefaultPluginsResult {
            success: true,
            installed: summary.installed,
            failed: summary.failed,
            error: None,
        },
        Err(_) => failure("Failed to sync default plugins"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn moves_plugins_between_groups_and_installs_only_selected() {
        let user_data =
            std::env::temp_dir().join(format!("monolith-defaults-{}", uuid::Uuid::new_v4()));
        let store = user_data.join("default-plugins");
        std::fs::create_dir_all(&store).unwrap();
        for name in ["Spark.jar", "Skript.jar"] {
            std::fs::write(store.join(name), "jar").unwrap();
        }
        std::fs::write(
            user_data.join("default-plugins.json"),
            serde_json::json!([
                { "source": "manual", "fileName": "Spark.jar", "sizeBytes": 3, "addedAt": "", "group": "Performance" },
                { "source": "manual", "fileName": "Skript.jar", "sizeBytes": 3, "addedAt": "" }
            ])
            .to_string(),
        )
        .unwrap();
        let _guard = paths::override_user_data_dir(user_data.clone());

        assert!(
            set_default_plugin_group("Skript.jar".into(), Some(" Skript ".into()))
                .await
                .unwrap()
                .success
        );
        let groups: Vec<_> = list_plugins().await.into_iter().map(|p| p.group).collect();
        assert_eq!(
            groups,
            vec![Some("Performance".to_string()), Some("Skript".to_string())]
        );

        let server_path =
            std::env::temp_dir().join(format!("monolith-server-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&server_path).unwrap();
        let only = vec!["Skript.jar".to_string()];
        let result = install_default_plugins(&server_path, "Paper", "1.21.4", Some(&only)).await;

        assert_eq!(result.installed, 1);
        assert!(result.failed.is_empty());
        let names: Vec<_> = std::fs::read_dir(server_path.join("plugins"))
            .unwrap()
            .filter_map(|e| e.ok().and_then(|e| e.file_name().into_string().ok()))
            .collect();
        assert_eq!(names, vec!["Skript.jar"]);
    }
}
