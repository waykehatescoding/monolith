use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

use crate::events::{emit, Event};
use crate::port_check::configured_port;
use crate::server_runner::re;
use crate::server_store::{get_server, load_server_list, save_server_list};

const SSDP_ADDR: &str = "239.255.255.250:1900";
const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
];
const LEASE_SECONDS: u32 = 3600;
const RENEW_EVERY: Duration = Duration::from_secs(45 * 60);
const SETTING: &str = "upnp";

static LOCATION_RE: Lazy<Regex> = Lazy::new(|| re(r"(?im)^location:\s*(\S+)"));
static SERVICE_RE: Lazy<Regex> = Lazy::new(|| re(r"(?s)<service>(.*?)</service>"));
static TYPE_RE: Lazy<Regex> = Lazy::new(|| re(r"(?s)<serviceType>\s*(.*?)\s*</serviceType>"));
static CONTROL_RE: Lazy<Regex> = Lazy::new(|| re(r"(?s)<controlURL>\s*(.*?)\s*</controlURL>"));
static ERROR_CODE_RE: Lazy<Regex> = Lazy::new(|| re(r"<errorCode>\s*(\d+)\s*</errorCode>"));
static EXTERNAL_IP_RE: Lazy<Regex> =
    Lazy::new(|| re(r"<NewExternalIPAddress>\s*([^<\s]+)\s*</NewExternalIPAddress>"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gateway {
    pub control_url: String,
    pub service_type: String,
    pub local_ip: Ipv4Addr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpnpState {
    Opening,
    Open {
        external_ip: String,
        ports: Vec<(&'static str, u16)>,
    },
    Failed(String),
}

struct Mapping {
    state: UpnpState,
    stop: Option<oneshot::Sender<oneshot::Sender<()>>>,
}

static MAPPINGS: Lazy<Mutex<HashMap<String, Mapping>>> = Lazy::new(Default::default);

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Mapping>> {
    MAPPINGS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn location_of(reply: &str) -> Option<String> {
    LOCATION_RE.captures(reply).map(|c| c[1].to_string())
}

pub fn wan_service(description: &str) -> Option<(String, String)> {
    SERVICE_RE.captures_iter(description).find_map(|block| {
        let kind = TYPE_RE.captures(&block[1])?[1].to_string();
        let wanted = kind.contains(":WANIPConnection:") || kind.contains(":WANPPPConnection:");
        let control = CONTROL_RE.captures(&block[1])?[1].to_string();
        wanted.then_some((kind, control))
    })
}

fn friendly_error(code: Option<u32>, port: u16) -> String {
    match code {
        Some(718) => format!("Another device on your network already uses port {port} on the router. Pick a different server port or remove that rule in your router."),
        Some(606) | Some(401) => "Your router refused the request. UPnP may be turned off or set to read-only in its settings.".into(),
        Some(code) => format!("Your router rejected the port mapping (UPnP error {code})."),
        None => "Your router did not accept the port mapping.".into(),
    }
}

async fn discover() -> Result<Gateway, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| e.to_string())?;
    for target in SEARCH_TARGETS {
        let search = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
        );
        socket
            .send_to(search.as_bytes(), SSDP_ADDR)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut buf = [0u8; 2048];
    let location = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let (len, _) = socket.recv_from(&mut buf).await.ok()?;
            if let Some(location) = location_of(&String::from_utf8_lossy(&buf[..len])) {
                return Some(location);
            }
        }
    })
    .await
    .ok()
    .flatten()
    .ok_or("No router answered. Your router has UPnP turned off, or this PC is not behind a home router.")?;

    let base = url::Url::parse(&location).map_err(|e| e.to_string())?;
    let description = reqwest::Client::new()
        .get(base.clone())
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let (service_type, control) =
        wan_service(&description).ok_or("Your router does not offer UPnP port forwarding.")?;
    let control_url = base.join(&control).map_err(|e| e.to_string())?;
    let router: IpAddr = control_url
        .host_str()
        .and_then(|host| host.parse().ok())
        .ok_or("The router address is not an IP address")?;
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    probe.connect((router, 1900)).map_err(|e| e.to_string())?;
    let IpAddr::V4(local_ip) = probe.local_addr().map_err(|e| e.to_string())?.ip() else {
        return Err("UPnP needs an IPv4 network".into());
    };
    Ok(Gateway {
        control_url: control_url.to_string(),
        service_type,
        local_ip,
    })
}

async fn soap(
    gateway: &Gateway,
    action: &str,
    args: &[(&str, String)],
) -> Result<String, Option<u32>> {
    let body: String = args
        .iter()
        .map(|(name, value)| format!("<{name}>{value}</{name}>"))
        .collect();
    let envelope = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{body}</u:{action}></s:Body></s:Envelope>",
        gateway.service_type
    );
    let response = reqwest::Client::new()
        .post(&gateway.control_url)
        .timeout(Duration::from_secs(5))
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header(
            "SOAPAction",
            format!("\"{}#{action}\"", gateway.service_type),
        )
        .body(envelope)
        .send()
        .await
        .map_err(|_| None)?;
    let ok = response.status().is_success();
    let text = response.text().await.map_err(|_| None)?;
    if ok {
        Ok(text)
    } else {
        Err(ERROR_CODE_RE
            .captures(&text)
            .and_then(|c| c[1].parse().ok()))
    }
}

async fn add(gateway: &Gateway, protocol: &str, port: u16) -> Result<(), String> {
    let mut lease = LEASE_SECONDS;
    loop {
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", protocol.to_string()),
            ("NewInternalPort", port.to_string()),
            ("NewInternalClient", gateway.local_ip.to_string()),
            ("NewEnabled", "1".to_string()),
            (
                "NewPortMappingDescription",
                format!("Monolith {protocol} {port}"),
            ),
            ("NewLeaseDuration", lease.to_string()),
        ];
        match soap(gateway, "AddPortMapping", &args).await {
            Ok(_) => return Ok(()),
            Err(Some(725)) if lease != 0 => lease = 0,
            Err(code) => return Err(friendly_error(code, port)),
        }
    }
}

async fn remove(gateway: &Gateway, protocol: &str, port: u16) {
    let args = [
        ("NewRemoteHost", String::new()),
        ("NewExternalPort", port.to_string()),
        ("NewProtocol", protocol.to_string()),
    ];
    let _ = soap(gateway, "DeletePortMapping", &args).await;
}

pub async fn external_ip() -> Result<String, String> {
    let gateway = discover().await?;
    let reply = soap(&gateway, "GetExternalIPAddress", &[])
        .await
        .map_err(|code| friendly_error(code, 0))?;
    EXTERNAL_IP_RE
        .captures(&reply)
        .map(|c| c[1].to_string())
        .ok_or_else(|| "The router did not report its internet address".into())
}

pub fn is_enabled_for(extra: &serde_json::Map<String, Value>) -> bool {
    extra.get(SETTING).and_then(Value::as_bool).unwrap_or(false)
}

pub async fn set_enabled(server_id: String, enabled: bool) -> Result<(), String> {
    let mut servers = load_server_list().await;
    let server = servers
        .iter_mut()
        .find(|s| s.id == server_id)
        .ok_or("Server not found")?;
    server.extra.insert(SETTING.into(), enabled.into());
    save_server_list(&servers).await?;
    if enabled {
        if crate::server_runner::is_running(&server_id) {
            open(server_id).await;
        }
    } else {
        close(server_id).await;
    }
    Ok(())
}

pub fn state(server_id: &str) -> Option<UpnpState> {
    lock().get(server_id).map(|m| m.state.clone())
}

fn set_state(
    server_id: &str,
    state: UpnpState,
    stop: Option<oneshot::Sender<oneshot::Sender<()>>>,
) {
    lock().insert(server_id.to_string(), Mapping { state, stop });
    emit(Event::NetworkChanged);
}

async fn wanted_ports(
    server_path: &Path,
    server: &crate::types::ServerRecord,
) -> Vec<(&'static str, u16)> {
    let mut ports = vec![("TCP", configured_port(server_path).await)];
    if let Some(bedrock) = crate::crossplay::bedrock_port(server).await {
        ports.push(("UDP", bedrock));
    }
    ports
}

pub async fn on_start(server_id: String) {
    let enabled = get_server(server_id.clone())
        .await
        .is_some_and(|s| is_enabled_for(&s.extra));
    if enabled {
        open(server_id).await;
    }
}

async fn open(server_id: String) {
    close(server_id.clone()).await;
    let Some(server) = get_server(server_id.clone()).await else {
        return;
    };
    set_state(&server_id, UpnpState::Opening, None);
    let ports = wanted_ports(Path::new(&server.server_path), &server).await;
    let result = async {
        let gateway = discover().await?;
        for (protocol, port) in &ports {
            add(&gateway, protocol, *port).await?;
        }
        let external = soap(&gateway, "GetExternalIPAddress", &[])
            .await
            .ok()
            .and_then(|reply| EXTERNAL_IP_RE.captures(&reply).map(|c| c[1].to_string()))
            .unwrap_or_default();
        Ok::<_, String>((gateway, external))
    }
    .await;
    let (gateway, external_ip) = match result {
        Ok(found) => found,
        Err(error) => {
            set_state(&server_id, UpnpState::Failed(error), None);
            return;
        }
    };
    let (stop_tx, mut stop_rx) = oneshot::channel();
    set_state(
        &server_id,
        UpnpState::Open {
            external_ip,
            ports: ports.clone(),
        },
        Some(stop_tx),
    );
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(RENEW_EVERY) => {
                    for (protocol, port) in &ports {
                        let _ = add(&gateway, protocol, *port).await;
                    }
                }
                ack = &mut stop_rx => {
                    for (protocol, port) in &ports {
                        remove(&gateway, protocol, *port).await;
                    }
                    if let Ok(ack) = ack {
                        let _ = ack.send(());
                    }
                    break;
                }
            }
        }
    });
}

pub async fn close(server_id: String) {
    let removed = lock().remove(&server_id);
    if let Some(stop) = removed.and_then(|m| m.stop) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if stop.send(ack_tx).is_ok() {
            let _ = ack_rx.await;
        }
    }
    emit(Event::NetworkChanged);
}

pub async fn close_all() {
    let ids: Vec<String> = lock().keys().cloned().collect();
    futures_util::future::join_all(ids.into_iter().map(close)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ssdp_location() {
        let reply = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLocation: http://192.168.1.1:5000/rootDesc.xml\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
        assert_eq!(
            location_of(reply).as_deref(),
            Some("http://192.168.1.1:5000/rootDesc.xml")
        );
        assert_eq!(location_of("HTTP/1.1 200 OK\r\n\r\n"), None);
    }

    #[test]
    fn finds_the_wan_service() {
        let xml = r#"<root><device><serviceList>
            <service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/ctl/L3F</controlURL></service>
            <service>
              <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
              <controlURL>/ctl/IPConn</controlURL>
            </service></serviceList></device></root>"#;
        assert_eq!(
            wan_service(xml),
            Some((
                "urn:schemas-upnp-org:service:WANIPConnection:1".into(),
                "/ctl/IPConn".into()
            ))
        );
        assert_eq!(wan_service("<root></root>"), None);
    }

    #[test]
    fn explains_router_errors() {
        assert!(friendly_error(Some(718), 25565).contains("25565"));
        assert!(friendly_error(Some(606), 1).contains("turned off"));
    }
}
