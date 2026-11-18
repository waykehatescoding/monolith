use std::net::TcpListener;
use std::path::Path;

use crate::server_props::read_properties;
use crate::server_runner::is_running;
use crate::server_store::load_server_list;
use crate::types::ServerRecord;

const DEFAULT_PORT: u16 = 25565;
const SEARCH_RANGE: u16 = 100;

pub fn suggest_free_port(start: u16, is_taken: impl Fn(u16) -> bool) -> Option<u16> {
    (start.saturating_add(1)..=start.saturating_add(SEARCH_RANGE))
        .filter(|p| *p != start)
        .find(|p| !is_taken(*p))
}

fn bindable(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

async fn configured_port(server_path: &Path) -> u16 {
    read_properties(server_path)
        .await
        .iter()
        .find(|p| p.key == "server-port")
        .and_then(|p| p.value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

pub async fn check_port(server: &ServerRecord) -> Result<(), String> {
    let port = configured_port(Path::new(&server.server_path)).await;
    let mut holder = None;
    for other in load_server_list().await {
        if other.id != server.id
            && is_running(&other.id)
            && configured_port(Path::new(&other.server_path)).await == port
        {
            holder = Some(other.name);
            break;
        }
    }

    let conflict = match holder {
        Some(name) => {
            format!("Port {port} is already used by the running Monolith server \"{name}\"")
        }
        None if !bindable(port) => format!("Port {port} is already in use by another process"),
        None => return Ok(()),
    };

    Err(match suggest_free_port(port, |p| !bindable(p)) {
        Some(free) => format!("{conflict}. Try port {free} instead."),
        None => format!("{conflict}."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_next_free_port() {
        assert_eq!(
            suggest_free_port(25565, |p| p == 25566 || p == 25567),
            Some(25568)
        );
        assert_eq!(suggest_free_port(25565, |_| false), Some(25566));
    }

    #[test]
    fn gives_up_when_range_is_full() {
        assert_eq!(suggest_free_port(25565, |_| true), None);
        assert_eq!(suggest_free_port(65535, |_| false), None);
    }
}
