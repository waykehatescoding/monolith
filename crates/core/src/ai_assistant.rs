use std::path::Path;

use serde_json::{json, Value};

use crate::ai_settings::read_ai_assistant_settings;
use crate::server_files::{read_server_file, write_server_file};
use crate::server_props::{read_properties, write_properties};
use crate::server_runner::{get_server_logs, is_running, send_command};
use crate::server_store::{get_server, update_server_status};
use crate::types::{
    AiAssistantAction, AiAssistantActionKind, AiAssistantActionResult, AiAssistantChatResult,
    AiAssistantClientContext, AiAssistantMessage, AiAssistantProvider, ConsoleLine, ServerProperty,
};

const MAX_MESSAGES: usize = 16;
const MAX_LOG_LINES: usize = 80;

const ALLOWED_SERVER_PROPERTY_ACTION_KEYS: &[&str] = &[
    "max-players",
    "view-distance",
    "simulation-distance",
    "motd",
    "difficulty",
    "pvp",
    "white-list",
    "enforce-whitelist",
    "spawn-protection",
];

const CONTEXT_PROPERTY_KEYS: &[&str] = &[
    "max-players",
    "server-port",
    "difficulty",
    "gamemode",
    "online-mode",
    "white-list",
    "enforce-whitelist",
    "view-distance",
    "simulation-distance",
    "motd",
    "pvp",
    "spawn-protection",
];

const CONFIG_FILE_EXTENSIONS: &[&str] = &[
    ".yml",
    ".yaml",
    ".json",
    ".toml",
    ".properties",
    ".conf",
    ".cfg",
    ".txt",
];

struct PromptServer {
    id: String,
    name: String,
    framework: String,
    version: String,
    status: String,
    ram_mb: u32,
    players: String,
}

#[derive(Default)]
struct PromptContext {
    locale: Option<String>,
    route: String,
    page_title: String,
    server: Option<PromptServer>,
    logs: Vec<ConsoleLine>,
    properties: Vec<ServerProperty>,
}

#[derive(serde::Deserialize)]
pub struct AiChatRequest {
    messages: Vec<Value>,
    context: AiAssistantClientContext,
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

fn truncate_utf16(s: &str, max: usize) -> String {
    String::from_utf16_lossy(&s.encode_utf16().take(max).collect::<Vec<u16>>())
}

fn non_empty_error(error: Option<String>, fallback: &str) -> String {
    error
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

struct ParsedResponse {
    reply: String,
    actions: Vec<AiAssistantAction>,
}

fn provider_label(provider: AiAssistantProvider) -> &'static str {
    match provider {
        AiAssistantProvider::Openai => "OpenAI",
        AiAssistantProvider::Openrouter => "OpenRouter",
        AiAssistantProvider::Groq => "Groq",
        AiAssistantProvider::Custom => "Custom OpenAI-compatible",
    }
}

fn provider_requires_api_key(provider: AiAssistantProvider) -> bool {
    provider != AiAssistantProvider::Custom
}

fn normalize_messages(messages: &[Value]) -> Vec<AiAssistantMessage> {
    let valid: Vec<AiAssistantMessage> = messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role")?.as_str()?;
            let content = m.get("content")?.as_str()?;
            ((role == "user" || role == "assistant") && !js_trim(content).is_empty()).then(|| {
                AiAssistantMessage {
                    role: role.to_string(),
                    content: content.to_string(),
                }
            })
        })
        .collect();
    let start = valid.len().saturating_sub(MAX_MESSAGES);
    valid[start..]
        .iter()
        .map(|m| AiAssistantMessage {
            role: m.role.clone(),
            content: truncate_utf16(js_trim(&m.content), 6000),
        })
        .collect()
}

fn js_number_string(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
        _ => n.to_string(),
    }
}

fn normalize_action_value(key: &str, value: Option<&Value>) -> Option<String> {
    let raw = match value? {
        Value::String(s) => s.clone(),
        Value::Number(n) => js_number_string(n),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    let raw = js_trim(&raw).to_string();
    if raw.is_empty() || raw.contains(['\r', '\n']) || raw.encode_utf16().count() > 160 {
        return None;
    }

    if key == "max-players" {
        let count: f64 = raw.parse().ok()?;
        if count.fract() != 0.0 || !(1.0..=100000.0).contains(&count) {
            return None;
        }
        return Some((count as u64).to_string());
    }

    Some(raw)
}

fn normalize_config_relative_path(value: Option<&Value>) -> Option<String> {
    let normalized = js_trim(value?.as_str()?).replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || has_drive_prefix(&normalized)
        || normalized
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return None;
    }

    let lower = normalized.to_lowercase();
    let is_config_dir =
        lower.starts_with("plugins/") || lower.starts_with("mods/") || lower.starts_with("config/");
    let extension = lower.rfind('.').map(|i| &lower[i..]).unwrap_or("");

    if !is_config_dir || !CONFIG_FILE_EXTENSIONS.contains(&extension) {
        return None;
    }
    Some(normalized)
}

fn normalize_config_content(value: Option<&Value>) -> Option<String> {
    let content = value?.as_str()?;
    if content.contains('\0') || content.len() > 120_000 {
        return None;
    }
    Some(content.to_string())
}

fn normalize_server_command(value: Option<&Value>) -> Option<String> {
    let trimmed = js_trim(value?.as_str()?);
    let command = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if command.is_empty() || command.contains(['\r', '\n']) || command.encode_utf16().count() > 200
    {
        return None;
    }
    Some(command.to_string())
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn parse_online_count(players: &str) -> Option<u64> {
    let (online, max) = players.split_once('/')?;
    let online = online.trim_end();
    let max = max.trim_start();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(online) && digits(max) {
        online.parse().ok()
    } else {
        None
    }
}

fn update_player_capacity_string(current_players: Option<&str>, max_players: &str) -> String {
    let max = match max_players.trim().parse::<f64>() {
        Ok(m) if m.fract() == 0.0 && m >= 1.0 => m as u64,
        _ => {
            return match current_players {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => "0/20".to_string(),
            }
        }
    };

    let online = current_players.and_then(parse_online_count).unwrap_or(0);
    format!("{online}/{max}")
}

fn normalize_assistant_actions(value: &Value, server_id: Option<&str>) -> Vec<AiAssistantAction> {
    let (Some(items), Some(server_id)) = (value.as_array(), server_id.filter(|s| !s.is_empty()))
    else {
        return Vec::new();
    };

    let mut actions = Vec::new();
    for item in items {
        let Some(candidate) = item.as_object() else {
            continue;
        };
        if candidate.get("serverId").and_then(Value::as_str) != Some(server_id) {
            continue;
        }

        match candidate.get("kind").and_then(Value::as_str) {
            Some("setServerProperty") => {
                let Some(key) = candidate
                    .get("key")
                    .and_then(Value::as_str)
                    .filter(|k| ALLOWED_SERVER_PROPERTY_ACTION_KEYS.contains(k))
                else {
                    continue;
                };
                let Some(value) = normalize_action_value(key, candidate.get("value")) else {
                    continue;
                };
                actions.push(AiAssistantAction {
                    id: format!("setServerProperty:{key}:{value}"),
                    server_id: server_id.to_string(),
                    label: format!("Set {key} to {value}"),
                    requires_confirmation: true,
                    kind: AiAssistantActionKind::SetServerProperty {
                        key: key.to_string(),
                        value,
                    },
                });
            }
            Some("writeServerFile") => {
                let Some(relative_path) =
                    normalize_config_relative_path(candidate.get("relativePath"))
                else {
                    continue;
                };
                let Some(content) = normalize_config_content(candidate.get("content")) else {
                    continue;
                };
                actions.push(AiAssistantAction {
                    id: format!("writeServerFile:{relative_path}"),
                    server_id: server_id.to_string(),
                    label: format!("Update {relative_path}"),
                    requires_confirmation: true,
                    kind: AiAssistantActionKind::WriteServerFile {
                        relative_path,
                        content,
                    },
                });
            }
            Some("sendServerCommand") => {
                let Some(command) = normalize_server_command(candidate.get("command")) else {
                    continue;
                };
                actions.push(AiAssistantAction {
                    id: format!("sendServerCommand:{command}"),
                    server_id: server_id.to_string(),
                    label: format!("Run {command}"),
                    requires_confirmation: true,
                    kind: AiAssistantActionKind::SendServerCommand { command },
                });
            }
            _ => {}
        }
    }
    actions
}

fn strip_json_fence(value: &str) -> String {
    let mut text = value.trim();
    if text.get(..3).is_some_and(|p| p == "```") {
        text = &text[3..];
        if text
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("json"))
        {
            text = &text[4..];
        }
        text = text.trim_start();
    }
    let text = text.trim_end();
    text.strip_suffix("```").unwrap_or(text).trim().to_string()
}

fn parse_assistant_model_response(raw_content: &str, server_id: Option<&str>) -> ParsedResponse {
    let content = raw_content.trim();
    if content.is_empty() {
        return ParsedResponse {
            reply: "I could not generate a response. Please try again.".to_string(),
            actions: Vec::new(),
        };
    }

    if let Ok(parsed) = serde_json::from_str::<Value>(&strip_json_fence(content)) {
        if let Some(reply) = parsed.get("reply").and_then(Value::as_str).map(str::trim) {
            if !reply.is_empty() {
                let actions = parsed
                    .get("actions")
                    .map(|a| normalize_assistant_actions(a, server_id))
                    .unwrap_or_default();
                return ParsedResponse {
                    reply: reply.to_string(),
                    actions,
                };
            }
        }
    }

    ParsedResponse {
        reply: content.to_string(),
        actions: Vec::new(),
    }
}

fn format_server_context(server: &PromptServer) -> String {
    [
        format!("id={}", server.id),
        format!("name={}", server.name),
        format!("framework={}", server.framework),
        format!("version={}", server.version),
        format!("status={}", server.status),
        format!("ramMB={}", server.ram_mb),
        format!("players={}", server.players),
    ]
    .join(", ")
}

fn build_ai_assistant_system_prompt(context: &PromptContext) -> String {
    let locale = context
        .locale
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or("unknown");
    let mut lines: Vec<String> = vec![
        "You are the optional AI assistant inside Monolith, a desktop Minecraft server manager.".into(),
        "Only answer questions related to Monolith or Minecraft server management. For unrelated topics, briefly say you can only help with Monolith/Minecraft.".into(),
        "Your job is to explain the current page, explain settings, diagnose server problems, suggest safe changes, edit plugin/mod config files, and propose Minecraft server commands.".into(),
        "Match the language of the latest user message. Do not use locale as the main language signal unless the message language is unclear.".into(),
        "Keep every reply short, casual, relaxed, and useful. Aim for 1-4 short sentences unless important detail would be lost.".into(),
        "Use Markdown in reply text: bullets, inline code, and short code blocks when helpful.".into(),
        format!("Locale: {locale}"),
        format!("Current route: {}", context.route),
        format!("Current page: {}", context.page_title),
    ];

    if let Some(server) = &context.server {
        lines.push(format!("Current server: {}", format_server_context(server)));
    }

    if !context.properties.is_empty() {
        lines.push("Relevant server.properties:".into());
        lines.extend(
            context
                .properties
                .iter()
                .map(|p| format!("{}={}", p.key, p.value)),
        );
    }

    if !context.logs.is_empty() {
        lines.push("Recent console logs:".into());
        lines.extend(context.logs.iter().map(|l| {
            let kind = serde_json::to_value(l.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            format!("[{kind}] {}", l.text)
        }));
    }

    lines.extend([
        "Never claim that you changed files or settings unless you return an allowed action and the user applies it.".to_string(),
        "Every action requires user confirmation in Monolith before it is applied.".to_string(),
        "Allowed server.properties action: {\"kind\":\"setServerProperty\",\"serverId\":\"...\",\"key\":\"max-players\",\"value\":\"40\",\"label\":\"Set max players to 40\"}.".to_string(),
        "Allowed plugin/mod config action: {\"kind\":\"writeServerFile\",\"serverId\":\"...\",\"relativePath\":\"plugins/Plugin/config.yml\",\"content\":\"...\",\"label\":\"Update config\"}.".to_string(),
        "Allowed Minecraft command action: {\"kind\":\"sendServerCommand\",\"serverId\":\"...\",\"command\":\"say Restart in 5 minutes\",\"label\":\"Announce restart\"}. Commands are sent only after user confirmation.".to_string(),
        "Only these server.properties keys can be actioned: max-players, view-distance, simulation-distance, motd, difficulty, pvp, white-list, enforce-whitelist, spawn-protection.".to_string(),
        "Plugin/mod config edits are limited to config-like files under plugins/, mods/, or config/.".to_string(),
        "Return JSON only in this shape: {\"reply\":\"Markdown response text\",\"actions\":[]}.".to_string(),
        "If no action is needed, return an empty actions array.".to_string(),
    ]);

    lines.join("\n")
}

async fn build_prompt_context(request: &AiChatRequest) -> PromptContext {
    let client = &request.context;
    let mut context = PromptContext {
        locale: client.locale.clone(),
        route: client.route.clone(),
        page_title: client.page_title.clone(),
        ..Default::default()
    };

    let Some(server_id) = client.server_id.as_deref().filter(|s| !s.is_empty()) else {
        return context;
    };
    let Some(server) = get_server(server_id.to_string()).await else {
        return context;
    };

    let status = serde_json::to_value(&server.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    context.server = Some(PromptServer {
        id: server.id.clone(),
        name: server.name.clone(),
        framework: server.framework.clone(),
        version: server.version.clone(),
        status,
        ram_mb: server.ram_mb,
        players: server.players.clone(),
    });

    let logs = get_server_logs(server.id.clone()).await;
    let start = logs.len().saturating_sub(MAX_LOG_LINES);
    context.logs = logs[start..].to_vec();
    context.properties = read_properties(Path::new(&server.server_path))
        .await
        .into_iter()
        .filter(|p| CONTEXT_PROPERTY_KEYS.contains(&p.key.as_str()))
        .map(|p| ServerProperty {
            key: p.key,
            value: p.value,
            comment: None,
        })
        .collect();

    context
}

fn get_error_message(payload: &Value, fallback: String) -> String {
    match payload.get("error") {
        Some(Value::String(s)) => return s.clone(),
        Some(Value::Object(o)) => {
            if let Some(Value::String(m)) = o.get("message") {
                return m.clone();
            }
        }
        _ => {}
    }
    match payload.get("message") {
        Some(Value::String(m)) => m.clone(),
        _ => fallback,
    }
}

fn read_assistant_content(payload: &Value) -> Option<String> {
    let first = payload.get("choices")?.get(0)?;
    if let Some(content) = first
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
    {
        return Some(content.to_string());
    }
    first
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn chat_error(message: impl Into<String>) -> AiAssistantChatResult {
    AiAssistantChatResult {
        success: false,
        error: Some(message.into()),
        ..Default::default()
    }
}

fn action_error(message: impl Into<String>) -> AiAssistantActionResult {
    AiAssistantActionResult {
        success: false,
        error: Some(message.into()),
        ..Default::default()
    }
}

pub async fn send_ai_assistant_message(
    request: AiChatRequest,
) -> Result<AiAssistantChatResult, String> {
    let settings = read_ai_assistant_settings().await;
    if !settings.enabled {
        return Ok(chat_error("AI assistant is disabled."));
    }

    if provider_requires_api_key(settings.provider) && settings.api_key.is_empty() {
        return Ok(chat_error(format!(
            "{} needs an API key first.",
            provider_label(settings.provider)
        )));
    }

    let messages = normalize_messages(&request.messages);
    if messages.is_empty() {
        return Ok(chat_error("Send a message first."));
    }

    let prompt_context = build_prompt_context(&request).await;
    let mut chat_messages = vec![json!({
        "role": "system",
        "content": build_ai_assistant_system_prompt(&prompt_context),
    })];
    chat_messages.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content })),
    );

    let url = format!(
        "{}/chat/completions",
        settings.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .build()
        .map_err(|_| "fetch failed".to_string())?;
    let mut builder = client.post(url).header("Content-Type", "application/json");
    if !settings.api_key.is_empty() {
        builder = builder.header("Authorization", format!("Bearer {}", settings.api_key));
    }
    if settings.provider == AiAssistantProvider::Openrouter {
        builder = builder
            .header("HTTP-Referer", "https://monolith.local")
            .header("X-Title", "Monolith");
    }

    let response = builder
        .body(
            json!({
                "model": settings.model,
                "temperature": 0.2,
                "max_tokens": 450,
                "messages": chat_messages,
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|_| "fetch failed".to_string())?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    let payload = if text.is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "error": text }))
    };

    if !status.is_success() {
        return Ok(chat_error(get_error_message(
            &payload,
            format!("Provider request failed with {}.", status.as_u16()),
        )));
    }

    let Some(content) = read_assistant_content(&payload) else {
        return Ok(chat_error("Provider returned no assistant message."));
    };
    if content.is_empty() {
        return Ok(chat_error("Provider returned no assistant message."));
    }

    let parsed = parse_assistant_model_response(&content, request.context.server_id.as_deref());
    Ok(AiAssistantChatResult {
        success: true,
        reply: Some(parsed.reply),
        actions: Some(parsed.actions),
        error: None,
    })
}

pub async fn apply_ai_assistant_action(action: Value) -> Result<AiAssistantActionResult, String> {
    let server_id = action
        .get("serverId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(normalized) =
        normalize_assistant_actions(&Value::Array(vec![action]), server_id.as_deref())
            .into_iter()
            .next()
    else {
        return Ok(action_error("This assistant action is not allowed."));
    };

    let Some(server) = get_server(normalized.server_id.clone()).await else {
        return Ok(action_error("Server not found."));
    };

    match &normalized.kind {
        AiAssistantActionKind::WriteServerFile {
            relative_path,
            content,
        } => {
            let previous = read_server_file(server.id.clone(), relative_path.clone()).await;
            let result =
                write_server_file(server.id.clone(), relative_path.clone(), content.clone()).await;
            if !result.success {
                return Ok(action_error(non_empty_error(
                    result.error,
                    "Failed to save config file.",
                )));
            }

            let undo_action = match (previous.success, previous.content) {
                (true, Some(previous_content)) => Some(AiAssistantAction {
                    id: format!("writeServerFile:{relative_path}:undo"),
                    server_id: server.id.clone(),
                    label: format!("Undo {relative_path}"),
                    requires_confirmation: true,
                    kind: AiAssistantActionKind::WriteServerFile {
                        relative_path: relative_path.clone(),
                        content: previous_content,
                    },
                }),
                _ => None,
            };

            Ok(AiAssistantActionResult {
                success: true,
                server_id: Some(server.id.clone()),
                message: Some(format!(
                    "{relative_path} was updated. Restart or reload the plugin if needed."
                )),
                undo_action,
                error: None,
            })
        }
        AiAssistantActionKind::SendServerCommand { command } => {
            if !is_running(&server.id) {
                return Ok(action_error(
                    "Server is offline. Start it before running commands.",
                ));
            }

            send_command(server.id.clone(), command.clone()).await;
            Ok(AiAssistantActionResult {
                success: true,
                server_id: Some(server.id.clone()),
                message: Some(format!("Command sent: {command}")),
                undo_action: None,
                error: None,
            })
        }
        AiAssistantActionKind::SetServerProperty { key, value } => {
            let server_path = Path::new(&server.server_path);
            let mut properties = read_properties(server_path).await;
            let index = properties.iter().position(|p| &p.key == key);
            let previous_value = index.map(|i| properties[i].value.clone());

            match index {
                Some(i) => properties[i].value = value.clone(),
                None => properties.push(ServerProperty {
                    key: key.clone(),
                    value: value.clone(),
                    comment: None,
                }),
            }

            let result = write_properties(server_path, &properties).await;
            if !result.success {
                return Ok(action_error(non_empty_error(
                    result.error,
                    "Failed to save server.properties.",
                )));
            }

            if key == "max-players" {
                let players = update_player_capacity_string(Some(&server.players), value);
                update_server_status(&server.id, server.status.clone(), Some(&players)).await;
            }

            let undo_action = previous_value.map(|previous| AiAssistantAction {
                id: format!("setServerProperty:{key}:{previous}:undo"),
                server_id: server.id.clone(),
                label: format!("Undo {key}"),
                requires_confirmation: true,
                kind: AiAssistantActionKind::SetServerProperty {
                    key: key.clone(),
                    value: previous,
                },
            });

            Ok(AiAssistantActionResult {
                success: true,
                server_id: Some(server.id.clone()),
                message: Some(format!(
                    "{key} was set to {value}. Restart the server if Minecraft requires it."
                )),
                undo_action,
                error: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConsoleLineType;

    fn property_action(id: &str, key: &str, value: &str, label: &str) -> AiAssistantAction {
        AiAssistantAction {
            id: id.to_string(),
            server_id: "srv-1".to_string(),
            label: label.to_string(),
            requires_confirmation: true,
            kind: AiAssistantActionKind::SetServerProperty {
                key: key.to_string(),
                value: value.to_string(),
            },
        }
    }

    fn as_json(actions: &[AiAssistantAction]) -> Value {
        serde_json::to_value(actions).unwrap_or(Value::Null)
    }

    #[test]
    fn parses_json_responses_and_falls_back_to_plain_text() {
        let parsed = parse_assistant_model_response(
            r#"{"reply":"Raise the max player limit first.","actions":[{"kind":"setServerProperty","serverId":"srv-1","key":"max-players","value":"40","label":"Set max players to 40"}]}"#,
            Some("srv-1"),
        );
        assert_eq!(parsed.reply, "Raise the max player limit first.");
        assert_eq!(
            as_json(&parsed.actions),
            as_json(&[property_action(
                "setServerProperty:max-players:40",
                "max-players",
                "40",
                "Set max-players to 40"
            )])
        );

        let plain = parse_assistant_model_response("Plain explanation", Some("srv-1"));
        assert_eq!(plain.reply, "Plain explanation");
        assert!(plain.actions.is_empty());
    }

    #[test]
    fn parses_fenced_json() {
        let parsed =
            parse_assistant_model_response("```json\n{\"reply\":\"hi\",\"actions\":[]}\n```", None);
        assert_eq!(parsed.reply, "hi");
    }

    #[test]
    fn allows_only_safe_property_actions_for_current_server() {
        let input = json!([
            {"kind":"setServerProperty","serverId":"srv-1","key":"max-players","value":"50","label":"Set max players to 50"},
            {"kind":"setServerProperty","serverId":"srv-2","key":"max-players","value":"100","label":"Touch another server"},
            {"kind":"setServerProperty","serverId":"srv-1","key":"rcon.password","value":"secret","label":"Change RCON password"}
        ]);
        let actions = normalize_assistant_actions(&input, Some("srv-1"));
        assert_eq!(
            as_json(&actions),
            as_json(&[property_action(
                "setServerProperty:max-players:50",
                "max-players",
                "50",
                "Set max-players to 50"
            )])
        );
    }

    #[test]
    fn allows_safe_config_edits_and_commands_only() {
        let input = json!([
            {"kind":"writeServerFile","serverId":"srv-1","relativePath":"plugins/Essentials/config.yml","content":"spawn-on-join: true\n","label":"Update Essentials config"},
            {"kind":"sendServerCommand","serverId":"srv-1","command":"say Server restarts in 5 minutes","label":"Announce restart"},
            {"kind":"writeServerFile","serverId":"srv-1","relativePath":"../server.properties","content":"bad","label":"Path traversal"},
            {"kind":"sendServerCommand","serverId":"srv-2","command":"stop","label":"Wrong server"}
        ]);
        let actions = normalize_assistant_actions(&input, Some("srv-1"));
        let expected = json!([
            {
                "id": "writeServerFile:plugins/Essentials/config.yml",
                "kind": "writeServerFile",
                "serverId": "srv-1",
                "relativePath": "plugins/Essentials/config.yml",
                "content": "spawn-on-join: true\n",
                "label": "Update plugins/Essentials/config.yml",
                "requiresConfirmation": true
            },
            {
                "id": "sendServerCommand:say Server restarts in 5 minutes",
                "kind": "sendServerCommand",
                "serverId": "srv-1",
                "command": "say Server restarts in 5 minutes",
                "label": "Run say Server restarts in 5 minutes",
                "requiresConfirmation": true
            }
        ]);
        assert_eq!(as_json(&actions), expected);
    }

    #[test]
    fn updates_player_capacity_string() {
        assert_eq!(update_player_capacity_string(Some("0/20"), "50"), "0/50");
        assert_eq!(update_player_capacity_string(Some("7/20"), "50"), "7/50");
        assert_eq!(update_player_capacity_string(Some("unknown"), "50"), "0/50");
    }

    #[test]
    fn builds_system_prompt_with_context() {
        let context = PromptContext {
            locale: Some("de-DE".into()),
            route: "/servers/srv-1".into(),
            page_title: "Server Detail".into(),
            server: Some(PromptServer {
                id: "srv-1".into(),
                name: "Survival".into(),
                framework: "Paper".into(),
                version: "1.21.4".into(),
                status: "Offline".into(),
                ram_mb: 4096,
                players: "0/20".into(),
            }),
            logs: vec![ConsoleLine {
                timestamp: "2026-06-13T17:00:00.000Z".into(),
                text: "java.lang.OutOfMemoryError".into(),
                kind: ConsoleLineType::Stderr,
            }],
            properties: vec![ServerProperty {
                key: "max-players".into(),
                value: "20".into(),
                comment: None,
            }],
        };
        let prompt = build_ai_assistant_system_prompt(&context);

        for needle in [
            "Monolith",
            "Server Detail",
            "Survival",
            "OutOfMemoryError",
            "max-players=20",
            "Markdown",
            "latest user message",
            "short",
            "Monolith or Minecraft",
            "user confirmation",
            "plugin",
            "[stderr]",
        ] {
            assert!(prompt.contains(needle), "missing {needle}");
        }
    }
}
