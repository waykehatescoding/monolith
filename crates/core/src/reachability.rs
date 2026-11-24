use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::port_check::configured_port;
use crate::types::ServerRecord;

const IP_URL: &str = "https://api.ipify.org";
const STATUS_URL: &str = "https://api.mcsrvstat.us/3";
const USER_AGENT: &str = "Monolith (Minecraft server manager)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachability {
    pub address: String,
    pub reachable: bool,
    pub cgnat: bool,
}

pub fn is_cgnat(public_ip: &str, router_ip: Option<&str>) -> bool {
    let Some(router) = router_ip.and_then(|ip| ip.parse::<Ipv4Addr>().ok()) else {
        return false;
    };
    let [a, b, ..] = router.octets();
    let shared = a == 100 && (64..128).contains(&b);
    shared || router.is_private() || router.to_string() != public_ip
}

pub fn firewall_command(port: u16, bedrock_port: Option<u16>) -> String {
    let mut rules = vec![("TCP", port)];
    rules.extend(bedrock_port.map(|p| ("UDP", p)));
    rules
        .iter()
        .map(|(protocol, port)| {
            let name = format!("Monolith-Minecraft-{protocol}-{port}");
            format!(
                "netsh advfirewall firewall delete rule name={name} & netsh advfirewall firewall add rule name={name} dir=in action=allow protocol={protocol} localport={port}"
            )
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn check(server: ServerRecord) -> Result<Reachability, String> {
    let port = configured_port(Path::new(&server.server_path)).await;
    let client = client()?;
    let public_ip = client
        .get(IP_URL)
        .send()
        .await
        .map_err(|e| format!("Could not look up your public address: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();
    public_ip
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("Unexpected public address: {public_ip}"))?;
    let address = format!("{public_ip}:{port}");
    let status: Value = client
        .get(format!("{STATUS_URL}/{address}"))
        .send()
        .await
        .map_err(|e| format!("The status checker could not be reached: {e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let reachable = status["online"].as_bool().unwrap_or(false);
    let router_ip = if reachable {
        None
    } else {
        crate::upnp::external_ip().await.ok()
    };
    Ok(Reachability {
        address,
        reachable,
        cgnat: is_cgnat(&public_ip, router_ip.as_deref()),
    })
}

pub async fn add_firewall_rule(port: u16, bedrock_port: Option<u16>) -> Result<(), String> {
    let script = format!(
        "$p = Start-Process -FilePath cmd.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '/c {}'; exit $p.ExitCode",
        firewall_command(port, bedrock_port)
    );
    let mut command = tokio::process::Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command.output().await.map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("Windows did not add the rule. Administrator permission is needed.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_carrier_grade_nat() {
        assert!(!is_cgnat("203.0.113.5", None));
        assert!(!is_cgnat("203.0.113.5", Some("203.0.113.5")));
        assert!(is_cgnat("203.0.113.5", Some("100.72.1.9")));
        assert!(is_cgnat("203.0.113.5", Some("192.168.1.2")));
        assert!(is_cgnat("203.0.113.5", Some("198.51.100.7")));
    }

    #[test]
    fn builds_firewall_rules() {
        let command = firewall_command(25565, Some(19132));
        assert!(command.contains("protocol=TCP localport=25565"));
        assert!(command.contains("protocol=UDP localport=19132"));
        assert!(!firewall_command(25565, None).contains("UDP"));
    }
}
