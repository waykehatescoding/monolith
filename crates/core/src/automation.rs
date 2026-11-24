use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDateTime, NaiveTime};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use crate::backup_manager::create_backup_of_type;
use crate::events::{emit, subscribe, Event};
use crate::schedule;
use crate::server_runner::{
    force_restart, is_running, ping, player_count, restart_server, send_command, send_console_line,
    start_server, stop_server,
};
use crate::server_store::{load_server_list, save_server_list};
use crate::types::{BackupType, ConsoleLineType, ServerRecord, ServerRuntimeStatus, SuccessResult};

const KEY: &str = "automation";
const WATCHDOG_INTERVAL: chrono::Duration = chrono::Duration::seconds(60);
const TRIGGER_COOLDOWN: Duration = Duration::from_secs(10);

const PREFIX: &str = r"^\[[^\]]*\](?: \[[^\]]*/INFO\])?: ";
static JOIN: Lazy<Regex> = Lazy::new(|| {
    pattern(&format!(
        r"{PREFIX}(?:System chat: )?([A-Za-z0-9_]{{1,16}}) joined the game$"
    ))
});
static LEAVE: Lazy<Regex> = Lazy::new(|| {
    pattern(&format!(
        r"{PREFIX}(?:System chat: )?([A-Za-z0-9_]{{1,16}}) left the game$"
    ))
});
static CHAT: Lazy<Regex> = Lazy::new(|| {
    pattern(&format!(
        r"{PREFIX}(?:\[Not Secure\] )?<([A-Za-z0-9_]{{1,16}})> (.*)$"
    ))
});
static ANSI: Lazy<Regex> = Lazy::new(|| pattern(r"\x1b\[[0-9;]*[A-Za-z]"));

fn pattern(text: &str) -> Regex {
    Regex::new(text).unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Automation {
    pub tasks: Vec<ScheduledTask>,
    pub triggers: Vec<Trigger>,
    pub wait_for_empty: bool,
    pub max_wait_minutes: u32,
    pub idle_stop_minutes: u32,
    pub watchdog: bool,
    pub watchdog_failures: u32,
    pub auto_update: bool,
    pub sleep_mode: bool,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            triggers: Vec::new(),
            wait_for_empty: false,
            max_wait_minutes: 60,
            idle_stop_minutes: 0,
            watchdog: false,
            watchdog_failures: 3,
            auto_update: false,
            sleep_mode: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub schedule: Schedule,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub only_when_online: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    Daily { times: Vec<String> },
    Weekly { days: Vec<u8>, time: String },
    Interval { minutes: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Step {
    Command(String),
    Say(String),
    Wait(u32),
    Restart,
    Stop,
    Start,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub id: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub event: TriggerEvent,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum TriggerEvent {
    Join(String),
    Leave(String),
    Chat(String),
    Console(String),
    Started,
    Stopped,
    Crashed,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn of(server: &ServerRecord) -> Automation {
    server
        .extra
        .get(KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

pub fn validate(automation: &Automation) -> Result<(), String> {
    for task in &automation.tasks {
        if task.name.trim().is_empty() {
            return Err("Give every task a name".into());
        }
        match &task.schedule {
            Schedule::Daily { times } => {
                if schedule::parse_times(times)?.is_empty() {
                    return Err(format!("\"{}\" needs at least one time", task.name));
                }
            }
            Schedule::Weekly { days, time } => {
                if days.is_empty() || days.iter().any(|d| *d > 6) {
                    return Err(format!("\"{}\" needs at least one weekday", task.name));
                }
                schedule::parse_times(std::slice::from_ref(time))?;
            }
            Schedule::Interval { minutes } => {
                if *minutes == 0 {
                    return Err(format!("\"{}\" needs an interval above zero", task.name));
                }
            }
        }
        validate_steps(&task.steps)?;
    }
    for trigger in &automation.triggers {
        match &trigger.event {
            TriggerEvent::Chat(text) | TriggerEvent::Console(text) if text.trim().is_empty() => {
                return Err("Enter the text the trigger should look for".into());
            }
            _ => {}
        }
        validate_steps(&trigger.steps)?;
    }
    Ok(())
}

fn validate_steps(steps: &[Step]) -> Result<(), String> {
    if steps.is_empty() {
        return Err("Add at least one step".into());
    }
    for step in steps {
        match step {
            Step::Command(text) | Step::Say(text) if text.trim().is_empty() => {
                return Err("Commands and messages cannot be empty".into());
            }
            Step::Wait(0) => return Err("Waits must be at least one second".into()),
            _ => {}
        }
    }
    Ok(())
}

pub async fn save(id: String, automation: Automation) -> SuccessResult {
    if let Err(error) = validate(&automation) {
        return SuccessResult::err(error);
    }
    let mut servers = load_server_list().await;
    let Some(server) = servers.iter_mut().find(|s| s.id == id) else {
        return SuccessResult::err("Server not found");
    };
    match serde_json::to_value(&automation) {
        Ok(value) => server.extra.insert(KEY.into(), value),
        Err(error) => return SuccessResult::err(error.to_string()),
    };
    match save_server_list(&servers).await {
        Ok(()) => SuccessResult::ok(),
        Err(error) => SuccessResult::err(error),
    }
}

pub fn next_run(schedule: &Schedule, after: NaiveDateTime) -> Option<NaiveDateTime> {
    match schedule {
        Schedule::Daily { times } => schedule::seconds_until_next(after, times)
            .map(|seconds| after + chrono::Duration::seconds(seconds)),
        Schedule::Weekly { days, time } => {
            let time = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
            (0..=7)
                .map(|offset| after.date() + chrono::Duration::days(offset))
                .filter(|date| days.contains(&(date.weekday().num_days_from_monday() as u8)))
                .map(|date| date.and_time(time))
                .find(|at| *at > after)
        }
        Schedule::Interval { minutes } => {
            (*minutes > 0).then(|| after + chrono::Duration::minutes(i64::from(*minutes)))
        }
    }
}

static DUE: Lazy<Mutex<HashMap<String, (Schedule, NaiveDateTime)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static RESULTS: Lazy<Mutex<HashMap<String, (NaiveDateTime, String)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static ACTIVE: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub fn next_run_at(task_id: &str) -> Option<NaiveDateTime> {
    DUE.lock().ok()?.get(task_id).map(|(_, at)| *at)
}

pub fn last_result(task_id: &str) -> Option<(NaiveDateTime, String)> {
    RESULTS.lock().ok()?.get(task_id).cloned()
}

pub fn is_task_running(task_id: &str) -> bool {
    ACTIVE.lock().is_ok_and(|a| a.contains(task_id))
}

fn is_due(task: &ScheduledTask, now: NaiveDateTime) -> bool {
    let Ok(mut due) = DUE.lock() else {
        return false;
    };
    match due.get(&task.id) {
        Some((schedule, at)) if *schedule == task.schedule => {
            if *at > now {
                return false;
            }
            match next_run(&task.schedule, now) {
                Some(next) => due.insert(task.id.clone(), (task.schedule.clone(), next)),
                None => due.remove(&task.id),
            };
            true
        }
        _ => {
            match next_run(&task.schedule, now) {
                Some(next) => due.insert(task.id.clone(), (task.schedule.clone(), next)),
                None => due.remove(&task.id),
            };
            false
        }
    }
}

pub fn run_tasks(servers: &[ServerRecord], now: NaiveDateTime) {
    let mut seen = HashSet::new();
    for server in servers {
        for task in of(server).tasks.into_iter().filter(|t| t.enabled) {
            seen.insert(task.id.clone());
            if is_due(&task, now) {
                tokio::spawn(run_task(server.id.clone(), task));
            }
        }
    }
    if let Ok(mut due) = DUE.lock() {
        due.retain(|id, _| seen.contains(id));
    }
}

pub async fn run_task(server_id: String, task: ScheduledTask) -> Result<String, String> {
    if !ACTIVE.lock().is_ok_and(|mut a| a.insert(task.id.clone())) {
        return Err("This task is already running".into());
    }
    let outcome = if task.only_when_online && !is_running(&server_id) {
        Ok("Skipped, the server was offline".to_string())
    } else {
        run_steps(&server_id, &task.steps, &task.name, &[])
            .await
            .map(|()| "Finished".to_string())
    };
    let text = match &outcome {
        Ok(text) => text.clone(),
        Err(error) => format!("Failed: {error}"),
    };
    if let Ok(mut results) = RESULTS.lock() {
        results.insert(task.id.clone(), (Local::now().naive_local(), text));
    }
    if let Ok(mut active) = ACTIVE.lock() {
        active.remove(&task.id);
    }
    outcome
}

pub fn fill(text: &str, vars: &[(&str, &str)]) -> String {
    vars.iter().fold(text.to_string(), |text, (key, value)| {
        text.replace(&format!("{{{key}}}"), value)
    })
}

fn checked(result: SuccessResult) -> Result<(), String> {
    match result.success {
        true => Ok(()),
        false => Err(result.error.unwrap_or_else(|| "Unknown error".into())),
    }
}

pub async fn run_steps(
    server_id: &str,
    steps: &[Step],
    label: &str,
    vars: &[(&str, &str)],
) -> Result<(), String> {
    let id = server_id.to_string();
    for step in steps {
        let online = is_running(&id);
        match step {
            Step::Command(_) | Step::Say(_) if !online => {
                return Err("The server is not running".into());
            }
            Step::Command(command) => checked(send_command(id.clone(), fill(command, vars)).await)?,
            Step::Say(message) => {
                let command = format!("say {}", fill(message, vars));
                checked(send_command(id.clone(), command).await)?
            }
            Step::Wait(seconds) => {
                tokio::time::sleep(Duration::from_secs(u64::from(*seconds))).await
            }
            Step::Restart if online => checked(restart_server(id.clone()).await)?,
            Step::Start if !online => checked(start_server(id.clone()).await)?,
            Step::Restart | Step::Start => {}
            Step::Stop => checked(stop_server(id.clone()).await)?,
            Step::Backup => {
                let result = create_backup_of_type(&id, Some(label), BackupType::Auto).await;
                if !result.success {
                    return Err(result.error.unwrap_or_else(|| "The backup failed".into()));
                }
            }
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Scheduler {
    remaining: HashMap<String, i64>,
    waiting: HashMap<String, (NaiveDateTime, i64)>,
    empty_since: HashMap<String, NaiveDateTime>,
    watchdog: HashMap<String, (NaiveDateTime, u32)>,
}

impl Scheduler {
    pub async fn tick(&mut self, now: NaiveDateTime) {
        let servers = load_server_list().await;
        remember_triggers(&servers);
        run_tasks(&servers, now);
        for server in &servers {
            let settings = of(server);
            self.restart(server, &settings, now).await;
            self.stop_when_empty(server, &settings, now);
            self.watch(server, &settings, now);
        }
    }

    async fn restart(&mut self, server: &ServerRecord, settings: &Automation, now: NaiveDateTime) {
        let id = &server.id;
        if !is_running(id) {
            self.remaining.remove(id);
            self.waiting.remove(id);
            return;
        }
        if let Some((deadline, previous)) = self.waiting.get(id).copied() {
            let left = (deadline - now).num_seconds();
            if left <= 0 || player_count(id) == Some(0) {
                self.waiting.remove(id);
                tokio::spawn(restart_server(id.clone()));
            } else {
                if let Some(seconds) = schedule::crossed_warning(previous, left) {
                    say(id, format!("Server restarting in {}", describe(seconds))).await;
                }
                self.waiting.insert(id.clone(), (deadline, left));
            }
            return;
        }
        let Some(current) = schedule::seconds_until_next(now, &server.restart_times) else {
            self.remaining.remove(id);
            return;
        };
        let Some(previous) = self.remaining.insert(id.clone(), current) else {
            return;
        };
        if current > previous {
            if settings.wait_for_empty && player_count(id).is_some_and(|n| n > 0) {
                let minutes = settings.max_wait_minutes;
                say(
                    id,
                    format!("Server restarts once everyone has left, or in {minutes} minute(s)"),
                )
                .await;
                let deadline = now + chrono::Duration::minutes(i64::from(minutes));
                self.waiting
                    .insert(id.clone(), (deadline, i64::from(minutes) * 60));
            } else {
                tokio::spawn(restart_server(id.clone()));
            }
        } else if !settings.wait_for_empty {
            if let Some(seconds) = schedule::crossed_warning(previous, current) {
                say(id, format!("Server restarting in {}", describe(seconds))).await;
            }
        }
    }

    fn stop_when_empty(
        &mut self,
        server: &ServerRecord,
        settings: &Automation,
        now: NaiveDateTime,
    ) {
        let id = &server.id;
        let minutes = settings.idle_stop_minutes;
        if minutes == 0 || player_count(id) != Some(0) {
            self.empty_since.remove(id);
            return;
        }
        let since = *self.empty_since.entry(id.clone()).or_insert(now);
        if now - since < chrono::Duration::minutes(i64::from(minutes)) {
            return;
        }
        self.empty_since.remove(id);
        send_console_line(
            id,
            &format!("Nobody has been online for {minutes} minute(s), stopping the server"),
            ConsoleLineType::System,
        );
        emit(Event::Notice {
            title: "Server stopped while empty",
            body: format!(
                "{} stopped after {minutes} minute(s) with nobody online.",
                server.name
            ),
        });
        tokio::spawn(stop_server(id.clone()));
    }

    fn watch(&mut self, server: &ServerRecord, settings: &Automation, now: NaiveDateTime) {
        let id = &server.id;
        if !settings.watchdog || player_count(id).is_none() {
            self.watchdog.remove(id);
            return;
        }
        let (last, failures) = self
            .watchdog
            .entry(id.clone())
            .or_insert((now - WATCHDOG_INTERVAL, 0));
        if now - *last < WATCHDOG_INTERVAL {
            return;
        }
        let Some(answered) = ping(id) else {
            return;
        };
        *last = now;
        *failures = if answered { 0 } else { *failures + 1 };
        if *failures < settings.watchdog_failures.max(1) {
            return;
        }
        self.watchdog.remove(id);
        send_console_line(
            id,
            "The server stopped responding, forcing a restart",
            ConsoleLineType::System,
        );
        if force_restart(id) {
            emit(Event::Notice {
                title: "Server not responding",
                body: format!("{} stopped responding and was restarted.", server.name),
            });
        }
    }
}

fn safe_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() && !matches!(c, '@' | '\u{a7}'))
        .take(256)
        .collect()
}

pub fn match_line(event: &TriggerEvent, line: &str) -> Option<Vec<(&'static str, String)>> {
    let player = |regex: &Regex, name: &str| {
        let player = regex.captures(line)?.get(1)?.as_str().to_string();
        let wanted = name.trim();
        (wanted.is_empty() || wanted.eq_ignore_ascii_case(&player))
            .then(|| vec![("player", player)])
    };
    match event {
        TriggerEvent::Join(name) => player(&JOIN, name),
        TriggerEvent::Leave(name) => player(&LEAVE, name),
        TriggerEvent::Chat(text) => {
            let captures = CHAT.captures(line)?;
            let message = safe_text(&captures[2]);
            message
                .to_lowercase()
                .contains(&text.trim().to_lowercase())
                .then(|| vec![("player", captures[1].to_string()), ("message", message)])
        }
        TriggerEvent::Console(text) => {
            let found = match Regex::new(text) {
                Ok(regex) => regex.is_match(line),
                Err(_) => line.contains(text.as_str()),
            };
            found.then(|| vec![("message", safe_text(line))])
        }
        _ => None,
    }
}

fn status_event(
    previous: ServerRuntimeStatus,
    current: ServerRuntimeStatus,
) -> Option<TriggerEvent> {
    use ServerRuntimeStatus::*;
    match (previous, current) {
        (Online, Online) | (Crashed, Crashed) => None,
        (_, Online) => Some(TriggerEvent::Started),
        (Online | Stopping, Offline) => Some(TriggerEvent::Stopped),
        (_, Crashed) => Some(TriggerEvent::Crashed),
        _ => None,
    }
}

static TRIGGERS: Lazy<Mutex<HashMap<String, Vec<Trigger>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn remember_triggers(servers: &[ServerRecord]) {
    if let Ok(mut triggers) = TRIGGERS.lock() {
        *triggers = servers
            .iter()
            .map(|server| {
                let enabled = of(server).triggers.into_iter().filter(|t| t.enabled);
                (server.id.clone(), enabled.collect())
            })
            .collect();
    }
}

async fn sleep(server_id: String) {
    if let Err(error) = crate::sleep_mode::sync(server_id.clone()).await {
        send_console_line(
            &server_id,
            &format!("Sleep mode could not start: {error}"),
            ConsoleLineType::Stderr,
        );
    }
}

pub async fn watch_events() {
    let mut events = subscribe();
    let mut statuses: HashMap<String, ServerRuntimeStatus> = HashMap::new();
    let mut fired: HashMap<String, Instant> = HashMap::new();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let (server_id, line, status) = match event {
            Event::ConsoleOutput { server_id, line } if line.kind != ConsoleLineType::System => (
                server_id,
                ANSI.replace_all(&line.text, "").into_owned(),
                None,
            ),
            Event::ServerStatus(update) => {
                if update.status == ServerRuntimeStatus::Offline {
                    tokio::spawn(sleep(update.server_id.clone()));
                }
                let previous = statuses
                    .insert(update.server_id.clone(), update.status)
                    .unwrap_or_default();
                match status_event(previous, update.status) {
                    Some(event) => (update.server_id, String::new(), Some(event)),
                    None => continue,
                }
            }
            _ => continue,
        };
        let triggers = TRIGGERS
            .lock()
            .ok()
            .and_then(|t| t.get(&server_id).cloned())
            .unwrap_or_default();
        for trigger in triggers {
            let vars = match &status {
                Some(event) => (*event == trigger.event).then(Vec::new),
                None => match_line(&trigger.event, &line),
            };
            let Some(vars) = vars else {
                continue;
            };
            let now = Instant::now();
            if fired
                .get(&trigger.id)
                .is_some_and(|at| now.duration_since(*at) < TRIGGER_COOLDOWN)
            {
                continue;
            }
            fired.insert(trigger.id.clone(), now);
            let id = server_id.clone();
            tokio::spawn(async move {
                let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
                if let Err(error) = run_steps(&id, &trigger.steps, "Trigger", &vars).await {
                    send_console_line(
                        &id,
                        &format!("An automation trigger failed: {error}"),
                        ConsoleLineType::System,
                    );
                }
            });
        }
    }
}

async fn say(id: &str, message: String) {
    send_command(id.to_string(), format!("say {message}")).await;
}

fn describe(seconds: i64) -> String {
    match seconds {
        s if s >= 60 => format!("{} minute(s)", s / 60),
        s => format!("{s} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::Value;

    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 6, day)
            .and_then(|d| d.and_hms_opt(h, m, 0))
            .unwrap_or_default()
    }

    #[test]
    fn matches_console_lines() {
        let join = "[12:00:00 INFO]: Steve joined the game";
        let chat = "[12:00:01] [Server thread/INFO]: [Not Secure] <Alex> Hello there";
        let any = TriggerEvent::Join(String::new());
        assert_eq!(
            match_line(&any, join),
            Some(vec![("player", "Steve".into())])
        );
        assert!(match_line(&TriggerEvent::Join("steve".into()), join).is_some());
        assert!(match_line(&TriggerEvent::Join("Alex".into()), join).is_none());
        let paper_26 = "[10:11:45] [Server thread/INFO]: System chat: notwayke joined the game";
        assert_eq!(
            match_line(&any, paper_26),
            Some(vec![("player", "notwayke".into())])
        );
        assert!(match_line(&TriggerEvent::Leave(String::new()), join).is_none());
        assert_eq!(
            match_line(&TriggerEvent::Chat("hello".into()), chat),
            Some(vec![
                ("player", "Alex".into()),
                ("message", "Hello there".into())
            ])
        );
        assert!(match_line(&TriggerEvent::Chat("bye".into()), chat).is_none());
        assert!(match_line(&TriggerEvent::Join(String::new()), chat).is_none());
        assert!(match_line(&TriggerEvent::Console(r"joined the \w+".into()), join).is_some());
        assert!(match_line(&TriggerEvent::Console("[unclosed".into()), "a [unclosed b").is_some());
        assert!(match_line(&TriggerEvent::Started, join).is_none());
    }

    #[test]
    fn chat_cannot_spoof_events_or_inject_selectors() {
        let spoof = "[12:00:00 INFO]: <Bob> x]: @a joined the game";
        assert!(match_line(&TriggerEvent::Join(String::new()), spoof).is_none());
        let chat = "[12:00:00 INFO]: <Bob> hi @a \u{a7}cred";
        assert_eq!(
            match_line(&TriggerEvent::Chat("hi".into()), chat),
            Some(vec![
                ("player", "Bob".into()),
                ("message", "hi a cred".into())
            ])
        );
    }

    #[test]
    fn maps_status_changes() {
        use ServerRuntimeStatus::*;
        assert_eq!(status_event(Starting, Online), Some(TriggerEvent::Started));
        assert_eq!(status_event(Stopping, Offline), Some(TriggerEvent::Stopped));
        assert_eq!(status_event(Online, Crashed), Some(TriggerEvent::Crashed));
        assert_eq!(status_event(Offline, Offline), None);
        assert_eq!(status_event(Starting, Offline), None);
        assert_eq!(status_event(Online, Online), None);
    }

    #[test]
    fn daily_runs_at_the_next_time() {
        let daily = Schedule::Daily {
            times: vec!["04:00".into(), "18:30".into()],
        };
        assert_eq!(next_run(&daily, at(1, 3, 0)), Some(at(1, 4, 0)));
        assert_eq!(next_run(&daily, at(1, 18, 30)), Some(at(2, 4, 0)));
    }

    #[test]
    fn weekly_skips_other_days() {
        let weekly = Schedule::Weekly {
            days: vec![0, 4],
            time: "20:00".into(),
        };
        assert_eq!(at(1, 0, 0).weekday(), chrono::Weekday::Mon);
        assert_eq!(next_run(&weekly, at(1, 19, 0)), Some(at(1, 20, 0)));
        assert_eq!(next_run(&weekly, at(1, 20, 0)), Some(at(5, 20, 0)));
        assert_eq!(next_run(&weekly, at(6, 9, 0)), Some(at(8, 20, 0)));
        let only_monday = Schedule::Weekly {
            days: vec![0],
            time: "20:00".into(),
        };
        assert_eq!(next_run(&only_monday, at(1, 21, 0)), Some(at(8, 20, 0)));
    }

    #[test]
    fn interval_counts_from_the_last_check() {
        let every = Schedule::Interval { minutes: 90 };
        assert_eq!(next_run(&every, at(1, 23, 0)), Some(at(2, 0, 30)));
        assert_eq!(
            next_run(&Schedule::Interval { minutes: 0 }, at(1, 0, 0)),
            None
        );
    }

    #[test]
    fn fires_once_when_due() {
        let task = ScheduledTask {
            id: new_id(),
            name: "Test".into(),
            enabled: true,
            schedule: Schedule::Interval { minutes: 10 },
            steps: vec![Step::Say("hi".into())],
            only_when_online: false,
        };
        assert!(!is_due(&task, at(1, 12, 0)));
        assert!(!is_due(&task, at(1, 12, 9)));
        assert!(is_due(&task, at(1, 12, 10)));
        assert!(!is_due(&task, at(1, 12, 11)));
        assert_eq!(next_run_at(&task.id), Some(at(1, 12, 20)));
    }

    #[test]
    fn validates_tasks() {
        let mut automation = Automation {
            tasks: vec![ScheduledTask {
                id: new_id(),
                name: "Backup".into(),
                enabled: true,
                schedule: Schedule::Daily {
                    times: vec!["03:00".into()],
                },
                steps: vec![Step::Backup],
                only_when_online: false,
            }],
            ..Default::default()
        };
        assert!(validate(&automation).is_ok());
        automation.tasks[0].steps.clear();
        assert!(validate(&automation).is_err());
        automation.tasks[0].steps = vec![Step::Command(" ".into())];
        assert!(validate(&automation).is_err());
        automation.tasks[0].steps = vec![Step::Wait(5)];
        automation.tasks[0].schedule = Schedule::Daily {
            times: vec!["25:00".into()],
        };
        assert!(validate(&automation).is_err());
    }

    #[test]
    fn reads_old_records_and_round_trips() {
        let server = ServerRecord::default();
        assert_eq!(of(&server), Automation::default());
        let automation = Automation {
            tasks: vec![ScheduledTask {
                id: "a".into(),
                name: "Greet".into(),
                enabled: false,
                schedule: Schedule::Weekly {
                    days: vec![5, 6],
                    time: "12:00".into(),
                },
                steps: vec![
                    Step::Say("Hello {player}".into()),
                    Step::Wait(3),
                    Step::Restart,
                ],
                only_when_online: true,
            }],
            ..Default::default()
        };
        let mut server = ServerRecord::default();
        server.extra.insert(
            KEY.into(),
            serde_json::to_value(&automation).unwrap_or(Value::Null),
        );
        assert_eq!(of(&server), automation);
        assert_eq!(
            fill("Hello {player}", &[("player", "Steve")]),
            "Hello Steve"
        );
    }
}
