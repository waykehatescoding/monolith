use std::ops::Range;

fn split_key(content: &str) -> Option<(String, &str)> {
    let (key, rest) = match content.chars().next()? {
        quote @ ('"' | '\'') => {
            let end = content[1..].find(quote)? + 1;
            (
                content[1..end].to_string(),
                content[end + 1..].strip_prefix(':')?,
            )
        }
        _ => {
            let at = content
                .find(": ")
                .or_else(|| content.ends_with(':').then(|| content.len() - 1))?;
            (content[..at].trim_end().to_string(), &content[at + 1..])
        }
    };
    (!key.is_empty() && !key.starts_with('#')).then_some((key, rest))
}

fn value_len(value: &str) -> usize {
    let mut quote = None;
    let mut previous = ' ';
    for (i, c) in value.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if i == 0 && (c == '"' || c == '\'') => quote = Some(c),
            None if c == '#' && previous.is_whitespace() => return value[..i].trim_end().len(),
            None => {}
        }
        previous = c;
    }
    value.trim_end().len()
}

fn locate(text: &str, path: &str) -> Option<Range<usize>> {
    let mut stack: Vec<(usize, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let content = body.trim_start_matches(' ');
        if content.is_empty() || content.starts_with('#') || content.starts_with("---") {
            continue;
        }
        let indent = body.len() - content.len();
        while stack.last().is_some_and(|(level, _)| *level >= indent) {
            stack.pop();
        }
        if content.starts_with('-') {
            stack.push((indent, "-".into()));
            continue;
        }
        let Some((key, rest)) = split_key(content) else {
            continue;
        };
        stack.push((indent, key));
        let joined = stack
            .iter()
            .map(|(_, key)| key.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if joined != path {
            continue;
        }
        let value = rest.trim_start();
        let len = value_len(value);
        if len == 0 || value.starts_with(['|', '>', '&', '*', '[', '{']) {
            return None;
        }
        let at = value.as_ptr() as usize - text.as_ptr() as usize;
        return Some(at..at + len);
    }
    None
}

pub fn yaml_get(text: &str, path: &str) -> Option<String> {
    let raw = &text[locate(text, path)?];
    Some(
        if let Some(inner) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
            inner.replace("''", "'")
        } else if let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            inner.replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            raw.to_string()
        },
    )
}

fn needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value.starts_with([
            ' ', '!', '&', '*', '-', '?', '{', '}', '[', ']', ',', '#', '|', '>', '@', '%', '`',
            '\'', '"',
        ])
        || value.ends_with([' ', ':'])
        || value.contains(": ")
        || value.contains(" #")
}

pub fn yaml_set(text: &str, path: &str, value: &str) -> Option<String> {
    let range = locate(text, path)?;
    let old = &text[range.clone()];
    let formatted = if old.starts_with('"') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else if old.starts_with('\'') || needs_quotes(value) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        value.to_string()
    };
    let mut result = text.to_string();
    result.replace_range(range, &formatted);
    Some(result)
}

pub fn validate(name: &str, text: &str) -> Result<(), (usize, String)> {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("json" | "mcmeta") => serde_json::from_str::<serde_json::Value>(text)
            .map(|_| ())
            .map_err(|e| {
                let message = e.to_string();
                let message = message
                    .rsplit_once(" at line ")
                    .map_or(message.as_str(), |(m, _)| m);
                (e.line(), message.to_string())
            }),
        Some("toml") => toml::de::DeTable::parse(text).map(|_| ()).map_err(|e| {
            let at = e.span().map_or(0, |span| span.start).min(text.len());
            (
                text[..at].matches('\n').count() + 1,
                e.message().to_string(),
            )
        }),
        Some("yml" | "yaml") => {
            for event in saphyr_parser::Parser::new_from_str(text) {
                if let Err(e) = event {
                    return Err((e.marker().line(), e.info().to_string()));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: [&str; 19] = [
        "# Paper config",
        "proxies:",
        "  velocity:",
        "    enabled: false # turn on behind velocity",
        "    secret: ''",
        "  bungee-cord:",
        "    online-mode: true",
        "spawn-limits:",
        "  monsters: 70",
        "aliases:",
        "  - icanhasbukkit",
        "  - name: inner",
        "    enabled: true",
        "messages:",
        r#"  whitelist: "You are not \"whitelisted\"""#,
        "  motd: |",
        "    hello",
        "enabled: top",
        "",
    ];

    #[test]
    fn reads_nested_scalars() {
        let sample = &LINES.join("\r\n");
        assert_eq!(
            yaml_get(sample, "proxies.velocity.enabled").as_deref(),
            Some("false")
        );
        assert_eq!(
            yaml_get(sample, "proxies.velocity.secret").as_deref(),
            Some("")
        );
        assert_eq!(
            yaml_get(sample, "proxies.bungee-cord.online-mode").as_deref(),
            Some("true")
        );
        assert_eq!(
            yaml_get(sample, "spawn-limits.monsters").as_deref(),
            Some("70")
        );
        assert_eq!(
            yaml_get(sample, "messages.whitelist").as_deref(),
            Some("You are not \"whitelisted\"")
        );
        assert_eq!(yaml_get(sample, "enabled").as_deref(), Some("top"));
        assert_eq!(yaml_get(sample, "proxies.velocity"), None);
        assert_eq!(yaml_get(sample, "messages.motd"), None);
        assert_eq!(yaml_get(sample, "aliases.enabled"), None);
        assert_eq!(yaml_get(sample, "missing.key"), None);
    }

    #[test]
    fn writes_keep_comments_and_layout() {
        let sample = &LINES.join("\r\n");
        let out = yaml_set(sample, "proxies.velocity.enabled", "true").unwrap();
        assert!(out.contains("    enabled: true # turn on behind velocity\r\n"));
        assert_eq!(out.len(), sample.len() - 1);
        let out = yaml_set(&out, "proxies.velocity.secret", "it's s3cret").unwrap();
        assert!(out.contains("    secret: 'it''s s3cret'\r\n"));
        assert_eq!(
            yaml_get(&out, "proxies.velocity.secret").as_deref(),
            Some("it's s3cret")
        );
        let out = yaml_set(&out, "messages.whitelist", "Say \"hi\"").unwrap();
        assert_eq!(
            yaml_get(&out, "messages.whitelist").as_deref(),
            Some("Say \"hi\"")
        );
        let out = yaml_set(&out, "spawn-limits.monsters", "").unwrap();
        assert!(out.contains("  monsters: ''\r\n"));
        assert!(out.starts_with("# Paper config\r\nproxies:\r\n"));
        assert_eq!(yaml_set(sample, "nope", "1"), None);
    }

    #[test]
    fn validates_by_extension() {
        assert!(validate("a.json", "{\"a\": [1, 2]}").is_ok());
        assert_eq!(validate("a.json", "{\n  \"a\": 1,\n}").unwrap_err().0, 3);
        assert!(validate("a.toml", "[server]\nport = 25565\n").is_ok());
        assert_eq!(validate("a.toml", "a = 1\nb = = 2\n").unwrap_err().0, 2);
        assert!(validate("a.yml", "a:\n  b: [1, 2]\n").is_ok());
        let (line, message) = validate("config.yaml", "a: 1\nb: [1, 2\nc: 3\n").unwrap_err();
        assert!(line >= 2 && !message.is_empty());
        assert!(validate("notes.txt", "{{{").is_ok());
    }
}
