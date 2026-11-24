use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::content_sources::loader_for;
use crate::modrinth::get_modrinth_project;
use crate::modrinth_installs::install_modrinth_project;
use crate::types::{ContentSource, ModrinthInstallRequest, ServerRecord};

pub const PROFILE_COMMAND: &str = "spark profiler start --timeout 60";
pub const HEALTH_COMMAND: &str = "spark health --upload";

static REPORT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"https://spark\.lucko\.me/[A-Za-z0-9]+").expect("static regex"));
static CHAT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\]: <[^>]+> ").expect("static regex"));

pub fn supports(framework: &str) -> bool {
    loader_for(framework).is_some()
}

pub fn report_url(line: &str) -> Option<String> {
    if CHAT.is_match(line) {
        return None;
    }
    REPORT.find(line).map(|found| found.as_str().to_string())
}

fn is_spark_jar(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("spark") && name.ends_with(".jar")
}

pub async fn is_installed(server_path: String) -> bool {
    for folder in ["plugins", "mods"] {
        let Ok(mut entries) = tokio::fs::read_dir(Path::new(&server_path).join(folder)).await
        else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if is_spark_jar(&entry.file_name().to_string_lossy()) {
                return true;
            }
        }
    }
    false
}

pub async fn install(server: ServerRecord) -> Result<(), String> {
    let (project_type, loader) = loader_for(&server.framework)
        .ok_or_else(|| format!("spark does not support {} servers.", server.framework))?;
    let project = get_modrinth_project("spark".into()).await?;
    let result = install_modrinth_project(
        server.id,
        ModrinthInstallRequest {
            source: ContentSource::Modrinth,
            project_id: project.project_id,
            project_type,
            loader: Some(loader.into()),
            game_version: Some(server.version),
            version_id: None,
            title: Some(project.title),
            slug: Some(project.slug),
            icon_url: project.icon_url,
        },
    )
    .await;
    match result.success {
        true => Ok(()),
        false => Err(result
            .error
            .unwrap_or_else(|| "spark could not be installed.".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_report_links_in_console_lines() {
        assert_eq!(
            report_url("[12:00:00 INFO]: [⚡] Profiler results: https://spark.lucko.me/AbC123xyz"),
            Some("https://spark.lucko.me/AbC123xyz".into())
        );
        assert_eq!(
            report_url(
                "[12:00:00] [Server thread/INFO]: Health report: https://spark.lucko.me/q1w2e3."
            ),
            Some("https://spark.lucko.me/q1w2e3".into())
        );
        assert_eq!(report_url("[12:00:00 INFO]: Profiler started"), None);
        assert_eq!(report_url("https://spark.lucko.me/"), None);
        assert_eq!(report_url("https://evil.io/spark.lucko.me/abc"), None);
    }

    #[test]
    fn ignores_links_typed_in_chat() {
        assert_eq!(
            report_url("[12:00:00 INFO]: <Bob> https://spark.lucko.me/fake"),
            None
        );
    }

    #[test]
    fn recognises_spark_jars() {
        assert!(is_spark_jar("spark-1.10.73-paper.jar"));
        assert!(is_spark_jar("Spark.jar"));
        assert!(!is_spark_jar("sparkles.txt"));
        assert!(!is_spark_jar("worldedit.jar"));
    }

    #[test]
    fn supports_modded_and_plugin_servers_only() {
        assert!(supports("Paper"));
        assert!(supports("NeoForge"));
        assert!(!supports("Vanilla"));
    }
}
