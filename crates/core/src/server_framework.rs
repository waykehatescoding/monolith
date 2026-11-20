use crate::events::{emit, Event};
use crate::java_manager::{ensure_java_installed, get_required_java_version, JavaInstallStage};
use crate::safety::sanitize_download_file_name;
use crate::server_create::{
    download_file, generate_start_bat, resolve_framework_download, run_installer,
};
use crate::server_store::{load_server_list, save_server_list};
use crate::types::{
    ChangeServerFrameworkResult, ServerCreationProgress, ServerCreationStage, ServerRecord,
    ServerRuntimeStatus,
};
use std::path::{Path, PathBuf};

const ANALYTICS_JAR: &str = "MonolithAnalytics.jar";

pub fn get_analytics_plugin_path() -> PathBuf {
    crate::paths::resource_path("plugins").join(ANALYTICS_JAR)
}

pub async fn install_analytics_plugin(server_path: &Path) -> bool {
    let src = get_analytics_plugin_path();
    if !src.exists() {
        eprintln!("MonolithAnalytics JAR not found at {}", src.display());
        return false;
    }
    let plugins_dir = server_path.join("plugins");
    if let Err(e) = tokio::fs::create_dir_all(&plugins_dir).await {
        eprintln!("Failed to create {}: {e}", plugins_dir.display());
        return false;
    }
    match tokio::fs::copy(&src, plugins_dir.join(ANALYTICS_JAR)).await {
        Ok(_) => true,
        Err(e) => {
            eprintln!("Failed to copy MonolithAnalytics JAR: {e}");
            false
        }
    }
}

pub async fn uninstall_analytics_plugin(server_path: &Path) {
    let _ = tokio::fs::remove_file(server_path.join("plugins").join(ANALYTICS_JAR)).await;
}

fn progress(
    stage: ServerCreationStage,
    message: impl Into<String>,
    percent: f64,
) -> ServerCreationProgress {
    ServerCreationProgress {
        stage,
        message: message.into(),
        percent,
        bytes_downloaded: None,
        bytes_total: None,
    }
}

fn send(p: ServerCreationProgress) {
    emit(Event::FrameworkChangeProgress(p));
}

async fn run_change(
    id: &str,
    framework: &str,
    version: &str,
) -> Result<Result<ServerRecord, String>, String> {
    let mut servers = load_server_list().await;
    let Some(index) = servers.iter().position(|s| s.id == id) else {
        return Ok(Err("Server not found".into()));
    };
    let status = servers[index].status;
    if !matches!(
        status,
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle | ServerRuntimeStatus::Crashed
    ) {
        return Ok(Err("Stop the server before changing its software".into()));
    }
    let server_path = PathBuf::from(&servers[index].server_path);
    let old_jar = servers[index].jar_file.clone();
    let ram_mb = servers[index].ram_mb;

    send(progress(
        ServerCreationStage::Downloading,
        format!("Resolving {framework} {version} download..."),
        5.0,
    ));
    let info = resolve_framework_download(framework, version).await?;

    send(progress(
        ServerCreationStage::SetupJava,
        "Checking required Java version...",
        12.0,
    ));
    let java_version = get_required_java_version(version);
    let java_progress: crate::java_manager::JavaProgress =
        Box::new(move |stage, percent, downloaded, total| {
            let message = if stage == JavaInstallStage::Extracting {
                format!("Extracting Java {java_version}...")
            } else {
                format!("Downloading Java {java_version}... {}%", percent.round())
            };
            send(ServerCreationProgress {
                stage: ServerCreationStage::SetupJava,
                message,
                percent: 12.0 + percent * 0.08,
                bytes_downloaded: downloaded,
                bytes_total: total,
            });
        });
    let java = ensure_java_installed(java_version, Some(java_progress)).await;

    send(progress(
        ServerCreationStage::Downloading,
        format!("Downloading {framework} {version}..."),
        20.0,
    ));
    let dest_filename = sanitize_download_file_name(&info.jar_file)?;
    let new_jar_path = server_path.join(&dest_filename);
    let (fw, ver) = (framework.to_string(), version.to_string());
    download_file(
        &info.url,
        &new_jar_path,
        move |downloaded, total| {
            let downloaded_mb = format!("{:.1}", downloaded as f64 / 1024.0 / 1024.0);
            if total > 0 {
                let pct = downloaded as f64 / total as f64 * 100.0;
                let total_mb = format!("{:.1}", total as f64 / 1024.0 / 1024.0);
                send(ServerCreationProgress {
                    stage: ServerCreationStage::Downloading,
                    message: format!(
                        "Downloading {fw} {ver}... {}% ({downloaded_mb} / {total_mb} MB)",
                        pct.round()
                    ),
                    percent: (20.0 + pct * 0.7).round(),
                    bytes_downloaded: Some(downloaded),
                    bytes_total: Some(total),
                });
            } else {
                let est = (downloaded as f64 / (50.0 * 1024.0 * 1024.0) * 100.0).min(99.0);
                send(ServerCreationProgress {
                    stage: ServerCreationStage::Downloading,
                    message: format!("Downloading {fw} {ver}... {downloaded_mb} MB"),
                    percent: (20.0 + est * 0.7).round(),
                    bytes_downloaded: Some(downloaded),
                    bytes_total: Some(0),
                });
            }
        },
        info.hashes.as_ref(),
    )
    .await?;

    if !info.install_args.is_empty() {
        send(progress(
            ServerCreationStage::WritingFiles,
            format!("Running the {framework} installer..."),
            90.0,
        ));
    }
    let launch = run_installer(&info, &server_path, java).await?;

    if let Some(old) = old_jar
        .as_deref()
        .filter(|j| !j.is_empty() && *j != launch && !j.starts_with('@'))
    {
        let _ = tokio::fs::remove_file(server_path.join(old)).await;
    }

    send(progress(
        ServerCreationStage::WritingFiles,
        "Updating start.bat...",
        92.0,
    ));
    tokio::fs::write(
        server_path.join("start.bat"),
        generate_start_bat(ram_mb, &launch),
    )
    .await
    .map_err(|e| e.to_string())?;

    let plugin_compatible = framework == "Paper" || framework == "Purpur";
    let server = &mut servers[index];
    if server.analytics_enabled == Some(true) && !plugin_compatible {
        uninstall_analytics_plugin(&server_path).await;
        server.analytics_enabled = Some(false);
    }
    server.framework = framework.to_string();
    server.version = version.to_string();
    server.jar_file = Some(launch);
    let updated = server.clone();

    send(progress(
        ServerCreationStage::WritingFiles,
        "Saving server configuration...",
        96.0,
    ));
    save_server_list(&servers).await?;

    send(progress(
        ServerCreationStage::Done,
        "Server software updated!",
        100.0,
    ));
    Ok(Ok(updated))
}

pub async fn change_server_framework(
    id: String,
    framework: String,
    version: String,
) -> ChangeServerFrameworkResult {
    match run_change(&id, &framework, &version).await {
        Ok(Ok(server)) => ChangeServerFrameworkResult {
            success: true,
            server: Some(server),
            error: None,
        },
        Ok(Err(error)) => ChangeServerFrameworkResult {
            success: false,
            server: None,
            error: Some(error),
        },
        Err(error) => {
            send(progress(ServerCreationStage::Error, error.clone(), 0.0));
            ChangeServerFrameworkResult {
                success: false,
                server: None,
                error: Some(error),
            }
        }
    }
}
