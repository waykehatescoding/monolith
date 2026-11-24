use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(4);
const MAX_PACKET: usize = 1 << 21;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerStatus {
    pub version: String,
    pub online: u32,
    pub max: u32,
    pub motd: String,
    pub favicon: Option<Vec<u8>>,
    pub ping_ms: u32,
}

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
}

fn read_varint(bytes: &[u8]) -> Option<(i32, usize)> {
    let mut value = 0u32;
    for (index, byte) in bytes.iter().take(5).enumerate() {
        value |= ((byte & 0x7f) as u32) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value as i32, index + 1));
        }
    }
    None
}

fn packet(id: i32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::new();
    write_varint(&mut inner, id);
    inner.extend_from_slice(body);
    let mut out = Vec::new();
    write_varint(&mut out, inner.len() as i32);
    out.extend(inner);
    out
}

fn handshake(host: &str, port: u16) -> Vec<u8> {
    let mut body = Vec::new();
    write_varint(&mut body, -1);
    write_varint(&mut body, host.len() as i32);
    body.extend_from_slice(host.as_bytes());
    body.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut body, 1);
    packet(0, &body)
}

async fn read_packet(stream: &mut TcpStream) -> Result<(i32, Vec<u8>), String> {
    let mut header = Vec::new();
    let length = loop {
        let byte = stream.read_u8().await.map_err(|e| e.to_string())?;
        header.push(byte);
        if byte & 0x80 == 0 {
            break read_varint(&header).ok_or("Bad packet length")?.0 as usize;
        }
        if header.len() >= 5 {
            return Err("Bad packet length".into());
        }
    };
    if length == 0 || length > MAX_PACKET {
        return Err("Bad packet length".into());
    }
    let mut data = vec![0; length];
    stream
        .read_exact(&mut data)
        .await
        .map_err(|e| e.to_string())?;
    let (id, used) = read_varint(&data).ok_or("Bad packet id")?;
    Ok((id, data[used..].to_vec()))
}

fn read_string(body: &[u8]) -> Option<String> {
    let (length, used) = read_varint(body)?;
    let text = body.get(used..used + usize::try_from(length).ok()?)?;
    String::from_utf8(text.to_vec()).ok()
}

async fn exchange(host: &str, port: u16) -> Result<(Value, u32), String> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| e.to_string())?;
    let mut request = handshake(host, port);
    request.extend(packet(0, &[]));
    stream
        .write_all(&request)
        .await
        .map_err(|e| e.to_string())?;
    let (id, body) = read_packet(&mut stream).await?;
    if id != 0 {
        return Err("The server sent an unexpected reply.".into());
    }
    let json = read_string(&body).ok_or("The server sent an unreadable reply.")?;
    let status = serde_json::from_str(&json).map_err(|e| e.to_string())?;

    let token = chrono::Utc::now().timestamp_millis();
    let sent = Instant::now();
    stream
        .write_all(&packet(1, &token.to_be_bytes()))
        .await
        .map_err(|e| e.to_string())?;
    let ping = match read_packet(&mut stream).await {
        Ok((1, _)) => sent.elapsed().as_millis() as u32,
        _ => 0,
    };
    Ok((status, ping))
}

pub async fn ping(host: String, port: u16) -> Result<ServerStatus, String> {
    let (status, ping_ms) = tokio::time::timeout(TIMEOUT, exchange(&host, port))
        .await
        .map_err(|_| "The server did not answer in time.".to_string())??;
    Ok(parse_status(&status, ping_ms))
}

fn parse_status(status: &Value, ping_ms: u32) -> ServerStatus {
    let count = |key: &str| status["players"][key].as_u64().unwrap_or(0) as u32;
    ServerStatus {
        version: status["version"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        online: count("online"),
        max: count("max"),
        motd: component_to_legacy(&status["description"]),
        favicon: status["favicon"]
            .as_str()
            .and_then(|uri| uri.strip_prefix("data:image/png;base64,"))
            .and_then(decode_base64),
        ping_ms,
    }
}

fn color_code(name: &str) -> Option<String> {
    const NAMES: [&str; 16] = [
        "black",
        "dark_blue",
        "dark_green",
        "dark_aqua",
        "dark_red",
        "dark_purple",
        "gold",
        "gray",
        "dark_gray",
        "blue",
        "green",
        "aqua",
        "red",
        "light_purple",
        "yellow",
        "white",
    ];
    if let Some(index) = NAMES.iter().position(|&n| n == name) {
        return Some(format!("§{index:x}"));
    }
    let hex = name
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))?;
    Some(hex.chars().fold("§x".to_string(), |mut out, c| {
        out.push('§');
        out.push(c);
        out
    }))
}

fn flatten(value: &Value, color: Option<&str>, bold: bool, out: &mut String) {
    match value {
        Value::String(text) => {
            out.push_str("§r");
            if let Some(code) = color.and_then(color_code) {
                out.push_str(&code);
            }
            if bold {
                out.push_str("§l");
            }
            out.push_str(text);
        }
        Value::Array(items) => {
            for item in items {
                flatten(item, color, bold, out);
            }
        }
        Value::Object(map) => {
            let color = map.get("color").and_then(Value::as_str).or(color);
            let bold = map.get("bold").and_then(Value::as_bool).unwrap_or(bold);
            if let Some(text) = map.get("text") {
                flatten(text, color, bold, out);
            }
            if let Some(extra) = map.get("extra") {
                flatten(extra, color, bold, out);
            }
        }
        _ => {}
    }
}

pub fn component_to_legacy(value: &Value) -> String {
    if let Value::String(text) = value {
        return text.clone();
    }
    let mut out = String::new();
    flatten(value, None, false, &mut out);
    out.strip_prefix("§r").map(str::to_string).unwrap_or(out)
}

pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' => continue,
            _ => return None,
        };
        buffer = buffer << 6 | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn varints_round_trip() {
        for (value, bytes) in [
            (0, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7f]),
            (128, vec![0x80, 0x01]),
            (25565, vec![0xdd, 0xc7, 0x01]),
            (-1, vec![0xff, 0xff, 0xff, 0xff, 0x0f]),
        ] {
            let mut out = Vec::new();
            write_varint(&mut out, value);
            assert_eq!(out, bytes);
            assert_eq!(read_varint(&bytes), Some((value, bytes.len())));
        }
        assert_eq!(read_varint(&[0x80, 0x80]), None);
    }

    #[test]
    fn handshake_is_length_prefixed_with_status_intent() {
        let bytes = handshake("127.0.0.1", 25565);
        let (length, used) = read_varint(&bytes).unwrap();
        assert_eq!(length as usize, bytes.len() - used);
        assert_eq!(bytes[used], 0x00);
        assert_eq!(&bytes[bytes.len() - 3..], &[0x63, 0xdd, 0x01]);
    }

    #[test]
    fn reads_length_prefixed_strings() {
        assert_eq!(read_string(&[3, b'a', b'b', b'c']), Some("abc".into()));
        assert_eq!(read_string(&[5, b'a']), None);
    }

    #[test]
    fn json_components_become_legacy_codes() {
        let motd = json!({
            "text": "",
            "extra": [
                { "text": "Hello ", "color": "gold", "bold": true },
                { "text": "world", "color": "#ff0000" },
                "!"
            ]
        });
        assert_eq!(
            component_to_legacy(&motd),
            "§r§6§lHello §r§x§f§f§0§0§0§0world§r!"
        );
        assert_eq!(component_to_legacy(&json!("§aPlain")), "§aPlain");
        assert_eq!(
            component_to_legacy(&json!({ "text": "A", "color": "red", "extra": ["B"] })),
            "§cA§r§cB"
        );
    }

    #[test]
    fn parses_status_response() {
        let status = json!({
            "version": { "name": "Paper 1.21.4", "protocol": 769 },
            "players": { "max": 20, "online": 3 },
            "description": "A server",
            "favicon": "data:image/png;base64,iVBORw0KGgo="
        });
        let parsed = parse_status(&status, 12);
        assert_eq!(parsed.version, "Paper 1.21.4");
        assert_eq!((parsed.online, parsed.max, parsed.ping_ms), (3, 20, 12));
        assert_eq!(parsed.motd, "A server");
        assert_eq!(
            parsed.favicon.as_deref(),
            Some(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..])
        );
    }

    #[test]
    fn base64_decodes_and_rejects_garbage() {
        assert_eq!(decode_base64("TWFu"), Some(b"Man".to_vec()));
        assert_eq!(decode_base64("TWE="), Some(b"Ma".to_vec()));
        assert_eq!(decode_base64("TQ=="), Some(b"M".to_vec()));
        assert_eq!(decode_base64("T*=="), None);
    }

    #[tokio::test]
    async fn pings_a_local_status_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_packet_from(&mut socket).await;
            read_packet_from(&mut socket).await;
            let json = r#"{"version":{"name":"1.21"},"players":{"max":5,"online":1},"description":{"text":"Hi"}}"#;
            let mut body = Vec::new();
            write_varint(&mut body, json.len() as i32);
            body.extend_from_slice(json.as_bytes());
            socket.write_all(&packet(0, &body)).await.unwrap();
            let ping = read_packet_from(&mut socket).await;
            socket.write_all(&packet(1, &ping)).await.unwrap();
        });
        let status = ping("127.0.0.1".into(), port).await.unwrap();
        assert_eq!(status.version, "1.21");
        assert_eq!((status.online, status.max), (1, 5));
        assert_eq!(status.motd, "Hi");
    }

    async fn read_packet_from(socket: &mut TcpStream) -> Vec<u8> {
        read_packet(socket).await.unwrap().1
    }
}
