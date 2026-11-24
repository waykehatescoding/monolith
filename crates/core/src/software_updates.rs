use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::automation;
use crate::events::{emit, Event};
use crate::server_create::resolve_framework_download;
use crate::server_framework::change_server_framework;
use crate::server_runner::{is_running, send_console_line};
use crate::server_store::{get_server, load_server_list, save_server_list};
use crate::types::{ConsoleLineType, ServerRecord};

const BUILD_KEY: &str = "installedBuild";
const PURPUR_API: &str = "https://api.purpurmc.org/v2/purpur";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

pub fn supports(framework: &str) -> bool {
    matches!(framework, "Paper" | "Purpur" | "Fabric")
}

pub fn installed_build(server: &ServerRecord) -> Option<&str> {
    server.extra.get(BUILD_KEY).and_then(Value::as_str)
}

pub fn label(build: &str) -> &str {
    build.strip_suffix(".jar").unwrap_or(build)
}

async fn purpur_build(version: &str) -> Result<String, String> {
    let data: Value = reqwest::get(format!("{PURPUR_API}/{version}"))
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    data["builds"]["latest"]
        .as_str()
        .map(|build| format!("purpur-{version}-{build}"))
        .ok_or_else(|| "Unexpected response from Purpur".into())
}

pub async fn latest_build(framework: &str, version: &str) -> Result<String, String> {
    let build = async {
        match framework {
            "Purpur" => purpur_build(version).await,
            "Paper" | "Fabric" => resolve_framework_download(framework, version)
                .await
                .map(|download| download.filename),
            _ => Err(format!("{framework} servers cannot update automatically")),
        }
    };
    match tokio::time::timeout(CHECK_TIMEOUT, build).await {
        Ok(Ok(build)) if !build.is_empty() => Ok(build),
        Ok(Ok(_)) => Err("No build was found".into()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("The update check timed out".into()),
    }
}

async fn remember_build(id: &str, build: &str) -> Result<(), String> {
    let mut servers = load_server_list().await;
    let server = servers
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("Server not found")?;
    server.extra.insert(BUILD_KEY.into(), build.into());
    save_server_list(&servers).await
}

async fn install(server: &ServerRecord, build: &str) -> Result<(), String> {
    let jar_file = server
        .jar_file
        .as_deref()
        .filter(|j| !j.is_empty())
        .unwrap_or("server.jar");
    let jar = Path::new(&server.server_path).join(jar_file);
    let previous = jar.with_extension("jar.previous");
    let saved = tokio::fs::copy(&jar, &previous).await.is_ok();
    let result = change_server_framework(
        server.id.clone(),
        server.framework.clone(),
        server.version.clone(),
    )
    .await;
    if !result.success && saved && !jar.exists() {
        let _ = tokio::fs::rename(&previous, &jar).await;
    }
    let _ = tokio::fs::remove_file(&previous).await;
    if !result.success {
        return Err(result.error.unwrap_or_else(|| "The update failed".into()));
    }
    remember_build(&server.id, build).await
}

async fn update(server: &ServerRecord) -> Result<Option<String>, String> {
    let latest = latest_build(&server.framework, &server.version).await?;
    if installed_build(server) == Some(latest.as_str()) {
        return Ok(None);
    }
    install(server, &latest).await?;
    Ok(Some(latest))
}

pub async fn before_start(server_id: &str) {
    let Some(server) = get_server(server_id.to_string()).await else {
        return;
    };
    if !automation::of(&server).auto_update || !supports(&server.framework) {
        return;
    }
    send_console_line(
        server_id,
        "Checking for a newer server build...",
        ConsoleLineType::System,
    );
    match update(&server).await {
        Ok(Some(build)) => {
            let build = label(&build);
            send_console_line(
                server_id,
                &format!("Updated to {build}"),
                ConsoleLineType::System,
            );
            emit(Event::Notice {
                title: "Server updated",
                body: format!("{} now runs {build}.", server.name),
            });
        }
        Ok(None) => {}
        Err(error) => send_console_line(
            server_id,
            &format!("Could not update the server, starting the current build: {error}"),
            ConsoleLineType::Stderr,
        ),
    }
}

pub async fn check_now(id: String) -> Result<String, String> {
    let server = get_server(id).await.ok_or("Server not found")?;
    if !supports(&server.framework) {
        return Err(format!(
            "{} servers cannot update automatically",
            server.framework
        ));
    }
    if !is_running(&server.id) {
        return Ok(match update(&server).await? {
            Some(build) => format!("Updated to {}", label(&build)),
            None => "Already on the latest build".into(),
        });
    }
    let latest = latest_build(&server.framework, &server.version).await?;
    if installed_build(&server) == Some(latest.as_str()) {
        return Ok("Already on the latest build".into());
    }
    let when = if automation::of(&server).auto_update {
        "It installs the next time the server starts."
    } else {
        "Stop the server and check again to install it."
    };
    Ok(format!("{} is available. {when}", label(&latest)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_installed_build() {
        let mut server = ServerRecord::default();
        assert_eq!(installed_build(&server), None);
        server
            .extra
            .insert(BUILD_KEY.into(), "paper-1.21.4-232.jar".into());
        assert_eq!(installed_build(&server), Some("paper-1.21.4-232.jar"));
        assert_eq!(label("paper-1.21.4-232.jar"), "paper-1.21.4-232");
        assert!(supports("Purpur") && !supports("Forge"));
    }
}
