use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use mslnk::ShellLink;
use sysinfo::{ProcessesToUpdate, System};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use crate::install::{app_path, installed_size_kb, uninstaller_path, EXE_NAME};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Monolith";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SHORTCUT: &str = "Monolith.lnk";

pub fn default_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public\AppData\Local"))
        .join("Programs")
        .join("Monolith")
}

pub fn installed_root() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let location: String = key.get_value("InstallLocation").ok()?;
    Some(PathBuf::from(location))
}

pub fn user_data_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Monolith"))
}

fn start_menu_shortcut() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| {
        dir.join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(SHORTCUT)
    })
}

fn desktop_shortcut() -> Option<PathBuf> {
    dirs::desktop_dir().map(|dir| dir.join(SHORTCUT))
}

fn create_shortcut(target: &Path, link: &Path) -> io::Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut shell = ShellLink::new(target).map_err(io::Error::other)?;
    shell.set_working_dir(target.parent().map(|dir| dir.display().to_string()));
    shell.set_icon_location(Some(target.display().to_string()));
    shell.create_lnk(link).map_err(io::Error::other)
}

pub fn create_shortcuts(root: &Path, desktop: bool) -> io::Result<()> {
    let target = app_path(root);
    if let Some(link) = start_menu_shortcut() {
        create_shortcut(&target, &link)?;
    }
    if desktop {
        if let Some(link) = desktop_shortcut() {
            create_shortcut(&target, &link)?;
        }
    }
    Ok(())
}

pub fn remove_shortcuts() {
    for link in [start_menu_shortcut(), desktop_shortcut()]
        .into_iter()
        .flatten()
    {
        let _ = fs::remove_file(link);
    }
}

pub fn register(root: &Path) -> io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
    let command = format!("\"{}\" --uninstall", uninstaller_path(root).display());
    key.set_value("DisplayName", &"Monolith")?;
    key.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
    key.set_value("Publisher", &"Monolith")?;
    key.set_value("DisplayIcon", &app_path(root).display().to_string())?;
    key.set_value("InstallLocation", &root.display().to_string())?;
    key.set_value("UninstallString", &command)?;
    key.set_value("QuietUninstallString", &format!("{command} --quiet"))?;
    key.set_value("EstimatedSize", &installed_size_kb(root))?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    Ok(())
}

pub fn unregister() {
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
}

pub fn app_running() -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let running = system
        .processes_by_exact_name(OsStr::new(EXE_NAME))
        .next()
        .is_some();
    running
}

pub fn close_app() {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    for process in system.processes_by_exact_name(OsStr::new(EXE_NAME)) {
        process.kill();
    }
}

pub fn wait_for_app_exit(limit: Duration) -> bool {
    let start = Instant::now();
    while app_running() {
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    true
}

pub fn launch(root: &Path) {
    let _ = Command::new(app_path(root)).current_dir(root).spawn();
}

pub fn delete_user_data() -> io::Result<()> {
    match user_data_dir() {
        Some(dir) if dir.exists() => fs::remove_dir_all(dir),
        _ => Ok(()),
    }
}

pub fn schedule_self_delete(root: &Path) {
    let _ = Command::new("cmd")
        .args([
            "/C",
            "ping -n 3 127.0.0.1 >nul & del /f /q \"%MONOLITH_SETUP%\" & rmdir \"%MONOLITH_ROOT%\"",
        ])
        .env("MONOLITH_SETUP", uninstaller_path(root))
        .env("MONOLITH_ROOT", root)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
