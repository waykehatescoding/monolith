use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::config_files::yaml_set;
use crate::modrinth_installs::install_at;
use crate::paths::servers_dir;
use crate::port_check::configured_port;
use crate::server_create::{download_file, fill_download, fill_value};
use crate::server_props::{read_properties, write_properties};
use crate::server_runner::{is_running, start_server};
use crate::server_store::{get_server, load_server_list, sanitize_server_name, save_server_list};
use crate::types::{
    ContentSource, ModrinthInstallRequest, ModrinthProjectType, ServerProperty, ServerRecord,
    ServerRuntimeStatus,
};

const BACKENDS: &str = "proxyBackends";
const SECRET_FILE: &str = "forwarding.secret";
const FABRIC_PROXY_LITE: &str = "fabricproxy-lite";
const FIRST_BACKEND_PORT: u16 = 25566;
const PROXY_PORT: u16 = 25565;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    pub id: String,
    pub name: String,
    pub framework: String,
    pub port: u16,
    pub status: ServerRuntimeStatus,
}

pub fn backend_ids(proxy: &ServerRecord) -> Vec<String> {
    proxy
        .extra
        .get(BACKENDS)
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn base_config(port: u16) -> String {
    format!(
        "config-version = \"2.7\"\nbind = \"0.0.0.0:{port}\"\nmotd = \"<#09add3>A Velocity Server\"\nshow-max-players = 500\nonline-mode = true\nforce-key-authentication = true\nplayer-info-forwarding-mode = \"modern\"\nforwarding-secret-file = \"{SECRET_FILE}\"\n\n[servers]\ntry = []\n\n[forced-hosts]\n"
    )
}

pub fn set_top_level(text: &str, key: &str, literal: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let end = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len());
    let line = format!("{key} = {literal}");
    let found = lines[..end].iter().position(|l| {
        l.split_once('=')
            .is_some_and(|(k, _)| k.trim() == key && !l.trim_start().starts_with('#'))
    });
    match found {
        Some(index) => lines[index] = line,
        None => lines.insert(0, line),
    }
    lines.join("\n") + "\n"
}

pub fn set_servers(text: &str, servers: &[(String, String)]) -> String {
    let mut section = vec!["[servers]".to_string()];
    section.extend(
        servers
            .iter()
            .map(|(name, address)| format!("{name} = \"{address}\"")),
    );
    let names: Vec<String> = servers
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    section.push(format!("try = [{}]", names.join(", ")));
    section.push(String::new());

    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == "[servers]") else {
        return format!("{}\n\n{}", text.trim_end(), section.join("\n"));
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    out.extend(section);
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    out.join("\n").trim_end().to_string() + "\n"
}

pub fn backend_name(name: &str, taken: &[String]) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() {
        "server".into()
    } else {
        base
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

pub fn free_port(used: &[u16]) -> u16 {
    (FIRST_BACKEND_PORT..u16::MAX)
        .find(|p| !used.contains(p))
        .unwrap_or(FIRST_BACKEND_PORT)
}

fn set_prop(props: &mut Vec<ServerProperty>, key: &str, value: String) {
    match props.iter_mut().find(|p| p.key == key) {
        Some(p) => p.value = value,
        None => props.push(ServerProperty {
            key: key.into(),
            value,
            comment: None,
        }),
    }
}

fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

async fn read_secret(proxy: &Path) -> Result<String, String> {
    let path = proxy.join(SECRET_FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => {
            let value = secret();
            tokio::fs::write(&path, &value)
                .await
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
    }
}

pub async fn create_proxy(name: String, ram_mb: u32) -> Result<ServerRecord, String> {
    let versions = fill_value("velocity").await?;
    let latest = versions["versions"]
        .as_object()
        .into_iter()
        .flat_map(|groups| groups.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .filter(|v| !v.contains("SNAPSHOT"))
        .max_by(|a, b| crate::update_checker::compare_versions(a, b).cmp(&0))
        .ok_or("No Velocity release found")?
        .to_string();
    let java = fill_value(&format!("velocity/versions/{latest}")).await?["version"]["java"]
        ["version"]["minimum"]
        .as_u64()
        .unwrap_or(21);
    let download = fill_download("velocity", &latest).await?;

    let folder = sanitize_server_name(&name);
    let mut dir = servers_dir().join(&folder);
    let mut suffix = 1;
    while tokio::fs::try_exists(&dir).await.unwrap_or(false) {
        dir = servers_dir().join(format!("{folder}-{suffix}"));
        suffix += 1;
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let created = async {
        download_file(
            &download.url,
            &dir.join("velocity.jar"),
            |_, _| {},
            download.hashes.as_ref(),
        )
        .await?;
        tokio::fs::write(dir.join("velocity.toml"), base_config(PROXY_PORT))
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(dir.join(SECRET_FILE), secret())
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(error) = created {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(error);
    }

    let mut record = ServerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        framework: "Velocity".into(),
        version: latest,
        ram_mb: ram_mb.max(512),
        status: ServerRuntimeStatus::Offline,
        players: "0/500".into(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        server_path: dir.to_string_lossy().into_owned(),
        eula_accepted: Some(true),
        jar_file: Some("velocity.jar".into()),
        ..Default::default()
    };
    record.extra.insert("javaVersion".into(), java.into());
    record
        .extra
        .insert(BACKENDS.into(), Value::Array(Vec::new()));
    let mut servers = load_server_list().await;
    servers.push(record.clone());
    save_server_list(&servers).await?;
    Ok(record)
}

pub async fn backends(proxy: &ServerRecord) -> Vec<Backend> {
    let mut out = Vec::new();
    for id in backend_ids(proxy) {
        if let Some(server) = get_server(id).await {
            out.push(Backend {
                port: configured_port(Path::new(&server.server_path)).await,
                id: server.id,
                name: server.name,
                framework: server.framework,
                status: server.status,
            });
        }
    }
    out
}

pub async fn proxy_of(server_id: &str) -> Option<ServerRecord> {
    load_server_list()
        .await
        .into_iter()
        .find(|s| backend_ids(s).iter().any(|id| id == server_id))
}

async fn write_velocity_servers(proxy: &ServerRecord, ids: &[String]) -> Result<(), String> {
    let mut entries = Vec::new();
    let mut taken = Vec::new();
    for id in ids {
        let Some(server) = get_server(id.clone()).await else {
            continue;
        };
        let name = backend_name(&server.name, &taken);
        taken.push(name.clone());
        let port = configured_port(Path::new(&server.server_path)).await;
        entries.push((name, format!("127.0.0.1:{port}")));
    }
    let path = PathBuf::from(&proxy.server_path).join("velocity.toml");
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| "velocity.toml is missing from the proxy folder")?;
    let text = set_top_level(&text, "player-info-forwarding-mode", "\"modern\"");
    let text = set_top_level(
        &text,
        "forwarding-secret-file",
        &format!("\"{SECRET_FILE}\""),
    );
    tokio::fs::write(&path, set_servers(&text, &entries))
        .await
        .map_err(|e| e.to_string())
}

async fn save_backend_ids(proxy_id: &str, ids: Vec<String>) -> Result<(), String> {
    let mut servers = load_server_list().await;
    let proxy = servers
        .iter_mut()
        .find(|s| s.id == proxy_id)
        .ok_or("Proxy not found")?;
    proxy.extra.insert(
        BACKENDS.into(),
        Value::Array(ids.into_iter().map(Value::from).collect()),
    );
    save_server_list(&servers).await
}

async fn configure_paper(path: &Path, enabled: bool, secret: &str) -> Result<(), String> {
    let file = path.join("config").join("paper-global.yml");
    let Ok(mut text) = tokio::fs::read_to_string(&file).await else {
        return if enabled {
            Err("Start this server once so Paper creates config/paper-global.yml, then add it again.".into())
        } else {
            Ok(())
        };
    };
    let mut values = vec![("proxies.velocity.enabled", enabled.to_string())];
    if enabled {
        values.push(("proxies.velocity.online-mode", "true".into()));
        values.push(("proxies.velocity.secret", secret.to_string()));
    }
    for (key, value) in values {
        text = yaml_set(&text, key, &value)
            .ok_or_else(|| format!("config/paper-global.yml has no {key} setting"))?;
    }
    tokio::fs::write(&file, text)
        .await
        .map_err(|e| e.to_string())
}

async fn configure_fabric(server: &ServerRecord, secret: &str) -> Result<(), String> {
    let path = Path::new(&server.server_path);
    let installed = install_at(
        path,
        ModrinthInstallRequest {
            source: ContentSource::Modrinth,
            project_id: FABRIC_PROXY_LITE.into(),
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
    if !installed.success {
        return Err(format!(
            "Could not install FabricProxy-Lite: {}",
            installed.error.unwrap_or_default()
        ));
    }
    let config = path.join("config");
    tokio::fs::create_dir_all(&config)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(
        config.join("FabricProxy-Lite.toml"),
        format!("hackOnlineMode = true\nsecret = \"{secret}\"\n"),
    )
    .await
    .map_err(|e| e.to_string())
}

async fn set_backend_properties(
    path: &Path,
    port: Option<u16>,
    proxied: bool,
) -> Result<(), String> {
    let mut props = read_properties(path).await;
    if let Some(port) = port {
        set_prop(&mut props, "server-port", port.to_string());
    }
    set_prop(&mut props, "online-mode", (!proxied).to_string());
    set_prop(
        &mut props,
        "server-ip",
        if proxied {
            "127.0.0.1".into()
        } else {
            String::new()
        },
    );
    let saved = write_properties(path, &props).await;
    saved
        .success
        .then_some(())
        .ok_or_else(|| saved.error.unwrap_or_default())
}

pub async fn attach(proxy_id: String, backend_id: String) -> Result<(), String> {
    let proxy = get_server(proxy_id.clone())
        .await
        .ok_or("Proxy not found")?;
    let server = get_server(backend_id.clone())
        .await
        .ok_or("Server not found")?;
    if is_running(&server.id) {
        return Err(format!(
            "Stop {} before adding it to the network.",
            server.name
        ));
    }
    if proxy_of(&server.id).await.is_some() {
        return Err(format!("{} is already part of a network.", server.name));
    }
    let path = Path::new(&server.server_path);
    let secret = read_secret(Path::new(&proxy.server_path)).await?;
    match server.framework.as_str() {
        "Paper" | "Purpur" => configure_paper(path, true, &secret).await?,
        "Fabric" => configure_fabric(&server, &secret).await?,
        other => {
            return Err(format!(
                "Velocity modern forwarding needs Paper, Purpur or Fabric. {} runs {other}.",
                server.name
            ))
        }
    }
    let mut used = Vec::new();
    for other in load_server_list().await {
        if other.id != server.id {
            used.push(configured_port(Path::new(&other.server_path)).await);
        }
    }
    let current = configured_port(path).await;
    let port = if used.contains(&current) {
        free_port(&used)
    } else {
        current
    };
    set_backend_properties(path, Some(port), true).await?;

    let mut ids = backend_ids(&proxy);
    ids.push(server.id);
    write_velocity_servers(&proxy, &ids).await?;
    save_backend_ids(&proxy.id, ids).await
}

pub async fn detach(proxy_id: String, backend_id: String) -> Result<(), String> {
    let proxy = get_server(proxy_id).await.ok_or("Proxy not found")?;
    let ids: Vec<String> = backend_ids(&proxy)
        .into_iter()
        .filter(|id| *id != backend_id)
        .collect();
    write_velocity_servers(&proxy, &ids).await?;
    save_backend_ids(&proxy.id, ids).await?;
    if let Some(server) = get_server(backend_id).await {
        let path = Path::new(&server.server_path);
        set_backend_properties(path, None, false).await?;
        if server.framework != "Fabric" {
            configure_paper(path, false, "").await?;
        }
    }
    Ok(())
}

async fn wait_online(id: &str) -> bool {
    for _ in 0..240 {
        match get_server(id.to_string()).await.map(|s| s.status) {
            Some(ServerRuntimeStatus::Online) => return true,
            Some(ServerRuntimeStatus::Starting) => {}
            _ => return false,
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

pub async fn start_network(proxy_id: String) -> Result<(), String> {
    let proxy = get_server(proxy_id.clone())
        .await
        .ok_or("Proxy not found")?;
    for backend in backends(&proxy).await {
        if !is_running(&backend.id) {
            let started = start_server(backend.id.clone()).await;
            if !started.success {
                return Err(format!(
                    "{} did not start: {}",
                    backend.name,
                    started.error.unwrap_or_default()
                ));
            }
        }
        if !wait_online(&backend.id).await {
            return Err(format!(
                "{} did not come online. Check its console.",
                backend.name
            ));
        }
    }
    if is_running(&proxy.id) {
        return Ok(());
    }
    let started = start_server(proxy.id).await;
    started
        .success
        .then_some(())
        .ok_or_else(|| started.error.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_the_servers_section() {
        let original = "# comment\nbind = \"0.0.0.0:25565\"\nplayer-info-forwarding-mode = \"none\"\n\n[servers]\nlobby = \"127.0.0.1:30066\"\nfactions = \"127.0.0.1:30067\"\ntry = [\"lobby\"]\n\n[forced-hosts]\n\"lobby.example.com\" = [\"lobby\"]\n";
        let text = set_top_level(original, "player-info-forwarding-mode", "\"modern\"");
        let text = set_servers(
            &text,
            &[
                ("survival".into(), "127.0.0.1:25566".into()),
                ("creative".into(), "127.0.0.1:25567".into()),
            ],
        );
        assert!(text.starts_with("# comment\n"));
        assert!(text.contains("player-info-forwarding-mode = \"modern\""));
        assert!(!text.contains("lobby = "));
        assert!(text.contains(
            "[servers]\nsurvival = \"127.0.0.1:25566\"\ncreative = \"127.0.0.1:25567\"\ntry = [\"survival\", \"creative\"]\n\n[forced-hosts]"
        ));
        assert!(text.contains("\"lobby.example.com\""));
    }

    #[test]
    fn base_config_is_valid_toml_with_modern_forwarding() {
        let text = set_servers(
            &base_config(25570),
            &[("a".into(), "127.0.0.1:25566".into())],
        );
        assert!(toml::de::DeTable::parse(&text).is_ok());
        assert!(text.contains("bind = \"0.0.0.0:25570\""));
        assert!(text.contains("try = [\"a\"]"));
        assert!(set_top_level("[x]\n", "k", "1").starts_with("k = 1\n[x]"));
    }

    #[test]
    fn names_and_ports_are_unique() {
        assert_eq!(backend_name("My Survival!", &[]), "my-survival");
        assert_eq!(
            backend_name("Lobby", &["lobby".into(), "lobby-2".into()]),
            "lobby-3"
        );
        assert_eq!(backend_name("!!", &[]), "server");
        assert_eq!(free_port(&[25565, 25566, 25567]), 25568);
    }
}
