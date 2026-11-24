use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::events::{emit, Event};
use crate::server_props::{read_properties, write_properties};
use crate::server_stats::calculate_folder_size;
use crate::server_store::get_server;
use crate::server_transfer::{
    copy_server_files, safe_join, MAX_IMPORT_ENTRIES, MAX_IMPORT_UNCOMPRESSED_BYTES,
};
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

fn variants(name: &str) -> [String; 3] {
    [
        name.to_string(),
        format!("{name}{}", SUFFIXES[0]),
        format!("{name}{}", SUFFIXES[1]),
    ]
}

pub fn world_folders(server_path: &Path, name: &str) -> Vec<PathBuf> {
    variants(name)
        .iter()
        .map(|folder| server_path.join(folder))
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

fn write_world_zip(folders: &[PathBuf], output: &Path) -> Result<(), String> {
    let mut zip = ZipWriter::new(File::create(output).map_err(|e| e.to_string())?);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    for folder in folders {
        let Some(parent) = folder.parent() else {
            continue;
        };
        for entry in WalkDir::new(folder) {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == "session.lock" {
                continue;
            }
            let name = entry
                .path()
                .strip_prefix(parent)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if entry.file_type().is_dir() {
                zip.add_directory(format!("{name}/"), options)
                    .map_err(|e| e.to_string())?;
            } else if entry.file_type().is_file() {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                let mut source = File::open(entry.path()).map_err(|e| e.to_string())?;
                io::copy(&mut source, &mut zip).map_err(|e| e.to_string())?;
            }
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn export_world(server_id: String, name: String, output: PathBuf) -> Result<(), String> {
    let path = server_path(&server_id).await?;
    let folders = world_folders(&path, &name);
    if folders.is_empty() {
        return Err("World not found".into());
    }
    let target = output.clone();
    let result = tokio::task::spawn_blocking(move || write_world_zip(&folders, &target))
        .await
        .map_err(|e| e.to_string())?;
    if result.is_err() {
        let _ = fs::remove_file(&output);
    }
    result
}

fn world_root(names: &[String]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| name.strip_suffix("level.dat"))
        .filter(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        .min_by_key(|prefix| prefix.len())
        .map(str::to_string)
}

fn import_targets(root: &str, name: &str) -> Vec<(String, String)> {
    let [_, nether, end] = variants(name);
    let mut targets = vec![(root.to_string(), name.to_string())];
    if let Some(dir) = root.strip_suffix('/') {
        targets.push((format!("{dir}{}/", SUFFIXES[0]), nether));
        targets.push((format!("{dir}{}/", SUFFIXES[1]), end));
    }
    targets
}

fn extract_world(zip_path: &Path, server: &Path, name: &str) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.len() > MAX_IMPORT_ENTRIES {
        return Err("The zip contains too many files".into());
    }
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|e| e.to_string())?;
        names.push(entry.name().replace('\\', "/"));
    }
    let root = world_root(&names).ok_or("No level.dat found in the zip")?;
    let targets = import_targets(&root, name);
    let mut total = 0u64;
    for (index, entry_name) in names.iter().enumerate() {
        if entry_name.ends_with('/') || entry_name.starts_with("__MACOSX/") {
            continue;
        }
        let Some((rest, target)) = targets.iter().find_map(|(prefix, target)| {
            entry_name
                .strip_prefix(prefix.as_str())
                .map(|rest| (rest, target))
        }) else {
            continue;
        };
        if rest.rsplit('/').next() == Some("session.lock") {
            continue;
        }
        let Some(output) = safe_join(&server.join(target), rest) else {
            continue;
        };
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let size = entry.size();
        total = total.saturating_add(size);
        if total > MAX_IMPORT_UNCOMPRESSED_BYTES {
            return Err("The world is too large to import".into());
        }
        if let Some(dir) = output.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(&output).map_err(|e| e.to_string())?;
        let written = io::copy(&mut entry.take(size + 1), &mut out).map_err(|e| e.to_string())?;
        if written > size {
            return Err("The world is too large to import".into());
        }
    }
    Ok(())
}

fn copy_world(source: &Path, server: &Path, name: &str) -> Result<(), String> {
    let root = if source.join("level.dat").is_file() {
        source.to_path_buf()
    } else {
        fs::read_dir(source)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.join("level.dat").is_file())
            .ok_or("No level.dat found in that folder")?
    };
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (from, to) in variants(&root_name).iter().zip(variants(name)) {
        let from = root.with_file_name(from);
        let to = server.join(to);
        if !from.is_dir() {
            continue;
        }
        if to.starts_with(&from) {
            return Err("Choose a world folder outside this world".into());
        }
        copy_server_files(&from, &to, |_| {})?;
    }
    Ok(())
}

pub async fn import_world(
    server_id: String,
    source: PathBuf,
    name: String,
    make_active: bool,
) -> Result<String, String> {
    let name = check_name(&name)?;
    let path = server_path(&server_id).await?;
    if variants(&name)
        .iter()
        .any(|folder| path.join(folder).exists())
    {
        return Err(format!(
            "A folder named {name} already exists in this server"
        ));
    }
    let server = path.clone();
    let folder = name.clone();
    tokio::task::spawn_blocking(move || {
        let result = if source.is_dir() {
            copy_world(&source, &server, &folder)
        } else {
            extract_world(&source, &server, &folder)
        };
        if result.is_err() {
            for target in variants(&folder) {
                let _ = fs::remove_dir_all(server.join(target));
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())??;
    if make_active {
        set_properties(&path, &[("level-name", &name)]).await?;
    }
    emit(Event::WorldsChanged { server_id });
    Ok(name)
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

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("monolith-worlds-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, files: &[&str]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for name in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zip, name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn finds_the_shallowest_level_dat() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            world_root(&names(&["a/b/level.dat", "a/level.dat", "a/level.dat_old"])).as_deref(),
            Some("a/")
        );
        assert_eq!(world_root(&names(&["level.dat"])).as_deref(), Some(""));
        assert_eq!(
            world_root(&names(&["mylevel.dat", "x/level.dat_old"])),
            None
        );
    }

    #[test]
    fn imports_a_zipped_world_with_its_dimensions() {
        let dir = temp_dir();
        let zip = dir.join("upload.zip");
        write_zip(
            &zip,
            &[
                "export/world/level.dat",
                "export/world/region/r.0.0.mca",
                "export/world/session.lock",
                "export/world_nether/DIM-1/region/r.0.0.mca",
                "export/world_the_end/DIM1/region/r.0.0.mca",
                "export/readme.txt",
                "export/world/../../escape.txt",
            ],
        );
        let server = dir.join("server");
        extract_world(&zip, &server, "survival").unwrap();
        assert!(server.join("survival/level.dat").is_file());
        assert!(server.join("survival/region/r.0.0.mca").is_file());
        assert!(!server.join("survival/session.lock").exists());
        assert!(server
            .join("survival_nether/DIM-1/region/r.0.0.mca")
            .is_file());
        assert!(server
            .join("survival_the_end/DIM1/region/r.0.0.mca")
            .is_file());
        assert!(!server.join("readme.txt").exists());
        assert!(!dir.join("escape.txt").exists());
        assert!(!server.join("escape.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exported_world_imports_again() {
        let dir = temp_dir();
        let server = dir.join("server");
        for folder in ["world", "world_nether"] {
            fs::create_dir_all(server.join(folder).join("region")).unwrap();
            fs::write(server.join(folder).join("level.dat"), b"x").unwrap();
        }
        let zip = dir.join("world.zip");
        write_world_zip(&world_folders(&server, "world"), &zip).unwrap();
        extract_world(&zip, &server, "copy").unwrap();
        assert!(server.join("copy/level.dat").is_file());
        assert!(server.join("copy_nether/level.dat").is_file());
        copy_world(&server.join("world"), &server, "copy2").unwrap();
        assert!(server.join("copy2_nether/level.dat").is_file());
        assert!(copy_world(&server.join("world"), &server.join("world"), "inner").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
