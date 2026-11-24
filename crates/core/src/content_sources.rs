use crate::modrinth::{
    fetch_modrinth_versions, list_modrinth_versions, published_millis, search_modrinth,
    version_option, CLIENT,
};
use crate::paths::user_data_dir;
use crate::types::{
    ContentSource, ModrinthProjectType, ModrinthSearchHit, ModrinthSearchParams,
    ModrinthSearchResult, ModrinthSort, ModrinthVersionOption,
};
use serde_json::{json, Value};
use std::cmp::Reverse;
use url::Url;

const HANGAR_API: &str = "https://hangar.papermc.io/api/v1";
const SPIGET_API: &str = "https://api.spiget.org/v2";
const CURSEFORGE_API: &str = "https://api.curseforge.com/v1";
const USER_AGENT: &str = "Monolith/1.0";
const MINECRAFT_GAME_ID: &str = "432";
const KEY_FILE: &str = "curseforge.json";
const NO_KEY: &str = "Add your CurseForge API key in Settings to use CurseForge.";
const EXTERNAL: &str =
    "This plugin is only available from an external site. Open it in your browser to download it.";

impl ContentSource {
    pub fn label(self) -> &'static str {
        match self {
            ContentSource::Modrinth => "Modrinth",
            ContentSource::Hangar => "Hangar",
            ContentSource::Spigot => "Spigot",
            ContentSource::CurseForge => "CurseForge",
        }
    }
}

pub fn loader_for(framework: &str) -> Option<(ModrinthProjectType, &'static str)> {
    match framework {
        "Paper" | "Purpur" => Some((ModrinthProjectType::Plugin, "paper")),
        "Fabric" => Some((ModrinthProjectType::Mod, "fabric")),
        "Quilt" => Some((ModrinthProjectType::Mod, "quilt")),
        "Forge" => Some((ModrinthProjectType::Mod, "forge")),
        "NeoForge" => Some((ModrinthProjectType::Mod, "neoforge")),
        _ => None,
    }
}

pub fn sources_for(project_type: ModrinthProjectType, curseforge: bool) -> Vec<ContentSource> {
    let mut sources = vec![ContentSource::Modrinth];
    if project_type == ModrinthProjectType::Plugin {
        sources.extend([ContentSource::Hangar, ContentSource::Spigot]);
    }
    if curseforge {
        sources.push(ContentSource::CurseForge);
    }
    sources
}

pub async fn curseforge_api_key() -> Option<String> {
    let raw = tokio::fs::read_to_string(user_data_dir().join(KEY_FILE))
        .await
        .ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value["apiKey"]
        .as_str()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(String::from)
}

pub async fn set_curseforge_api_key(key: String) -> Result<(), String> {
    let path = user_data_dir().join(KEY_FILE);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&json!({ "apiKey": key.trim() }))
        .map_err(|e| e.to_string())?;
    tokio::fs::write(path, json)
        .await
        .map_err(|e| e.to_string())
}

fn api_url(base: &str, segments: &[&str]) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Invalid URL".to_string())?
        .extend(segments);
    Ok(url)
}

async fn send(request: reqwest::RequestBuilder, label: &str) -> Result<reqwest::Response, String> {
    let res = request
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("{label} request failed: {}", res.status().as_u16()));
    }
    Ok(res)
}

async fn get_json(url: Url, label: &str) -> Result<Value, String> {
    send(CLIENT.get(url), label)
        .await?
        .json()
        .await
        .map_err(|e| e.to_string())
}

async fn curseforge(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let key = curseforge_api_key().await.ok_or(NO_KEY)?;
    send(request.header("x-api-key", key), "CurseForge")
        .await?
        .json()
        .await
        .map_err(|e| e.to_string())
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn id_text(value: &Value) -> String {
    value
        .as_u64()
        .map(|id| id.to_string())
        .unwrap_or_else(|| text(value))
}

fn count(value: &Value) -> u64 {
    value.as_f64().map(|n| n.max(0.0) as u64).unwrap_or(0)
}

fn unix_date(value: &Value) -> String {
    value
        .as_i64()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|d| d.to_rfc3339())
        .unwrap_or_default()
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.is_empty())
}

fn page(params: &ModrinthSearchParams) -> (u32, u32) {
    (
        params.limit.unwrap_or(20).max(1),
        params.offset.unwrap_or(0),
    )
}

pub async fn search(
    source: ContentSource,
    params: ModrinthSearchParams,
) -> Result<ModrinthSearchResult, String> {
    match source {
        ContentSource::Modrinth => {
            search_modrinth(serde_json::to_value(params).map_err(|e| e.to_string())?).await
        }
        ContentSource::Hangar => search_hangar(&params).await,
        ContentSource::Spigot => search_spiget(&params).await,
        ContentSource::CurseForge => search_curseforge(&params).await,
    }
}

async fn search_hangar(params: &ModrinthSearchParams) -> Result<ModrinthSearchResult, String> {
    let (limit, offset) = page(params);
    let mut url = api_url(HANGAR_API, &["projects"])?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("limit", &limit.to_string())
            .append_pair("offset", &offset.to_string())
            .append_pair("platform", "PAPER");
        if !params.query.is_empty() {
            query.append_pair("q", &params.query);
        }
        if let Some(version) = non_empty(&params.game_version) {
            query.append_pair("version", version);
        }
        let sort = match params.sort {
            Some(ModrinthSort::Downloads) => Some("-downloads"),
            Some(ModrinthSort::Updated) => Some("-updated"),
            Some(ModrinthSort::Newest) => Some("-newest"),
            _ if params.query.is_empty() => Some("-downloads"),
            _ => None,
        };
        if let Some(sort) = sort {
            query.append_pair("sort", sort);
        }
    }
    let data = get_json(url, "Hangar").await?;
    Ok(ModrinthSearchResult {
        hits: data["result"]
            .as_array()
            .map(|items| items.iter().map(hangar_hit).collect())
            .unwrap_or_default(),
        total_hits: count(&data["pagination"]["count"]),
    })
}

fn hangar_hit(project: &Value) -> ModrinthSearchHit {
    let owner = text(&project["namespace"]["owner"]);
    let slug = text(&project["namespace"]["slug"]);
    ModrinthSearchHit {
        project_id: id_text(&project["id"]),
        page_url: Some(format!("https://hangar.papermc.io/{owner}/{slug}")),
        slug,
        title: text(&project["name"]),
        description: text(&project["description"]),
        icon_url: project["avatarUrl"].as_str().map(String::from),
        categories: project["category"].as_str().map(|c| vec![c.to_string()]),
        client_side: None,
        server_side: None,
        downloads: count(&project["stats"]["downloads"]),
        follows: count(&project["stats"]["stars"]),
        author: owner,
        date_modified: text(&project["lastUpdated"]),
        external: false,
    }
}

async fn hangar_versions(
    project_id: &str,
    game_version: Option<&str>,
) -> Result<Vec<Value>, String> {
    let mut url = api_url(HANGAR_API, &["projects", project_id, "versions"])?;
    url.query_pairs_mut()
        .append_pair("limit", "25")
        .append_pair("platform", "PAPER");
    if let Some(version) = game_version.filter(|v| !v.is_empty()) {
        url.query_pairs_mut()
            .append_pair("platformVersion", version);
    }
    let data = get_json(url, "Hangar").await?;
    Ok(data["result"]
        .as_array()
        .map(|items| items.iter().map(hangar_version).collect())
        .unwrap_or_default())
}

fn hangar_version(version: &Value) -> Value {
    let download = &version["downloads"]["PAPER"];
    let files: Vec<Value> = download["downloadUrl"]
        .as_str()
        .map(|url| {
            json!({
                "url": url,
                "filename": download["fileInfo"]["name"],
                "size": download["fileInfo"]["sizeBytes"],
                "primary": true,
                "hashes": { "sha256": download["fileInfo"]["sha256Hash"] },
            })
        })
        .into_iter()
        .collect();
    let dependencies: Vec<Value> = version["pluginDependencies"]["PAPER"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["required"] == true && d["externalUrl"].is_null())
        .filter_map(|d| {
            Some(json!({
                "project_id": d["projectId"].as_u64()?.to_string(),
                "dependency_type": "required",
                "title": d["name"],
            }))
        })
        .collect();
    let version_type = match version["channel"]["name"].as_str().map(str::to_lowercase) {
        Some(c) if c == "release" => "release",
        Some(c) if c == "beta" => "beta",
        _ => "alpha",
    };
    json!({
        "id": id_text(&version["id"]),
        "name": version["name"],
        "version_number": version["name"],
        "version_type": version_type,
        "date_published": version["createdAt"],
        "loaders": ["paper"],
        "game_versions": version["platformDependencies"]["PAPER"],
        "files": files,
        "dependencies": dependencies,
    })
}

async fn search_spiget(params: &ModrinthSearchParams) -> Result<ModrinthSearchResult, String> {
    let (limit, offset) = page(params);
    let mut url = if params.query.is_empty() {
        api_url(SPIGET_API, &["resources", "free"])?
    } else {
        api_url(SPIGET_API, &["search", "resources", &params.query])?
    };
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("size", &limit.to_string())
            .append_pair("page", &(offset / limit + 1).to_string())
            .append_pair(
                "fields",
                "id,name,tag,external,premium,file,downloads,updateDate",
            );
        if !params.query.is_empty() {
            query.append_pair("field", "name");
        }
        let sort = match params.sort {
            Some(ModrinthSort::Downloads) => Some("-downloads"),
            Some(ModrinthSort::Updated) => Some("-updateDate"),
            Some(ModrinthSort::Newest) => Some("-releaseDate"),
            _ if params.query.is_empty() => Some("-downloads"),
            _ => None,
        };
        if let Some(sort) = sort {
            query.append_pair("sort", sort);
        }
    }
    let res = CLIENT
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(ModrinthSearchResult::default());
    }
    if !res.status().is_success() {
        return Err(format!("Spiget request failed: {}", res.status().as_u16()));
    }
    let header = |name: &str| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let total = header("x-total")
        .or_else(|| header("x-page-count").map(|pages| pages * u64::from(limit)))
        .unwrap_or(0);
    let items: Vec<Value> = res.json().await.map_err(|e| e.to_string())?;
    Ok(ModrinthSearchResult {
        hits: items.iter().map(spiget_hit).collect(),
        total_hits: total,
    })
}

fn spigot_installable(resource: &Value) -> bool {
    resource["external"] != true
        && resource["premium"] != true
        && resource["file"]["type"] == ".jar"
}

fn spiget_hit(resource: &Value) -> ModrinthSearchHit {
    let id = id_text(&resource["id"]);
    ModrinthSearchHit {
        page_url: Some(format!("https://www.spigotmc.org/resources/{id}/")),
        slug: id.clone(),
        project_id: id,
        title: text(&resource["name"]),
        description: text(&resource["tag"]),
        icon_url: None,
        categories: None,
        client_side: None,
        server_side: None,
        downloads: count(&resource["downloads"]),
        follows: 0,
        author: String::new(),
        date_modified: unix_date(&resource["updateDate"]),
        external: !spigot_installable(resource),
    }
}

fn jar_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    format!("{}.jar", if name.is_empty() { "plugin" } else { name })
}

async fn spigot_versions(project_id: &str) -> Result<Vec<Value>, String> {
    let resource = get_json(api_url(SPIGET_API, &["resources", project_id])?, "Spiget").await?;
    if !spigot_installable(&resource) {
        return Err(EXTERNAL.into());
    }
    let latest = get_json(
        api_url(SPIGET_API, &["resources", project_id, "versions", "latest"])?,
        "Spiget",
    )
    .await?;
    let download = api_url(SPIGET_API, &["resources", project_id, "download"])?;
    Ok(vec![json!({
        "id": id_text(&latest["id"]),
        "name": latest["name"],
        "version_number": latest["name"],
        "version_type": "release",
        "date_published": unix_date(&latest["releaseDate"]),
        "loaders": ["paper"],
        "game_versions": resource["testedVersions"],
        "files": [{
            "url": download.as_str(),
            "filename": jar_name(&text(&resource["name"])),
            "primary": true,
        }],
        "dependencies": [],
    })])
}

fn curseforge_class(project_type: ModrinthProjectType) -> &'static str {
    match project_type {
        ModrinthProjectType::Plugin => "5",
        ModrinthProjectType::Mod => "6",
        ModrinthProjectType::Modpack => "4471",
    }
}

fn curseforge_loader(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("1"),
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        _ => None,
    }
}

async fn search_curseforge(params: &ModrinthSearchParams) -> Result<ModrinthSearchResult, String> {
    let (limit, offset) = page(params);
    let mut url = api_url(CURSEFORGE_API, &["mods", "search"])?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("gameId", MINECRAFT_GAME_ID)
            .append_pair("classId", curseforge_class(params.project_type))
            .append_pair("index", &offset.to_string())
            .append_pair("pageSize", &limit.min(50).to_string())
            .append_pair("sortOrder", "desc");
        if !params.query.is_empty() {
            query.append_pair("searchFilter", &params.query);
        }
        let sort = match params.sort {
            Some(ModrinthSort::Downloads) => "6",
            Some(ModrinthSort::Updated) => "3",
            Some(ModrinthSort::Newest) => "11",
            _ => "2",
        };
        query.append_pair("sortField", sort);
        if let Some(version) = non_empty(&params.game_version) {
            query.append_pair("gameVersion", version);
            if let Some(loader) = params.loader.as_deref().and_then(curseforge_loader) {
                query.append_pair("modLoaderType", loader);
            }
        }
    }
    let data = curseforge(CLIENT.get(url)).await?;
    Ok(ModrinthSearchResult {
        hits: data["data"]
            .as_array()
            .map(|items| items.iter().map(curseforge_hit).collect())
            .unwrap_or_default(),
        total_hits: count(&data["pagination"]["totalCount"]).min(10_000),
    })
}

fn curseforge_hit(project: &Value) -> ModrinthSearchHit {
    ModrinthSearchHit {
        project_id: id_text(&project["id"]),
        slug: text(&project["slug"]),
        title: text(&project["name"]),
        description: text(&project["summary"]),
        icon_url: project["logo"]["thumbnailUrl"].as_str().map(String::from),
        categories: None,
        client_side: None,
        server_side: None,
        downloads: count(&project["downloadCount"]),
        follows: count(&project["thumbsUpCount"]),
        author: text(&project["authors"][0]["name"]),
        date_modified: text(&project["dateModified"]),
        external: project["allowModDistribution"] == false,
        page_url: project["links"]["websiteUrl"].as_str().map(String::from),
    }
}

async fn curseforge_versions(
    project_id: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Vec<Value>, String> {
    let mut url = api_url(CURSEFORGE_API, &["mods", project_id, "files"])?;
    url.query_pairs_mut().append_pair("pageSize", "50");
    if let Some(version) = game_version.filter(|v| !v.is_empty()) {
        url.query_pairs_mut().append_pair("gameVersion", version);
    }
    if let Some(loader) = loader.and_then(curseforge_loader) {
        url.query_pairs_mut().append_pair("modLoaderType", loader);
    }
    let data = curseforge(CLIENT.get(url)).await?;
    Ok(data["data"]
        .as_array()
        .map(|items| items.iter().map(curseforge_file).collect())
        .unwrap_or_default())
}

pub(crate) async fn curseforge_files(file_ids: &[u64]) -> Result<Vec<Value>, String> {
    let url = api_url(CURSEFORGE_API, &["mods", "files"])?;
    let data = curseforge(CLIENT.post(url).json(&json!({ "fileIds": file_ids }))).await?;
    Ok(data["data"]
        .as_array()
        .map(|items| items.iter().map(curseforge_file).collect())
        .unwrap_or_default())
}

fn curseforge_file(file: &Value) -> Value {
    let sha1 = file["hashes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| h["algo"] == 1)
        .map(|h| h["value"].clone())
        .unwrap_or(Value::Null);
    let files: Vec<Value> = file["downloadUrl"]
        .as_str()
        .map(|url| {
            json!({
                "url": url,
                "filename": file["fileName"],
                "size": file["fileLength"],
                "primary": true,
                "hashes": { "sha1": sha1 },
            })
        })
        .into_iter()
        .collect();
    let dependencies: Vec<Value> = file["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["relationType"] == 3)
        .map(|d| json!({ "project_id": id_text(&d["modId"]), "dependency_type": "required" }))
        .collect();
    let (game_versions, loaders): (Vec<String>, Vec<String>) = file["gameVersions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .partition(|v| v.starts_with(|c: char| c.is_ascii_digit()));
    let version_type = match file["releaseType"].as_u64() {
        Some(1) => "release",
        Some(2) => "beta",
        _ => "alpha",
    };
    json!({
        "id": id_text(&file["id"]),
        "project_id": id_text(&file["modId"]),
        "name": file["displayName"],
        "version_number": file["displayName"],
        "version_type": version_type,
        "date_published": file["fileDate"],
        "loaders": loaders.iter().map(|l| l.to_lowercase()).collect::<Vec<_>>(),
        "game_versions": game_versions,
        "files": files,
        "dependencies": dependencies,
    })
}

pub async fn versions(
    source: ContentSource,
    project_id: &str,
    loader: Option<&str>,
    game_version: Option<&str>,
) -> Result<Vec<Value>, String> {
    let mut list = match source {
        ContentSource::Modrinth => {
            return fetch_modrinth_versions(project_id, loader, game_version).await
        }
        ContentSource::Hangar => hangar_versions(project_id, game_version).await?,
        ContentSource::Spigot => spigot_versions(project_id).await?,
        ContentSource::CurseForge => curseforge_versions(project_id, loader, game_version).await?,
    };
    list.sort_by_key(|v| (v["version_type"] != "release", Reverse(published_millis(v))));
    Ok(list)
}

pub async fn list_versions(
    source: ContentSource,
    project_id: String,
    loader: Option<String>,
    game_version: Option<String>,
) -> Result<Vec<ModrinthVersionOption>, String> {
    if source == ContentSource::Modrinth {
        return list_modrinth_versions(project_id, loader, game_version).await;
    }
    let loader = non_empty(&loader);
    let mut list = versions(source, &project_id, loader, non_empty(&game_version)).await?;
    if list.is_empty() && non_empty(&game_version).is_some() {
        list = versions(source, &project_id, loader, None).await?;
    }
    Ok(list
        .iter()
        .filter(|v| v["files"].as_array().is_some_and(|f| !f.is_empty()))
        .map(version_option)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_hangar_versions() {
        let version = hangar_version(&json!({
            "id": 32762,
            "name": "5.12.2",
            "createdAt": "2026-10-27T19:47:24.138887Z",
            "channel": { "name": "Release" },
            "downloads": { "PAPER": {
                "fileInfo": { "name": "ViaVersion-5.12.2.jar", "sizeBytes": 10, "sha256Hash": "abc" },
                "externalUrl": null,
                "downloadUrl": "https://hangarcdn.papermc.io/x.jar"
            }},
            "pluginDependencies": { "PAPER": [
                { "name": "ViaBackwards", "projectId": 12, "required": false, "externalUrl": null },
                { "name": "ProtocolLib", "projectId": 7, "required": true, "externalUrl": null },
                { "name": "Vault", "projectId": null, "required": true, "externalUrl": "https://x" }
            ]},
            "platformDependencies": { "PAPER": ["1.21.4"] }
        }));
        assert_eq!(version["id"], "32762");
        assert_eq!(version["version_type"], "release");
        assert_eq!(version["files"][0]["hashes"]["sha256"], "abc");
        assert_eq!(version["dependencies"].as_array().unwrap().len(), 1);
        assert_eq!(version["dependencies"][0]["project_id"], "7");
        assert_eq!(version_option(&version).file_name, "ViaVersion-5.12.2.jar");
    }

    #[test]
    fn hangar_external_downloads_have_no_files() {
        let version = hangar_version(&json!({
            "id": 1,
            "channel": { "name": "Snapshot" },
            "downloads": { "PAPER": { "fileInfo": null, "externalUrl": "https://x", "downloadUrl": null }}
        }));
        assert_eq!(version["version_type"], "alpha");
        assert!(version["files"].as_array().unwrap().is_empty());
    }

    #[test]
    fn spigot_marks_external_and_premium() {
        let file = json!({ "type": ".jar" });
        assert!(spigot_installable(
            &json!({ "external": false, "file": file })
        ));
        assert!(
            spiget_hit(&json!({ "id": 1, "external": true, "file": { "type": "external" } }))
                .external
        );
        assert!(spiget_hit(&json!({ "id": 1, "premium": true, "file": file })).external);
        assert_eq!(jar_name("Vault [Storage]"), "Vault--Storage.jar");
        assert_eq!(jar_name("???"), "plugin.jar");
    }

    #[test]
    fn maps_curseforge_files() {
        let version = curseforge_file(&json!({
            "id": 5, "modId": 9, "displayName": "Mod 1.0", "fileName": "mod-1.0.jar",
            "fileDate": "2024-01-01T00:00:00Z", "releaseType": 2,
            "downloadUrl": "https://edge.forgecdn.net/files/mod-1.0.jar",
            "hashes": [{ "value": "md5", "algo": 2 }, { "value": "sha", "algo": 1 }],
            "gameVersions": ["1.20.1", "Forge"],
            "dependencies": [{ "modId": 3, "relationType": 3 }, { "modId": 4, "relationType": 2 }]
        }));
        assert_eq!(version["version_type"], "beta");
        assert_eq!(version["files"][0]["hashes"]["sha1"], "sha");
        assert_eq!(version["loaders"], json!(["forge"]));
        assert_eq!(version["game_versions"], json!(["1.20.1"]));
        assert_eq!(
            version["dependencies"],
            json!([{ "project_id": "3", "dependency_type": "required" }])
        );
        let blocked = curseforge_file(&json!({ "id": 6, "downloadUrl": null }));
        assert!(blocked["files"].as_array().unwrap().is_empty());
    }
}
