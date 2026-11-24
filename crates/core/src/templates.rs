use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::modrinth_installs::{install_at, list_installs_at};
use crate::paths::user_data_dir;
use crate::server_props::{
    read_list, read_properties, write_list, write_properties, ListEntry, OPS, WHITELIST,
};
use crate::server_store::{clamp_ram, get_server, update_server_settings};
use crate::server_transfer::safe_join;
use crate::types::{
    ContentSource, ModrinthInstallRequest, ModrinthProjectType, ServerProperty, ServerRecord,
    ServerSettingsUpdate,
};

const MAX_TEMPLATE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_FILES: usize = 200;
const MAX_CONTENT: usize = 500;
const SKIPPED_PROPERTIES: [&str; 4] = ["level-seed", "level-name", "server-port", "rcon.password"];
const CONFIG_EXTENSIONS: [&str; 7] = ["yml", "yaml", "json", "toml", "properties", "conf", "txt"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerTemplate {
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub framework: String,
    pub version: String,
    #[serde(rename = "ramMB")]
    pub ram_mb: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jvm_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jvm_args: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    #[serde(default)]
    pub content: Vec<TemplateContent>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whitelist: Option<Vec<ListEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<ListEntry>>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateContent {
    #[serde(default)]
    pub source: ContentSource,
    pub project_id: String,
    pub version_id: String,
    pub project_type: ModrinthProjectType,
    pub title: String,
}

fn templates_dir() -> PathBuf {
    user_data_dir().join("templates")
}

fn template_path(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Template not found".into());
    }
    Ok(templates_dir().join(format!("{id}.json")))
}

fn config_paths(server: &Path) -> Vec<String> {
    let mut found = vec!["bukkit.yml".to_string(), "spigot.yml".to_string()];
    if let Ok(entries) = std::fs::read_dir(server.join("config")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("paper-") && name.ends_with(".yml") {
                found.push(format!("config/{name}"));
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(server.join("plugins")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            found.push(format!("plugins/{name}/config.yml"));
        }
    }
    found.retain(|rel| server.join(rel).is_file());
    found.sort();
    found
}

async fn read_configs(server: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for rel in config_paths(server).into_iter().take(MAX_FILES) {
        if let Ok(text) = tokio::fs::read_to_string(server.join(&rel)).await {
            if text.len() <= MAX_FILE_BYTES {
                files.insert(rel, text);
            }
        }
    }
    files
}

pub async fn capture(
    server: &ServerRecord,
    name: &str,
    configs: bool,
    players: bool,
) -> ServerTemplate {
    let path = Path::new(&server.server_path);
    let properties = read_properties(path)
        .await
        .into_iter()
        .filter(|p| p.comment.is_none() && !SKIPPED_PROPERTIES.contains(&p.key.as_str()))
        .map(|p| (p.key, p.value))
        .collect();
    let mut content = Vec::new();
    for kind in [ModrinthProjectType::Plugin, ModrinthProjectType::Mod] {
        content.extend(list_installs_at(path, kind).await.into_iter().map(|entry| {
            TemplateContent {
                source: entry.source,
                project_id: entry.project_id,
                version_id: entry.version_id,
                project_type: entry.project_type,
                title: entry.title,
            }
        }));
    }
    let extra = |key: &str| {
        server
            .extra
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    ServerTemplate {
        id: String::new(),
        name: name.trim().to_string(),
        framework: server.framework.clone(),
        version: server.version.clone(),
        ram_mb: server.ram_mb,
        jvm_preset: extra("jvmPreset"),
        jvm_args: extra("jvmArgs"),
        properties,
        content,
        files: if configs {
            read_configs(path).await
        } else {
            BTreeMap::new()
        },
        whitelist: if players {
            Some(read_list(&path.join(WHITELIST)).await)
        } else {
            None
        },
        ops: if players {
            Some(read_list(&path.join(OPS)).await)
        } else {
            None
        },
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.contains("..")
}

fn safe_config_path(rel: &str) -> bool {
    let extension = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    !rel.contains('\\')
        && !rel.contains(':')
        && safe_join(Path::new("server"), rel).is_some()
        && extension.is_some_and(|e| CONFIG_EXTENSIONS.contains(&e.as_str()))
}

pub fn validate(mut template: ServerTemplate) -> Result<ServerTemplate, String> {
    let name = template.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("The template needs a name of up to 64 characters.".into());
    }
    template.name = name.to_string();
    if template.framework.trim().is_empty() || template.version.trim().is_empty() {
        return Err("The template does not say which platform and version to use.".into());
    }
    template.ram_mb = clamp_ram(template.ram_mb);
    let bad_text = |s: &str| s.contains(['\n', '\r', '\0']);
    if template
        .properties
        .iter()
        .any(|(key, value)| key.is_empty() || key.contains('=') || bad_text(key) || bad_text(value))
    {
        return Err("The template has an invalid server property.".into());
    }
    for key in SKIPPED_PROPERTIES {
        template.properties.remove(key);
    }
    if template.files.len() > MAX_FILES {
        return Err("The template has too many config files.".into());
    }
    if let Some((rel, _)) = template
        .files
        .iter()
        .find(|(rel, text)| !safe_config_path(rel) || text.len() > MAX_FILE_BYTES)
    {
        return Err(format!(
            "The template contains a file Monolith will not copy: {rel}"
        ));
    }
    if template.content.len() > MAX_CONTENT
        || template
            .content
            .iter()
            .any(|c| !safe_id(&c.project_id) || !safe_id(&c.version_id))
    {
        return Err("The template has an invalid content list.".into());
    }
    Ok(template)
}

fn parse(raw: &str, id: String) -> Result<ServerTemplate, String> {
    let template: ServerTemplate =
        serde_json::from_str(raw).map_err(|_| "This is not a Monolith template.".to_string())?;
    Ok(ServerTemplate {
        id,
        ..validate(template)?
    })
}

async fn write(template: &ServerTemplate) -> Result<(), String> {
    let path = template_path(&template.id)?;
    tokio::fs::create_dir_all(templates_dir())
        .await
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(template).map_err(|e| e.to_string())?;
    tokio::fs::write(path, json)
        .await
        .map_err(|e| e.to_string())
}

async fn load(id: &str) -> Result<ServerTemplate, String> {
    let raw = tokio::fs::read_to_string(template_path(id)?)
        .await
        .map_err(|_| "Template not found".to_string())?;
    parse(&raw, id.to_string())
}

pub async fn list_templates() -> Vec<ServerTemplate> {
    let Ok(mut entries) = tokio::fs::read_dir(templates_dir()).await else {
        return Vec::new();
    };
    let mut templates = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if let Ok(template) = load(&id).await {
            templates.push(template);
        }
    }
    templates.sort_by_key(|t| t.name.to_lowercase());
    templates
}

pub async fn save_template(
    server_id: String,
    name: String,
    configs: bool,
    players: bool,
) -> Result<ServerTemplate, String> {
    let server = get_server(server_id).await.ok_or("Server not found")?;
    let template = ServerTemplate {
        id: uuid::Uuid::new_v4().to_string(),
        ..validate(capture(&server, &name, configs, players).await)?
    };
    write(&template).await?;
    Ok(template)
}

pub async fn rename_template(id: String, name: String) -> Result<(), String> {
    let template = load(&id).await?;
    let template = ServerTemplate {
        id,
        ..validate(ServerTemplate { name, ..template })?
    };
    write(&template).await
}

pub async fn delete_template(id: String) -> Result<(), String> {
    tokio::fs::remove_file(template_path(&id)?)
        .await
        .map_err(|e| e.to_string())
}

pub async fn export_template(id: String, target: PathBuf) -> Result<(), String> {
    let template = load(&id).await?;
    let json = serde_json::to_string_pretty(&template).map_err(|e| e.to_string())?;
    tokio::fs::write(target, json)
        .await
        .map_err(|e| e.to_string())
}

pub async fn import_template(path: PathBuf) -> Result<ServerTemplate, String> {
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|e| e.to_string())?
        .len();
    if size > MAX_TEMPLATE_BYTES {
        return Err("The file is too large to be a template.".into());
    }
    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| "This is not a Monolith template.".to_string())?;
    let template = parse(&raw, uuid::Uuid::new_v4().to_string())?;
    write(&template).await?;
    Ok(template)
}

async fn apply_files(server: &Path, template: &ServerTemplate) -> Result<(), String> {
    let mut properties = read_properties(server).await;
    for (key, value) in &template.properties {
        match properties.iter_mut().find(|p| &p.key == key) {
            Some(existing) => existing.value = value.clone(),
            None => properties.push(ServerProperty {
                key: key.clone(),
                value: value.clone(),
                comment: None,
            }),
        }
    }
    let written = write_properties(server, &properties).await;
    if !written.success {
        return Err(written.error.unwrap_or_default());
    }
    for (rel, text) in &template.files {
        let target = safe_join(server, rel).ok_or_else(|| format!("Unsafe path {rel}"))?;
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::write(&target, text)
            .await
            .map_err(|e| e.to_string())?;
    }
    for (file, list) in [(WHITELIST, &template.whitelist), (OPS, &template.ops)] {
        if let Some(entries) = list {
            let written = write_list(&server.join(file), entries).await;
            if !written.success {
                return Err(written.error.unwrap_or_default());
            }
        }
    }
    Ok(())
}

pub async fn apply_template(
    server_id: String,
    template: ServerTemplate,
) -> Result<Vec<String>, String> {
    let template = validate(template)?;
    let server = get_server(server_id.clone())
        .await
        .ok_or("Server not found")?;
    let path = PathBuf::from(&server.server_path);
    apply_files(&path, &template).await?;
    if template.jvm_preset.is_some() || template.jvm_args.is_some() {
        let update = ServerSettingsUpdate {
            jvm_preset: template.jvm_preset.clone(),
            jvm_args: template.jvm_args.clone(),
            ..Default::default()
        };
        let result = update_server_settings(server_id, update).await;
        if !result.success {
            return Err(result.error.unwrap_or_default());
        }
    }
    let mut failed = Vec::new();
    for item in &template.content {
        let request = ModrinthInstallRequest {
            source: item.source,
            project_id: item.project_id.clone(),
            project_type: item.project_type,
            loader: None,
            game_version: None,
            version_id: Some(item.version_id.clone()),
            title: Some(item.title.clone()),
            slug: None,
            icon_url: None,
        };
        if !install_at(&path, request).await.success {
            failed.push(item.title.clone());
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServerTemplate {
        ServerTemplate {
            name: "Survival".into(),
            framework: "Paper".into(),
            version: "1.21.4".into(),
            ram_mb: 4096,
            ..Default::default()
        }
    }

    #[test]
    fn rejects_unsafe_files() {
        for rel in [
            "../evil.yml",
            "/abs.yml",
            "C:/x.yml",
            "plugins\\x.yml",
            "plugins/x.jar",
            "start.bat",
            "run.exe",
        ] {
            let mut template = sample();
            template.files.insert(rel.into(), String::new());
            assert!(validate(template).is_err(), "{rel} was accepted");
        }
        let mut template = sample();
        template
            .files
            .insert("plugins/Essentials/config.yml".into(), "a: 1".into());
        assert!(validate(template).is_ok());
    }

    #[test]
    fn rejects_bad_properties_and_ids() {
        let mut template = sample();
        template.properties.insert("motd".into(), "a\nb=c".into());
        assert!(validate(template).is_err());
        let mut template = sample();
        template.content.push(TemplateContent {
            source: ContentSource::Modrinth,
            project_id: "../x".into(),
            version_id: "abc".into(),
            project_type: ModrinthProjectType::Plugin,
            title: "X".into(),
        });
        assert!(validate(template).is_err());
    }

    #[test]
    fn strips_world_specific_properties() {
        let mut template = sample();
        template.properties.insert("level-seed".into(), "1".into());
        template.properties.insert("motd".into(), "hi".into());
        let template = validate(template).unwrap();
        assert!(!template.properties.contains_key("level-seed"));
        assert_eq!(template.properties["motd"], "hi");
    }

    #[tokio::test]
    async fn captures_and_applies_settings_and_configs() {
        let root = std::env::temp_dir().join(format!("monolith-template-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(source.join("config")).unwrap();
        std::fs::create_dir_all(source.join("plugins/Essentials")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            source.join("server.properties"),
            "#comment\nmotd=Hello\nlevel-seed=42\nserver-port=25570\nrcon.password=pw\npvp=false\n",
        )
        .unwrap();
        std::fs::write(source.join("bukkit.yml"), "settings: {}\n").unwrap();
        std::fs::write(source.join("config/paper-global.yml"), "a: 1\n").unwrap();
        std::fs::write(source.join("plugins/Essentials/config.yml"), "b: 2\n").unwrap();
        std::fs::write(source.join("plugins/Essentials.jar"), "jar").unwrap();
        std::fs::write(source.join("whitelist.json"), r#"[{"name":"Steve"}]"#).unwrap();
        std::fs::write(
            target.join("server.properties"),
            "server-port=25566\nmotd=Old\n",
        )
        .unwrap();

        let mut server = ServerRecord {
            framework: "Paper".into(),
            version: "1.21.4".into(),
            ram_mb: 4096,
            server_path: source.to_string_lossy().into_owned(),
            ..Default::default()
        };
        server.extra.insert("jvmPreset".into(), "g1".into());
        let template = validate(capture(&server, " Base ", true, true).await).unwrap();
        assert_eq!(template.name, "Base");
        assert_eq!(template.jvm_preset.as_deref(), Some("g1"));
        assert_eq!(
            template.files.keys().collect::<Vec<_>>(),
            [
                "bukkit.yml",
                "config/paper-global.yml",
                "plugins/Essentials/config.yml"
            ]
        );
        assert!(!template.properties.contains_key("rcon.password"));
        assert!(template.ops.as_ref().is_some_and(|ops| ops.is_empty()));

        let json = serde_json::to_string(&template).unwrap();
        let template = parse(&json, "x".into()).unwrap();
        apply_files(&target, &template).await.unwrap();

        let props = std::fs::read_to_string(target.join("server.properties")).unwrap();
        assert!(props.contains("server-port=25566"));
        assert!(props.contains("motd=Hello"));
        assert!(props.contains("pvp=false"));
        assert!(!props.contains("level-seed"));
        assert_eq!(
            std::fs::read_to_string(target.join("plugins/Essentials/config.yml")).unwrap(),
            "b: 2\n"
        );
        assert!(!target.join("plugins/Essentials.jar").exists());
        assert_eq!(read_list(&target.join(WHITELIST)).await[0].name(), "Steve");
        let _ = std::fs::remove_dir_all(root);
    }
}
