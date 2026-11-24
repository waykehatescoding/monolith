use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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
const MAX_BODY: usize = 64 * 1024;

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

#[derive(Default)]
struct Registry {
    mappings: HashMap<String, Mapping>,
    generations: HashMap<String, u64>,
}

impl Registry {
    fn generation(&self, server_id: &str) -> u64 {
        self.generations.get(server_id).copied().unwrap_or(0)
    }
}

static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(Default::default);

fn lock() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())
}

async fn read_capped(mut response: reqwest::Response) -> Result<String, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BODY {
            return Err("The router sent an unexpectedly large reply".into());
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

pub fn trusted_responder(source: SocketAddr) -> Option<Ipv4Addr> {
    match source.ip() {
        IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    }
}

pub fn on_router(url: &url::Url, router: Ipv4Addr) -> bool {
    url.scheme() == "http" && url.host_str() == Some(router.to_string().as_str())
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
        Some(725) => "Your router only accepts permanent port mappings. Monolith does not create those, so nothing stays open after the server stops. Forward the port in your router's settings instead.".into(),
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
    let (router, base) = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let (len, source) = socket.recv_from(&mut buf).await.ok()?;
            let Some(router) = trusted_responder(source) else {
                continue;
            };
            let location = location_of(&String::from_utf8_lossy(&buf[..len]))
                .and_then(|l| url::Url::parse(&l).ok())
                .filter(|url| on_router(url, router));
            if let Some(location) = location {
                return Some((router, location));
            }
        }
    })
    .await
    .ok()
    .flatten()
    .ok_or("No router answered. Your router has UPnP turned off, or this PC is not behind a home router.")?;

    let response = http()?
        .get(base.clone())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "The router answered with HTTP {}",
            response.status().as_u16()
        ));
    }
    let description = read_capped(response).await?;
    let (service_type, control) =
        wan_service(&description).ok_or("Your router does not offer UPnP port forwarding.")?;
    let control_url = base.join(&control).map_err(|e| e.to_string())?;
    if !on_router(&control_url, router) {
        return Err(
            "The router pointed port forwarding at a different address, so Monolith ignored it."
                .into(),
        );
    }
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
    let response = http()
        .map_err(|_| None)?
        .post(&gateway.control_url)
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
    let text = read_capped(response).await.map_err(|_| None)?;
    if ok {
        Ok(text)
    } else {
        Err(ERROR_CODE_RE
            .captures(&text)
            .and_then(|c| c[1].parse().ok()))
    }
}

async fn add(gateway: &Gateway, protocol: &str, port: u16) -> Result<(), String> {
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
        ("NewLeaseDuration", LEASE_SECONDS.to_string()),
    ];
    soap(gateway, "AddPortMapping", &args)
        .await
        .map(|_| ())
        .map_err(|code| friendly_error(code, port))
}

async fn add_all(gateway: &Gateway, ports: &[(&'static str, u16)]) -> Result<(), String> {
    for (index, (protocol, port)) in ports.iter().enumerate() {
        if let Err(error) = add(gateway, protocol, *port).await {
            remove_all(gateway, &ports[..index]).await;
            return Err(error);
        }
    }
    Ok(())
}

async fn remove_all(gateway: &Gateway, ports: &[(&'static str, u16)]) {
    for (protocol, port) in ports {
        remove(gateway, protocol, *port).await;
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
    lock().mappings.get(server_id).map(|m| m.state.clone())
}

fn set_state(
    server_id: &str,
    generation: u64,
    state: UpnpState,
    stop: Option<oneshot::Sender<oneshot::Sender<()>>>,
) -> bool {
    let mut registry = lock();
    if registry.generation(server_id) != generation {
        return false;
    }
    registry
        .mappings
        .insert(server_id.to_string(), Mapping { state, stop });
    drop(registry);
    emit(Event::NetworkChanged);
    true
}

async fn still_wanted(server_id: &str) -> bool {
    crate::server_runner::is_running(server_id)
        && get_server(server_id.to_string())
            .await
            .is_some_and(|s| is_enabled_for(&s.extra))
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
    let generation = lock().generation(&server_id);
    let Some(server) = get_server(server_id.clone()).await else {
        return;
    };
    if !set_state(&server_id, generation, UpnpState::Opening, None) {
        return;
    }
    let ports = wanted_ports(Path::new(&server.server_path), &server).await;
    let result = async {
        let gateway = discover().await?;
        add_all(&gateway, &ports).await?;
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
            set_state(&server_id, generation, UpnpState::Failed(error), None);
            return;
        }
    };
    let (stop_tx, mut stop_rx) = oneshot::channel();
    let installed = still_wanted(&server_id).await
        && set_state(
            &server_id,
            generation,
            UpnpState::Open {
                external_ip,
                ports: ports.clone(),
            },
            Some(stop_tx),
        );
    if !installed {
        remove_all(&gateway, &ports).await;
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(RENEW_EVERY) => {
                    for (protocol, port) in &ports {
                        let _ = add(&gateway, protocol, *port).await;
                    }
                }
                ack = &mut stop_rx => {
                    remove_all(&gateway, &ports).await;
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
    let removed = {
        let mut registry = lock();
        *registry.generations.entry(server_id.clone()).or_default() += 1;
        registry.mappings.remove(&server_id)
    };
    if let Some(stop) = removed.and_then(|m| m.stop) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if stop.send(ack_tx).is_ok() {
            let _ = ack_rx.await;
        }
    }
    emit(Event::NetworkChanged);
}

pub async fn close_all() {
    let ids: Vec<String> = lock().mappings.keys().cloned().collect();
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
    fn only_trusts_the_router_on_the_lan() {
        let router: Ipv4Addr = "192.168.1.1".parse().unwrap();
        assert_eq!(
            trusted_responder("192.168.1.1:1900".parse().unwrap()),
            Some(router)
        );
        assert_eq!(trusted_responder("8.8.8.8:1900".parse().unwrap()), None);
        let ok = url::Url::parse("http://192.168.1.1:5000/rootDesc.xml").unwrap();
        assert!(on_router(&ok, router));
        assert!(on_router(&ok.join("/ctl/IPConn").unwrap(), router));
        for bad in [
            "http://192.168.1.2:5000/rootDesc.xml",
            "https://192.168.1.1/rootDesc.xml",
            "http://example.com/rootDesc.xml",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!on_router(&url::Url::parse(bad).unwrap(), router), "{bad}");
        }
        assert!(!on_router(&ok.join("http://10.0.0.5/ctl").unwrap(), router));
    }

    #[test]
    fn close_cancels_a_pending_open() {
        let id = "generation-test";
        let generation = lock().generation(id);
        assert!(set_state(id, generation, UpnpState::Opening, None));
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(close(id.to_string()));
        assert!(!set_state(id, generation, UpnpState::Opening, None));
        assert_eq!(state(id), None);
    }

    #[test]
    fn explains_router_errors() {
        assert!(friendly_error(Some(725), 1).contains("permanent"));
        assert!(friendly_error(Some(718), 25565).contains("25565"));
        assert!(friendly_error(Some(606), 1).contains("turned off"));
    }
}
