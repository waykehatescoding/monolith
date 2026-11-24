use crate::types::{
    ModrinthGalleryImage, ModrinthProjectDetails, ModrinthSearchHit, ModrinthSearchParams,
    ModrinthSearchResult, ModrinthSort, ModrinthVersionOption, ModrinthVersionType,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;

const MODRINTH_API_BASE: &str = "https://api.modrinth.com/v2";
const USER_AGENT: &str = "Monolith/1.0 (Modrinth API)";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

pub(crate) static CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);
static VERSION_CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<Value>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static PROJECT_CACHE: Lazy<Mutex<HashMap<String, (Instant, ModrinthProjectDetails)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cache_get<T: Clone>(cache: &Mutex<HashMap<String, (Instant, T)>>, key: &str) -> Option<T> {
    let mut guard = cache.lock().ok()?;
    match guard.get(key) {
        Some((expires, value)) if *expires > Instant::now() => Some(value.clone()),
        Some(_) => {
            guard.remove(key);
            None
        }
        None => None,
    }
}

fn cache_set<T>(cache: &Mutex<HashMap<String, (Instant, T)>>, key: String, value: T) {
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, (Instant::now() + CACHE_TTL, value));
    }
}

fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_f64()
        .filter(|n| n.is_finite())
        .map(|n| n.max(0.0) as u64)
        .unwrap_or(0))
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Value::deserialize(d)?
        .as_str()
        .unwrap_or_default()
        .to_string())
}

fn coerce_number(value: Option<&Value>, default: f64) -> f64 {
    match value {
        None | Some(Value::Null) => default,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

pub(crate) fn published_millis(version: &Value) -> i64 {
    version
        .get("date_published")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(i64::MIN)
}

fn build_versions_url(
    project_id: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Url, String> {
    let mut url = Url::parse(&format!("{MODRINTH_API_BASE}/project/{project_id}/version"))
        .map_err(|e| e.to_string())?;
    if let Some(loader) = loader.filter(|l| !l.is_empty()) {
        url.query_pairs_mut()
            .append_pair("loaders", &serde_json::json!([loader]).to_string());
    }
    if let Some(version) = game_version.filter(|v| !v.is_empty()) {
        url.query_pairs_mut()
            .append_pair("game_versions", &serde_json::json!([version]).to_string());
    }
    Ok(url)
}

pub async fn fetch_modrinth_versions(
    project_id: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Vec<Value>, String> {
    let loader = loader.filter(|l| !l.is_empty());
    let game_version = game_version.filter(|v| !v.is_empty());
    let key = format!(
        "{}:{}:{}",
        project_id,
        loader.unwrap_or("any"),
        game_version.unwrap_or("any")
    );
    if let Some(cached) = cache_get(&VERSION_CACHE, &key) {
        return Ok(cached);
    }

    let url = build_versions_url(project_id, loader, game_version)?;
    let res = CLIENT
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!(
            "Modrinth versions failed: {}",
            res.status().as_u16()
        ));
    }

    let mut versions: Vec<Value> = res.json().await.map_err(|e| e.to_string())?;
    versions.sort_by_key(|v| std::cmp::Reverse(published_millis(v)));
    cache_set(&VERSION_CACHE, key, versions.clone());
    Ok(versions)
}

#[derive(Deserialize)]
struct RawGalleryItem {
    #[serde(default, deserialize_with = "lenient_string")]
    url: String,
    title: Option<String>,
    description: Option<String>,
    featured: Option<bool>,
}

#[derive(Deserialize)]
struct RawProject {
    #[serde(default, deserialize_with = "lenient_string")]
    slug: String,
    #[serde(default, deserialize_with = "lenient_string")]
    title: String,
    #[serde(default, deserialize_with = "lenient_string")]
    description: String,
    #[serde(default, deserialize_with = "lenient_string")]
    body: String,
    icon_url: Option<String>,
    #[serde(default, deserialize_with = "lenient_u64")]
    downloads: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    followers: u64,
    client_side: Option<String>,
    server_side: Option<String>,
    categories: Option<Vec<String>>,
    project_url: Option<String>,
    source_url: Option<String>,
    issues_url: Option<String>,
    wiki_url: Option<String>,
    gallery: Option<Vec<RawGalleryItem>>,
}

async fn fetch_modrinth_project(project_id: &str) -> Result<ModrinthProjectDetails, String> {
    if let Some(cached) = cache_get(&PROJECT_CACHE, project_id) {
        return Ok(cached);
    }

    let url = Url::parse(&format!("{MODRINTH_API_BASE}/project/{project_id}"))
        .map_err(|e| e.to_string())?;
    let res = CLIENT
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!(
            "Modrinth project failed: {}",
            res.status().as_u16()
        ));
    }

    let project: RawProject = res.json().await.map_err(|e| e.to_string())?;
    let detail = ModrinthProjectDetails {
        project_id: project_id.to_string(),
        slug: project.slug,
        title: project.title,
        description: project.description,
        body: project.body,
        icon_url: project.icon_url,
        downloads: project.downloads,
        followers: project.followers,
        client_side: project.client_side,
        server_side: project.server_side,
        categories: project.categories,
        project_url: project.project_url,
        source_url: project.source_url,
        issues_url: project.issues_url,
        wiki_url: project.wiki_url,
        gallery: project
            .gallery
            .unwrap_or_default()
            .into_iter()
            .map(|item| ModrinthGalleryImage {
                url: item.url,
                title: item.title,
                description: item.description,
                featured: item.featured,
            })
            .collect(),
    };
    cache_set(&PROJECT_CACHE, project_id.to_string(), detail.clone());
    Ok(detail)
}

pub(crate) fn version_option(version: &Value) -> ModrinthVersionOption {
    let files = version.get("files").and_then(Value::as_array);
    let file = files.and_then(|files| {
        files
            .iter()
            .find(|f| f.get("primary").and_then(Value::as_bool).unwrap_or(false))
            .or_else(|| files.first())
    });
    let text = |key: &str| {
        version
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list = |key: &str| -> Vec<String> {
        version
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };

    ModrinthVersionOption {
        id: text("id"),
        name: text("name"),
        version_number: text("version_number"),
        version_type: serde_json::from_value(
            version.get("version_type").cloned().unwrap_or_default(),
        )
        .unwrap_or(ModrinthVersionType::Release),
        date_published: text("date_published"),
        loaders: list("loaders"),
        game_versions: list("game_versions"),
        file_name: file
            .and_then(|f| f.get("filename"))
            .and_then(Value::as_str)
            .unwrap_or("No primary file")
            .to_string(),
        file_size: file.and_then(|f| f.get("size")).and_then(Value::as_u64),
        required: required_dependencies(version),
    }
}

fn required_dependencies(version: &Value) -> Vec<String> {
    version
        .get("dependencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|d| d.get("dependency_type").and_then(Value::as_str) == Some("required"))
        .filter_map(|d| d.get("project_id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .map(String::from)
        .collect()
}

fn build_facets(
    params: &ModrinthSearchParams,
    include_loader: bool,
    include_version: bool,
) -> String {
    let project_type = match params.project_type {
        crate::types::ModrinthProjectType::Plugin => "plugin",
        crate::types::ModrinthProjectType::Mod => "mod",
        crate::types::ModrinthProjectType::Modpack => "modpack",
    };
    let mut facets = vec![vec![format!("project_type:{project_type}")]];
    if let Some(version) = params.game_version.as_deref().filter(|v| !v.is_empty()) {
        if include_version {
            facets.push(vec![format!("versions:{version}")]);
        }
    }
    if let Some(loader) = params.loader.as_deref().filter(|l| !l.is_empty()) {
        if include_loader {
            facets.push(vec![format!("categories:{loader}")]);
        }
    }
    serde_json::json!(facets).to_string()
}

#[derive(Deserialize)]
struct RawHit {
    #[serde(default, deserialize_with = "lenient_string")]
    project_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    slug: String,
    #[serde(default, deserialize_with = "lenient_string")]
    title: String,
    #[serde(default, deserialize_with = "lenient_string")]
    description: String,
    icon_url: Option<String>,
    categories: Option<Vec<String>>,
    client_side: Option<String>,
    server_side: Option<String>,
    #[serde(default, deserialize_with = "lenient_u64")]
    downloads: u64,
    #[serde(default, deserialize_with = "lenient_u64")]
    follows: u64,
    #[serde(default, deserialize_with = "lenient_string")]
    author: String,
    #[serde(default, deserialize_with = "lenient_string")]
    date_modified: String,
}

#[derive(Deserialize)]
struct RawSearch {
    #[serde(default)]
    hits: Vec<RawHit>,
    #[serde(default, deserialize_with = "lenient_u64")]
    total_hits: u64,
}

async fn try_search(
    params: &ModrinthSearchParams,
    limit: u64,
    offset: u64,
    facets: String,
) -> Result<RawSearch, String> {
    let index = match params.sort.unwrap_or(ModrinthSort::Relevance) {
        ModrinthSort::Relevance => "relevance",
        ModrinthSort::Downloads => "downloads",
        ModrinthSort::Updated => "updated",
        ModrinthSort::Newest => "newest",
    };

    let mut url = Url::parse(&format!("{MODRINTH_API_BASE}/search")).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("query", &params.query)
        .append_pair("limit", &limit.to_string())
        .append_pair("offset", &offset.to_string())
        .append_pair("index", index)
        .append_pair("facets", &facets);

    let res = CLIENT
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Modrinth search failed: {status} {body}"));
    }
    res.json().await.map_err(|e| e.to_string())
}

fn is_bad_request(e: &str) -> bool {
    e.starts_with("Modrinth search failed: 400")
}

pub async fn search_modrinth(mut params: Value) -> Result<ModrinthSearchResult, String> {
    let limit_raw = coerce_number(params.get("limit"), 20.0);
    let limit = if limit_raw.is_finite() {
        limit_raw.floor().clamp(1.0, 100.0) as u64
    } else {
        20
    };
    let offset_raw = coerce_number(params.get("offset"), 0.0);
    let offset = if offset_raw.is_finite() {
        offset_raw.floor().max(0.0) as u64
    } else {
        0
    };
    if let Some(map) = params.as_object_mut() {
        map.remove("limit");
        map.remove("offset");
    }
    let params: ModrinthSearchParams = serde_json::from_value(params).map_err(|e| e.to_string())?;

    let data = match try_search(&params, limit, offset, build_facets(&params, true, true)).await {
        Err(e) if is_bad_request(&e) => {
            match try_search(&params, limit, offset, build_facets(&params, false, true)).await {
                Err(e) if is_bad_request(&e) => {
                    try_search(&params, limit, offset, build_facets(&params, false, false)).await?
                }
                other => other?,
            }
        }
        other => other?,
    };

    Ok(ModrinthSearchResult {
        hits: data
            .hits
            .into_iter()
            .map(|hit| ModrinthSearchHit {
                project_id: hit.project_id,
                slug: hit.slug,
                title: hit.title,
                description: hit.description,
                icon_url: hit.icon_url,
                categories: hit.categories,
                client_side: hit.client_side,
                server_side: hit.server_side,
                downloads: hit.downloads,
                follows: hit.follows,
                author: hit.author,
                date_modified: hit.date_modified,
                external: false,
                page_url: None,
            })
            .collect(),
        total_hits: data.total_hits,
    })
}

pub async fn get_modrinth_project(project_id: String) -> Result<ModrinthProjectDetails, String> {
    fetch_modrinth_project(&project_id).await
}

pub async fn list_modrinth_versions(
    project_id: String,
    loader: Option<String>,
    game_version: Option<String>,
) -> Result<Vec<ModrinthVersionOption>, String> {
    let loader = loader.as_deref().filter(|l| !l.is_empty());
    let game_version = game_version.as_deref().filter(|v| !v.is_empty());

    let mut versions = fetch_modrinth_versions(&project_id, loader, game_version).await?;
    if versions.is_empty() && game_version.is_some() {
        versions = fetch_modrinth_versions(&project_id, loader, None).await?;
    }
    if versions.is_empty() && loader.is_some() {
        versions = fetch_modrinth_versions(&project_id, None, game_version).await?;
    }
    if versions.is_empty() {
        versions = fetch_modrinth_versions(&project_id, None, None).await?;
    }
    Ok(versions.iter().map(version_option).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_option_lists_required_dependencies() {
        let option = version_option(&serde_json::json!({
            "id": "v1",
            "version_type": "release",
            "files": [{ "filename": "a.jar", "primary": true }],
            "dependencies": [
                { "project_id": "P7BTp6Z7", "dependency_type": "required" },
                { "project_id": "opt", "dependency_type": "optional" },
                { "project_id": null, "version_id": "x", "dependency_type": "required" }
            ]
        }));
        assert_eq!(option.required, vec!["P7BTp6Z7".to_string()]);
        assert_eq!(option.file_name, "a.jar");
    }
}
