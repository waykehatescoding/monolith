use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, Notify};
use tokio::task::JoinHandle;

use crate::events::{emit, Event};
use crate::java_manager::{ensure_java_installed, get_java_home, get_required_java_version};
use crate::ngrok_manager::{is_ngrok_authtoken_configured, is_ngrok_enabled, start_ngrok};
use crate::port_check::check_port;
use crate::restart_policy::next_delay;
use crate::server_framework::install_analytics_plugin;
use crate::server_props::read_properties;
use crate::server_store::{
    get_server, load_server_list, update_server_settings, update_server_status,
};
use crate::shutdown_policy::{
    build_graceful_stop_commands, SAVE_ALL_FLUSH_COMMAND, WORLD_SAVE_INTERVAL_MS,
};
use crate::types::{
    ConsoleLine, ConsoleLineType, ServerRuntimeStatus, ServerSettingsUpdate, ServerStats,
    ServerStatusUpdate, SuccessResult,
};

const MAX_LOG_LINES: usize = 500;
const STATS_POLL_INTERVAL_MS: u64 = 10_000;
const PROCESS_MEMORY_POLL_INTERVAL_MS: u64 = 30_000;
const IN_SERVER_STATS_FRAMEWORKS: [&str; 2] = ["Paper", "Purpur"];

struct Running {
    tx: mpsc::UnboundedSender<String>,
    kill: Arc<Notify>,
    closed: watch::Receiver<bool>,
    pid: Option<u32>,
    save_task: Option<JoinHandle<()>>,
}

struct StatsState {
    stats: ServerStats,
    ready: bool,
    last_mem_at: u64,
    internal_poll_until: u64,
    user_bypass_until: u64,
    disable_in_server: bool,
    task: Option<JoinHandle<()>>,
}

static RUNNING: Lazy<Mutex<HashMap<String, Running>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static STATS: Lazy<Mutex<HashMap<String, StatsState>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static LOGS: Lazy<Mutex<HashMap<String, Vec<ConsoleLine>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static STOPPING: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static CRASHES: Lazy<Mutex<HashMap<String, Vec<u64>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static RESTARTING: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub(crate) fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()))
}

static ANSI: Lazy<Regex> = Lazy::new(|| re(r"\x1b\[[0-9;]*[A-Za-z]"));
static UNKNOWN_COMMAND: Lazy<Regex> =
    Lazy::new(|| re(r#"Unknown command\. Type "/help" for help\."#));
static STATS_FILTERS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"^> tps$",
        r"^\[.*\]: Showing TPS for",
        r"^\[.*\]: TPS from last",
        r"^\[.*\]: .*\d+\.\d+.*\d+\.\d+.*\d+\.\d+",
        r#"^\[.*\]: Unknown command\. Type "/help" for help\."#,
        r"^\[.*\]: \[\?\]",
        r"(?i)^\[.*\]: \[\?\].*TPS from last",
        r"(?i)^\[.*\]: \[\?\].*Tick durations",
        r"(?i)^\[.*\]: \[\?\].*CPU usage",
        r"(?i)^> memory$",
        r"(?i)^\[.*\]: Memory usage",
        r"(?i)^\[.*\]: Free memory:",
        r"(?i)^\[.*\]: Max memory:",
        r"(?i)^\[.*\]: Allocated memory:",
        r"(?i)^\[.*\]: Current Memory Usage:",
    ]
    .iter()
    .map(|p| re(p))
    .collect()
});
static DONE: Lazy<Regex> = Lazy::new(|| re(r"Done \(\d+\.\d+s\)"));
static RESTART_HINTS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        re(r"(?i)Server is restarting"),
        re(r"(?i)\[RestartCommand\]"),
        re(r"(?i)Startup script.*does not exist"),
    ]
});
static CHAT: Lazy<Regex> = Lazy::new(|| re(r"\]: (\[Not Secure\] )?<[^>]+>"));
static JOIN: Lazy<Regex> = Lazy::new(|| re(r"\]: (\S+) joined the game"));
static LEAVE: Lazy<Regex> = Lazy::new(|| re(r"\]: (\S+) left the game"));
static TPS: Lazy<Regex> = Lazy::new(|| re(r"TPS from last .+?:\s+\*?([\d.,]+)"));
static MAX_PLAYERS: Lazy<Regex> = Lazy::new(|| re(r"(?i)of a max of (\d+) players"));
static MEM_CURRENT: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)Current Memory Usage:\s*([\d,]+)\s*MB\s*/\s*([\d,]+)\s*MB"));
static MEM_USAGE: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)Memory usage:\s*([\d,]+)\s*MB(?:\s*/\s*([\d,]+)\s*MB)?"));
static MEM_FREE: Lazy<Regex> = Lazy::new(|| {
    re(r"(?i)Free memory:\s*([\d,]+)\s*MB\s*/\s*Total:\s*([\d,]+)\s*MB\s*/\s*Max:\s*([\d,]+)\s*MB")
});
static MEM_ALLOC: Lazy<Regex> = Lazy::new(|| re(r"(?i)Allocated memory:\s*([\d,]+)\s*MB"));
static MEM_MAX: Lazy<Regex> = Lazy::new(|| re(r"(?i)Max memory:\s*([\d,]+)\s*MB"));
static STATS_COMMAND: Lazy<Regex> = Lazy::new(|| re(r"(?i)^\s*(tps|memory)\s*$"));
static RESTART_COMMAND: Lazy<Regex> = Lazy::new(|| re(r"(?i)^\s*restart\s*$"));

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn strip_ansi(text: &str) -> String {
    ANSI.replace_all(text, "").into_owned()
}

fn parse_int(text: &str) -> f64 {
    text.replace(',', "").parse::<f64>().unwrap_or(0.0)
}

fn parse_float_prefix(text: &str) -> Option<f64> {
    let mut end = 0;
    let mut seen_dot = false;
    for (i, c) in text.char_indices() {
        if c.is_ascii_digit() {
            end = i + 1;
        } else if c == '.' && !seen_dot {
            seen_dot = true;
            end = i + 1;
        } else {
            break;
        }
    }
    text[..end].parse::<f64>().ok()
}

pub fn build_java_args(ram_mb: u32, jar_file: &str) -> Vec<String> {
    let safe_ram = ram_mb.max(256);
    let initial_ram = 256.max(safe_ram.min((safe_ram as f64 * 0.4).floor() as u32));
    let mut args = vec![format!("-Xms{initial_ram}M"), format!("-Xmx{safe_ram}M")];
    args.extend(
        [
            "-XX:+DisableExplicitGC",
            "-XX:+ParallelRefProcEnabled",
            "-XX:+PerfDisableSharedMem",
            "-XX:+UnlockExperimentalVMOptions",
            "-XX:+UseG1GC",
            "-XX:G1HeapRegionSize=8M",
            "-XX:G1HeapWastePercent=5",
            "-XX:G1MaxNewSizePercent=40",
            "-XX:G1MixedGCCountTarget=4",
            "-XX:G1MixedGCLiveThresholdPercent=90",
            "-XX:G1NewSizePercent=30",
            "-XX:G1RSetUpdatingPauseTimePercent=5",
            "-XX:G1ReservePercent=20",
            "-XX:InitiatingHeapOccupancyPercent=15",
            "-XX:MaxGCPauseMillis=200",
            "-XX:MaxTenuringThreshold=1",
            "-XX:SurvivorRatio=32",
            "-Dusing.aikars.flags=https://mcflags.emc.gs",
            "-Daikars.new.flags=true",
            "-jar",
        ]
        .map(String::from),
    );
    args.push(jar_file.to_string());
    args.push("nogui".to_string());
    args
}

pub fn update_player_capacity_string(current_players: Option<&str>, max_players: &str) -> String {
    let fallback = || {
        current_players
            .filter(|p| !p.is_empty())
            .unwrap_or("0/20")
            .to_string()
    };
    let max = match max_players.trim().parse::<f64>() {
        Ok(n) if n.fract() == 0.0 && (1.0..1e15).contains(&n) => n as u64,
        _ => return fallback(),
    };
    let online = re(r"^(\d+)\s*/\s*\d+$")
        .captures(current_players.unwrap_or(""))
        .and_then(|c| c[1].parse::<u64>().ok())
        .unwrap_or(0);
    format!("{online}/{max}")
}

fn parse_port(value: &str) -> u16 {
    let t = value.trim_start();
    let t = t.strip_prefix('+').unwrap_or(t);
    let end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    t[..end].parse::<u16>().unwrap_or(25565)
}

pub fn is_running(server_id: &str) -> bool {
    RUNNING
        .lock()
        .map(|r| r.contains_key(server_id))
        .unwrap_or(false)
}

pub fn get_running_server_count() -> usize {
    RUNNING.lock().map(|r| r.len()).unwrap_or(0)
}

fn write_server_command(server_id: &str, command: &str) -> bool {
    RUNNING
        .lock()
        .ok()
        .and_then(|r| {
            r.get(server_id)
                .map(|s| s.tx.send(command.to_string()).is_ok())
        })
        .unwrap_or(false)
}

fn send_console_line(server_id: &str, text: &str, kind: ConsoleLineType) {
    let line = ConsoleLine {
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        text: text.to_string(),
        kind,
    };
    push_console_line(server_id, line);
}

pub fn push_console_line(server_id: &str, line: ConsoleLine) {
    if let Ok(mut logs) = LOGS.lock() {
        let entry = logs.entry(server_id.to_string()).or_default();
        entry.push(line.clone());
        if entry.len() > MAX_LOG_LINES {
            entry.remove(0);
        }
    }
    emit(Event::ConsoleOutput {
        server_id: server_id.to_string(),
        line,
    });
}

fn send_status_update(server_id: &str, status: ServerRuntimeStatus, players: Option<&str>) {
    emit(Event::ServerStatus(ServerStatusUpdate {
        server_id: server_id.to_string(),
        status,
        players: players.map(String::from),
    }));
}

fn send_stats(stats: &ServerStats) {
    emit(Event::ServerStats(stats.clone()));
}

async fn set_offline(server_id: &str, notify: bool) {
    update_server_status(server_id, ServerRuntimeStatus::Offline, Some("0/20")).await;
    if notify {
        send_status_update(server_id, ServerRuntimeStatus::Offline, Some("0/20"));
    }
}

fn can_use_in_server_stats(server_id: &str, framework: &str) -> bool {
    if !IN_SERVER_STATS_FRAMEWORKS.contains(&framework) {
        return false;
    }
    STATS
        .lock()
        .map(|s| {
            s.get(server_id)
                .map(|st| !st.disable_in_server)
                .unwrap_or(true)
        })
        .unwrap_or(true)
}

fn is_stats_line(server_id: &str, line: &str) -> bool {
    let now = now_ms();
    let (bypass, poll) = match STATS.lock() {
        Ok(s) => match s.get(server_id) {
            Some(st) => (st.user_bypass_until, st.internal_poll_until),
            None => (0, 0),
        },
        Err(_) => (0, 0),
    };
    if now < bypass || now > poll {
        return false;
    }
    STATS_FILTERS.iter().any(|p| p.is_match(line))
}

fn parse_stats_from_line(server_id: &str, line: &str) {
    let normalized = strip_ansi(line);
    let mut became_online: Option<String> = None;

    {
        let Ok(mut map) = STATS.lock() else { return };
        let Some(state) = map.get_mut(server_id) else {
            return;
        };

        if DONE.is_match(&normalized) {
            let was_ready = state.ready;
            state.ready = true;
            if !was_ready {
                became_online = Some(format!(
                    "{}/{}",
                    state.stats.player_count, state.stats.max_players
                ));
            }
        }

        if !CHAT.is_match(&normalized) && RESTART_HINTS.iter().any(|p| p.is_match(&normalized)) {
            if let Ok(mut set) = RESTARTING.lock() {
                set.insert(server_id.to_string());
            }
        }

        let stats = &mut state.stats;
        if let Some(c) = JOIN.captures(&normalized) {
            let name = c[1].to_string();
            if !stats.online_players.contains(&name) {
                stats.online_players.push(name);
                stats.player_count = stats.online_players.len() as u32;
            }
        }
        if let Some(c) = LEAVE.captures(&normalized) {
            let name = c[1].to_string();
            stats.online_players.retain(|p| p != &name);
            stats.player_count = stats.online_players.len() as u32;
        }
        if let Some(c) = TPS.captures(&normalized) {
            let value = c[1].replacen(',', ".", 1);
            if let Some(tps) = parse_float_prefix(&value) {
                stats.tps = Some(tps);
            }
        }
        if let Some(c) = MAX_PLAYERS.captures(&normalized) {
            if let Ok(n) = c[1].parse::<u32>() {
                stats.max_players = n;
            }
        }

        let mut mem_touched = false;
        if let Some(c) = MEM_CURRENT.captures(&normalized) {
            stats.memory_used_mb = Some(parse_int(&c[1]));
            stats.memory_max_mb = Some(parse_int(&c[2]));
            mem_touched = true;
        }
        if let Some(c) = MEM_USAGE.captures(&normalized) {
            stats.memory_used_mb = Some(parse_int(&c[1]));
            if let Some(m) = c.get(2) {
                stats.memory_max_mb = Some(parse_int(m.as_str()));
            }
            mem_touched = true;
        }
        if let Some(c) = MEM_FREE.captures(&normalized) {
            let free = parse_int(&c[1]);
            let total = parse_int(&c[2]);
            stats.memory_used_mb = Some((total - free).max(0.0));
            stats.memory_max_mb = Some(parse_int(&c[3]));
            mem_touched = true;
        }
        if mem_touched {
            state.last_mem_at = now_ms();
        }
        let stats = &mut state.stats;
        if let Some(c) = MEM_ALLOC.captures(&normalized) {
            stats.memory_used_mb = Some(parse_int(&c[1]));
        }
        if let Some(c) = MEM_MAX.captures(&normalized) {
            stats.memory_max_mb = Some(parse_int(&c[1]));
        }
    }

    if let Some(players) = became_online {
        let id = server_id.to_string();
        tokio::spawn(async move {
            update_server_status(&id, ServerRuntimeStatus::Online, Some(&players)).await;
            send_status_update(&id, ServerRuntimeStatus::Online, Some(&players));
        });
    }
}

async fn get_process_memory_mb(pid: u32) -> Option<f64> {
    if pid == 0 {
        return None;
    }
    let output = if cfg!(windows) {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        tokio::time::timeout(Duration::from_secs(5), cmd.output()).await
    } else {
        let mut cmd = Command::new("ps");
        cmd.args(["-o", "rss=", "-p", &pid.to_string()]);
        tokio::time::timeout(Duration::from_secs(5), cmd.output()).await
    };
    let output = output.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let kb = if cfg!(windows) {
        let m = re(r#""([0-9,\.]+)\s*K""#)
            .captures(&stdout)?
            .get(1)?
            .as_str()
            .replace(['.', ','], "");
        m.parse::<f64>().ok()?
    } else {
        stdout.trim().parse::<f64>().ok()?
    };
    Some((kb / 1024.0).round())
}

fn running_pid(server_id: &str) -> Option<u32> {
    RUNNING
        .lock()
        .ok()
        .and_then(|r| r.get(server_id).and_then(|s| s.pid))
}

fn start_stats_polling(server_id: &str, ram_mb: u32, framework: &str) {
    let initial = ServerStats {
        server_id: server_id.to_string(),
        tps: None,
        memory_used_mb: None,
        memory_max_mb: Some(ram_mb as f64),
        online_players: Vec::new(),
        player_count: 0,
        max_players: 20,
    };
    if let Ok(mut map) = STATS.lock() {
        map.insert(
            server_id.to_string(),
            StatsState {
                stats: initial.clone(),
                ready: false,
                last_mem_at: 0,
                internal_poll_until: 0,
                user_bypass_until: 0,
                disable_in_server: false,
                task: None,
            },
        );
    }

    let id = server_id.to_string();
    let fw = framework.to_string();
    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(STATS_POLL_INTERVAL_MS)).await;
            if !STATS.lock().map(|s| s.contains_key(&id)).unwrap_or(false) {
                return;
            }

            if let Some(pid) = running_pid(&id) {
                let last = STATS
                    .lock()
                    .ok()
                    .and_then(|s| s.get(&id).map(|st| st.last_mem_at))
                    .unwrap_or(0);
                if now_ms().saturating_sub(last) > PROCESS_MEMORY_POLL_INTERVAL_MS {
                    let mem = get_process_memory_mb(pid).await;
                    if let Ok(mut map) = STATS.lock() {
                        if let Some(st) = map.get_mut(&id) {
                            st.last_mem_at = now_ms();
                            if let Some(m) = mem {
                                st.stats.memory_used_mb = Some(m);
                            }
                        }
                    }
                }
            }

            let ready = STATS
                .lock()
                .ok()
                .and_then(|s| s.get(&id).map(|st| st.ready))
                .unwrap_or(false);
            if ready && can_use_in_server_stats(&id, &fw) {
                if let Ok(mut map) = STATS.lock() {
                    if let Some(st) = map.get_mut(&id) {
                        st.internal_poll_until = now_ms() + 12_000;
                    }
                }
                write_server_command(&id, "tps");
            }

            let snapshot = STATS
                .lock()
                .ok()
                .and_then(|s| s.get(&id).map(|st| st.stats.clone()));
            if let Some(stats) = snapshot {
                send_stats(&stats);
            }
        }
    });
    if let Ok(mut map) = STATS.lock() {
        if let Some(st) = map.get_mut(server_id) {
            st.task = Some(handle);
        } else {
            handle.abort();
        }
    }

    let pid = running_pid(server_id);
    if pid.is_some() && !can_use_in_server_stats(server_id, framework) {
        let id = server_id.to_string();
        let pid = pid.unwrap_or(0);
        tokio::spawn(async move {
            let mem = get_process_memory_mb(pid).await;
            let snapshot = STATS.lock().ok().and_then(|mut s| {
                s.get_mut(&id).map(|st| {
                    if let Some(m) = mem {
                        st.stats.memory_used_mb = Some(m);
                    }
                    st.stats.clone()
                })
            });
            if let Some(stats) = snapshot {
                send_stats(&stats);
            }
        });
    } else {
        send_stats(&initial);
    }
}

fn stop_stats_polling(server_id: &str) {
    let removed = STATS.lock().ok().and_then(|mut s| s.remove(server_id));
    if let Some(task) = removed.and_then(|s| s.task) {
        task.abort();
    }
}

fn start_world_save_timer(server_id: &str) {
    let id = server_id.to_string();
    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(WORLD_SAVE_INTERVAL_MS)).await;
            if !is_running(&id) {
                continue;
            }
            write_server_command(&id, SAVE_ALL_FLUSH_COMMAND);
        }
    });
    if let Ok(mut map) = RUNNING.lock() {
        if let Some(r) = map.get_mut(server_id) {
            if let Some(old) = r.save_task.replace(handle) {
                old.abort();
            }
        }
    }
}

fn stop_world_save_timer(server_id: &str) {
    if let Ok(mut map) = RUNNING.lock() {
        if let Some(task) = map.get_mut(server_id).and_then(|r| r.save_task.take()) {
            task.abort();
        }
    }
}

fn remove_running(server_id: &str) {
    let removed = RUNNING.lock().ok().and_then(|mut r| r.remove(server_id));
    if let Some(task) = removed.and_then(|r| r.save_task) {
        task.abort();
    }
}

pub async fn refresh_server_statuses() {
    for server in load_server_list().await {
        let active = matches!(
            server.status,
            ServerRuntimeStatus::Starting
                | ServerRuntimeStatus::Online
                | ServerRuntimeStatus::Stopping
        );
        if active && !is_running(&server.id) {
            set_offline(&server.id, true).await;
        }
    }
}

async fn read_lines<R: AsyncRead + Unpin>(reader: R, mut on_line: impl FnMut(String)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if buf.last() != Some(&b'\n') {
                    return;
                }
                buf.pop();
                let line = String::from_utf8_lossy(&buf).into_owned();
                if !line.trim().is_empty() {
                    on_line(line);
                }
            }
        }
    }
}

async fn start_ngrok_for(server_id: &str, server_path: &str) {
    if !(is_ngrok_enabled().await && is_ngrok_authtoken_configured().await) {
        return;
    }
    let properties = read_properties(Path::new(server_path)).await;
    let port = properties
        .iter()
        .find(|p| p.key == "server-port")
        .map(|p| parse_port(&p.value))
        .unwrap_or(25565);
    let _ = start_ngrok(server_id.to_string(), port).await;
}

fn start_boxed(server_id: String) -> Pin<Box<dyn Future<Output = SuccessResult> + Send>> {
    Box::pin(start_server_core(server_id))
}

async fn handle_close(server_id: String, server_path: String, code: Option<i32>) {
    remove_running(&server_id);
    stop_stats_polling(&server_id);

    let was_restarting = RESTARTING
        .lock()
        .map(|mut s| s.remove(&server_id))
        .unwrap_or(false);

    let stopped_by_user = STOPPING
        .lock()
        .map(|mut s| s.remove(&server_id))
        .unwrap_or(false);

    let code_text = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "null".to_string());
    send_console_line(
        &server_id,
        &format!("Server process exited with code {code_text}"),
        ConsoleLineType::System,
    );

    if was_restarting {
        send_console_line(
            &server_id,
            "Detected restart command, restarting server...",
            ConsoleLineType::System,
        );
        tokio::time::sleep(Duration::from_secs(2)).await;

        let result = start_boxed(server_id.clone()).await;
        if !result.success {
            let error = result.error.unwrap_or_default();
            send_console_line(
                &server_id,
                &format!("Failed to restart server: {error}"),
                ConsoleLineType::Stderr,
            );
            set_offline(&server_id, true).await;
            return;
        }
        start_ngrok_for(&server_id, &server_path).await;
        return;
    }

    set_offline(&server_id, true).await;

    if !stopped_by_user && code != Some(0) {
        restart_after_crash(&server_id, &server_path).await;
    }
}

async fn restart_after_crash(server_id: &str, server_path: &str) {
    let enabled = get_server(server_id.to_string())
        .await
        .map(|s| s.auto_restart_enabled())
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let delay = CRASHES.lock().ok().and_then(|mut map| {
        next_delay(
            map.entry(server_id.to_string()).or_default(),
            now_ms() / 1000,
        )
    });
    let Some(delay) = delay else {
        send_console_line(
            server_id,
            "Server crashed 3 times in 10 minutes, auto-restart gave up",
            ConsoleLineType::System,
        );
        return;
    };

    send_console_line(
        server_id,
        &format!(
            "Server stopped unexpectedly, restarting automatically in {}s",
            delay.as_secs()
        ),
        ConsoleLineType::System,
    );
    tokio::time::sleep(delay).await;
    if is_running(server_id) {
        return;
    }

    let result = start_boxed(server_id.to_string()).await;
    if result.success {
        start_ngrok_for(server_id, server_path).await;
    } else {
        let error = result.error.unwrap_or_default();
        send_console_line(
            server_id,
            &format!("Automatic restart failed: {error}"),
            ConsoleLineType::Stderr,
        );
    }
}

async fn start_server_core(server_id: String) -> SuccessResult {
    if is_running(&server_id) {
        return SuccessResult::err("Server is already running");
    }

    let Some(server) = get_server(server_id.clone()).await else {
        return SuccessResult::err("Server not found");
    };

    if let Err(conflict) = check_port(&server).await {
        send_console_line(&server_id, &conflict, ConsoleLineType::Stderr);
        return SuccessResult::err(conflict);
    }

    let jar_file = server
        .jar_file
        .clone()
        .filter(|j| !j.is_empty())
        .unwrap_or_else(|| "server.jar".to_string());
    let args = build_java_args(server.ram_mb, &jar_file);

    let java_executable = match server.java_path.clone().filter(|p| !p.is_empty()) {
        Some(path) => {
            send_console_line(
                &server_id,
                &format!("Using custom Java path: {path}"),
                ConsoleLineType::System,
            );
            path
        }
        None => {
            let required = get_required_java_version(&server.version);
            send_console_line(
                &server_id,
                &format!("Checking Java {required} runtime..."),
                ConsoleLineType::System,
            );
            match ensure_java_installed(required, None).await {
                Ok(path) => {
                    let path = path.to_string_lossy().into_owned();
                    send_console_line(
                        &server_id,
                        &format!("Using Java runtime: {path}"),
                        ConsoleLineType::System,
                    );
                    path
                }
                Err(msg) => {
                    send_console_line(
                        &server_id,
                        &format!("Failed to setup Java: {msg}"),
                        ConsoleLineType::Stderr,
                    );
                    return SuccessResult::err(format!("Java setup failed: {msg}"));
                }
            }
        }
    };

    if server.analytics_enabled.unwrap_or(false)
        && install_analytics_plugin(Path::new(&server.server_path)).await
    {
        send_console_line(
            &server_id,
            "MonolithAnalytics plugin verified.",
            ConsoleLineType::System,
        );
    }

    let mut command = Command::new(&java_executable);
    command
        .args(&args)
        .current_dir(&server.server_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if java_executable != "java" {
        command.env("JAVA_HOME", get_java_home(Path::new(&java_executable)));
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            send_console_line(
                &server_id,
                &format!("Error: {err}"),
                ConsoleLineType::Stderr,
            );
            set_offline(&server_id, true).await;
            return SuccessResult::ok();
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let kill = Arc::new(Notify::new());
    let (closed_tx, closed_rx) = watch::channel(false);
    let pid = child.id();

    if let Ok(mut map) = RUNNING.lock() {
        map.insert(
            server_id.clone(),
            Running {
                tx,
                kill: kill.clone(),
                closed: closed_rx,
                pid,
                save_task: None,
            },
        );
    }
    start_world_save_timer(&server_id);

    send_console_line(&server_id, "Starting server...", ConsoleLineType::System);
    update_server_status(&server_id, ServerRuntimeStatus::Starting, None).await;
    send_status_update(&server_id, ServerRuntimeStatus::Starting, None);
    start_stats_polling(&server_id, server.ram_mb, &server.framework);

    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                if stdin
                    .write_all(format!("{command}\n").as_bytes())
                    .await
                    .is_err()
                    || stdin.flush().await.is_err()
                {
                    return;
                }
            }
        });
    }

    let stdout_task = child.stdout.take().map(|out| {
        let id = server_id.clone();
        tokio::spawn(read_lines(out, move |line| {
            let cleaned = strip_ansi(&line);
            parse_stats_from_line(&id, &cleaned);
            let stats_line = is_stats_line(&id, &cleaned);
            if stats_line && UNKNOWN_COMMAND.is_match(&cleaned) {
                if let Ok(mut map) = STATS.lock() {
                    if let Some(st) = map.get_mut(&id) {
                        st.disable_in_server = true;
                    }
                }
            }
            if !stats_line {
                send_console_line(&id, &line, ConsoleLineType::Stdout);
            }
        }))
    });
    let stderr_task = child.stderr.take().map(|err| {
        let id = server_id.clone();
        tokio::spawn(read_lines(err, move |line| {
            send_console_line(&id, &line, ConsoleLineType::Stderr)
        }))
    });

    let id = server_id.clone();
    let server_path = server.server_path.clone();
    tokio::spawn(async move {
        let status = tokio::select! {
            result = child.wait() => result,
            _ = kill.notified() => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        for task in [stdout_task, stderr_task].into_iter().flatten() {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        let _ = closed_tx.send(true);
        match status {
            Ok(status) => handle_close(id, server_path, status.code()).await,
            Err(err) => {
                remove_running(&id);
                stop_stats_polling(&id);
                send_console_line(&id, &format!("Error: {err}"), ConsoleLineType::Stderr);
                set_offline(&id, true).await;
            }
        }
    });

    SuccessResult::ok()
}

async fn clear_ngrok_url(server_id: &str) {
    let _ = update_server_settings(
        server_id.to_string(),
        ServerSettingsUpdate {
            ngrok_url: Some(String::new()),
            ..Default::default()
        },
    )
    .await;
}

async fn stop_server_inner(server_id: &str, notify: bool) -> SuccessResult {
    if let Ok(mut set) = RESTARTING.lock() {
        set.remove(server_id);
    }
    let handles = RUNNING
        .lock()
        .ok()
        .and_then(|r| r.get(server_id).map(|s| (s.kill.clone(), s.closed.clone())));
    let Some((kill, mut closed)) = handles else {
        set_offline(server_id, notify).await;
        return SuccessResult::ok();
    };

    if let Ok(mut set) = STOPPING.lock() {
        set.insert(server_id.to_string());
    }
    update_server_status(server_id, ServerRuntimeStatus::Stopping, None).await;
    if notify {
        send_status_update(server_id, ServerRuntimeStatus::Stopping, None);
    }

    stop_world_save_timer(server_id);
    for command in build_graceful_stop_commands() {
        write_server_command(server_id, command);
    }

    let timed_out = tokio::time::timeout(Duration::from_secs(15), closed.wait_for(|v| *v))
        .await
        .is_err();
    if timed_out {
        kill.notify_one();
        clear_ngrok_url(server_id).await;
        let _ = tokio::time::timeout(Duration::from_millis(1500), closed.wait_for(|v| *v)).await;
    }

    if let Ok(mut set) = STOPPING.lock() {
        set.remove(server_id);
    }
    remove_running(server_id);
    stop_stats_polling(server_id);
    clear_ngrok_url(server_id).await;
    set_offline(server_id, notify).await;
    SuccessResult::ok()
}

pub async fn stop_all_servers() {
    let ids: Vec<String> = RUNNING
        .lock()
        .map(|r| r.keys().cloned().collect())
        .unwrap_or_default();
    join_all(ids.iter().map(|id| stop_server_inner(id, false))).await;
}

pub async fn start_server(id: String) -> SuccessResult {
    start_boxed(id).await
}

pub async fn stop_server(id: String) -> SuccessResult {
    stop_server_inner(&id, true).await
}

pub async fn restart_server(id: String) -> SuccessResult {
    if !is_running(&id) {
        return SuccessResult::err("Server is not running");
    }

    send_console_line(&id, "Restarting server...", ConsoleLineType::System);

    let stopped = stop_server_inner(&id, true).await;
    if !stopped.success {
        return SuccessResult::err(
            stopped
                .error
                .unwrap_or_else(|| "Failed to stop server".to_string()),
        );
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    let started = start_boxed(id.clone()).await;
    if !started.success {
        return SuccessResult::err(
            started
                .error
                .unwrap_or_else(|| "Failed to start server".to_string()),
        );
    }

    if let Some(server) = get_server(id.clone()).await {
        start_ngrok_for(&id, &server.server_path).await;
    }

    SuccessResult::ok()
}

pub async fn send_command(id: String, command: String) -> SuccessResult {
    if RESTART_COMMAND.is_match(&command) {
        return restart_server(id).await;
    }
    if is_running(&id) {
        if STATS_COMMAND.is_match(&command) {
            if let Ok(mut map) = STATS.lock() {
                if let Some(st) = map.get_mut(&id) {
                    st.user_bypass_until = now_ms() + 6000;
                }
            }
        }
        write_server_command(&id, &command);
    }
    SuccessResult::ok()
}

pub async fn get_server_logs(server_id: String) -> Vec<ConsoleLine> {
    LOGS.lock()
        .ok()
        .and_then(|l| l.get(&server_id).cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod chat_tests {
    use super::*;

    #[test]
    fn restart_hints_ignore_player_chat() {
        let chat = "[12:00:00 INFO]: <Steve> Server is restarting";
        let unsigned = "[12:00:00 INFO]: [Not Secure] <Steve> [RestartCommand]";
        let real = "[12:00:00 INFO]: Server is restarting...";
        assert!(CHAT.is_match(chat) && CHAT.is_match(unsigned));
        assert!(!CHAT.is_match(real) && RESTART_HINTS.iter().any(|p| p.is_match(real)));
    }
}
