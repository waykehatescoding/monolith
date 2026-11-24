use std::path::{Path, PathBuf};

use sysinfo::Disks;

use crate::server_stats::calculate_folder_size;
use crate::server_store::load_server_list;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskUsage {
    pub worlds: u64,
    pub backups: u64,
    pub content: u64,
    pub logs: u64,
    pub other: u64,
}

impl DiskUsage {
    pub fn total(&self) -> u64 {
        self.worlds + self.backups + self.content + self.logs + self.other
    }

    fn slot(&mut self, name: &str, is_world: bool) -> &mut u64 {
        match name.to_ascii_lowercase().as_str() {
            _ if is_world => &mut self.worlds,
            "backups" => &mut self.backups,
            "plugins" | "mods" => &mut self.content,
            "logs" | "crash-reports" => &mut self.logs,
            _ => &mut self.other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveSpace {
    pub mount: String,
    pub free: u64,
    pub total: u64,
}

pub async fn server_disk_usage(server_path: String) -> DiskUsage {
    let mut usage = DiskUsage::default();
    let Ok(mut entries) = tokio::fs::read_dir(&server_path).await else {
        return usage;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(kind) = entry.file_type().await else {
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let (size, is_world) = if kind.is_dir() {
            let is_world = tokio::fs::metadata(path.join("level.dat")).await.is_ok();
            (calculate_folder_size(&path).await, is_world)
        } else {
            let size = entry.metadata().await.map(|meta| meta.len()).unwrap_or(0);
            (size, false)
        };
        *usage.slot(&name, is_world) += size;
    }
    usage
}

pub async fn all_servers_size() -> u64 {
    let mut total = 0;
    for server in load_server_list().await {
        total += calculate_folder_size(Path::new(&server.server_path)).await;
    }
    total
}

fn best_mount<'a>(path: &Path, mounts: impl Iterator<Item = &'a Path>) -> Option<&'a Path> {
    mounts
        .filter(|mount| path.starts_with(mount))
        .max_by_key(|mount| mount.as_os_str().len())
}

pub async fn drive_space(server_path: String) -> Option<DriveSpace> {
    tokio::task::spawn_blocking(move || {
        let path = std::fs::canonicalize(&server_path)
            .map(|path| {
                PathBuf::from(
                    path.to_string_lossy()
                        .trim_start_matches(r"\\?\")
                        .to_string(),
                )
            })
            .unwrap_or_else(|_| PathBuf::from(&server_path));
        let disks = Disks::new_with_refreshed_list();
        let mount = best_mount(&path, disks.iter().map(|disk| disk.mount_point()))?;
        let disk = disks.iter().find(|disk| disk.mount_point() == mount)?;
        Some(DriveSpace {
            mount: mount.to_string_lossy().into_owned(),
            free: disk.available_space(),
            total: disk.total_space(),
        })
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sorts_top_level_entries_into_categories() {
        let root = std::env::temp_dir().join(format!("monolith-disk-{}", uuid::Uuid::new_v4()));
        for dir in [
            "world",
            "world_nether",
            "backups",
            "plugins",
            "logs",
            "cache",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("world/level.dat"), [0; 10]).unwrap();
        std::fs::write(root.join("world_nether/level.dat"), [0; 5]).unwrap();
        std::fs::write(root.join("backups/a.zip"), [0; 100]).unwrap();
        std::fs::write(root.join("plugins/p.jar"), [0; 20]).unwrap();
        std::fs::write(root.join("logs/latest.log"), [0; 7]).unwrap();
        std::fs::write(root.join("cache/x"), [0; 3]).unwrap();
        std::fs::write(root.join("server.jar"), [0; 50]).unwrap();

        let usage = server_disk_usage(root.to_string_lossy().into_owned()).await;
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            usage,
            DiskUsage {
                worlds: 15,
                backups: 100,
                content: 20,
                logs: 7,
                other: 53,
            }
        );
        assert_eq!(usage.total(), 195);
    }

    #[test]
    fn picks_the_deepest_matching_mount() {
        let mounts = [Path::new("C:\\"), Path::new("C:\\Games"), Path::new("D:\\")];
        let path = Path::new("C:\\Games\\servers\\one");
        assert_eq!(
            best_mount(path, mounts.iter().copied()),
            Some(Path::new("C:\\Games"))
        );
        assert_eq!(best_mount(Path::new("E:\\x"), mounts.iter().copied()), None);
    }
}
