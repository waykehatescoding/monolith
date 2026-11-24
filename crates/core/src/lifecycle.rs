use std::time::Duration;

use crate::{
    app_preferences, automation, backup_manager, ngrok_manager, server_runner, server_store,
    shutdown_policy, sleep_mode,
};

const SCHEDULE_TICK: Duration = Duration::from_secs(5);
const AUTO_BACKUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub async fn startup() {
    server_runner::refresh_server_statuses().await;
    for server in server_store::load_server_list().await {
        if server.start_on_launch {
            tokio::spawn(server_runner::start_server(server.id));
        } else {
            tokio::spawn(sleep_mode::sync(server.id));
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
        ngrok_manager::stop_all_tunnels(),
        sleep_mode::stop_all(),
        crate::playit::stop(),
        crate::upnp::close_all()
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

pub async fn run_automation() {
    let mut interval = tokio::time::interval(SCHEDULE_TICK);
    let mut scheduler = automation::Scheduler::default();
    tokio::spawn(automation::watch_events());
    loop {
        interval.tick().await;
        scheduler.tick(chrono::Local::now().naive_local()).await;
    }
}
