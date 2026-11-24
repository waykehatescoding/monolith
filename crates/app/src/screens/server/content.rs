use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::{sized, sized_icon};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

use iced::widget::image::{self, Handle};
use iced::widget::{
    button, column, container, pick_list, row, rule, scrollable, text, text_input, Space,
};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::content_sources::{
    curseforge_api_key, list_versions, loader_for, search as search_source, sources_for,
};
use monolith_core::events::Event;
use monolith_core::modrinth::get_modrinth_project;
use monolith_core::modrinth_installs::{
    install_modrinth_project, list_modrinth_installs, remove_modrinth_install, set_install_enabled,
    update_modrinth_install,
};
use monolith_core::types::{
    ContentSource, ModrinthInstallEntry, ModrinthInstallRequest, ModrinthProjectDetails,
    ModrinthProjectType, ModrinthSearchHit, ModrinthSearchParams, ModrinthSearchResult,
    ModrinthSort, ModrinthVersionOption, ModrinthVersionType, SuccessResult,
};
use monolith_theme::style;
use monolith_theme::token::*;
use monolith_theme::{icon, icon_colored, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::Ctx;
use crate::demo;

const PAGE_SIZE: u32 = 20;
const DRAWER_WIDTH: f32 = 360.0;
const ICON_HOSTS: [&str; 3] = [
    "https://cdn.modrinth.com/",
    "https://hangarcdn.papermc.io/",
    "https://media.forgecdn.net/",
];
const LINK_HOSTS: [&str; 4] = [
    "https://modrinth.com/",
    "https://hangar.papermc.io/",
    "https://www.spigotmc.org/",
    "https://www.curseforge.com/",
];
const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Installed,
    Browse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortChoice {
    #[default]
    Relevance,
    Downloads,
    Updated,
    Newest,
}

impl SortChoice {
    const ALL: [SortChoice; 4] = [
        SortChoice::Relevance,
        SortChoice::Downloads,
        SortChoice::Updated,
        SortChoice::Newest,
    ];

    fn sort(self) -> ModrinthSort {
        match self {
            SortChoice::Relevance => ModrinthSort::Relevance,
            SortChoice::Downloads => ModrinthSort::Downloads,
            SortChoice::Updated => ModrinthSort::Updated,
            SortChoice::Newest => ModrinthSort::Newest,
        }
    }
}

impl fmt::Display for SortChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SortChoice::Relevance => "Best match",
            SortChoice::Downloads => "Most downloaded",
            SortChoice::Updated => "Recently updated",
            SortChoice::Newest => "Newest",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VersionChoice {
    id: String,
    label: String,
    required: Vec<String>,
}

impl fmt::Display for VersionChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

impl From<&ModrinthVersionOption> for VersionChoice {
    fn from(version: &ModrinthVersionOption) -> Self {
        let kind = match version.version_type {
            ModrinthVersionType::Release => "",
            ModrinthVersionType::Beta => " (beta)",
            ModrinthVersionType::Alpha => " (alpha)",
        };
        Self {
            id: version.id.clone(),
            label: format!("{}{}", version.version_number, kind),
            required: version.required.clone(),
        }
    }
}

#[derive(Debug)]
struct Detail {
    hit: ModrinthSearchHit,
    project: Option<Result<ModrinthProjectDetails, String>>,
    versions: Option<Result<Vec<VersionChoice>, String>>,
    selected: Option<VersionChoice>,
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    pane: Pane,
    source: ContentSource,
    curseforge: bool,
    installed: Option<Vec<ModrinthInstallEntry>>,
    latest: HashMap<String, String>,
    busy: HashSet<String>,
    updating_all: bool,
    query: String,
    sort: SortChoice,
    page: u32,
    results: Option<Result<ModrinthSearchResult, String>>,
    searching: bool,
    search_seq: u64,
    detail: Option<Detail>,
    icons: HashMap<String, Option<Handle>>,
    confirming: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(Pane),
    Source(ContentSource),
    CurseForgeKey(bool),
    Loaded(String, Vec<ModrinthInstallEntry>),
    Latest(String, HashMap<String, String>),
    Update(ContentSource, String),
    Updated(String, String, String, Result<bool, String>),
    UpdateAll,
    UpdatedAll(String, usize, Option<String>),
    AskRemove(Option<String>),
    Remove(ContentSource, String, String),
    Toggle(ContentSource, String, bool),
    Toggled(String, String, SuccessResult),
    Removed(String, String, String, Result<(), String>),
    Query(String),
    Sort(SortChoice),
    Search,
    Debounced(u64),
    Searched(u64, u32, Result<ModrinthSearchResult, String>),
    Page(bool),
    Open(Box<ModrinthSearchHit>),
    Close,
    Project(String, Result<ModrinthProjectDetails, String>),
    Versions(String, Result<Vec<VersionChoice>, String>),
    PickVersion(VersionChoice),
    Install(Box<ModrinthSearchHit>),
    Installed(String, String, String, Result<Vec<String>, String>),
    Icon(String, Option<Handle>),
    OpenLink(String),
}

struct Target {
    server_id: String,
    project_type: ModrinthProjectType,
    loader: &'static str,
    game_version: String,
}

impl Target {
    fn noun(&self) -> &'static str {
        match self.project_type {
            ModrinthProjectType::Plugin => "plugins",
            ModrinthProjectType::Mod | ModrinthProjectType::Modpack => "mods",
        }
    }

    fn request(&self, source: ContentSource, project_id: &str) -> ModrinthInstallRequest {
        ModrinthInstallRequest {
            source,
            project_id: project_id.to_string(),
            project_type: self.project_type,
            loader: Some(self.loader.to_string()),
            game_version: Some(self.game_version.clone()),
            version_id: None,
            title: None,
            slug: None,
            icon_url: None,
        }
    }
}

fn target(ctx: &Ctx) -> Option<Target> {
    let server = ctx.selected_server()?;
    let (project_type, loader) = loader_for(&server.framework)?;
    Some(Target {
        server_id: server.id.clone(),
        project_type,
        loader,
        game_version: server.version.clone(),
    })
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_safe_path(path: &str) -> bool {
    !path.contains("..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

fn icon_source(project_id: &str, icon_url: Option<&str>) -> Option<String> {
    let direct = icon_url.filter(|url| {
        let path = url.split('?').next().unwrap_or_default();
        ICON_HOSTS.iter().any(|host| {
            path.strip_prefix(host)
                .is_some_and(|path| !path.is_empty() && is_safe_path(path))
        })
    });
    match direct {
        Some(url) => Some(url.to_string()),
        None => is_slug(project_id).then(|| format!("{}data/{project_id}/icon.png", ICON_HOSTS[0])),
    }
}

async fn fetch_icon(url: String) -> Option<Handle> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|len| len > MAX_ICON_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_ICON_BYTES {
            return None;
        }
    }
    Some(Handle::from_bytes(bytes))
}

fn safe_link(url: &str) -> bool {
    LINK_HOSTS
        .iter()
        .any(|host| url.strip_prefix(host).is_some_and(is_safe_path))
}

fn load_icons(state: &mut State, sources: Vec<Option<String>>) -> Task<Message> {
    let missing: Vec<_> = sources
        .into_iter()
        .flatten()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|url| !state.icons.contains_key(url))
        .collect();
    let tasks: Vec<_> = missing
        .into_iter()
        .map(|url| {
            state.icons.insert(url.clone(), None);
            Task::perform(fetch_icon(url.clone()), move |handle| {
                Message::Icon(url.clone(), handle)
            })
        })
        .collect();
    Task::batch(tasks)
}

fn load_installed(target: &Target) -> Task<Message> {
    let id = target.server_id.clone();
    Task::perform(
        list_modrinth_installs(id.clone(), target.project_type),
        move |entries| Message::Loaded(id.clone(), entries),
    )
}

fn check_latest(target: &Target, entries: &[ModrinthInstallEntry]) -> Task<Message> {
    let id = target.server_id.clone();
    if let Some(latest) = demo::latest_versions(entries) {
        return Task::done(Message::Latest(id, latest));
    }
    let loader = target.loader.to_string();
    let game_version = target.game_version.clone();
    let projects: Vec<_> = entries
        .iter()
        .map(|e| {
            (
                e.source,
                e.project_id.clone(),
                e.loader.clone(),
                e.game_version.clone(),
            )
        })
        .collect();
    Task::perform(
        async move {
            let mut latest = HashMap::new();
            for (source, project_id, entry_loader, entry_version) in projects {
                let versions = list_versions(
                    source,
                    project_id.clone(),
                    Some(entry_loader.unwrap_or_else(|| loader.clone())),
                    Some(entry_version.unwrap_or_else(|| game_version.clone())),
                )
                .await;
                if let Some(first) = versions.ok().and_then(|v| v.into_iter().next()) {
                    latest.insert(project_id, first.id);
                }
            }
            latest
        },
        move |latest| Message::Latest(id.clone(), latest),
    )
}

fn search(state: &mut State, target: &Target, page: u32) -> Task<Message> {
    state.search_seq += 1;
    state.searching = true;
    let seq = state.search_seq;
    let params = ModrinthSearchParams {
        query: state.query.trim().to_string(),
        project_type: target.project_type,
        loader: Some(target.loader.to_string()),
        game_version: Some(target.game_version.clone()),
        limit: Some(PAGE_SIZE),
        offset: Some(page * PAGE_SIZE),
        sort: Some(state.sort.sort()),
    };
    Task::perform(search_source(state.source, params), move |result| {
        Message::Searched(seq, page, result)
    })
}

fn outdated(state: &State) -> Vec<&ModrinthInstallEntry> {
    state
        .installed
        .iter()
        .flatten()
        .filter(|e| {
            state
                .latest
                .get(&e.project_id)
                .is_some_and(|latest| *latest != e.version_id)
        })
        .collect()
}

fn is_installed(state: &State, project_id: &str) -> bool {
    state
        .installed
        .iter()
        .flatten()
        .any(|e| e.source == state.source && e.project_id == project_id)
}

fn installed_title(state: &State, project_id: &str) -> String {
    state
        .installed
        .iter()
        .flatten()
        .find(|e| e.project_id == project_id)
        .map(|e| e.title.clone())
        .unwrap_or_default()
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(target) = target(ctx) else {
        return Task::none();
    };
    let reload = |id: &str| {
        if id == target.server_id {
            load_installed(&target)
        } else {
            Task::none()
        }
    };
    match message {
        Message::ShowPane(pane) => {
            state.pane = pane;
            if pane == Pane::Browse && state.results.is_none() && !state.searching {
                return search(state, &target, 0);
            }
            Task::none()
        }
        Message::Source(source) => {
            if state.source == source {
                return Task::none();
            }
            state.source = source;
            state.results = None;
            state.detail = None;
            search(state, &target, 0)
        }
        Message::CurseForgeKey(present) => {
            state.curseforge = present;
            if !present && state.source == ContentSource::CurseForge {
                state.source = ContentSource::Modrinth;
                state.results = None;
                state.detail = None;
            }
            Task::none()
        }
        Message::Loaded(id, entries) => {
            if id != target.server_id {
                return Task::none();
            }
            let sources = entries
                .iter()
                .map(|e| icon_source(&e.project_id, e.icon_url.as_deref()))
                .collect();
            let check = check_latest(&target, &entries);
            state.latest.clear();
            state.installed = Some(entries);
            Task::batch([check, load_icons(state, sources)])
        }
        Message::Latest(id, latest) => {
            if id == target.server_id {
                state.latest = latest;
            }
            Task::none()
        }
        Message::Update(source, project_id) => {
            let title = installed_title(state, &project_id);
            state.busy.insert(project_id.clone());
            let id = target.server_id.clone();
            Task::perform(
                update_modrinth_install(id.clone(), target.request(source, &project_id)),
                move |result| {
                    let outcome = if result.success {
                        Ok(result.updated)
                    } else {
                        Err(result.error.unwrap_or_else(|| "The update failed.".into()))
                    };
                    Message::Updated(id.clone(), project_id.clone(), title.clone(), outcome)
                },
            )
        }
        Message::Updated(id, project_id, title, result) => {
            state.busy.remove(&project_id);
            match result {
                Ok(true) => ctx.success(format!("{title} updated. Restart the server to load it.")),
                Ok(false) => ctx.info(format!("{title} is already up to date")),
                Err(error) => ctx.error(format!("Couldn't update {title}. {error}")),
            }
            reload(&id)
        }
        Message::UpdateAll => {
            let requests: Vec<_> = outdated(state)
                .iter()
                .map(|e| target.request(e.source, &e.project_id))
                .collect();
            if requests.is_empty() {
                return Task::none();
            }
            state.updating_all = true;
            let id = target.server_id.clone();
            let worker_id = id.clone();
            Task::perform(
                async move {
                    let mut updated = 0;
                    let mut first_error = None;
                    for request in requests {
                        let result = update_modrinth_install(worker_id.clone(), request).await;
                        if result.success {
                            updated += usize::from(result.updated);
                        } else if first_error.is_none() {
                            first_error = result.error;
                        }
                    }
                    (updated, first_error)
                },
                move |(updated, error)| Message::UpdatedAll(id.clone(), updated, error),
            )
        }
        Message::UpdatedAll(id, updated, error) => {
            state.updating_all = false;
            match error {
                Some(error) if updated == 0 => {
                    ctx.error(format!("Couldn't update {}. {error}", target.noun()))
                }
                Some(error) => ctx.error(format!(
                    "Updated {updated}, but another update failed. {error}"
                )),
                None => ctx.success(format!("Updated {updated} {}", target.noun())),
            }
            reload(&id)
        }
        Message::AskRemove(project_id) => {
            state.confirming = project_id;
            Task::none()
        }
        Message::Remove(source, project_id, title) => {
            state.confirming = None;
            state.busy.insert(project_id.clone());
            let id = target.server_id.clone();
            Task::perform(
                remove_modrinth_install(id.clone(), source, project_id.clone()),
                move |result| {
                    let outcome = if result.success {
                        Ok(())
                    } else {
                        Err(result
                            .error
                            .unwrap_or_else(|| "The file could not be deleted.".into()))
                    };
                    Message::Removed(id.clone(), project_id.clone(), title.clone(), outcome)
                },
            )
        }
        Message::Removed(id, project_id, title, result) => {
            state.busy.remove(&project_id);
            match result {
                Ok(()) => ctx.success(format!("{title} removed")),
                Err(error) => ctx.error(format!("Couldn't remove {title}. {error}")),
            }
            reload(&id)
        }
        Message::Toggle(source, project_id, enabled) => {
            state.busy.insert(project_id.clone());
            let id = target.server_id.clone();
            Task::perform(
                set_install_enabled(id.clone(), source, project_id.clone(), enabled),
                move |result| Message::Toggled(id.clone(), project_id.clone(), result),
            )
        }
        Message::Toggled(id, project_id, result) => {
            state.busy.remove(&project_id);
            if !result.success {
                ctx.error(format!(
                    "Couldn't change {}. {}",
                    installed_title(state, &project_id),
                    result.error.unwrap_or_default()
                ));
            }
            reload(&id)
        }
        Message::Query(query) => {
            state.query = query;
            state.search_seq += 1;
            let seq = state.search_seq;
            Task::perform(tokio::time::sleep(Duration::from_millis(250)), move |_| {
                Message::Debounced(seq)
            })
        }
        Message::Debounced(seq) if seq == state.search_seq => search(state, &target, 0),
        Message::Debounced(_) => Task::none(),
        Message::Sort(sort) => {
            state.sort = sort;
            search(state, &target, 0)
        }
        Message::Search => search(state, &target, 0),
        Message::Searched(seq, page, result) => {
            if seq != state.search_seq {
                return Task::none();
            }
            state.searching = false;
            match result {
                Ok(found) => {
                    let sources = found
                        .hits
                        .iter()
                        .map(|h| icon_source(&h.project_id, h.icon_url.as_deref()))
                        .collect();
                    state.page = page;
                    state.results = Some(Ok(found));
                    load_icons(state, sources)
                }
                Err(error) if matches!(state.results, Some(Ok(_))) => {
                    ctx.error(format!("Couldn't load that page. {error}"));
                    Task::none()
                }
                Err(error) => {
                    state.results = Some(Err(error));
                    Task::none()
                }
            }
        }
        Message::Page(forward) => {
            let page = if forward {
                state.page + 1
            } else {
                state.page.saturating_sub(1)
            };
            search(state, &target, page)
        }
        Message::Open(hit) => {
            let project_id = hit.project_id.clone();
            let versions_id = project_id.clone();
            let loader = Some(target.loader.to_string());
            let game_version = Some(target.game_version.clone());
            let source = state.source;
            let external = hit.external;
            state.detail = Some(Detail {
                hit: *hit,
                project: None,
                versions: external.then(|| Ok(Vec::new())),
                selected: None,
            });
            let project = if source == ContentSource::Modrinth {
                Task::perform(get_modrinth_project(project_id.clone()), move |result| {
                    Message::Project(project_id.clone(), result)
                })
            } else {
                Task::none()
            };
            if external {
                return project;
            }
            Task::batch([
                project,
                Task::perform(
                    list_versions(source, versions_id.clone(), loader, game_version),
                    move |result| {
                        Message::Versions(
                            versions_id.clone(),
                            result.map(|v| v.iter().map(VersionChoice::from).collect()),
                        )
                    },
                ),
            ])
        }
        Message::Close => {
            state.detail = None;
            Task::none()
        }
        Message::Project(project_id, result) => {
            if let Some(detail) = state
                .detail
                .as_mut()
                .filter(|d| d.hit.project_id == project_id)
            {
                detail.project = Some(result);
            }
            Task::none()
        }
        Message::Versions(project_id, result) => {
            if let Some(detail) = state
                .detail
                .as_mut()
                .filter(|d| d.hit.project_id == project_id)
            {
                if let Ok(versions) = &result {
                    detail.selected = versions.first().cloned();
                }
                detail.versions = Some(result);
            }
            Task::none()
        }
        Message::PickVersion(choice) => {
            if let Some(detail) = &mut state.detail {
                detail.selected = Some(choice);
            }
            Task::none()
        }
        Message::Install(hit) => {
            let version_id = state
                .detail
                .as_ref()
                .filter(|d| d.hit.project_id == hit.project_id)
                .and_then(|d| d.selected.as_ref())
                .map(|v| v.id.clone());
            state.busy.insert(hit.project_id.clone());
            let id = target.server_id.clone();
            let request = ModrinthInstallRequest {
                version_id,
                title: Some(hit.title.clone()),
                slug: Some(hit.slug.clone()),
                icon_url: hit.icon_url.clone(),
                ..target.request(state.source, &hit.project_id)
            };
            Task::perform(
                install_modrinth_project(id.clone(), request),
                move |result| {
                    let outcome = if result.success {
                        Ok(result.dependencies)
                    } else {
                        Err(result
                            .error
                            .unwrap_or_else(|| "The download failed.".into()))
                    };
                    Message::Installed(
                        id.clone(),
                        hit.project_id.clone(),
                        hit.title.clone(),
                        outcome,
                    )
                },
            )
        }
        Message::Installed(id, project_id, title, result) => {
            state.busy.remove(&project_id);
            match result {
                Ok(dependencies) if dependencies.is_empty() => {
                    ctx.success(format!("{title} installed. Restart the server to load it."))
                }
                Ok(dependencies) => ctx.success(format!(
                    "{title} installed with {}. Restart the server to load them.",
                    dependencies.join(", ")
                )),
                Err(error) => ctx.error(format!("Couldn't install {title}. {error}")),
            }
            reload(&id)
        }
        Message::Icon(url, handle) => {
            match handle {
                Some(handle) => state.icons.insert(url, Some(handle)),
                None => state.icons.remove(&url),
            };
            Task::none()
        }
        Message::OpenLink(url) => {
            if !safe_link(&url) || open::that_detached(&url).is_err() {
                ctx.error("Couldn't open the link. Copy it from your browser instead.");
            }
            Task::none()
        }
    }
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}...", cut.trim_end())
    }
}

fn mod_icon<'a>(
    state: &'a State,
    project_id: &str,
    icon_url: Option<&str>,
    size: f32,
) -> Element<'a, Message> {
    match icon_source(project_id, icon_url).and_then(|source| state.icons.get(&source)) {
        Some(Some(handle)) => image::Image::new(handle.clone())
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .border_radius(RADIUS_CONTROL)
            .into(),
        _ => container(icon_colored(Icon::Package, size * 0.5, INK_TERTIARY))
            .center(Length::Fixed(size))
            .into(),
    }
}

fn primary_label<'a>(glyph: Icon, label: &'a str) -> Element<'a, Message> {
    row![
        icon_colored(glyph, 16.0, GROUND),
        text(label).size(TEXT_MD).font(UI_MEDIUM)
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

fn ghost_label<'a>(glyph: Icon, label: &'a str) -> Element<'a, Message> {
    row![icon(glyph, 16.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
}

fn centered_message<'a>(
    title: &'a str,
    body: &'a str,
    action: Option<(&'a str, Message)>,
) -> Element<'a, Message> {
    let mut content = column![
        text(title).size(TEXT_LG).font(UI_SEMIBOLD),
        text(body).size(TEXT_MD).style(style::muted),
    ]
    .spacing(SPACE_2)
    .align_x(Alignment::Center);
    if let Some((label, message)) = action {
        content = content.push(Space::new().height(SPACE_2)).push(
            sized(text(label).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::primary)
                .on_press(message),
        );
    }
    container(content)
        .center(Length::Fill)
        .padding(SPACE_6)
        .into()
}

fn tabs(state: &State) -> Element<'_, Message> {
    let installed_count = state.installed.as_ref().map_or(0, Vec::len);
    let pane = |label: String, pane: Pane| {
        button(text(label).size(TEXT_MD).font(UI_MEDIUM))
            .padding([SPACE_2, SPACE_3])
            .style(style::tab(state.pane == pane))
            .on_press(Message::ShowPane(pane))
    };
    let mut bar = row![
        pane(format!("Installed ({installed_count})"), Pane::Installed),
        pane("Browse".to_string(), Pane::Browse),
    ]
    .spacing(SPACE_1)
    .align_y(Alignment::Center);
    let pending = outdated(state).len();
    if state.pane == Pane::Installed && pending > 0 {
        let label = if state.updating_all {
            "Updating".to_string()
        } else {
            format!("Update all ({pending})")
        };
        bar = bar.push(Space::new().width(Length::Fill)).push(
            sized(text(label).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::secondary)
                .on_press_maybe((!state.updating_all).then_some(Message::UpdateAll)),
        );
    }
    bar.into()
}

fn installed_view<'a>(state: &'a State, target: &Target) -> Element<'a, Message> {
    let Some(entries) = &state.installed else {
        return container(text("Loading").size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into();
    };
    if entries.is_empty() {
        return container(
            column![
                text(format!("No {} installed", target.noun()))
                    .size(TEXT_LG)
                    .font(UI_SEMIBOLD),
                text(format!(
                    "Browse to find {}. Restart the server after installing to load them.",
                    target.noun()
                ))
                .size(TEXT_MD)
                .style(style::muted),
                Space::new().height(SPACE_2),
                sized(text("Browse").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                    .style(style::primary)
                    .on_press(Message::ShowPane(Pane::Browse)),
            ]
            .spacing(SPACE_2)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into();
    }

    let mut list = column![];
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            list = list.push(rule::horizontal(1).style(style::hairline));
        }
        let has_update = state
            .latest
            .get(&entry.project_id)
            .is_some_and(|latest| *latest != entry.version_id);
        let busy = state.busy.contains(&entry.project_id) || state.updating_all;
        let mut line = row![
            mod_icon(state, &entry.project_id, entry.icon_url.as_deref(), 36.0),
            column![
                text(&entry.title).size(TEXT_MD).font(UI_MEDIUM),
                text(match entry.source {
                    ContentSource::Modrinth => entry.file_name.clone(),
                    source => format!("{} from {}", entry.file_name, source.label()),
                })
                .size(TEXT_SM)
                .style(style::muted),
            ]
            .spacing(2)
            .width(Length::Fill),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center);
        if has_update {
            line = line
                .push(
                    text("Update available")
                        .size(TEXT_SM)
                        .style(style::colored(MOSS)),
                )
                .push(
                    sized(text("Update").size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                        .style(style::secondary)
                        .on_press_maybe(
                            (!busy)
                                .then(|| Message::Update(entry.source, entry.project_id.clone())),
                        ),
                );
        } else if busy {
            line = line.push(text("Working").size(TEXT_SM).style(style::muted));
        } else if entry.disabled {
            line = line.push(text("Disabled").size(TEXT_SM).style(style::muted));
        }
        line =
            line.push(
                sized(
                    text(if entry.disabled { "Enable" } else { "Disable" })
                        .size(TEXT_SM)
                        .font(UI_MEDIUM),
                    BUTTON_SM,
                )
                .style(style::secondary)
                .on_press_maybe((!busy).then(|| {
                    Message::Toggle(entry.source, entry.project_id.clone(), entry.disabled)
                })),
            );
        if state.confirming.as_deref() == Some(entry.project_id.as_str()) {
            line = line
                .push(
                    sized(text("Keep").size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                        .style(style::secondary)
                        .on_press(Message::AskRemove(None)),
                )
                .push(
                    sized(text("Delete file").size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                        .style(style::danger)
                        .on_press_maybe((!busy).then(|| {
                            Message::Remove(
                                entry.source,
                                entry.project_id.clone(),
                                entry.title.clone(),
                            )
                        })),
                );
        } else {
            line = line.push(
                sized(text("Remove").size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                    .style(style::danger)
                    .on_press_maybe(
                        (!busy).then(|| Message::AskRemove(Some(entry.project_id.clone()))),
                    ),
            );
        }
        list = list.push(container(line).padding([SPACE_3, 0.0]));
    }
    scrollable(list)
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn result_row<'a>(state: &'a State, hit: &'a ModrinthSearchHit) -> Element<'a, Message> {
    let selected = state
        .detail
        .as_ref()
        .is_some_and(|d| d.hit.project_id == hit.project_id);
    let info = row![
        mod_icon(state, &hit.project_id, hit.icon_url.as_deref(), 40.0),
        column![
            row![
                text(&hit.title).size(TEXT_MD).font(UI_MEDIUM),
                text(by(hit)).size(TEXT_SM).style(style::muted),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
            text(truncate(&hit.description, 110))
                .size(TEXT_SM)
                .style(style::muted),
        ]
        .spacing(2)
        .width(Length::Fill),
        text(format!("{} downloads", compact(hit.downloads)))
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    let installing = state.busy.contains(&hit.project_id);
    let action: Element<'a, Message> = if is_installed(state, &hit.project_id) {
        container(text("Installed").size(TEXT_SM).style(style::muted))
            .center_x(Length::Fixed(84.0))
            .into()
    } else if hit.external {
        sized(
            text("Open")
                .size(TEXT_SM)
                .font(UI_MEDIUM)
                .width(Length::Fill)
                .align_x(Alignment::Center),
            BUTTON_SM,
        )
        .width(Length::Fixed(84.0))
        .style(style::secondary)
        .on_press_maybe(hit.page_url.clone().map(Message::OpenLink))
        .into()
    } else {
        sized(
            text(if installing { "Installing" } else { "Install" })
                .size(TEXT_SM)
                .font(UI_MEDIUM)
                .width(Length::Fill)
                .align_x(Alignment::Center),
            BUTTON_SM,
        )
        .width(Length::Fixed(84.0))
        .style(style::secondary)
        .on_press_maybe((!installing).then(|| Message::Install(Box::new(hit.clone()))))
        .into()
    };

    row![
        button(info)
            .width(Length::Fill)
            .padding([SPACE_2, SPACE_3])
            .style(style::sidebar_item(selected))
            .on_press(Message::Open(Box::new(hit.clone()))),
        action,
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

fn by(hit: &ModrinthSearchHit) -> String {
    if hit.author.is_empty() {
        String::new()
    } else {
        format!("by {}", hit.author)
    }
}

fn source_tabs(state: &State, target: &Target) -> Option<Element<'static, Message>> {
    let sources = sources_for(target.project_type, state.curseforge);
    (sources.len() > 1).then(|| {
        let mut bar = row![].spacing(SPACE_1);
        for source in sources {
            bar = bar.push(
                button(text(source.label()).size(TEXT_SM).font(UI_MEDIUM))
                    .padding([SPACE_1, SPACE_3])
                    .style(style::tab(state.source == source))
                    .on_press(Message::Source(source)),
            );
        }
        bar.into()
    })
}

fn browse_view<'a>(state: &'a State, target: &Target) -> Element<'a, Message> {
    let search_button = if state.searching {
        sized(ghost_label(Icon::Search, "Searching"), BUTTON_MD).style(style::secondary)
    } else {
        sized(primary_label(Icon::Search, "Search"), BUTTON_MD)
            .style(style::primary)
            .on_press(Message::Search)
    };
    let controls = row![
        text_input(&format!("Search {}", target.noun()), &state.query)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .padding([SPACE_2, SPACE_3])
            .style(style::input)
            .width(Length::Fill),
        pick_list(SortChoice::ALL, Some(state.sort), Message::Sort)
            .padding([SPACE_2, SPACE_3])
            .style(style::pick)
            .menu_style(style::pick_menu),
        search_button,
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let body: Element<'a, Message> = match &state.results {
        None => container(text("Loading").size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into(),
        Some(Err(error)) => centered_message(
            match state.source {
                ContentSource::Modrinth => "Couldn't reach Modrinth",
                ContentSource::Hangar => "Couldn't reach Hangar",
                ContentSource::Spigot => "Couldn't reach Spigot",
                ContentSource::CurseForge => "Couldn't reach CurseForge",
            },
            if error.is_empty() {
                "Check your internet connection, then try again."
            } else {
                error
            },
            Some(("Try again", Message::Search)),
        ),
        Some(Ok(found)) if found.hits.is_empty() => centered_message(
            "Nothing matched",
            "Try a shorter search, or a different sort order.",
            None,
        ),
        Some(Ok(found)) => {
            let mut list = column![].spacing(SPACE_1);
            for hit in &found.hits {
                list = list.push(result_row(state, hit));
            }
            let first = state.page * PAGE_SIZE + 1;
            let last = first + found.hits.len() as u32 - 1;
            let can_page = !state.searching;
            let pager = row![
                text(format!("{first} to {last} of {}", found.total_hits))
                    .size(TEXT_SM)
                    .style(style::muted),
                Space::new().width(Length::Fill),
                sized_icon(icon(Icon::ChevronLeft, 16.0), BUTTON_MD)
                    .style(style::icon_button)
                    .on_press_maybe((can_page && state.page > 0).then_some(Message::Page(false))),
                sized_icon(icon(Icon::ChevronRight, 16.0), BUTTON_MD)
                    .style(style::icon_button)
                    .on_press_maybe(
                        (can_page && u64::from(last) < found.total_hits)
                            .then_some(Message::Page(true))
                    ),
            ]
            .spacing(SPACE_1)
            .align_y(Alignment::Center);
            column![
                scrollable(list)
                    .style(style::scroll)
                    .width(Length::Fill)
                    .height(Length::Fill),
                pager,
            ]
            .spacing(SPACE_2)
            .into()
        }
    };

    let list_side = column![]
        .push(source_tabs(state, target))
        .push(controls)
        .push(body)
        .spacing(SPACE_4)
        .width(Length::Fill)
        .height(Length::Fill);
    match &state.detail {
        Some(detail) => row![list_side, drawer(state, detail)]
            .spacing(SPACE_5)
            .height(Length::Fill)
            .into(),
        None => list_side.into(),
    }
}

fn drawer<'a>(state: &'a State, detail: &'a Detail) -> Element<'a, Message> {
    let hit = &detail.hit;
    let head = row![
        mod_icon(state, &hit.project_id, hit.icon_url.as_deref(), 48.0),
        column![
            text(&hit.title).size(TEXT_LG).font(UI_SEMIBOLD),
            text(by(hit)).size(TEXT_SM).style(style::muted),
        ]
        .spacing(2)
        .width(Length::Fill),
        sized_icon(icon(Icon::Close, 14.0), BUTTON_SM)
            .style(style::icon_button)
            .on_press(Message::Close),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    let mut body = column![head, text(&hit.description).size(TEXT_MD)].spacing(SPACE_4);

    let (followers, categories, link) = match &detail.project {
        Some(Ok(project)) => (
            Some(project.followers),
            project
                .categories
                .clone()
                .or_else(|| hit.categories.clone()),
            project.project_url.clone().or_else(|| hit.page_url.clone()),
        ),
        _ => (None, hit.categories.clone(), hit.page_url.clone()),
    };
    let mut facts = format!("{} downloads", compact(hit.downloads));
    if let Some(followers) = followers {
        facts.push_str(&format!(", {} followers", compact(followers)));
    }
    body = body.push(text(facts).size(TEXT_SM).style(style::muted));
    if let Some(categories) = categories.filter(|c| !c.is_empty()) {
        body = body.push(
            text(categories.join(", "))
                .size(TEXT_SM)
                .style(style::muted),
        );
    }
    if let Some(Ok(project)) = &detail.project {
        if !project.body.trim().is_empty() {
            body = body.push(
                text(truncate(project.body.trim(), 700))
                    .size(TEXT_SM)
                    .style(style::muted),
            );
        }
    }
    if let Some(Err(error)) = &detail.project {
        body = body.push(
            text(format!("Couldn't load the full details. {error}"))
                .size(TEXT_SM)
                .style(style::colored(CORAL)),
        );
    }

    let version_picker: Element<'a, Message> = match &detail.versions {
        _ if hit.external => text(
            "This is hosted on an external site, so it can't be installed here. Open it in your browser to download it.",
        )
        .size(TEXT_SM)
        .style(style::muted)
        .into(),
        None => text("Finding versions")
            .size(TEXT_SM)
            .style(style::muted)
            .into(),
        Some(Err(error)) => text(format!("Couldn't load versions. {error}"))
            .size(TEXT_SM)
            .style(style::colored(CORAL))
            .into(),
        Some(Ok(versions)) if versions.is_empty() => text("No versions found for this server")
            .size(TEXT_SM)
            .style(style::muted)
            .into(),
        Some(Ok(versions)) => column![
            text("Version").size(TEXT_SM).style(style::muted),
            pick_list(
                versions.clone(),
                detail.selected.clone(),
                Message::PickVersion
            )
            .padding([SPACE_2, SPACE_3])
            .width(Length::Fill)
            .style(style::pick)
            .menu_style(style::pick_menu),
        ]
        .spacing(SPACE_1)
        .into(),
    };
    body = body.push(version_picker);
    let missing = detail
        .selected
        .iter()
        .flat_map(|v| &v.required)
        .filter(|id| !is_installed(state, id))
        .count();
    if missing > 0 {
        body = body.push(
            text(if missing == 1 {
                "Also installs 1 required dependency.".to_string()
            } else {
                format!("Also installs {missing} required dependencies.")
            })
            .size(TEXT_SM)
            .style(style::muted),
        );
    }

    let installing = state.busy.contains(&hit.project_id);
    let label = if installing {
        "Installing"
    } else if is_installed(state, &hit.project_id) {
        "Reinstall"
    } else {
        "Install"
    };
    if !hit.external {
        body = body.push(
            sized(
                text(label)
                    .size(TEXT_MD)
                    .font(UI_MEDIUM)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
                BUTTON_MD,
            )
            .width(Length::Fill)
            .padding([SPACE_2 + 2.0, SPACE_4])
            .style(style::primary)
            .on_press_maybe(
                (detail.selected.is_some() && !installing)
                    .then(|| Message::Install(Box::new(hit.clone()))),
            ),
        );
    }
    if let Some(url) = link {
        let label = if hit.external {
            "Open in browser"
        } else {
            match state.source {
                ContentSource::Modrinth => "View on Modrinth",
                ContentSource::Hangar => "View on Hangar",
                ContentSource::Spigot => "View on Spigot",
                ContentSource::CurseForge => "View on CurseForge",
            }
        };
        let button = sized(ghost_label(Icon::ExternalLink, label), BUTTON_MD)
            .on_press(Message::OpenLink(url));
        body = body.push(if hit.external {
            button.style(style::primary)
        } else {
            button.style(style::ghost)
        });
    }

    container(
        scrollable(body.padding(SPACE_5))
            .style(style::scroll)
            .height(Length::Fill),
    )
    .width(Length::Fixed(DRAWER_WIDTH))
    .height(Length::Fill)
    .style(style::panel)
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(target) = target(ctx) else {
        return centered_message(
            "This server can't use plugins or mods",
            "Vanilla servers have no add-on support. Switch to a plugin or mod platform in Settings to install content.",
            None,
        );
    };
    let body = match state.pane {
        Pane::Installed => installed_view(state, &target),
        Pane::Browse => browse_view(state, &target),
    };
    column![tabs(state), body]
        .spacing(SPACE_4)
        .height(Length::Fill)
        .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(target) = target(ctx) else {
        return Task::none();
    };
    let key = format!("{}@{}", target.server_id, target.game_version);
    if state.server_id.as_deref() != Some(key.as_str()) {
        *state = State {
            server_id: Some(key),
            search_seq: state.search_seq,
            icons: std::mem::take(&mut state.icons),
            ..State::default()
        };
    }
    Task::batch([
        load_installed(&target),
        Task::perform(curseforge_api_key(), |key| {
            Message::CurseForgeKey(key.is_some())
        }),
    ])
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
