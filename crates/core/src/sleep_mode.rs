use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use once_cell::sync::Lazy;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::automation;
use crate::events::{emit, Event};
use crate::port_check::configured_port;
use crate::server_props::read_properties;
use crate::server_runner::{is_running, send_console_line, start_server};
use crate::server_store::get_server;
use crate::types::{ConsoleLineType, ServerRecord, ServerRuntimeStatus};

const MAX_HANDSHAKE: usize = 1024;
const MAX_STATUS_REQUEST: usize = 1;
const MAX_PING: usize = 9;
const MAX_LOGIN: usize = 512;
const MAX_CONNECTIONS: usize = 32;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const LINGER: Duration = Duration::from_secs(2);
const KICK: &str = "Starting the server, rejoin in about a minute";

struct Listener {
    task: JoinHandle<()>,
    port: u16,
}

static SLEEPING: Lazy<Mutex<HashMap<String, Listener>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub motd: String,
    pub max_players: u32,
}

pub fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    while value >= 0x80 {
        buf.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

pub fn read_varint(buf: &mut &[u8]) -> Option<i32> {
    let mut value = 0u32;
    for shift in [0, 7, 14, 21, 28] {
        let (&byte, rest) = buf.split_first()?;
        *buf = rest;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value as i32);
        }
    }
    None
}

pub fn write_string(buf: &mut Vec<u8>, text: &str) {
    write_varint(buf, text.len() as i32);
    buf.extend_from_slice(text.as_bytes());
}

pub fn read_string(buf: &mut &[u8], max_chars: usize) -> Option<String> {
    let length = usize::try_from(read_varint(buf)?).ok()?;
    if length > max_chars * 4 || length > buf.len() {
        return None;
    }
    let (bytes, rest) = buf.split_at(length);
    *buf = rest;
    let text = String::from_utf8(bytes.to_vec()).ok()?;
    (text.chars().count() <= max_chars).then_some(text)
}

pub fn packet(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + 5);
    write_varint(&mut body, id);
    body.extend_from_slice(payload);
    let mut framed = Vec::with_capacity(body.len() + 5);
    write_varint(&mut framed, body.len() as i32);
    framed.extend_from_slice(&body);
    framed
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "malformed packet")
}

async fn read_frame<S: AsyncRead + Unpin>(
    stream: &mut S,
    first: Option<u8>,
    limit: usize,
) -> io::Result<Vec<u8>> {
    let mut length = 0usize;
    for shift in [0, 7, 14] {
        let byte = match first {
            Some(byte) if shift == 0 => byte,
            _ => stream.read_u8().await?,
        };
        length |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            if length == 0 || length > limit {
                return Err(invalid());
            }
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await?;
            return Ok(body);
        }
    }
    Err(invalid())
}

#[derive(Debug, PartialEq)]
pub struct Handshake {
    pub protocol: i32,
    pub next_state: i32,
}

pub fn parse_handshake(body: &[u8]) -> Option<Handshake> {
    let mut buf = body;
    if read_varint(&mut buf)? != 0 {
        return None;
    }
    let protocol = read_varint(&mut buf)?;
    read_string(&mut buf, 255)?;
    buf = buf.get(2..)?;
    let next_state = read_varint(&mut buf)?;
    Some(Handshake {
        protocol,
        next_state,
    })
}

pub fn status_json(status: &Status, protocol: i32) -> String {
    json!({
        "version": { "name": "Sleeping", "protocol": protocol },
        "players": { "max": status.max_players, "online": 0, "sample": [] },
        "description": { "text": format!("{}\n\u{a7}7Sleeping \u{2014} join to start", status.motd) },
    })
    .to_string()
}

async fn linger<S: AsyncRead + Unpin>(stream: &mut S) {
    let mut byte = [0u8; 1];
    let _ = tokio::time::timeout(LINGER, stream.read(&mut byte)).await;
}

pub async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    status: &Status,
) -> io::Result<bool> {
    let first = stream.read_u8().await?;
    if first == 0xFE {
        return Ok(false);
    }
    let body = read_frame(stream, Some(first), MAX_HANDSHAKE).await?;
    let handshake = parse_handshake(&body).ok_or_else(invalid)?;
    match handshake.next_state {
        1 => {
            if read_frame(stream, None, MAX_STATUS_REQUEST).await? != [0] {
                return Ok(false);
            }
            let mut payload = Vec::new();
            write_string(&mut payload, &status_json(status, handshake.protocol));
            stream.write_all(&packet(0, &payload)).await?;
            let ping = read_frame(stream, None, MAX_PING).await?;
            if ping.len() == 9 && ping[0] == 1 {
                stream.write_all(&packet(1, &ping[1..])).await?;
                stream.flush().await?;
                linger(stream).await;
            }
            Ok(false)
        }
        2 | 3 => {
            let _ = tokio::time::timeout(LINGER, read_frame(stream, None, MAX_LOGIN)).await;
            let mut payload = Vec::new();
            write_string(&mut payload, &json!({ "text": KICK }).to_string());
            stream.write_all(&packet(0, &payload)).await?;
            stream.flush().await?;
            linger(stream).await;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn sleeping_port(server_id: &str) -> Option<u16> {
    SLEEPING
        .lock()
        .ok()?
        .get(server_id)
        .map(|listener| listener.port)
}

fn first_motd_line(raw: &str) -> String {
    raw.replace("\\u00A7", "\u{a7}")
        .replace("\\u00a7", "\u{a7}")
        .split("\\n")
        .next()
        .unwrap_or_default()
        .to_string()
}

async fn start(server: &ServerRecord) -> Result<(), String> {
    if is_running(&server.id) || sleeping_port(&server.id).is_some() {
        return Ok(());
    }
    let path = Path::new(&server.server_path);
    let properties = read_properties(path).await;
    let property = |key: &str| {
        properties
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.value.trim().to_string())
    };
    let port = configured_port(path).await;
    let host = property("server-ip")
        .filter(|ip| !ip.is_empty())
        .unwrap_or_else(|| "0.0.0.0".into());
    let status = Status {
        motd: first_motd_line(&property("motd").unwrap_or_else(|| "A Minecraft Server".into())),
        max_players: property("max-players")
            .and_then(|m| m.parse().ok())
            .unwrap_or(20),
    };
    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| format!("Could not listen on port {port}: {e}"))?;
    let Ok(mut sleeping) = SLEEPING.lock() else {
        return Err("Sleep mode is unavailable".into());
    };
    if sleeping.contains_key(&server.id) {
        return Ok(());
    }
    let task = tokio::spawn(listen(server.clone(), listener, Arc::new(status)));
    sleeping.insert(server.id.clone(), Listener { task, port });
    Ok(())
}

async fn listen(server: ServerRecord, listener: TcpListener, status: Arc<Status>) {
    let (wake, mut woken) = mpsc::channel::<()>(1);
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((mut stream, _)) = accepted else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    continue;
                };
                let (status, wake) = (status.clone(), wake.clone());
                tokio::spawn(async move {
                    let _slot = slot;
                    let login = tokio::time::timeout(CONNECTION_TIMEOUT, handle(&mut stream, &status)).await;
                    if let Ok(Ok(true)) = login {
                        let _ = wake.try_send(());
                    }
                });
            }
            _ = woken.recv() => break,
        }
    }
    drop(listener);
    if let Ok(mut sleeping) = SLEEPING.lock() {
        sleeping.remove(&server.id);
    }
    send_console_line(
        &server.id,
        "A player tried to join, waking the server",
        ConsoleLineType::System,
    );
    emit(Event::Notice {
        title: "Server waking up",
        body: format!("Someone is joining {}, so it is starting.", server.name),
    });
    let result = start_server(server.id.clone()).await;
    if let Some(error) = result.error.filter(|_| !result.success) {
        send_console_line(
            &server.id,
            &format!("Could not wake the server: {error}"),
            ConsoleLineType::Stderr,
        );
    }
}

pub async fn stop(server_id: &str) {
    let entry = SLEEPING.lock().ok().and_then(|mut s| s.remove(server_id));
    if let Some(listener) = entry {
        listener.task.abort();
        let _ = listener.task.await;
    }
}

pub async fn stop_all() {
    let ids: Vec<String> = SLEEPING
        .lock()
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default();
    for id in ids {
        stop(&id).await;
    }
}

pub async fn sync(server_id: String) -> Result<(), String> {
    let Some(server) = get_server(server_id.clone()).await else {
        stop(&server_id).await;
        return Ok(());
    };
    let stopped = matches!(
        server.status,
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle
    );
    if automation::of(&server).sleep_mode && stopped {
        start(&server).await
    } else {
        stop(&server_id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn varint(value: i32) -> Vec<u8> {
        let mut buf = Vec::new();
        write_varint(&mut buf, value);
        buf
    }

    #[test]
    fn encodes_varints() {
        let cases: [(i32, &[u8]); 8] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (i32::MAX, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
        ];
        for (value, bytes) in cases {
            assert_eq!(varint(value), bytes, "{value}");
            let mut slice = bytes;
            assert_eq!(read_varint(&mut slice), Some(value));
            assert!(slice.is_empty());
        }
        assert_eq!(read_varint(&mut &[0x80, 0x80][..]), None);
        assert_eq!(read_varint(&mut &[0xff; 6][..]), None);
        assert_eq!(read_varint(&mut &[][..]), None);
    }

    #[test]
    fn encodes_strings() {
        let mut buf = Vec::new();
        write_string(&mut buf, "h\u{e9}llo");
        assert_eq!(buf[0], 6);
        let mut slice = &buf[..];
        assert_eq!(read_string(&mut slice, 255).as_deref(), Some("h\u{e9}llo"));
        assert_eq!(read_string(&mut &buf[..], 3), None);
        assert_eq!(read_string(&mut &[0x05, b'a'][..], 255), None);
        assert_eq!(read_string(&mut &[0x01, 0xff][..], 255), None);
    }

    fn handshake(next_state: i32) -> Vec<u8> {
        let mut payload = varint(767);
        write_string(&mut payload, "play.example.com");
        payload.extend_from_slice(&25565u16.to_be_bytes());
        payload.extend(varint(next_state));
        packet(0, &payload)
    }

    #[test]
    fn parses_handshakes() {
        let framed = handshake(1);
        let mut slice = &framed[..];
        let length = read_varint(&mut slice).unwrap_or_default() as usize;
        assert_eq!(length, slice.len());
        assert_eq!(
            parse_handshake(slice),
            Some(Handshake {
                protocol: 767,
                next_state: 1
            })
        );
        assert_eq!(parse_handshake(&[0x00, 0x01]), None);
        assert_eq!(parse_handshake(&[0x05]), None);
    }

    fn status() -> Status {
        Status {
            motd: "Welcome".into(),
            max_players: 10,
        }
    }

    async fn reply<S: AsyncRead + Unpin>(stream: &mut S) -> (i32, Vec<u8>) {
        let body = read_frame(stream, None, 2 * 1024 * 1024)
            .await
            .unwrap_or_default();
        let mut slice = &body[..];
        let id = read_varint(&mut slice).unwrap_or(-1);
        (id, slice.to_vec())
    }

    #[tokio::test]
    async fn answers_status_and_ping() {
        let (mut client, mut server) = duplex(64 * 1024);
        let task = tokio::spawn(async move { handle(&mut server, &status()).await });
        client.write_all(&handshake(1)).await.unwrap_or_default();
        client.write_all(&packet(0, &[])).await.unwrap_or_default();
        let (id, payload) = reply(&mut client).await;
        assert_eq!(id, 0);
        let mut slice = &payload[..];
        let json: serde_json::Value =
            serde_json::from_str(&read_string(&mut slice, 32767).unwrap_or_default())
                .unwrap_or_default();
        assert_eq!(json["version"]["name"], "Sleeping");
        assert_eq!(json["version"]["protocol"], 767);
        assert_eq!(json["players"]["max"], 10);
        assert!(json["description"]["text"]
            .as_str()
            .is_some_and(|t| t.starts_with("Welcome\n") && t.contains("join to start")));
        client
            .write_all(&packet(1, &42i64.to_be_bytes()))
            .await
            .unwrap_or_default();
        assert_eq!(reply(&mut client).await, (1, 42i64.to_be_bytes().to_vec()));
        drop(client);
        assert!(matches!(task.await, Ok(Ok(false))));
    }

    #[tokio::test]
    async fn kicks_and_wakes_on_login_and_transfer() {
        for state in [2, 3] {
            let (mut client, mut server) = duplex(64 * 1024);
            let task = tokio::spawn(async move { handle(&mut server, &status()).await });
            client
                .write_all(&handshake(state))
                .await
                .unwrap_or_default();
            let mut login = Vec::new();
            write_string(&mut login, "Steve");
            login.extend_from_slice(&[0; 16]);
            client
                .write_all(&packet(0, &login))
                .await
                .unwrap_or_default();
            let (id, payload) = reply(&mut client).await;
            assert_eq!(id, 0);
            let text = read_string(&mut &payload[..], 32767).unwrap_or_default();
            assert!(text.contains("rejoin in about a minute"));
            drop(client);
            assert!(matches!(task.await, Ok(Ok(true))));
        }
    }

    #[tokio::test]
    async fn rejects_legacy_and_malformed_input() {
        let inputs: [&[u8]; 5] = [
            &[0xfe, 0x01],
            &[0xff, 0xff, 0xff, 0x7f],
            &[0x05, 0x00],
            &[0x03, 0x07, 0x00, 0x00],
            &[0x80, 0x10],
        ];
        for input in inputs {
            let (mut client, mut server) = duplex(1024);
            let task = tokio::spawn(async move { handle(&mut server, &status()).await });
            client.write_all(input).await.unwrap_or_default();
            drop(client);
            assert!(!matches!(task.await, Ok(Ok(true))), "{input:?}");
        }
    }

    #[test]
    fn reads_the_first_motd_line() {
        assert_eq!(
            first_motd_line("\\u00A7aHello\\nSecond"),
            "\u{a7}aHello".to_string()
        );
    }
}
