use std::time::Duration;

use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use url::Url;

use crate::modrinth::CLIENT;

const MAX_PACK_BYTES: u64 = 250 * 1024 * 1024;
const TOO_LARGE: &str = "The pack is larger than 250 MB, which is more than Minecraft accepts";

fn parse(url: &str) -> Result<Url, String> {
    Url::parse(url.trim())
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .ok_or_else(|| "Enter a full link that starts with https://".to_string())
}

pub fn link_warning(url: &str) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    let url = match parse(url) {
        Ok(url) => url,
        Err(error) => return Some(error),
    };
    let host = url.host_str().unwrap_or_default();
    let query = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    if host.ends_with("dropbox.com") && query("dl").as_deref() != Some("1") {
        return Some(
            "Dropbox links open a preview page. Change dl=0 to dl=1, or add ?dl=1, so the zip downloads directly."
                .into(),
        );
    }
    if host == "drive.google.com" && query("export").as_deref() != Some("download") {
        return Some("Google Drive share links open a web page. Use https://drive.google.com/uc?export=download&id=FILE_ID instead.".into());
    }
    None
}

pub async fn pack_sha1(url: String) -> Result<String, String> {
    let url = parse(&url)?;
    let response = CLIENT
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Could not download the pack: {e}"))?;
    if response.content_length().unwrap_or(0) > MAX_PACK_BYTES {
        return Err(TOO_LARGE.into());
    }
    let mut hasher = Sha1::new();
    let mut total = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("The download stopped: {e}"))?;
        total += chunk.len() as u64;
        if total > MAX_PACK_BYTES {
            return Err(TOO_LARGE.into());
        }
        hasher.update(&chunk);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn warns_about_share_links() {
        assert_eq!(link_warning(""), None);
        assert_eq!(link_warning("https://example.com/pack.zip"), None);
        assert!(link_warning("example.com/pack.zip").is_some());
        assert!(link_warning("ftp://example.com/pack.zip").is_some());
        assert!(link_warning("https://www.dropbox.com/s/abc/pack.zip?dl=0").is_some());
        assert_eq!(
            link_warning("https://www.dropbox.com/s/abc/pack.zip?dl=1"),
            None
        );
        assert!(link_warning("https://drive.google.com/file/d/abc/view?usp=sharing").is_some());
        assert_eq!(
            link_warning("https://drive.google.com/uc?export=download&id=abc"),
            None
        );
    }

    async fn serve(response: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            socket.write_all(response).await.unwrap();
        });
        format!("http://{address}/pack.zip")
    }

    #[tokio::test]
    async fn hashes_the_downloaded_pack() {
        let url =
            serve(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc").await;
        assert_eq!(
            pack_sha1(url).await.unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[tokio::test]
    async fn refuses_packs_that_are_too_large() {
        let url =
            serve(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999999\r\nConnection: close\r\n\r\n")
                .await;
        assert_eq!(pack_sha1(url).await, Err(TOO_LARGE.to_string()));
        let missing = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        assert!(pack_sha1(missing).await.is_err());
    }
}
