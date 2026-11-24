use std::path::{Path, PathBuf};
use std::time::SystemTime;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::server_runner::re;

const LOG_TAIL_LINES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashCause {
    pub cause: String,
    pub fix: String,
}

fn found(cause: impl Into<String>, fix: impl Into<String>) -> Option<CrashCause> {
    Some(CrashCause {
        cause: cause.into(),
        fix: fix.into(),
    })
}

static CLASS_VERSION: Lazy<Regex> = Lazy::new(|| re(r"class file version (\d+)"));
static FABRIC_MISSING: Lazy<Regex> = Lazy::new(|| {
    re(
        r"requires (?:any version|version .+?) of (?:mod )?'?([^',(\n]+?)'?(?: \([^)\n]*\))?, which is missing",
    )
});
static FORGE_MISSING: Lazy<Regex> = Lazy::new(|| re(r"Mod ID: '([^']+)', Requested by: '([^']+)'"));
static SUSPECTED_MOD: Lazy<Regex> = Lazy::new(|| re(r"Suspected Mods?:\s*([^\n(]+?)\s*\("));
static MOD_FILE: Lazy<Regex> = Lazy::new(|| re(r"Mod File: (?:[^\n]*[/\\])?([^/\\\n]+\.jar)"));
static PLUGIN_ENABLE: Lazy<Regex> = Lazy::new(|| re(r"Error occurred while enabling (\S+)"));
static PLUGIN_LOAD: Lazy<Regex> =
    Lazy::new(|| re(r"Could not load '(?:[^'\n]*[/\\])?([^'/\\\n]+)'"));
static WORLD: Lazy<Regex> = Lazy::new(|| {
    re(
        r"(?i)(?:exception|error|failed|corrupt)[^\n]*level\.dat|level\.dat[^\n]*(?:corrupt|exception)|ReportedNbtException",
    )
});
static TICKING: Lazy<Regex> = Lazy::new(|| re(r"Ticking (block entity|entity)"));
static LOCATION: Lazy<Regex> =
    Lazy::new(|| re(r"(?:Entity's Exact location|Block location): ([^\n]+)"));

pub fn analyse(log: &str) -> Option<CrashCause> {
    let log = log.replace('\r', "");
    let log = log.as_str();
    if log.contains("You need to agree to the EULA") {
        return found(
            "The Minecraft EULA has not been accepted.",
            "Accept the EULA on the Overview page, then start the server again.",
        );
    }
    if log.contains("FAILED TO BIND TO PORT") || log.contains("Address already in use") {
        return found(
            "The server port is already in use by another program.",
            "Stop the other server or program using the port, or change server-port in the server's properties.",
        );
    }
    if log.contains("UnsupportedClassVersionError")
        || log.contains("compiled by a more recent version of the Java Runtime")
    {
        let java = CLASS_VERSION
            .captures(log)
            .and_then(|c| c[1].parse::<u32>().ok())
            .filter(|v| *v > 44)
            .map(|v| v - 44);
        return match java {
            Some(java) => found(
                format!("This server needs Java {java}, but it was started with an older Java."),
                format!("Pick Java {java} or newer in the server's settings, then start it again."),
            ),
            None => found(
                "This server needs a newer Java version than the one it was started with.",
                "Pick a newer Java version in the server's settings, then start it again.",
            ),
        };
    }
    if log.contains("java.lang.OutOfMemoryError") {
        return found(
            "The server ran out of memory.",
            "Give the server more RAM in its settings, or remove heavy mods and plugins.",
        );
    }
    if log.contains("Missing or unsupported mandatory dependencies") {
        if let Some(c) = FORGE_MISSING.captures(log) {
            return found(
                format!(
                    "The mod {} needs {}, which is missing or the wrong version.",
                    &c[2], &c[1]
                ),
                format!(
                    "Install a version of {} that matches your Minecraft version, or remove {}.",
                    &c[1], &c[2]
                ),
            );
        }
        return found(
            "A mod is missing one of its dependencies.",
            "Check the log for the missing mod and install it, or remove the mod that needs it.",
        );
    }
    if let Some(c) = FABRIC_MISSING.captures(log) {
        let dependency = c[1].trim();
        return found(
            format!("A mod needs {dependency}, which is not installed."),
            format!(
                "Install {dependency} for your Minecraft version, or remove the mod that needs it."
            ),
        );
    }
    if let Some(c) = TICKING.captures(log) {
        let place = LOCATION
            .captures(log)
            .map(|l| format!(" at {}", l[1].trim()))
            .unwrap_or_default();
        return found(
            format!("A broken {} in the world{place} crashed the server.", &c[1]),
            "Restore the world from a backup, or remove that entity with a world editor.",
        );
    }
    if let Some(name) = SUSPECTED_MOD
        .captures(log)
        .or_else(|| MOD_FILE.captures(log))
        .map(|c| c[1].trim().to_string())
    {
        return found(
            format!("The mod {name} caused the crash."),
            format!("Update {name} to a version for this Minecraft version, or remove it."),
        );
    }
    if let Some(name) = PLUGIN_ENABLE
        .captures(log)
        .or_else(|| PLUGIN_LOAD.captures(log))
        .map(|c| c[1].to_string())
    {
        return found(
            format!("The plugin {name} failed to load."),
            format!(
                "Update {name} to a version for this server, or remove it from the plugins folder."
            ),
        );
    }
    if WORLD.is_match(log) {
        return found(
            "The world data (level.dat) looks corrupted.",
            "Restore the world from a backup, or replace level.dat with level.dat_old in the world folder.",
        );
    }
    if log.contains("Exception in server tick loop") {
        return found(
            "The server hit an error while running and stopped.",
            "Open the newest crash report in Logs to see which mod or plugin is in the stack trace, then update or remove it.",
        );
    }
    None
}

pub async fn analyse_server(server_path: PathBuf, console: Vec<String>) -> Option<CrashCause> {
    tokio::task::spawn_blocking(move || analyse(&crash_text(&server_path, console)))
        .await
        .ok()
        .flatten()
}

fn crash_text(root: &Path, console: Vec<String>) -> String {
    let latest = root.join("logs").join("latest.log");
    let log = std::fs::read(&latest)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_else(|_| console.join("\n"));
    let lines: Vec<&str> = log.lines().collect();
    let mut text = lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n");
    let started = std::fs::metadata(&latest)
        .and_then(|m| m.created())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let report = std::fs::read_dir(root.join("crash-reports"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(modified, path)| {
            *modified >= started && path.extension().is_some_and(|e| e == "txt")
        })
        .max();
    if let Some((_, path)) = report {
        if let Ok(bytes) = std::fs::read(path) {
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(log: &str) -> String {
        analyse(log).map(|c| c.cause).unwrap_or_default()
    }

    #[test]
    fn memory_and_java() {
        assert!(
            cause("Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space")
                .contains("memory")
        );
        let java = "java.lang.UnsupportedClassVersionError: net/minecraft/server/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0), this version of the Java Runtime only recognizes class file versions up to 61.0";
        assert!(cause(java).contains("Java 21"));
        assert!(cause("UnsupportedClassVersionError (class file version 52.0)").contains("Java 8"));
        assert!(cause("UnsupportedClassVersionError class file version 69.0").contains("Java 25"));
    }

    #[test]
    fn port_and_eula() {
        assert!(cause("[WARN]: **** FAILED TO BIND TO PORT!").contains("port"));
        assert!(
            cause("[INFO]: You need to agree to the EULA in order to run the server.")
                .contains("EULA")
        );
    }

    #[test]
    fn mods_and_plugins() {
        assert_eq!(
            cause("Suspected Mods: \n\tSodium (sodium), Version: 0.5.8\n"),
            "The mod Sodium caused the crash."
        );
        assert!(
            cause("Suspected Mods: Unknown\nMod File: C:\\server\\mods\\create-1.20.jar")
                .contains("create-1.20.jar")
        );
        assert!(cause(
            "[ERROR]: Error occurred while enabling Essentials v2.20 (Is it up to date?)"
        )
        .contains("Essentials"));
        assert!(
            cause("[ERROR]: Could not load 'plugins/Broken.jar' in folder 'plugins'")
                .contains("Broken.jar")
        );
    }

    #[test]
    fn missing_dependencies() {
        assert!(cause("- Mod 'Sodium Extra' (sodium-extra) 0.4.18 requires any version of sodium, which is missing!").contains("needs sodium"));
        assert!(cause(
            "requires version 0.5.0 or later of mod 'Fabric API' (fabric-api), which is missing!"
        )
        .contains("needs Fabric API"));
        let forge = "Missing or unsupported mandatory dependencies:\n\tMod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'";
        assert_eq!(
            cause(forge),
            "The mod mowziesmobs needs geckolib, which is missing or the wrong version."
        );
    }

    #[test]
    fn world_and_ticking() {
        assert!(cause("[ERROR]: Exception reading .\\world\\level.dat").contains("level.dat"));
        let ticking = "Description: Ticking entity\n\tEntity's Exact location: 10.5, 64.0, -3.2\nException in server tick loop";
        assert_eq!(
            cause(ticking),
            "A broken entity in the world at 10.5, 64.0, -3.2 crashed the server."
        );
        assert!(cause("Description: Ticking block entity").contains("block entity"));
        assert!(cause(
            "[ERROR]: Encountered an unexpected exception\nException in server tick loop"
        )
        .contains("error while running"));
        assert!(analyse("[INFO]: Done (3.2s)!").is_none());
    }
}
