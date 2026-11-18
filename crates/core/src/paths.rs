use std::path::PathBuf;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard, RwLock};

#[cfg(test)]
static USER_DATA_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);
#[cfg(test)]
static USER_DATA_TEST_LOCK: Mutex<()> = Mutex::new(());

pub const APP_NAME: &str = "Monolith";

pub fn user_data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = USER_DATA_OVERRIDE.read().ok().and_then(|o| o.clone()) {
        return dir;
    }
    if let Some(dir) = std::env::var_os("MONOLITH_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
}

pub fn servers_dir() -> PathBuf {
    user_data_dir().join("servers")
}

pub fn runtimes_dir() -> PathBuf {
    user_data_dir().join("runtimes")
}

pub fn ngrok_dir() -> PathBuf {
    user_data_dir().join("ngrok")
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

pub fn resource_path(rel: &str) -> PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets").join(rel)));
    match bundled {
        Some(path) if path.exists() => path,
        _ if cfg!(debug_assertions) => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join(rel),
        Some(path) => path,
        None => PathBuf::from("assets").join(rel),
    }
}

#[cfg(test)]
pub struct UserDataOverride {
    _guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for UserDataOverride {
    fn drop(&mut self) {
        if let Ok(mut slot) = USER_DATA_OVERRIDE.write() {
            *slot = None;
        }
    }
}

#[cfg(test)]
pub fn override_user_data_dir(dir: PathBuf) -> UserDataOverride {
    let guard = USER_DATA_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Ok(mut slot) = USER_DATA_OVERRIDE.write() {
        *slot = Some(dir);
    }
    UserDataOverride { _guard: guard }
}
