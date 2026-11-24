use std::path::PathBuf;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

use crate::server_runner::re;

const UPLOAD_URL: &str = "https://api.mclo.gs/1/log";
const MAX_UPLOAD_LINES: usize = 25_000;

static CODES: Lazy<Regex> = Lazy::new(|| re(r"\x1b\[[0-9;]*[A-Za-z]|§[0-9a-fk-orx]"));

pub fn plain_text(text: &str) -> String {
    CODES.replace_all(text, "").into_owned()
}

#[derive(Deserialize)]
struct UploadReply {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub async fn upload_log(server_path: PathBuf, console: Vec<String>) -> Result<String, String> {
    let content = match tokio::fs::read(server_path.join("logs").join("latest.log")).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => console.join("\n"),
    };
    let lines: Vec<&str> = content.lines().collect();
    let content = plain_text(&lines[lines.len().saturating_sub(MAX_UPLOAD_LINES)..].join("\n"));
    if content.trim().is_empty() {
        return Err("There is no log to upload yet.".into());
    }
    let reply: UploadReply = reqwest::Client::new()
        .post(UPLOAD_URL)
        .form(&[("content", content)])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    match reply {
        UploadReply { url: Some(url), .. } => Ok(url),
        UploadReply { error, .. } => {
            Err(error.unwrap_or_else(|| "mclo.gs rejected the log.".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(plain_text("\x1b[33m§cHello§r world\x1b[0m"), "Hello world");
    }
}
