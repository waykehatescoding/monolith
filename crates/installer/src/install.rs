use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const EXE_NAME: &str = "Monolith.exe";
pub const UNINSTALLER_NAME: &str = "Uninstall Monolith.exe";
const PLUGIN_DIR: [&str; 2] = ["assets", "plugins"];
const PLUGIN_NAME: &str = "MonolithAnalytics.jar";

pub struct Payload {
    pub exe: &'static [u8],
    pub plugin: &'static [u8],
}

impl Payload {
    pub fn is_empty(&self) -> bool {
        self.exe.is_empty()
    }
}

pub fn app_path(root: &Path) -> PathBuf {
    root.join(EXE_NAME)
}

pub fn uninstaller_path(root: &Path) -> PathBuf {
    root.join(UNINSTALLER_NAME)
}

fn plugin_dir(root: &Path) -> PathBuf {
    PLUGIN_DIR
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let partial = path.with_extension("partial");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path).inspect_err(|_| {
        let _ = fs::remove_file(&partial);
    })
}

pub fn install_files(root: &Path, payload: &Payload) -> io::Result<()> {
    if payload.is_empty() {
        return Err(io::Error::other("Development build: no payload"));
    }
    let plugins = plugin_dir(root);
    fs::create_dir_all(&plugins)?;
    write_replacing(&app_path(root), payload.exe)?;
    write_replacing(&plugins.join(PLUGIN_NAME), payload.plugin)
}

pub fn write_uninstaller(root: &Path, setup: &Path) -> io::Result<()> {
    let target = uninstaller_path(root);
    if setup == target {
        return Ok(());
    }
    fs::copy(setup, target).map(|_| ())
}

pub fn remove_program_files(root: &Path) -> io::Result<()> {
    remove_file(&app_path(root))?;
    remove_file(&plugin_dir(root).join(PLUGIN_NAME))?;
    let _ = fs::remove_dir(plugin_dir(root));
    let _ = fs::remove_dir(root.join(PLUGIN_DIR[0]));
    Ok(())
}

pub fn installed_size_kb(root: &Path) -> u32 {
    let bytes: u64 = [
        app_path(root),
        uninstaller_path(root),
        plugin_dir(root).join(PLUGIN_NAME),
    ]
    .iter()
    .filter_map(|path| fs::metadata(path).ok())
    .map(|meta| meta.len())
    .sum();
    (bytes / 1024) as u32
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: Payload = Payload {
        exe: b"exe-bytes",
        plugin: b"jar-bytes",
    };

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("monolith-setup-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn install_writes_expected_layout() {
        let base = scratch("install");
        let root = base.join("Programs").join("Monolith");
        let setup = base.join("setup.exe");
        fs::write(&setup, b"setup-bytes").unwrap();

        install_files(&root, &PAYLOAD).unwrap();
        write_uninstaller(&root, &setup).unwrap();

        assert_eq!(fs::read(root.join("Monolith.exe")).unwrap(), b"exe-bytes");
        assert_eq!(
            fs::read(root.join("assets/plugins/MonolithAnalytics.jar")).unwrap(),
            b"jar-bytes"
        );
        assert_eq!(
            fs::read(root.join("Uninstall Monolith.exe")).unwrap(),
            b"setup-bytes"
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn reinstall_replaces_files_and_keeps_user_files() {
        let base = scratch("update");
        let root = base.join("Monolith");
        install_files(&root, &PAYLOAD).unwrap();
        fs::write(root.join("keep.txt"), b"mine").unwrap();
        let newer = Payload {
            exe: b"newer",
            plugin: b"jar-bytes",
        };

        install_files(&root, &newer).unwrap();

        assert_eq!(fs::read(root.join("Monolith.exe")).unwrap(), b"newer");
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"mine");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn empty_payload_leaves_install_untouched() {
        let base = scratch("empty");
        let root = base.join("Monolith");
        install_files(&root, &PAYLOAD).unwrap();
        let empty = Payload {
            exe: b"",
            plugin: b"",
        };

        assert!(install_files(&root, &empty).is_err());

        assert_eq!(fs::read(root.join("Monolith.exe")).unwrap(), b"exe-bytes");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn uninstall_removes_program_files_only() {
        let base = scratch("uninstall");
        let root = base.join("Monolith");
        let setup = base.join("setup.exe");
        fs::write(&setup, b"setup").unwrap();
        install_files(&root, &PAYLOAD).unwrap();
        write_uninstaller(&root, &setup).unwrap();

        remove_program_files(&root).unwrap();
        remove_program_files(&root).unwrap();

        assert!(!root.join("Monolith.exe").exists());
        assert!(!root.join("assets").exists());
        assert!(root.join("Uninstall Monolith.exe").exists());
        let _ = fs::remove_dir_all(base);
    }
}
