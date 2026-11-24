use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::paths::user_data_dir;
use crate::server_props::read_properties;
use crate::types::ServerProperty;

pub const DEFAULT_PORT: u16 = 25575;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH: i32 = 3;
const EXEC: i32 = 2;
const RESPONSE: i32 = 0;
const MAX_PACKET: i32 = 1 << 20;
const MAX_COMMAND: usize = 1446;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub kind: i32,
    pub body: String,
}

pub fn encode(id: i32, kind: i32, body: &str) -> Vec<u8> {
    let length = body.len() as i32 + 10;
    let mut out = Vec::with_capacity(length as usize + 4);
    out.extend(length.to_le_bytes());
    out.extend(id.to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(body.as_bytes());
    out.extend([0, 0]);
    out
}

pub async fn read_packet<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Packet> {
    let length = reader.read_i32_le().await?;
    if !(10..=MAX_PACKET).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The server sent a reply that is not RCON.",
        ));
    }
    let mut buf = vec![0; length as usize];
    reader.read_exact(&mut buf).await?;
    let int = |at: usize| i32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
    Ok(Packet {
        id: int(0),
        kind: int(4),
        body: String::from_utf8_lossy(&buf[8..])
            .trim_end_matches('\0')
            .to_string(),
    })
}

#[derive(Debug)]
pub struct Rcon<S> {
    stream: S,
    next_id: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Rcon<S> {
    pub async fn login(stream: S, password: &str) -> Result<Self, String> {
        let mut rcon = Rcon { stream, next_id: 1 };
        let id = rcon.send(AUTH, password).await?;
        loop {
            let packet = rcon.read().await?;
            if packet.kind != EXEC {
                continue;
            }
            if packet.id == -1 {
                return Err("Wrong RCON password.".into());
            }
            if packet.id == id {
                return Ok(rcon);
            }
        }
    }

    pub async fn command(&mut self, command: &str) -> Result<String, String> {
        if command.len() > MAX_COMMAND {
            return Err(format!(
                "That command is too long. RCON accepts up to {MAX_COMMAND} bytes."
            ));
        }
        let id = self.send(EXEC, command).await?;
        let end = self.send(RESPONSE, "").await?;
        let mut output = String::new();
        loop {
            let packet = self.read().await?;
            if packet.id == end {
                return Ok(output);
            }
            if packet.id == -1 {
                return Err("The server no longer accepts this login. Connect again.".into());
            }
            if packet.id == id {
                output.push_str(&packet.body);
            }
        }
    }

    async fn send(&mut self, kind: i32, body: &str) -> Result<i32, String> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        timeout(READ_TIMEOUT, self.stream.write_all(&encode(id, kind, body)))
            .await
            .map_err(|_| "Sending to the server timed out.".to_string())?
            .map_err(describe)?;
        Ok(id)
    }

    async fn read(&mut self) -> Result<Packet, String> {
        timeout(READ_TIMEOUT, read_packet(&mut self.stream))
            .await
            .map_err(|_| "The server did not answer within 10 seconds.".to_string())?
            .map_err(describe)
    }
}

pub async fn connect(host: &str, port: u16, password: &str) -> Result<Rcon<TcpStream>, String> {
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| format!("Could not reach {host}:{port} within 5 seconds."))?
        .map_err(describe)?;
    Rcon::login(stream, password).await
}

fn describe(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => {
            "The connection was refused. Check that the server is running and RCON is enabled on this port.".into()
        }
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => "The server closed the connection.".into(),
        io::ErrorKind::TimedOut => "The connection timed out.".into(),
        _ => format!("Could not connect: {error}"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RconConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub password: String,
}

fn connections_path() -> PathBuf {
    user_data_dir().join("rcon_connections.json")
}

pub async fn list_connections() -> Vec<RconConnection> {
    match tokio::fs::read_to_string(connections_path()).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub async fn save_connections(connections: Vec<RconConnection>) -> Result<(), String> {
    let path = connections_path();
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&connections).map_err(|e| e.to_string())?;
    tokio::fs::write(path, json)
        .await
        .map_err(|e| e.to_string())
}

fn prop<'a>(props: &'a [ServerProperty], key: &str) -> Option<&'a str> {
    props.iter().find(|p| p.key == key).map(|p| p.value.trim())
}

pub fn rcon_settings(props: &[ServerProperty]) -> Option<(u16, String)> {
    if prop(props, "enable-rcon") != Some("true") {
        return None;
    }
    let port = prop(props, "rcon.port")
        .and_then(|port| port.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    Some((port, prop(props, "rcon.password").unwrap_or("").to_string()))
}

pub async fn local_rcon(server_path: &Path) -> Option<(u16, String)> {
    rcon_settings(&read_properties(server_path).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, DuplexStream};

    async fn reply(server: &mut DuplexStream, id: i32, kind: i32, body: &str) {
        server.write_all(&encode(id, kind, body)).await.unwrap();
    }

    fn fake_server(password: &'static str, output: String) -> DuplexStream {
        let (client, mut server) = duplex(64 * 1024);
        tokio::spawn(async move {
            let auth = read_packet(&mut server).await.unwrap();
            assert_eq!(auth.kind, AUTH);
            reply(&mut server, auth.id, RESPONSE, "").await;
            let id = if auth.body == password { auth.id } else { -1 };
            reply(&mut server, id, EXEC, "").await;
            while let Ok(packet) = read_packet(&mut server).await {
                if packet.kind == EXEC {
                    let bytes = output.as_bytes();
                    for chunk in bytes.chunks(4096) {
                        let text = std::str::from_utf8(chunk).unwrap();
                        reply(&mut server, packet.id, RESPONSE, text).await;
                    }
                } else {
                    let unknown = format!("Unknown request {:x}", packet.kind);
                    reply(&mut server, packet.id, RESPONSE, &unknown).await;
                }
            }
        });
        client
    }

    #[tokio::test]
    async fn packets_round_trip() {
        let bytes = encode(7, EXEC, "list");
        assert_eq!(&bytes[..4], &14i32.to_le_bytes());
        assert_eq!(bytes.len(), 18);
        let packet = read_packet(&mut bytes.as_slice()).await.unwrap();
        assert_eq!(
            packet,
            Packet {
                id: 7,
                kind: EXEC,
                body: "list".into()
            }
        );
    }

    #[tokio::test]
    async fn rejects_oversized_lengths() {
        let mut bytes = encode(1, EXEC, "x");
        bytes[..4].copy_from_slice(&(MAX_PACKET + 1).to_le_bytes());
        assert!(read_packet(&mut bytes.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn runs_a_command() {
        let stream = fake_server("secret", "There are 0 of a max of 20 players online".into());
        let mut rcon = Rcon::login(stream, "secret").await.unwrap();
        let output = rcon.command("list").await.unwrap();
        assert_eq!(output, "There are 0 of a max of 20 players online");
    }

    #[tokio::test]
    async fn joins_fragmented_responses() {
        let long = "abcdefghij".repeat(1200);
        let stream = fake_server("secret", long.clone());
        let mut rcon = Rcon::login(stream, "secret").await.unwrap();
        assert_eq!(rcon.command("help").await.unwrap(), long);
        assert_eq!(rcon.command("help").await.unwrap(), long);
    }

    #[tokio::test]
    async fn reports_wrong_password() {
        let stream = fake_server("secret", String::new());
        let error = Rcon::login(stream, "nope").await.unwrap_err();
        assert_eq!(error, "Wrong RCON password.");
    }

    #[tokio::test]
    async fn reports_closed_connection() {
        let (client, server) = duplex(1024);
        drop(server);
        let error = Rcon::login(client, "x").await.unwrap_err();
        assert_eq!(error, "The server closed the connection.");
    }

    #[test]
    fn reads_local_rcon_settings() {
        let props = |pairs: &[(&str, &str)]| -> Vec<ServerProperty> {
            pairs
                .iter()
                .map(|(key, value)| ServerProperty {
                    key: key.to_string(),
                    value: value.to_string(),
                    comment: None,
                })
                .collect()
        };
        assert_eq!(rcon_settings(&props(&[("enable-rcon", "false")])), None);
        assert_eq!(
            rcon_settings(&props(&[
                ("enable-rcon", "true"),
                ("rcon.port", "25580"),
                ("rcon.password", "pw")
            ])),
            Some((25580, "pw".into()))
        );
        assert_eq!(
            rcon_settings(&props(&[("enable-rcon", "true")])),
            Some((DEFAULT_PORT, String::new()))
        );
    }
}
