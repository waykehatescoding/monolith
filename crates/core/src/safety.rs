use std::path::{Component, Path, PathBuf};

use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

pub fn get_safe_external_url(input: &str) -> Option<String> {
    let url = url::Url::parse(input).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !path.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        normalize(&std::env::current_dir().unwrap_or_default().join(path))
    }
}

fn starts_with_root(target: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(target).starts_with(lower(root))
    } else {
        target.starts_with(root)
    }
}

pub fn is_path_within(target_path: &Path, root_path: &Path) -> bool {
    starts_with_root(&absolute(target_path), &absolute(root_path))
}

pub fn resolve_safe_server_path(
    server_root: &Path,
    relative_path: &str,
    allow_root: bool,
) -> Result<PathBuf, String> {
    if relative_path.contains('\0') {
        return Err("Invalid path".into());
    }

    let relative = Path::new(relative_path);
    if relative.is_absolute() || relative.has_root() {
        return Err("Absolute paths are not allowed".into());
    }

    let resolved_root = absolute(server_root);
    let resolved_target = normalize(&resolved_root.join(relative));

    if !starts_with_root(&resolved_target, &resolved_root) {
        return Err("Path resolves outside the server directory".into());
    }

    if !allow_root && resolved_target == resolved_root {
        return Err("Refusing to operate on the server root".into());
    }

    Ok(resolved_target)
}

pub fn sanitize_download_file_name(file_name: &str) -> Result<String, String> {
    let invalid = || "Invalid filename".to_string();
    if file_name.is_empty() || file_name.contains('\0') {
        return Err(invalid());
    }

    let trimmed = file_name.trim();
    let bytes = trimmed.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';

    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || has_drive
        || trimmed.encode_utf16().count() > 255
    {
        return Err(invalid());
    }

    Ok(trimmed.to_string())
}

pub fn verify_buffer_hash(
    buffer: &[u8],
    algorithm: HashAlgorithm,
    expected_hash: Option<&str>,
) -> bool {
    let Some(expected) = expected_hash
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
    else {
        return true;
    };

    let actual = match algorithm {
        HashAlgorithm::Sha1 => hex::encode(Sha1::digest(buffer)),
        HashAlgorithm::Sha256 => hex::encode(Sha256::digest(buffer)),
        HashAlgorithm::Sha512 => hex::encode(Sha512::digest(buffer)),
    };
    actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_http_and_https_external_urls() {
        assert_eq!(
            get_safe_external_url("https://example.com/docs").as_deref(),
            Some("https://example.com/docs")
        );
        assert_eq!(
            get_safe_external_url("http://localhost:3000").as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(
            get_safe_external_url("file:///C:/Windows/System32/calc.exe"),
            None
        );
        assert_eq!(get_safe_external_url("javascript:alert(1)"), None);
        assert_eq!(
            get_safe_external_url("data:text/html,<script>alert(1)</script>"),
            None
        );
        assert_eq!(get_safe_external_url("not a url"), None);
    }

    #[test]
    fn resolves_relative_server_paths_safely() {
        let root = absolute(Path::new(if cfg!(windows) {
            "C:/servers/example"
        } else {
            "/servers/example"
        }));
        let err = |r: &str| {
            resolve_safe_server_path(&root, r, false)
                .unwrap_err()
                .to_lowercase()
        };

        assert_eq!(
            resolve_safe_server_path(&root, "world/level.dat", false).unwrap(),
            root.join("world").join("level.dat")
        );
        assert_eq!(
            resolve_safe_server_path(&root, "world", false).unwrap(),
            root.join("world")
        );
        assert_eq!(resolve_safe_server_path(&root, "", true).unwrap(), root);

        assert!(err("").contains("root"));
        assert!(err(".").contains("root"));
        assert!(err("..").contains("outside"));
        assert!(err("../outside").contains("outside"));
        let abs = if cfg!(windows) {
            "C:/outside"
        } else {
            "/outside"
        };
        assert!(err(abs).contains("absolute"));
    }

    #[test]
    fn accepts_only_plain_download_filenames() {
        assert_eq!(
            sanitize_download_file_name("Plugin-1.0.0.jar").unwrap(),
            "Plugin-1.0.0.jar"
        );
        for bad in [
            "",
            ".",
            "../Plugin.jar",
            "plugins/Plugin.jar",
            "C:/temp/Plugin.jar",
        ] {
            assert!(sanitize_download_file_name(bad)
                .unwrap_err()
                .to_lowercase()
                .contains("filename"));
        }
    }

    #[test]
    fn verifies_expected_download_hashes() {
        let content = b"hello";
        assert!(verify_buffer_hash(
            content,
            HashAlgorithm::Sha1,
            Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
        ));
        assert!(!verify_buffer_hash(
            content,
            HashAlgorithm::Sha1,
            Some("0000000000000000000000000000000000000000")
        ));
        assert!(verify_buffer_hash(content, HashAlgorithm::Sha512, None));
    }
}
