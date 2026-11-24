use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::events::{emit, Event};
use crate::server_props::{read_properties, write_properties};
use crate::server_stats::calculate_folder_size;
use crate::server_store::get_server;
use crate::types::ServerProperty;

const SUFFIXES: [&str; 2] = ["_nether", "_the_end"];

#[derive(Debug, Clone, PartialEq)]
pub struct World {
    pub name: String,
    pub size: u64,
    pub modified: Option<String>,
    pub active: bool,
    pub generated: bool,
}

pub fn level_name(props: &[ServerProperty]) -> String {
    props
        .iter()
        .find(|p| p.key == "level-name")
        .map(|p| p.value.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "world".into())
}

pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.');
    if name.is_empty() {
        return Err("Enter a world name".into());
    }
    if name.len() > 64 || name.starts_with('.') || name.ends_with('.') || !name.chars().all(allowed)
    {
        return Err("Use up to 64 letters, numbers, spaces, dashes, dots or underscores".into());
    }
    Ok(name.to_string())
}

fn world_names(server_path: &Path) -> Vec<String> {
    let all: Vec<String> = fs::read_dir(server_path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("level.dat").is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect();
    let mut names: Vec<String> = all
        .iter()
        .filter(|name| {
            !SUFFIXES.iter().any(|suffix| {
                name.strip_suffix(suffix)
                    .is_some_and(|base| all.iter().any(|n| n == base))
            })
        })
        .cloned()
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

pub fn world_folders(server_path: &Path, name: &str) -> Vec<PathBuf> {
    std::iter::once(String::new())
        .chain(SUFFIXES.iter().map(|s| s.to_string()))
        .map(|suffix| server_path.join(format!("{name}{suffix}")))
        .filter(|path| path.is_dir())
        .collect()
}

async fn server_path(server_id: &str) -> Result<PathBuf, String> {
    get_server(server_id.to_string())
        .await
        .map(|server| PathBuf::from(server.server_path))
        .ok_or_else(|| "Server not found".to_string())
}

pub async fn list_worlds(server_id: String) -> Result<Vec<World>, String> {
    let path = server_path(&server_id).await?;
    let active = level_name(&read_properties(&path).await);
    let names = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || world_names(&path))
            .await
            .map_err(|e| e.to_string())?
    };
    let mut worlds = Vec::new();
    for name in &names {
        let mut size = 0;
        for folder in world_folders(&path, name) {
            size += calculate_folder_size(&folder).await;
        }
        let modified = fs::metadata(path.join(name).join("level.dat"))
            .and_then(|meta| meta.modified())
            .ok()
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339());
        worlds.push(World {
            active: *name == active,
            name: name.clone(),
            size,
            modified,
            generated: true,
        });
    }
    if !names.contains(&active) {
        worlds.insert(
            0,
            World {
                name: active,
                size: 0,
                modified: None,
                active: true,
                generated: false,
            },
        );
    }
    Ok(worlds)
}

pub(crate) async fn set_properties(path: &Path, values: &[(&str, &str)]) -> Result<(), String> {
    let mut props = read_properties(path).await;
    for (key, value) in values {
        match props.iter_mut().find(|p| p.key == *key) {
            Some(existing) => existing.value = value.to_string(),
            None => props.push(ServerProperty {
                key: key.to_string(),
                value: value.to_string(),
                comment: None,
            }),
        }
    }
    let result = write_properties(path, &props).await;
    if result.success {
        Ok(())
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "Could not save server.properties".into()))
    }
}

pub async fn set_active_world(server_id: String, name: String) -> Result<(), String> {
    let name = check_name(&name)?;
    let path = server_path(&server_id).await?;
    set_properties(&path, &[("level-name", &name)]).await?;
    emit(Event::WorldsChanged { server_id });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_paper_dimension_folders_with_their_world() {
        let dir = std::env::temp_dir().join(format!("monolith-worlds-{}", uuid::Uuid::new_v4()));
        for name in [
            "world",
            "world_nether",
            "world_the_end",
            "creative",
            "lonely_nether",
        ] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("level.dat"), b"").unwrap();
        }
        fs::create_dir_all(dir.join("plugins")).unwrap();
        assert_eq!(
            world_names(&dir),
            vec!["creative", "lonely_nether", "world"]
        );
        assert_eq!(world_folders(&dir, "world").len(), 3);
        assert_eq!(world_folders(&dir, "creative").len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unsafe_world_names() {
        assert_eq!(check_name(" survival 2 ").as_deref(), Ok("survival 2"));
        for bad in ["", "..", "../world", "a/b", "C:", "world\\x", ".hidden"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn level_name_defaults_to_world() {
        assert_eq!(level_name(&[]), "world");
        let props = [ServerProperty {
            key: "level-name".into(),
            value: "hub".into(),
            comment: None,
        }];
        assert_eq!(level_name(&props), "hub");
    }
}
