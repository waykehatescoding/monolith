use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use zip::ZipArchive;

use crate::events::{emit, Event};
use crate::safety::sanitize_download_file_name;
use crate::server_store::get_server;
use crate::worlds::check_name;

const ENABLED: &str = "datapacks";
const DISABLED: &str = "datapacks_disabled";
const MAX_MCMETA_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Datapack {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

fn plain_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(plain_text).collect(),
        Value::Object(map) => {
            let mut text = map.get("text").map(plain_text).unwrap_or_default();
            if let Some(extra) = map.get("extra") {
                text.push_str(&plain_text(extra));
            }
            text
        }
        _ => String::new(),
    }
}

fn strip_codes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn description(bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes)
        .map(|meta| strip_codes(&plain_text(&meta["pack"]["description"])))
        .unwrap_or_default()
}

fn zip_mcmeta(path: &Path) -> Option<Vec<u8>> {
    let mut archive = ZipArchive::new(File::open(path).ok()?).ok()?;
    let entry = archive.by_name("pack.mcmeta").ok()?;
    let mut bytes = Vec::new();
    entry.take(MAX_MCMETA_BYTES).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn read_packs(world: &Path) -> Vec<Datapack> {
    let mut packs = Vec::new();
    for (folder, enabled) in [(ENABLED, true), (DISABLED, false)] {
        for entry in fs::read_dir(world.join(folder))
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = if path.is_dir() {
                let file = path.join("pack.mcmeta");
                if fs::metadata(&file).map_or(true, |m| m.len() > MAX_MCMETA_BYTES) {
                    continue;
                }
                fs::read(file).ok()
            } else if name.to_lowercase().ends_with(".zip") {
                zip_mcmeta(&path)
            } else {
                continue;
            };
            packs.push(Datapack {
                description: meta.as_deref().map(description).unwrap_or_default(),
                name,
                enabled,
            });
        }
    }
    packs.sort_by_key(|pack| pack.name.to_lowercase());
    packs
}

async fn world_path(server_id: &str, world: &str) -> Result<PathBuf, String> {
    let world = check_name(world)?;
    let server = get_server(server_id.to_string())
        .await
        .ok_or("Server not found")?;
    Ok(Path::new(&server.server_path).join(world))
}

pub async fn list_datapacks(server_id: String, world: String) -> Result<Vec<Datapack>, String> {
    let path = world_path(&server_id, &world).await?;
    tokio::task::spawn_blocking(move || read_packs(&path))
        .await
        .map_err(|e| e.to_string())
}

pub async fn add_datapack(
    server_id: String,
    world: String,
    source: PathBuf,
) -> Result<String, String> {
    let path = world_path(&server_id, &world).await?;
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = sanitize_download_file_name(&name)?;
    if !name.to_lowercase().ends_with(".zip") {
        return Err("Choose a data pack .zip".into());
    }
    if [ENABLED, DISABLED]
        .iter()
        .any(|folder| path.join(folder).join(&name).exists())
    {
        return Err(format!("{name} is already installed"));
    }
    let target = path.join(ENABLED).join(&name);
    tokio::task::spawn_blocking(move || {
        zip_mcmeta(&source)
            .ok_or("This zip has no pack.mcmeta at its root, so it is not a data pack")?;
        fs::create_dir_all(target.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        fs::copy(&source, &target).map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    emit(Event::WorldsChanged { server_id });
    Ok(name)
}

fn pack_path(world: &Path, name: &str, enabled: bool) -> Result<PathBuf, String> {
    let name = sanitize_download_file_name(name)?;
    Ok(world
        .join(if enabled { ENABLED } else { DISABLED })
        .join(name))
}

pub async fn set_datapack_enabled(
    server_id: String,
    world: String,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let path = world_path(&server_id, &world).await?;
    let from = pack_path(&path, &name, !enabled)?;
    let to = pack_path(&path, &name, enabled)?;
    if to.exists() {
        return Err(format!("{name} already exists in {}", to.display()));
    }
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::rename(&from, &to)
        .await
        .map_err(|e| e.to_string())?;
    emit(Event::WorldsChanged { server_id });
    Ok(())
}

pub async fn remove_datapack(
    server_id: String,
    world: String,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let path = world_path(&server_id, &world).await?;
    let target = pack_path(&path, &name, enabled)?;
    let removed = if target.is_dir() {
        tokio::fs::remove_dir_all(&target).await
    } else {
        tokio::fs::remove_file(&target).await
    };
    removed.map_err(|e| e.to_string())?;
    emit(Event::WorldsChanged { server_id });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn reads_plain_and_component_descriptions() {
        assert_eq!(
            description(r#"{"pack":{"pack_format":48,"description":"§aGreen §lpack"}}"#.as_bytes()),
            "Green pack"
        );
        assert_eq!(
            description(
                br#"{"pack":{"description":[{"text":"Better "},{"text":"trees","extra":["!"]}]}}"#
            ),
            "Better trees!"
        );
        assert_eq!(description(b"not json"), "");
    }

    #[test]
    fn lists_enabled_and_disabled_packs() {
        let world = std::env::temp_dir().join(format!("monolith-packs-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(world.join("datapacks/folder_pack")).unwrap();
        fs::write(
            world.join("datapacks/folder_pack/pack.mcmeta"),
            r#"{"pack":{"description":"From a folder"}}"#,
        )
        .unwrap();
        fs::create_dir_all(world.join("datapacks_disabled")).unwrap();
        let mut zip =
            zip::ZipWriter::new(File::create(world.join("datapacks_disabled/zipped.zip")).unwrap());
        zip.start_file("pack.mcmeta", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"pack":{"description":"From a zip"}}"#)
            .unwrap();
        zip.finish().unwrap();
        fs::write(world.join("datapacks/notes.txt"), "x").unwrap();

        let packs = read_packs(&world);
        assert_eq!(
            packs,
            vec![
                Datapack {
                    name: "folder_pack".into(),
                    description: "From a folder".into(),
                    enabled: true
                },
                Datapack {
                    name: "zipped.zip".into(),
                    description: "From a zip".into(),
                    enabled: false
                },
            ]
        );
        assert!(pack_path(&world, "../level.dat", true).is_err());
        fs::remove_dir_all(&world).unwrap();
    }
}
