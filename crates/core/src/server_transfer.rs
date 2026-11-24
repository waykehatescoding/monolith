use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::events::{emit, Event};
use crate::paths::servers_dir;
use crate::port_check::{bindable, configured_port, suggest_free_port};
use crate::server_props::{read_properties, write_properties};
use crate::server_store::{load_server_list, sanitize_server_name, save_server_list};
use crate::types::{
    BackupConfig, ExportServerResult, ImportServerResult, ServerProperty, ServerRecord,
    ServerRuntimeStatus,
};

pub(crate) const MAX_IMPORT_ENTRIES: usize = 20000;
pub(crate) const MAX_IMPORT_UNCOMPRESSED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_IMPORT_METADATA_BYTES: u64 = 64 * 1024;
const MAX_IMPORT_JAR_SCAN_BYTES: u64 = 256 * 1024 * 1024;
const SKIPPED_EXPORT_PREFIXES: [&str; 3] = ["logs/", "backups/", "cache/"];
const SKIPPED_DUPLICATE_DIRS: [&str; 3] = ["logs", "crash-reports", "backups"];

struct ImportedServer {
    platform: String,
    mc_version: String,
    ram_mb: u32,
    jar_file: Option<String>,
    name: String,
}

struct ImportFile {
    name: String,
    index: usize,
    size: u64,
}

pub async fn export_server(id: String, output: PathBuf) -> ExportServerResult {
    let Some(server) = crate::server_store::get_server(id).await else {
        return export_error("Server not found");
    };

    let output_clone = output.clone();
    let result = tokio::task::spawn_blocking(move || write_export(&server, &output_clone)).await;
    match result {
        Ok(Ok(())) => ExportServerResult {
            success: true,
            error: None,
            path: Some(output.to_string_lossy().into_owned()),
        },
        Ok(Err(e)) => export_error(&e),
        Err(e) => export_error(&e.to_string()),
    }
}

fn export_error(message: &str) -> ExportServerResult {
    ExportServerResult {
        success: false,
        error: Some(message.to_string()),
        path: None,
    }
}

fn write_export(server: &ServerRecord, output: &Path) -> Result<(), String> {
    let file = File::create(output).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(9))
        .large_file(true);

    let mut manifest_server = json!({
        "name": server.name,
        "platform": server.framework,
        "mcVersion": server.version,
        "ramMB": server.ram_mb,
    });
    if let Some(jar) = &server.jar_file {
        manifest_server["jarFile"] = json!(jar);
    }
    let manifest = json!({
        "version": "1.0",
        "exportedAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "server": manifest_server,
    });
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    zip.start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(manifest_text.as_bytes())
        .map_err(|e| e.to_string())?;

    let root = Path::new(&server.server_path);
    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|e| e.to_string())?;
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let name = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let probe = if entry.file_type().is_dir() {
            format!("{name}/")
        } else {
            name.clone()
        };
        if SKIPPED_EXPORT_PREFIXES.iter().any(|p| probe.starts_with(p)) {
            continue;
        }
        if entry.file_type().is_dir() {
            zip.add_directory(probe, options)
                .map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            let mut source = File::open(entry.path()).map_err(|e| e.to_string())?;
            io::copy(&mut source, &mut zip).map_err(|e| e.to_string())?;
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn get_import_root(names: &[String]) -> String {
    let mut root = String::new();
    loop {
        let rest: Vec<&str> = names
            .iter()
            .filter(|n| n.starts_with(&root))
            .map(|n| &n[root.len()..])
            .filter(|n| !n.is_empty())
            .collect();
        let Some(first) = rest.first().map(|n| n.split('/').next().unwrap_or("")) else {
            return root;
        };
        if first.is_empty() {
            return root;
        }
        let prefix = format!("{first}/");
        if !rest.iter().all(|n| n.starts_with(&prefix)) {
            return root;
        }
        root.push_str(&prefix);
    }
}

fn read_entry(archive: &mut ZipArchive<File>, index: usize) -> Result<Vec<u8>, String> {
    let entry = archive.by_index(index).map_err(|e| e.to_string())?;
    let size = entry.size();
    let mut data = Vec::new();
    entry
        .take(size)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    Ok(data)
}

fn read_manifest(
    archive: &mut ZipArchive<File>,
    file: Option<&ImportFile>,
) -> Option<ImportedServer> {
    let file = file?;
    if file.size > MAX_IMPORT_METADATA_BYTES {
        return None;
    }
    let data = read_entry(archive, file.index).ok()?;
    let parsed: Value = serde_json::from_slice(&data).ok()?;
    let server = parsed.get("server")?;
    let platform = server.get("platform")?.as_str()?.to_string();
    let mc_version = server.get("mcVersion")?.as_str()?.to_string();
    Some(ImportedServer {
        platform,
        mc_version,
        ram_mb: server
            .get("ramMB")
            .and_then(Value::as_f64)
            .map(|n| n as u32)
            .unwrap_or(0),
        jar_file: server
            .get("jarFile")
            .and_then(Value::as_str)
            .map(str::to_string),
        name: server
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Imported Server")
            .to_string(),
    })
}

fn read_jar_version(archive: &mut ZipArchive<File>, file: &ImportFile) -> Option<String> {
    if file.size > MAX_IMPORT_JAR_SCAN_BYTES {
        return None;
    }
    let data = read_entry(archive, file.index).ok()?;
    let mut jar = ZipArchive::new(Cursor::new(data)).ok()?;
    let mut entry = jar.by_name("version.json").ok()?;
    if entry.size() > MAX_IMPORT_METADATA_BYTES {
        return None;
    }
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    if text.is_empty() {
        return None;
    }
    let parsed: Value = serde_json::from_str(&text).ok()?;
    parsed.get("id")?.as_str().map(str::to_string)
}

fn version_from_jar_name(name: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)(\d+\.\d+(?:\.\d+)?(?:-(?:pre|rc|snapshot)-?\d+)?)").ok()?;
    re.captures(name)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn detect_imported_server(
    archive: &mut ZipArchive<File>,
    files: &[ImportFile],
) -> Option<ImportedServer> {
    let jars: Vec<&ImportFile> = files
        .iter()
        .filter(|f| !f.name.contains('/') && f.name.to_lowercase().ends_with(".jar"))
        .collect();
    let pick = |pattern: &str| {
        let re = regex::Regex::new(pattern).ok()?;
        jars.iter()
            .find(|f| re.is_match(&f.name.to_lowercase()))
            .copied()
    };
    let jar = pick(r"^fabric-server")
        .or_else(|| pick(r"^(server|paper|purpur|minecraft_server)"))
        .or_else(|| jars.first().copied())?;

    let lower = jar.name.to_lowercase();
    let has = |prefix: &str| files.iter().any(|f| f.name.starts_with(prefix));
    let platform = if lower.contains("fabric") || has(".fabric/") {
        "Fabric"
    } else if lower.contains("purpur") || has("purpur.yml") {
        "Purpur"
    } else if lower.contains("paper") || has("plugins/") || has("config/paper-global.yml") {
        "Paper"
    } else if has("mods/") {
        "Fabric"
    } else {
        "Vanilla"
    };
    let mc_version = read_jar_version(archive, jar)
        .or_else(|| version_from_jar_name(&jar.name))
        .unwrap_or_else(|| "Unknown".to_string());

    Some(ImportedServer {
        name: "Imported Server".to_string(),
        platform: platform.to_string(),
        mc_version,
        ram_mb: 4096,
        jar_file: Some(jar.name.clone()),
    })
}

pub(crate) fn safe_join(base: &Path, name: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    let mut depth = 0usize;
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                out.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (depth > 0).then_some(out)
}

fn extract_import(
    zip_path: &str,
    custom_name: &str,
    server_path: &Path,
    server_id: &str,
) -> Result<ServerRecord, String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.len() > MAX_IMPORT_ENTRIES {
        return Err("Import archive contains too many files".to_string());
    }

    let mut named = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if name.starts_with("__MACOSX/") {
            continue;
        }
        named.push((name, index, entry.size(), entry.is_dir()));
    }

    let names: Vec<String> = named.iter().map(|n| n.0.clone()).collect();
    let root = get_import_root(&names);
    let files: Vec<ImportFile> = named
        .into_iter()
        .filter(|(name, _, _, is_dir)| !is_dir && name.starts_with(&root))
        .map(|(name, index, size, _)| ImportFile {
            name: name[root.len()..].to_string(),
            index,
            size,
        })
        .collect();

    let manifest_file = files.iter().find(|f| f.name == "manifest.json");
    let manifest_server = read_manifest(&mut archive, manifest_file);
    let used_manifest = manifest_server.is_some();
    let detected = match manifest_server {
        Some(server) => Some(server),
        None => detect_imported_server(&mut archive, &files),
    };
    let Some(detected) = detected else {
        return Err("No Monolith manifest or server .jar found in the archive".to_string());
    };

    fs::create_dir_all(server_path).map_err(|e| e.to_string())?;

    let mut total: u64 = 0;
    for f in &files {
        if used_manifest && f.name == "manifest.json" {
            continue;
        }
        total = total.saturating_add(f.size);
        if total > MAX_IMPORT_UNCOMPRESSED_BYTES {
            return Err("Import archive is too large".to_string());
        }
        let Some(output) = safe_join(server_path, &f.name) else {
            continue;
        };
        if let Some(dir) = output.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let entry = archive.by_index(f.index).map_err(|e| e.to_string())?;
        let mut out = File::create(&output).map_err(|e| e.to_string())?;
        let written = io::copy(&mut entry.take(f.size + 1), &mut out).map_err(|e| e.to_string())?;
        if written > f.size {
            return Err("Import archive is too large".to_string());
        }
    }

    Ok(ServerRecord {
        id: server_id.to_string(),
        name: if custom_name.is_empty() {
            detected.name
        } else {
            custom_name.to_string()
        },
        version: detected.mc_version,
        framework: detected.platform,
        server_path: server_path.to_string_lossy().into_owned(),
        ram_mb: if detected.ram_mb == 0 {
            4096
        } else {
            detected.ram_mb
        },
        status: ServerRuntimeStatus::Offline,
        players: "0/20".to_string(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        jar_file: Some(
            detected
                .jar_file
                .unwrap_or_else(|| "server.jar".to_string()),
        ),
        eula_accepted: None,
        java_path: None,
        priority: None,
        backup_config: None,
        use_ngrok: None,
        ..ServerRecord::default()
    })
}

pub async fn import_server(zip_path: String, custom_name: String) -> ImportServerResult {
    let server_id = uuid::Uuid::new_v4().to_string();
    let server_path = servers_dir().join(&server_id);

    let extract_path = server_path.clone();
    let extract_id = server_id.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        let result = extract_import(&zip_path, &custom_name, &extract_path, &extract_id);
        if result.is_err() {
            let _ = fs::remove_dir_all(&extract_path);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    let saved = match extracted {
        Ok(record) => {
            let mut servers = crate::server_store::load_server_list().await;
            servers.push(record.clone());
            crate::server_store::save_server_list(&servers)
                .await
                .map(|_| record)
        }
        Err(e) => Err(e),
    };

    match saved {
        Ok(server) => ImportServerResult {
            success: true,
            server: Some(server),
            error: None,
        },
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&server_path).await;
            ImportServerResult {
                success: false,
                server: None,
                error: Some(error),
            }
        }
    }
}

pub(crate) fn copy_server_files(
    from: &Path,
    to: &Path,
    progress: impl Fn(f64),
) -> Result<(), String> {
    let entries: Vec<walkdir::DirEntry> = WalkDir::new(from)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| {
            !(entry.depth() == 1
                && entry.file_type().is_dir()
                && SKIPPED_DUPLICATE_DIRS
                    .iter()
                    .any(|dir| entry.file_name().eq_ignore_ascii_case(dir)))
        })
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let size = |entry: &walkdir::DirEntry| entry.metadata().map(|m| m.len()).unwrap_or(0);
    let total: u64 = entries
        .iter()
        .filter(|e| !e.file_type().is_dir())
        .map(size)
        .sum();
    let mut copied = 0u64;
    let mut reported = 0.0;
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in &entries {
        let relative = entry.path().strip_prefix(from).map_err(|e| e.to_string())?;
        let target = to.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        copied += size(entry);
        let percent = if total > 0 {
            copied as f64 / total as f64 * 100.0
        } else {
            100.0
        };
        if percent - reported >= 1.0 {
            reported = percent;
            progress(percent);
        }
    }
    Ok(())
}

async fn give_free_port(server_path: &Path, others: &[ServerRecord]) -> Result<(), String> {
    let mut taken = Vec::new();
    for other in others {
        taken.push(configured_port(Path::new(&other.server_path)).await);
    }
    let port = configured_port(server_path).await;
    if !taken.contains(&port) {
        return Ok(());
    }
    let free = suggest_free_port(port, |p| taken.contains(&p) || !bindable(p))
        .ok_or("No free port found for the copy")?;
    let mut properties = read_properties(server_path).await;
    match properties.iter_mut().find(|p| p.key == "server-port") {
        Some(existing) => existing.value = free.to_string(),
        None => properties.push(ServerProperty {
            key: "server-port".to_string(),
            value: free.to_string(),
            comment: None,
        }),
    }
    let written = write_properties(server_path, &properties).await;
    if written.success {
        Ok(())
    } else {
        Err(written.error.unwrap_or_default())
    }
}

async fn duplicate_into(
    source: &ServerRecord,
    name: &str,
    target: &Path,
) -> Result<ServerRecord, String> {
    let from = PathBuf::from(&source.server_path);
    let to = target.to_path_buf();
    let server_id = source.id.clone();
    tokio::task::spawn_blocking(move || {
        copy_server_files(&from, &to, |percent| {
            emit(Event::DuplicateProgress {
                server_id: server_id.clone(),
                percent,
            })
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut servers = load_server_list().await;
    give_free_port(target, &servers).await?;
    let record = ServerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        server_path: target.to_string_lossy().into_owned(),
        status: ServerRuntimeStatus::Offline,
        players: "0/20".to_string(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        backup_config: source.backup_config.clone().map(|config| BackupConfig {
            last_backup_at: None,
            ..config
        }),
        ngrok_url: None,
        start_on_launch: false,
        ..source.clone()
    };
    servers.push(record.clone());
    save_server_list(&servers).await?;
    Ok(record)
}

pub async fn duplicate_server(id: String, name: String) -> ImportServerResult {
    let failure = |error: &str| ImportServerResult {
        success: false,
        server: None,
        error: Some(error.to_string()),
    };
    let name = name.trim();
    if name.is_empty() {
        return failure("Enter a name for the copy");
    }
    let Some(source) = crate::server_store::get_server(id.clone()).await else {
        return failure("Server not found");
    };
    if crate::server_runner::is_running(&id) {
        return failure("Stop the server before duplicating it");
    }

    let folder = sanitize_server_name(name);
    let mut target = servers_dir().join(&folder);
    let mut suffix = 1;
    while target.exists() {
        target = servers_dir().join(format!("{folder}-{suffix}"));
        suffix += 1;
    }

    match duplicate_into(&source, name, &target).await {
        Ok(server) => ImportServerResult {
            success: true,
            server: Some(server),
            error: None,
        },
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&target).await;
            failure(&error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_zip(files: &[(&str, &[u8])]) -> String {
        let dir = std::env::temp_dir().join(format!("monolith-zip-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        for (name, data) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        path.to_string_lossy().into_owned()
    }

    fn vanilla_jar(id: &str) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(&mut buf);
        zip.start_file("version.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(json!({ "id": id }).to_string().as_bytes())
            .unwrap();
        zip.finish().unwrap();
        buf.into_inner()
    }

    fn run(zip: &str, name: &str) -> Result<ServerRecord, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = std::env::temp_dir().join(format!("monolith-import-{id}"));
        extract_import(zip, name, &path, &id)
    }

    #[test]
    fn imports_a_manifest_export() {
        let manifest = json!({
            "version": "1.0",
            "server": { "name": "Old", "platform": "Paper", "mcVersion": "1.21.4", "ramMB": 6144, "jarFile": "server.jar" }
        })
        .to_string();
        let zip = write_zip(&[
            ("manifest.json", manifest.as_bytes()),
            ("server.jar", b"jar"),
            ("world/level.dat", b"level"),
        ]);
        let server = run(&zip, "").unwrap();
        assert_eq!(server.name, "Old");
        assert_eq!(server.framework, "Paper");
        assert_eq!(server.version, "1.21.4");
        assert_eq!(server.ram_mb, 6144);
        assert!(Path::new(&server.server_path)
            .join("world/level.dat")
            .exists());
        assert!(!Path::new(&server.server_path)
            .join("manifest.json")
            .exists());
    }

    #[test]
    fn imports_a_wrapped_server_folder() {
        let jar = vanilla_jar("1.21.5-pre1");
        let zip = write_zip(&[
            ("My Server/server.jar", &jar),
            ("My Server/plugins/Essentials.jar", b"plugin"),
            ("My Server/server.properties", b"motd=hi"),
        ]);
        let server = run(&zip, "Imported").unwrap();
        assert_eq!(server.framework, "Paper");
        assert_eq!(server.version, "1.21.5-pre1");
        assert_eq!(server.jar_file.as_deref(), Some("server.jar"));
        assert!(Path::new(&server.server_path)
            .join("server.properties")
            .exists());
    }

    #[test]
    fn detects_fabric_from_the_launcher_jar_name() {
        let zip = write_zip(&[
            (
                "fabric-server-mc.26.4-snapshot-3-loader.0.17.2-launcher.1.1.0.jar",
                b"jar",
            ),
            ("mods/sodium.jar", b"mod"),
        ]);
        let server = run(&zip, "Fabric").unwrap();
        assert_eq!(server.framework, "Fabric");
        assert_eq!(server.version, "26.4-snapshot-3");
    }

    #[test]
    fn rejects_archives_without_a_manifest_or_jar() {
        let zip = write_zip(&[("readme.txt", b"hi")]);
        assert!(run(&zip, "x").is_err());
    }

    #[test]
    fn duplicate_copy_skips_logs_crash_reports_and_backups() {
        let root = std::env::temp_dir().join(format!("monolith-dup-{}", uuid::Uuid::new_v4()));
        let from = root.join("from");
        let to = root.join("to");
        for dir in ["logs", "crash-reports", "backups", "world", "plugins/logs"] {
            fs::create_dir_all(from.join(dir)).unwrap();
        }
        fs::write(from.join("logs/latest.log"), "log").unwrap();
        fs::write(from.join("crash-reports/c.txt"), "crash").unwrap();
        fs::write(from.join("backups/b.zip"), "zip").unwrap();
        fs::write(from.join("world/level.dat"), "level").unwrap();
        fs::write(from.join("plugins/logs/keep.txt"), "keep").unwrap();
        fs::write(from.join("server.properties"), "server-port=25565").unwrap();

        copy_server_files(&from, &to, |_| {}).unwrap();

        assert!(to.join("world/level.dat").exists());
        assert!(to.join("server.properties").exists());
        assert!(to.join("plugins/logs/keep.txt").exists());
        assert!(!to.join("logs").exists());
        assert!(!to.join("crash-reports").exists());
        assert!(!to.join("backups").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_zip_slip_entries() {
        let dir = std::env::temp_dir().join("monolith-slip");
        assert!(safe_join(&dir, "../evil.txt").is_none());
        assert!(safe_join(&dir, "/abs.txt").is_none());
        assert!(safe_join(&dir, "a/b.txt").is_some());
    }
}
