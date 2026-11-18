use once_cell::sync::Lazy;
use tokio::sync::broadcast;

use crate::types::{
    BackupCompletedEvent, BackupProgressEvent, ConsoleLine, DownloadUpdateProgress,
    NgrokInstallProgress, NgrokTunnelInfo, ServerCreationProgress, ServerStats, ServerStatusUpdate,
};

#[derive(Debug, Clone)]
pub enum Event {
    ServerStatus(ServerStatusUpdate),
    ServerStats(ServerStats),
    ConsoleOutput {
        server_id: String,
        line: ConsoleLine,
    },
    CreationProgress(ServerCreationProgress),
    FrameworkChangeProgress(ServerCreationProgress),
    BackupProgress(BackupProgressEvent),
    BackupCompleted(BackupCompletedEvent),
    NgrokUrlChanged(NgrokTunnelInfo),
    NgrokInstallProgress(NgrokInstallProgress),
    UpdateDownloadProgress(DownloadUpdateProgress),
}

static BUS: Lazy<broadcast::Sender<Event>> = Lazy::new(|| broadcast::channel(1024).0);

pub fn subscribe() -> broadcast::Receiver<Event> {
    BUS.subscribe()
}

pub fn emit(event: Event) {
    let _ = BUS.send(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_without_listeners_is_harmless() {
        emit(Event::NgrokInstallProgress(NgrokInstallProgress {
            percent: 1.0,
        }));
    }

    #[test]
    fn subscribers_receive_events() {
        let mut rx = subscribe();
        emit(Event::NgrokInstallProgress(NgrokInstallProgress {
            percent: 42.0,
        }));
        loop {
            match rx.try_recv() {
                Ok(Event::NgrokInstallProgress(p)) if p.percent == 42.0 => break,
                Ok(_) => continue,
                Err(e) => panic!("{e}"),
            }
        }
    }
}
