use serde_json::Value;
use url::Url;

use crate::paths::user_data_dir;
use crate::types::{AiAssistantProvider, AiAssistantSettings, PublicAiAssistantSettings};

const SETTINGS_FILE: &str = "ai_assistant_settings.json";

struct ProviderDefinition {
    default_model: &'static str,
    default_base_url: &'static str,
}

pub fn requires_api_key(provider: AiAssistantProvider) -> bool {
    !matches!(provider, AiAssistantProvider::Custom)
}

fn provider_definition(provider: AiAssistantProvider) -> ProviderDefinition {
    match provider {
        AiAssistantProvider::Openai => ProviderDefinition {
            default_model: "gpt-4o-mini",
            default_base_url: "https://api.openai.com/v1",
        },
        AiAssistantProvider::Openrouter => ProviderDefinition {
            default_model: "openai/gpt-4o-mini",
            default_base_url: "https://openrouter.ai/api/v1",
        },
        AiAssistantProvider::Groq => ProviderDefinition {
            default_model: "llama-3.3-70b-versatile",
            default_base_url: "https://api.groq.com/openai/v1",
        },
        AiAssistantProvider::Custom => ProviderDefinition {
            default_model: "gpt-4o-mini",
            default_base_url: "http://localhost:1234/v1",
        },
    }
}

fn default_settings() -> AiAssistantSettings {
    let definition = provider_definition(AiAssistantProvider::Openai);
    AiAssistantSettings {
        enabled: false,
        onboarding_completed: false,
        provider: AiAssistantProvider::Openai,
        model: definition.default_model.to_string(),
        base_url: definition.default_base_url.to_string(),
        api_key: String::new(),
    }
}

fn normalize_base_url(value: Option<&str>, provider: AiAssistantProvider) -> String {
    let fallback = provider_definition(provider).default_base_url.to_string();
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return fallback;
    };
    match Url::parse(raw) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {
            let text = url.to_string();
            text.strip_suffix('/').unwrap_or(&text).to_string()
        }
        _ => fallback,
    }
}

fn normalize_settings(value: &Value) -> AiAssistantSettings {
    let provider = value
        .get("provider")
        .and_then(|v| serde_json::from_value::<AiAssistantProvider>(v.clone()).ok())
        .unwrap_or_default();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| provider_definition(provider).default_model.to_string());

    AiAssistantSettings {
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        onboarding_completed: value
            .get("onboardingCompleted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        provider,
        model,
        base_url: normalize_base_url(value.get("baseUrl").and_then(Value::as_str), provider),
        api_key: value
            .get("apiKey")
            .and_then(Value::as_str)
            .map(|k| k.trim().to_string())
            .unwrap_or_default(),
    }
}

fn merge_settings(current: &AiAssistantSettings, updates: &Value) -> AiAssistantSettings {
    let provider = updates
        .get("provider")
        .and_then(|v| serde_json::from_value::<AiAssistantProvider>(v.clone()).ok())
        .unwrap_or(current.provider);
    let provider_changed = provider != current.provider;
    let definition = provider_definition(provider);

    let text = |key: &str| updates.get(key).and_then(Value::as_str).map(str::to_string);
    let flag = |key: &str, fallback: bool| {
        updates
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(fallback)
    };

    let model = text("model").unwrap_or_else(|| {
        if provider_changed {
            definition.default_model.to_string()
        } else {
            current.model.clone()
        }
    });
    let base_url = text("baseUrl").unwrap_or_else(|| {
        if provider_changed {
            definition.default_base_url.to_string()
        } else {
            current.base_url.clone()
        }
    });

    let merged = serde_json::json!({
        "enabled": flag("enabled", current.enabled),
        "onboardingCompleted": flag("onboardingCompleted", current.onboarding_completed),
        "provider": provider,
        "model": model,
        "baseUrl": base_url,
        "apiKey": text("apiKey").unwrap_or_else(|| current.api_key.clone()),
    });
    normalize_settings(&merged)
}

fn censor_api_key(api_key: &str) -> Option<String> {
    if api_key.is_empty() {
        return None;
    }
    let units: Vec<u16> = api_key.encode_utf16().collect();
    if units.len() <= 8 {
        return Some("*".repeat(units.len()));
    }
    let head = String::from_utf16_lossy(&units[..4]);
    let tail = String::from_utf16_lossy(&units[units.len() - 4..]);
    Some(format!("{head}...{tail}"))
}

fn to_public(settings: &AiAssistantSettings) -> PublicAiAssistantSettings {
    PublicAiAssistantSettings {
        enabled: settings.enabled,
        onboarding_completed: settings.onboarding_completed,
        provider: settings.provider,
        model: settings.model.clone(),
        base_url: settings.base_url.clone(),
        has_api_key: !settings.api_key.is_empty(),
        censored_api_key: censor_api_key(&settings.api_key),
    }
}

pub async fn read_ai_assistant_settings() -> AiAssistantSettings {
    let path = user_data_dir().join(SETTINGS_FILE);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => raw,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Failed to read AI assistant settings: {e}");
            }
            return default_settings();
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) => normalize_settings(&value),
        Err(e) => {
            eprintln!("Failed to read AI assistant settings: {e}");
            default_settings()
        }
    }
}

async fn write_ai_assistant_settings(settings: &AiAssistantSettings) -> Result<(), String> {
    let path = user_data_dir().join(SETTINGS_FILE);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_ai_assistant_settings() -> PublicAiAssistantSettings {
    to_public(&read_ai_assistant_settings().await)
}

pub async fn update_ai_assistant_settings(
    updates: Value,
) -> Result<PublicAiAssistantSettings, String> {
    let current = read_ai_assistant_settings().await;
    let next = merge_settings(&current, &updates);
    write_ai_assistant_settings(&next).await?;
    Ok(to_public(&next))
}
