use serde::Serialize;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CHUNK_SIZE: u64 = 256 * 1024;
const SMALL_READ_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingFileReadResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more_lines: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingFileWriteResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub size: u64,
    pub line_count: u64,
    pub estimated_chunks: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformFileResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_processed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn count_file_lines(file_path: &Path) -> Result<u64, String> {
    let mut reader = BufReader::new(File::open(file_path).await.map_err(|e| e.to_string())?);
    let mut buf = Vec::new();
    let mut count = 0;
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += 1;
    }
    Ok(count)
}

pub async fn get_file_metadata(file_path: &Path) -> Result<FileMetadata, String> {
    let wrap = |e: String| format!("Failed to get file metadata: {e}");
    let size = tokio::fs::metadata(file_path)
        .await
        .map_err(|e| wrap(e.to_string()))?
        .len();
    let line_count = count_file_lines(file_path).await.map_err(wrap)?;
    Ok(FileMetadata {
        size,
        line_count,
        estimated_chunks: size.div_ceil(CHUNK_SIZE),
    })
}

fn read_failure(error: String) -> StreamingFileReadResult {
    StreamingFileReadResult {
        success: false,
        error: Some(error),
        ..Default::default()
    }
}

pub async fn stream_read_file(
    file_path: &Path,
    start_line: u64,
    max_lines: u64,
) -> StreamingFileReadResult {
    let meta = match tokio::fs::metadata(file_path).await {
        Ok(m) => m,
        Err(e) => return read_failure(format!("Error reading file: {e}")),
    };
    if meta.is_dir() {
        return read_failure("Cannot read a directory".into());
    }

    if meta.len() <= SMALL_READ_LIMIT {
        let bytes = match tokio::fs::read(file_path).await {
            Ok(b) => b,
            Err(e) => return read_failure(format!("Error reading file: {e}")),
        };
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = content.split('\n').collect();
        let total = lines.len() as u64;
        let end = start_line.saturating_add(max_lines).min(total);
        let from = start_line.min(end) as usize;
        return StreamingFileReadResult {
            success: true,
            content: Some(lines[from..end as usize].join("\n")),
            total_lines: Some(total),
            has_more_lines: Some(end < total),
            start_line: Some(start_line),
            end_line: Some(end),
            error: None,
        };
    }

    let file = match File::open(file_path).await {
        Ok(f) => f,
        Err(e) => return read_failure(format!("Failed to read file: {e}")),
    };
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut collected: Vec<String> = Vec::new();
    let mut total_lines = 0u64;
    let mut index = 0u64;
    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf).await {
            Ok(n) => n,
            Err(e) => return read_failure(format!("Failed to read file: {e}")),
        };
        if n == 0 {
            break;
        }
        if buf.last() == Some(&b'\n') {
            total_lines += 1;
            buf.pop();
        }
        if index >= start_line && (collected.len() as u64) < max_lines {
            collected.push(String::from_utf8_lossy(&buf).into_owned());
        }
        index += 1;
    }
    let taken = collected.len() as u64;
    StreamingFileReadResult {
        success: true,
        content: Some(collected.join("\n")),
        total_lines: Some(total_lines),
        has_more_lines: Some(start_line + taken < total_lines),
        start_line: Some(start_line),
        end_line: Some(start_line + taken),
        error: None,
    }
}

pub async fn stream_write_file(
    file_path: &Path,
    content: &str,
    append: bool,
) -> StreamingFileWriteResult {
    let failure = |error: String| StreamingFileWriteResult {
        success: false,
        bytes_written: None,
        error: Some(error),
    };
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    let mut file = match options.open(file_path).await {
        Ok(f) => f,
        Err(e) => return failure(format!("Error writing file: {e}")),
    };
    let small = content.encode_utf16().count() <= 5 * 1024 * 1024;
    let result = async {
        for chunk in content.as_bytes().chunks(CHUNK_SIZE as usize) {
            file.write_all(chunk).await?;
        }
        file.flush().await
    }
    .await;
    match result {
        Ok(()) => StreamingFileWriteResult {
            success: true,
            bytes_written: Some(content.len() as u64),
            error: None,
        },
        Err(e) if small => failure(format!("Error writing file: {e}")),
        Err(e) => failure(format!("Failed to write file: {e}")),
    }
}

pub async fn transform_file<F>(file_path: &Path, transform: F) -> TransformFileResult
where
    F: Fn(&str, u64) -> String + Send + 'static,
{
    let fail = |error: String| TransformFileResult {
        success: false,
        lines_processed: None,
        error: Some(error),
    };
    let mut temp_name = file_path.as_os_str().to_owned();
    temp_name.push(".tmp");
    let temp_path = std::path::PathBuf::from(temp_name);

    let input = match File::open(file_path).await {
        Ok(f) => f,
        Err(e) => return fail(format!("Read error: {e}")),
    };
    let output = match File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => return fail(format!("Write error: {e}")),
    };
    let mut reader = BufReader::new(input);
    let mut writer = tokio::io::BufWriter::new(output);
    let mut buf = Vec::new();
    let mut processed = 0u64;

    let run = async {
        loop {
            buf.clear();
            let n = reader
                .read_until(b'\n', &mut buf)
                .await
                .map_err(|e| format!("Read error: {e}"))?;
            if n == 0 {
                break;
            }
            let terminated = buf.last() == Some(&b'\n');
            if terminated {
                buf.pop();
            }
            let mut out = transform(&String::from_utf8_lossy(&buf), processed);
            processed += 1;
            if terminated {
                out.push('\n');
            }
            writer
                .write_all(out.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
        }
        writer
            .flush()
            .await
            .map_err(|e| format!("Write error: {e}"))
    }
    .await;
    drop(writer);

    if let Err(e) = run {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return fail(e);
    }
    if let Err(e) = tokio::fs::rename(&temp_path, file_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return fail(format!("Failed to save file: {e}"));
    }
    TransformFileResult {
        success: true,
        lines_processed: Some(processed),
        error: None,
    }
}
