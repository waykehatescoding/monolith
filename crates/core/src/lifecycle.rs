use std::collections::HashMap;
use std::time::Duration;

use crate::{
    app_preferences, backup_manager, ngrok_manager, schedule, server_runner, server_store,
    shutdown_policy,
};

const SCHEDULE_TICK: Duration = Duration::from_secs(5);
const AUTO_BACKUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub async fn startup() {
    server_runner::refresh_server_statuses().await;
    for server in server_store::load_server_list().await {
        if server.start_on_launch {
            tokio::spawn(server_runner::start_server(server.id));
        }
    }
}

pub async fn should_confirm_close() -> bool {
    let preferences = app_preferences::get_app_preferences().await;
    shutdown_policy::should_confirm_close(&preferences, server_runner::get_running_server_count())
}

pub async fn shutdown() {
    tokio::join!(
        server_runner::stop_all_servers(),
        ngrok_manager::stop_all_tunnels()
    );
}

pub async fn run_auto_backups() {
    let mut interval = tokio::time::interval(AUTO_BACKUP_INTERVAL);
    interval.tick().await;
    loop {
        interval.tick().await;
        backup_manager::check_and_run_auto_backups().await;
    }
}

pub async fn run_scheduled_restarts() {
    let mut interval = tokio::time::interval(SCHEDULE_TICK);
    let mut remaining: HashMap<String, i64> = HashMap::new();
    loop {
        interval.tick().await;
        let now = chrono::Local::now().naive_local();
        for server in server_store::load_server_list().await {
            let current = match schedule::seconds_until_next(now, &server.restart_times) {
                Some(current) if server_runner::is_running(&server.id) => current,
                _ => {
                    remaining.remove(&server.id);
                    continue;
                }
            };
            let Some(previous) = remaining.insert(server.id.clone(), current) else {
                continue;
            };
            if current > previous {
                tokio::spawn(server_runner::restart_server(server.id));
            } else if let Some(seconds) = schedule::crossed_warning(previous, current) {
                let command = format!("say Server restarting in {}", describe(seconds));
                server_runner::send_command(server.id, command).await;
            }
        }
    }
}

fn describe(seconds: i64) -> String {
    match seconds {
        s if s >= 60 => format!("{} minute(s)", s / 60),
        s => format!("{s} seconds"),
    }
}
