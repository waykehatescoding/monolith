use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::server_store::get_server;

static PROGRESS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"Task (running|finished) for (.+?)\. Processed: (\d+) chunks \(([\d.,]+)%\)(?:, ETA: ([\d:]+), Rate: ([\d.,]+) cps)?",
    )
    .expect("valid regex")
});

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkyProgress {
    pub world: String,
    pub chunks: u64,
    pub percent: f32,
    pub eta: Option<String>,
    pub rate: Option<f32>,
    pub finished: bool,
}

fn number(text: &str) -> Option<f32> {
    text.replace(',', ".").parse().ok()
}

static ANSI: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid regex"));

pub fn parse_progress(line: &str) -> Option<ChunkyProgress> {
    let line = ANSI.replace_all(line, "");
    let caps = PROGRESS.captures(&line)?;
    Some(ChunkyProgress {
        finished: &caps[1] == "finished",
        world: caps[2].to_string(),
        chunks: caps[3].parse().ok()?,
        percent: number(&caps[4])?,
        eta: caps.get(5).map(|m| m.as_str().to_string()),
        rate: caps.get(6).and_then(|m| number(m.as_str())),
    })
}

fn has_chunky(server_path: &Path) -> bool {
    ["plugins", "mods"].iter().any(|folder| {
        std::fs::read_dir(server_path.join(folder))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.starts_with("chunky") && name.ends_with(".jar")
            })
    })
}

pub async fn chunky_installed(server_id: String) -> bool {
    match get_server(server_id).await {
        Some(server) => has_chunky(Path::new(&server.server_path)),
        None => false,
    }
}

pub fn start_commands(world: &str, x: i64, z: i64, radius: u32) -> Vec<String> {
    vec![
        format!("chunky world {world}"),
        format!("chunky center {x} {z}"),
        format!("chunky radius {radius}"),
        "chunky start".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_and_finished_lines() {
        assert_eq!(
            parse_progress("[12:00:00 INFO]: [Chunky] Task running for minecraft:overworld. Processed: 12345 chunks (12.34%), ETA: 0:12:34, Rate: 456.7 cps, Current: 12, -34"),
            Some(ChunkyProgress {
                world: "minecraft:overworld".into(),
                chunks: 12345,
                percent: 12.34,
                eta: Some("0:12:34".into()),
                rate: Some(456.7),
                finished: false,
            })
        );
        assert_eq!(
            parse_progress("[Chunky] Task finished for world_nether. Processed: 40401 chunks (100.00%), Total time: 0:03:10"),
            Some(ChunkyProgress {
                world: "world_nether".into(),
                chunks: 40401,
                percent: 100.0,
                eta: None,
                rate: None,
                finished: true,
            })
        );
        assert_eq!(
            parse_progress("[Chunky] Task running for world. Processed: 5 chunks (0,50%), ETA: 1:02:03, Rate: 12,5 cps, Current: 0, 0")
                .map(|p| (p.percent, p.rate)),
            Some((0.5, Some(12.5)))
        );
        assert_eq!(
            parse_progress("\x1b[0m[Chunky] Task running for my world. Processed: \x1b[1m7\x1b[m chunks (1.00%), ETA: 0:00:09, Rate: 3.0 cps, Current: 0, 0")
                .map(|p| (p.world, p.chunks)),
            Some(("my world".to_string(), 7))
        );
        assert_eq!(parse_progress("[Chunky] Task started for world."), None);
        assert_eq!(parse_progress("<Steve> Task running for me"), None);
    }

    #[test]
    fn builds_start_commands() {
        assert_eq!(
            start_commands("world", -100, 250, 5000),
            vec![
                "chunky world world",
                "chunky center -100 250",
                "chunky radius 5000",
                "chunky start"
            ]
        );
    }
}
