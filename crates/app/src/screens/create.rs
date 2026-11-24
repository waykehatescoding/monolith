use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::sized;
use std::collections::HashSet;
use std::path::PathBuf;

use iced::widget::{
    button, checkbox, column, container, pick_list, progress_bar, row, scrollable, slider, text,
    text_input, toggler, Column, Row, Space,
};
use iced::{clipboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::default_plugins::list_default_plugins;
use monolith_core::events::Event;
use monolith_core::java_manager::get_required_java_version;
use monolith_core::modpack::{
    download_modrinth_modpack, inspect_modpack, install_modpack, ModpackInfo,
};
use monolith_core::modrinth::search_modrinth;
use monolith_core::ngrok_manager::get_local_ip;
use monolith_core::server_create::{create_server, get_minecraft_versions};
use monolith_core::server_runner::start_server;
use monolith_core::server_stats::get_system_info;
use monolith_core::server_transfer::import_server;
use monolith_core::types::{
    CreateServerParams, CreateServerResult, DefaultPlugin, DefaultPluginSource, MinecraftVersion,
    ModrinthSearchHit, ServerCreationProgress, ServerCreationStage, ServerRecord,
};
use monolith_theme::token::*;
use monolith_theme::{icon, icon_colored, style, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{Ctx, Route, SettingsPage};
use crate::ui::busy_button;

const MIN_RAM: u32 = 1024;
const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Step {
    #[default]
    Name,
    Platform,
    Modpack,
    Memory,
    Plugins,
    Review,
    Creating,
    Importing,
    Done,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Paper,
    Purpur,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    Vanilla,
}

impl Platform {
    const ALL: [Platform; 7] = [
        Platform::Paper,
        Platform::Purpur,
        Platform::Fabric,
        Platform::Quilt,
        Platform::Forge,
        Platform::NeoForge,
        Platform::Vanilla,
    ];

    fn name(self) -> &'static str {
        match self {
            Platform::Paper => "Paper",
            Platform::Purpur => "Purpur",
            Platform::Fabric => "Fabric",
            Platform::Quilt => "Quilt",
            Platform::Forge => "Forge",
            Platform::NeoForge => "NeoForge",
            Platform::Vanilla => "Vanilla",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Platform::Paper => {
                "Fast and stable, and runs plugins. The best choice for most servers with friends."
            }
            Platform::Purpur => {
                "Paper with extra gameplay options you can switch on. Also runs plugins."
            }
            Platform::Fabric => "Light and quick to update. Choose it to play with Fabric mods.",
            Platform::Quilt => "A Fabric fork that runs most Fabric mods plus Quilt mods.",
            Platform::Forge => "The classic mod loader. Choose it for big modpacks and older mods.",
            Platform::NeoForge => "The modern Forge fork. Most new Forge-style mods target it.",
            Platform::Vanilla => "The official server from Mojang. No plugins and no mods.",
        }
    }

    fn plugins(self) -> bool {
        matches!(self, Platform::Paper | Platform::Purpur)
    }
}

#[derive(Debug, Default)]
pub struct State {
    step: Step,
    name: String,
    platform: Platform,
    versions: Option<Result<Vec<MinecraftVersion>, String>>,
    version: Option<String>,
    snapshots: bool,
    ram: u32,
    recommended: u32,
    max_ram: u32,
    plugins: Vec<DefaultPlugin>,
    plugins_loaded: bool,
    disabled: HashSet<String>,
    analytics: bool,
    progress: Option<ServerCreationProgress>,
    error: Option<String>,
    import_error: Option<String>,
    created: Option<ServerRecord>,
    imported: bool,
    address: String,
    starting: bool,
    start_error: Option<String>,
    picking: bool,
    modpack: Option<(PathBuf, ModpackInfo)>,
    pack_query: String,
    pack_results: Option<Result<Vec<ModrinthSearchHit>, String>>,
    pack_searching: bool,
    pack_loading: Option<String>,
    pack_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Name(String),
    Next,
    Back,
    Platform(Platform),
    Versions(Platform, Result<Vec<MinecraftVersion>, String>),
    RetryVersions,
    Version(String),
    Snapshots(bool),
    Ram(u32),
    UseRecommended,
    System(u32, u32),
    PluginsLoaded(Vec<DefaultPlugin>),
    Group(String, bool),
    Plugin(String),
    Analytics(bool),
    OpenDefaults,
    Create,
    Created(CreateServerResult),
    Address(String),
    CopyAddress,
    Start,
    Started(Result<(), String>),
    Open,
    Another,
    Import,
    ImportPicked(Option<PathBuf>),
    Imported(CreateServerResult),
    UseModpack,
    PackQuery(String),
    PackSearch,
    PackSearched(Result<Vec<ModrinthSearchHit>, String>),
    PackPick(String, String),
    PackFile,
    PackFilePicked(Option<PathBuf>),
    PackLoaded(Result<(PathBuf, ModpackInfo), String>),
    PackInstalled(ServerRecord, Result<Vec<String>, String>),
}

fn plugin_key(plugin: &DefaultPlugin) -> &str {
    match &plugin.source {
        DefaultPluginSource::Modrinth { project_id, .. } => project_id,
        DefaultPluginSource::Manual { file_name, .. } => file_name,
    }
}

fn plugin_title(plugin: &DefaultPlugin) -> &str {
    match &plugin.source {
        DefaultPluginSource::Modrinth { title, .. } => title,
        DefaultPluginSource::Manual { file_name, .. } => file_name,
    }
}

fn group_of(plugin: &DefaultPlugin) -> String {
    plugin.group.clone().unwrap_or_else(|| "General".into())
}

fn version_key(id: &str) -> Vec<u32> {
    id.split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn format_gb(mb: u32) -> String {
    let gb = mb as f32 / 1024.0;
    if gb.fract() == 0.0 {
        format!("{} GB", gb as u32)
    } else {
        format!("{gb:.1} GB")
    }
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn join_address(state: &State) -> Option<String> {
    (!state.address.is_empty()).then(|| format!("{}:{}", state.address, DEFAULT_PORT))
}

impl State {
    fn visible_versions(&self) -> Vec<String> {
        let Some(Ok(list)) = &self.versions else {
            return Vec::new();
        };
        let mut stable: Vec<&MinecraftVersion> = list.iter().filter(|v| v.stable).collect();
        stable.sort_by_key(|v| std::cmp::Reverse(version_key(&v.id)));
        let rest = list.iter().filter(|v| !v.stable && self.snapshots);
        stable
            .into_iter()
            .chain(rest)
            .map(|v| v.id.clone())
            .collect()
    }

    fn keep_version_valid(&mut self) {
        let visible = self.visible_versions();
        let still_there = self
            .version
            .as_ref()
            .is_some_and(|version| visible.contains(version));
        if !still_there {
            self.version = visible.into_iter().next();
        }
    }

    fn name_problem(&self, ctx: &Ctx) -> Option<&'static str> {
        name_issue(&self.name, ctx)
    }

    fn name_ready(&self, ctx: &Ctx) -> bool {
        !self.name.trim().is_empty() && self.name_problem(ctx).is_none()
    }

    fn enabled_keys(&self) -> Vec<String> {
        self.plugins
            .iter()
            .map(plugin_key)
            .filter(|key| !self.disabled.contains(*key))
            .map(str::to_string)
            .collect()
    }

    fn plugin_step(&self) -> bool {
        self.platform.plugins() && self.modpack.is_none()
    }

    fn next_step(&self) -> Step {
        match self.step {
            Step::Name => Step::Platform,
            Step::Platform | Step::Modpack => Step::Memory,
            Step::Memory if self.plugin_step() => Step::Plugins,
            Step::Memory | Step::Plugins => Step::Review,
            other => other,
        }
    }

    fn previous_step(&self) -> Step {
        match self.step {
            Step::Platform => Step::Name,
            Step::Modpack => Step::Platform,
            Step::Memory if self.modpack.is_some() => Step::Modpack,
            Step::Memory => Step::Platform,
            Step::Plugins => Step::Memory,
            Step::Review if self.plugin_step() => Step::Plugins,
            Step::Review => Step::Memory,
            Step::Creating => Step::Review,
            other => other,
        }
    }

    fn finish(&mut self, server: ServerRecord, imported: bool, ctx: &Ctx) {
        self.created = Some(server);
        self.imported = imported;
        self.step = Step::Done;
        ctx.refresh_servers();
    }
}

fn name_issue(name: &str, ctx: &Ctx) -> Option<&'static str> {
    let name = name.trim();
    if name.chars().count() > 48 {
        return Some("Use 48 characters or fewer.");
    }
    if name.chars().any(|c| r#"<>:"/\|?*"#.contains(c)) {
        return Some(r#"Names cannot contain < > : " / \ | ? *"#);
    }
    let taken = ctx
        .servers
        .iter()
        .any(|server| server.name.trim().eq_ignore_ascii_case(name));
    (taken && !name.is_empty())
        .then_some("You already have a server with this name. Pick a different one.")
}

fn search_packs(query: &str) -> Task<Message> {
    let params = serde_json::json!({
        "query": query.trim(),
        "projectType": "modpack",
        "limit": 10,
        "sort": if query.trim().is_empty() { "downloads" } else { "relevance" },
    });
    Task::perform(
        async move { search_modrinth(params).await.map(|found| found.hits) },
        Message::PackSearched,
    )
}

async fn load_pack(path: PathBuf) -> Result<(PathBuf, ModpackInfo), String> {
    let info = inspect_modpack(path.clone()).await?;
    Ok((path, info))
}

fn load_versions(platform: Platform) -> Task<Message> {
    Task::perform(
        get_minecraft_versions(platform.name().to_string()),
        move |result| Message::Versions(platform, result),
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Name(name) => {
            state.name = name;
            state.import_error = None;
            Task::none()
        }
        Message::Next => {
            let ready = match state.step {
                Step::Name => state.name_ready(ctx),
                Step::Platform => state.version.is_some(),
                Step::Modpack => state.modpack.is_some(),
                Step::Memory | Step::Plugins => true,
                _ => false,
            };
            if ready {
                state.step = state.next_step();
            }
            Task::none()
        }
        Message::Back => {
            state.error = None;
            if state.step == Step::Modpack {
                state.modpack = None;
                state.pack_error = None;
            }
            state.step = state.previous_step();
            Task::none()
        }
        Message::Platform(platform) => {
            if state.platform == platform {
                return Task::none();
            }
            state.platform = platform;
            state.versions = None;
            state.version = None;
            load_versions(platform)
        }
        Message::Versions(platform, result) => {
            if platform == state.platform {
                state.versions = Some(result);
                state.keep_version_valid();
            }
            Task::none()
        }
        Message::RetryVersions => {
            state.versions = None;
            load_versions(state.platform)
        }
        Message::Version(version) => {
            state.version = Some(version);
            Task::none()
        }
        Message::Snapshots(on) => {
            state.snapshots = on;
            state.keep_version_valid();
            Task::none()
        }
        Message::Ram(ram) => {
            state.ram = ram;
            Task::none()
        }
        Message::UseRecommended => {
            state.ram = state.recommended;
            Task::none()
        }
        Message::System(total, max) => {
            state.max_ram = max.max(MIN_RAM);
            let quarter = (total / 4 / 512) * 512;
            state.recommended = quarter.clamp(2048, 6144).min(state.max_ram);
            if state.ram == 0 {
                state.ram = state.recommended;
            }
            Task::none()
        }
        Message::PluginsLoaded(plugins) => {
            let keys: HashSet<&str> = plugins.iter().map(plugin_key).collect();
            state.disabled.retain(|key| keys.contains(key.as_str()));
            state.plugins = plugins;
            state.plugins_loaded = true;
            Task::none()
        }
        Message::Group(group, on) => {
            for plugin in state.plugins.iter().filter(|p| group_of(p) == group) {
                let key = plugin_key(plugin).to_string();
                if on {
                    state.disabled.remove(&key);
                } else {
                    state.disabled.insert(key);
                }
            }
            Task::none()
        }
        Message::Plugin(key) => {
            if !state.disabled.remove(&key) {
                state.disabled.insert(key);
            }
            Task::none()
        }
        Message::Analytics(on) => {
            state.analytics = on;
            Task::none()
        }
        Message::OpenDefaults => {
            ctx.navigate(Route::Settings(SettingsPage::Defaults));
            Task::none()
        }
        Message::Create => {
            let (framework, version) = match &state.modpack {
                Some((_, info)) => (info.framework.clone(), info.game_version.clone()),
                None => match state.version.clone() {
                    Some(version) => (state.platform.name().to_string(), version),
                    None => return Task::none(),
                },
            };
            if state.ram == 0 {
                return Task::none();
            }
            let compatible = state.plugin_step();
            let params = CreateServerParams {
                name: state.name.trim().to_string(),
                framework,
                version,
                ram_mb: state.ram,
                enable_analytics: Some(compatible && state.analytics),
                default_plugins: compatible.then(|| state.enabled_keys()),
            };
            state.error = None;
            state.progress = None;
            state.step = Step::Creating;
            Task::perform(create_server(params), Message::Created)
        }
        Message::Created(result) => {
            match result.server {
                Some(server) if result.success => {
                    if let Some((path, _)) = &state.modpack {
                        state.progress = None;
                        let path = path.clone();
                        let server_path = server.server_path.clone();
                        return Task::perform(install_modpack(server_path, path), move |result| {
                            Message::PackInstalled(server.clone(), result)
                        });
                    }
                    state.finish(server, false, ctx)
                }
                _ => {
                    state.error = Some(result.error.unwrap_or_else(|| {
                        "The server could not be created. Check your connection and try again."
                            .into()
                    }));
                }
            }
            Task::none()
        }
        Message::Address(address) => {
            state.address = address;
            Task::none()
        }
        Message::CopyAddress => {
            let Some(address) = join_address(state) else {
                return Task::none();
            };
            ctx.success("Address copied");
            clipboard::write(address)
        }
        Message::Start => {
            let Some(server) = &state.created else {
                return Task::none();
            };
            state.starting = true;
            state.start_error = None;
            Task::perform(start_server(server.id.clone()), |result| {
                Message::Started(if result.success {
                    Ok(())
                } else {
                    Err(result
                        .error
                        .unwrap_or_else(|| "The server did not start.".into()))
                })
            })
        }
        Message::Started(result) => {
            state.starting = false;
            match result {
                Ok(()) => {
                    ctx.refresh_servers();
                    if let Some(server) = &state.created {
                        ctx.select_server(server.id.clone());
                    }
                }
                Err(error) => state.start_error = Some(error),
            }
            Task::none()
        }
        Message::Open => {
            if let Some(server) = &state.created {
                ctx.select_server(server.id.clone());
            }
            Task::none()
        }
        Message::Another => {
            *state = State::default();
            on_show(state, ctx)
        }
        Message::Import => {
            if state.picking {
                return Task::none();
            }
            state.picking = true;
            state.import_error = None;
            Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose a server zip")
                        .add_filter("Server zip", &["zip"])
                        .pick_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                Message::ImportPicked,
            )
        }
        Message::ImportPicked(None) => {
            state.picking = false;
            Task::none()
        }
        Message::ImportPicked(Some(path)) => {
            state.picking = false;
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(problem) = name_issue(&name, ctx).or(name
                .trim()
                .is_empty()
                .then_some("The zip has no usable name."))
            {
                state.import_error = Some(format!(
                    "Could not import \"{name}\". {problem} Rename the zip file and try again."
                ));
                return Task::none();
            }
            state.step = Step::Importing;
            Task::perform(
                import_server(path.to_string_lossy().into_owned(), name),
                Message::Imported,
            )
        }
        Message::UseModpack => {
            state.step = Step::Modpack;
            if state.pack_results.is_none() && !state.pack_searching {
                state.pack_searching = true;
                return search_packs(&state.pack_query);
            }
            Task::none()
        }
        Message::PackQuery(query) => {
            state.pack_query = query;
            Task::none()
        }
        Message::PackSearch => {
            state.pack_searching = true;
            search_packs(&state.pack_query)
        }
        Message::PackSearched(result) => {
            state.pack_searching = false;
            state.pack_results = Some(result);
            Task::none()
        }
        Message::PackPick(project_id, title) => {
            state.pack_loading = Some(title);
            state.pack_error = None;
            state.modpack = None;
            Task::perform(
                async move { load_pack(download_modrinth_modpack(project_id).await?).await },
                Message::PackLoaded,
            )
        }
        Message::PackFile => {
            if state.picking || state.pack_loading.is_some() {
                return Task::none();
            }
            state.picking = true;
            Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose a modpack")
                        .add_filter("Modpack", &["mrpack", "zip"])
                        .pick_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                Message::PackFilePicked,
            )
        }
        Message::PackFilePicked(None) => {
            state.picking = false;
            Task::none()
        }
        Message::PackFilePicked(Some(path)) => {
            state.picking = false;
            state.pack_loading = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            state.pack_error = None;
            state.modpack = None;
            Task::perform(load_pack(path), Message::PackLoaded)
        }
        Message::PackLoaded(result) => {
            state.pack_loading = None;
            match result {
                Ok(pack) => state.modpack = Some(pack),
                Err(error) => state.pack_error = Some(error),
            }
            Task::none()
        }
        Message::PackInstalled(server, result) => {
            match result {
                Ok(manual) if !manual.is_empty() => ctx.info(format!(
                    "CurseForge does not allow downloading {}. Get them from CurseForge and put them in the mods folder.",
                    manual.join(", ")
                )),
                Ok(_) => {}
                Err(error) => ctx.error(format!(
                    "The server was created, but the modpack did not fully install. {error}"
                )),
            }
            state.finish(server, false, ctx);
            Task::none()
        }
        Message::Imported(result) => {
            match result.server {
                Some(server) if result.success => state.finish(server, true, ctx),
                _ => {
                    state.step = Step::Name;
                    state.import_error = Some(result.error.unwrap_or_else(|| {
                        "The import failed. Use a zip made by Monolith or a zipped server folder."
                            .into()
                    }));
                }
            }
            Task::none()
        }
    }
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    if let Event::CreationProgress(progress) = event {
        if state.step == Step::Creating {
            state.progress = Some(progress.clone());
        }
    }
    Task::none()
}

pub fn on_show(state: &mut State, _ctx: &Ctx) -> Task<Message> {
    if state.step == Step::Done {
        *state = State::default();
    }
    let mut tasks = vec![
        Task::perform(async { get_system_info() }, |info| {
            Message::System(info.total_memory_mb as u32, info.max_ram_mb as u32)
        }),
        Task::perform(list_default_plugins(), Message::PluginsLoaded),
        Task::perform(get_local_ip(), Message::Address),
    ];
    if state.versions.is_none() {
        tasks.push(load_versions(state.platform));
    }
    Task::batch(tasks)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let body = match state.step {
        Step::Name => name_step(state, ctx),
        Step::Platform => platform_step(state),
        Step::Modpack => modpack_step(state),
        Step::Memory => memory_step(state),
        Step::Plugins => plugins_step(state),
        Step::Review => review_step(state),
        Step::Creating => creating_step(state),
        Step::Importing => importing_step(),
        Step::Done => done_step(state),
    };
    scrollable(
        container(body.max_width(520))
            .center_x(Length::Fill)
            .padding([SPACE_7 + SPACE_5, SPACE_5]),
    )
    .style(style::scroll)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn heading<'a>(title: String, detail: &'a str) -> Column<'a, Message> {
    column![
        text(title).size(TEXT_2XL).font(UI_SEMIBOLD),
        text(detail).size(TEXT_MD).style(style::muted)
    ]
    .spacing(SPACE_2)
}

fn primary<'a>(label: &'a str, on_press: Option<Message>) -> Element<'a, Message> {
    busy_button(label, label, false, on_press, style::primary, BUTTON_MD)
}

fn back<'a>() -> Element<'a, Message> {
    sized(
        row![
            icon(Icon::ChevronLeft, 16.0),
            text("Back").size(TEXT_MD).font(UI_MEDIUM)
        ]
        .spacing(SPACE_1)
        .align_y(Alignment::Center),
        BUTTON_MD,
    )
    .style(style::ghost)
    .on_press(Message::Back)
    .into()
}

fn footer<'a>(next: Element<'a, Message>, with_back: bool) -> Element<'a, Message> {
    let left: Element<'a, Message> = if with_back {
        back()
    } else {
        Space::new().into()
    };
    row![left, Space::new().width(Length::Fill), next]
        .align_y(Alignment::Center)
        .into()
}

fn step_layout<'a>(
    head: Column<'a, Message>,
    content: Element<'a, Message>,
    foot: Element<'a, Message>,
) -> Column<'a, Message> {
    column![head, content, foot].spacing(SPACE_6)
}

fn link<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    sized(text(label).size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
        .style(style::ghost)
        .on_press(message)
        .into()
}

fn name_step<'a>(state: &'a State, ctx: &'a Ctx) -> Column<'a, Message> {
    let problem = state.name_problem(ctx);
    let ready = state.name_ready(ctx);
    let mut input = text_input("Friends Survival", &state.name)
        .on_input(Message::Name)
        .padding([SPACE_3, SPACE_3])
        .size(TEXT_LG)
        .style(if problem.is_some() {
            style::input_error
        } else {
            style::input
        });
    if ready {
        input = input.on_submit(Message::Next);
    }
    let mut field = column![input].spacing(SPACE_2);
    if let Some(problem) = problem {
        field = field.push(text(problem).size(TEXT_SM).style(style::colored(CORAL)));
    }

    let mut import = column![row![
        text("Already have a server?")
            .size(TEXT_SM)
            .style(style::muted),
        if state.picking {
            text("Choosing a file")
                .size(TEXT_SM)
                .style(style::muted)
                .into()
        } else {
            link("Import it from a zip", Message::Import)
        },
    ]
    .align_y(Alignment::Center)]
    .spacing(SPACE_1);
    if let Some(error) = &state.import_error {
        import = import.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    }

    step_layout(
        heading(
            "Name your server".into(),
            "This is what you and your friends will see. You can rename it later.",
        ),
        column![field, import].spacing(SPACE_4).into(),
        footer(primary("Continue", ready.then_some(Message::Next)), false),
    )
}

fn platform_step(state: &State) -> Column<'_, Message> {
    let mut choices = column![].spacing(SPACE_1);
    for platform in Platform::ALL {
        let selected = state.platform == platform;
        let mark: Element<Message> = if selected {
            icon_colored(Icon::Check, 16.0, MOSS).into()
        } else {
            Space::new().width(16).into()
        };
        choices = choices.push(
            button(
                row![
                    column![
                        text(platform.name()).size(TEXT_MD).font(UI_MEDIUM),
                        text(platform.blurb()).size(TEXT_SM).style(style::muted)
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    mark
                ]
                .spacing(SPACE_3)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([SPACE_3, SPACE_4])
            .style(style::sidebar_item(selected))
            .on_press(Message::Platform(platform)),
        );
    }

    let version: Element<Message> = match &state.versions {
        None => text("Loading versions")
            .size(TEXT_SM)
            .style(style::muted)
            .into(),
        Some(Err(error)) => row![
            text(format!("Could not load versions: {error}"))
                .size(TEXT_SM)
                .style(style::colored(CORAL))
                .width(Length::Fill),
            sized(text("Try again").size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                .style(style::secondary)
                .on_press(Message::RetryVersions)
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center)
        .into(),
        Some(Ok(_)) => column![
            row![
                text("Minecraft version").size(TEXT_MD).width(Length::Fill),
                pick_list(
                    state.visible_versions(),
                    state.version.clone(),
                    Message::Version
                )
                .padding([6, 12])
                .text_size(TEXT_MD)
                .width(160)
                .style(style::pick)
                .menu_style(style::pick_menu),
            ]
            .align_y(Alignment::Center),
            checkbox(state.snapshots)
                .label("Show snapshots and pre-releases")
                .size(18)
                .text_size(TEXT_SM)
                .style(style::check)
                .on_toggle(Message::Snapshots),
        ]
        .spacing(SPACE_3)
        .into(),
    };

    let modpack = row![
        text("Want a ready-made modpack?")
            .size(TEXT_SM)
            .style(style::muted),
        link("Start from a modpack", Message::UseModpack),
    ]
    .align_y(Alignment::Center);

    step_layout(
        heading(
            "Choose a platform".into(),
            "The platform decides what your server can run. Paper is a safe pick.",
        ),
        column![choices, version, modpack].spacing(SPACE_5).into(),
        footer(
            primary("Continue", state.version.is_some().then_some(Message::Next)),
            true,
        ),
    )
}

fn modpack_step(state: &State) -> Column<'_, Message> {
    let search = row![
        text_input("Search Modrinth modpacks", &state.pack_query)
            .on_input(Message::PackQuery)
            .on_submit(Message::PackSearch)
            .padding([SPACE_2, SPACE_3])
            .style(style::input)
            .width(Length::Fill),
        busy_button(
            "Search",
            "Searching",
            state.pack_searching,
            Some(Message::PackSearch),
            style::secondary,
            BUTTON_MD,
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let results: Element<Message> = match &state.pack_results {
        None => text("Loading modpacks")
            .size(TEXT_SM)
            .style(style::muted)
            .into(),
        Some(Err(error)) => text(format!("Could not reach Modrinth: {error}"))
            .size(TEXT_SM)
            .style(style::colored(CORAL))
            .into(),
        Some(Ok(hits)) if hits.is_empty() => text("No modpacks matched. Try a shorter search.")
            .size(TEXT_SM)
            .style(style::muted)
            .into(),
        Some(Ok(hits)) => {
            let mut list = column![].spacing(SPACE_1);
            for hit in hits {
                let selected = state
                    .modpack
                    .as_ref()
                    .is_some_and(|(_, info)| info.name == hit.title);
                list = list.push(
                    button(
                        column![
                            text(&hit.title).size(TEXT_MD).font(UI_MEDIUM),
                            text(format!("by {}", hit.author))
                                .size(TEXT_SM)
                                .style(style::muted)
                        ]
                        .spacing(2),
                    )
                    .width(Length::Fill)
                    .padding([SPACE_2, SPACE_3])
                    .style(style::sidebar_item(selected))
                    .on_press_maybe(
                        state
                            .pack_loading
                            .is_none()
                            .then(|| Message::PackPick(hit.project_id.clone(), hit.title.clone())),
                    ),
                );
            }
            list.into()
        }
    };

    let file = row![
        text("Have a .mrpack or CurseForge zip?")
            .size(TEXT_SM)
            .style(style::muted),
        link("Choose a file", Message::PackFile),
    ]
    .align_y(Alignment::Center);

    let mut content = column![search, results, file].spacing(SPACE_4);
    if let Some(name) = &state.pack_loading {
        content = content.push(
            text(format!("Getting {name}"))
                .size(TEXT_SM)
                .style(style::muted),
        );
    } else if let Some(error) = &state.pack_error {
        content = content.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    } else if let Some((_, info)) = &state.modpack {
        content = content.push(
            column![
                text(format!("{} {}", info.name, info.version))
                    .size(TEXT_MD)
                    .font(UI_MEDIUM),
                text(format!(
                    "{} {}, {} server files",
                    info.framework, info.game_version, info.files
                ))
                .size(TEXT_SM)
                .style(style::muted),
            ]
            .spacing(2),
        );
    }

    step_layout(
        heading(
            "Start from a modpack".into(),
            "Monolith installs the loader, downloads the server-side mods and copies the pack's config.",
        ),
        content.into(),
        footer(
            primary(
                "Continue",
                (state.modpack.is_some() && state.pack_loading.is_none())
                    .then_some(Message::Next),
            ),
            true,
        ),
    )
}

fn memory_step(state: &State) -> Column<'_, Message> {
    let max = state.max_ram.max(state.ram).max(MIN_RAM + 512);
    let advice: Element<Message> = if state.ram == state.recommended {
        text("Recommended for your computer.")
            .size(TEXT_SM)
            .style(style::muted)
            .into()
    } else {
        row![
            text(format!(
                "{} is recommended for your computer.",
                format_gb(state.recommended)
            ))
            .size(TEXT_SM)
            .style(style::muted),
            link("Use it", Message::UseRecommended)
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
    };
    if state.max_ram == 0 {
        return step_layout(
            heading(
                "Set the memory".into(),
                "Checking how much memory your computer has.",
            ),
            text("Loading").size(TEXT_SM).style(style::muted).into(),
            footer(primary("Continue", None), true),
        );
    }
    let content = column![
        text(format_gb(state.ram)).size(TEXT_2XL).font(UI_SEMIBOLD),
        slider(MIN_RAM..=max, state.ram, Message::Ram)
            .step(256u32)
            .style(style::range),
        row![
            text(format_gb(MIN_RAM)).size(TEXT_SM).style(style::faint),
            Space::new().width(Length::Fill),
            text(format_gb(max)).size(TEXT_SM).style(style::faint),
        ],
        advice,
    ]
    .spacing(SPACE_3);
    step_layout(
        heading(
            "Set the memory".into(),
            "More memory helps with more players and plugins. Leave some for Windows and the game itself.",
        ),
        content.into(),
        footer(primary("Continue", Some(Message::Next)), true),
    )
}

fn plugin_chips<'a>(state: &'a State, members: &[&'a DefaultPlugin]) -> Element<'a, Message> {
    let mut chips = row![].spacing(SPACE_2);
    for plugin in members {
        let key = plugin_key(plugin);
        let on = !state.disabled.contains(key);
        chips = chips.push(
            sized(
                row![
                    icon_colored(
                        if on { Icon::Check } else { Icon::Plus },
                        14.0,
                        if on { MOSS } else { INK_TERTIARY }
                    ),
                    text(plugin_title(plugin)).size(TEXT_SM)
                ]
                .spacing(SPACE_1)
                .align_y(Alignment::Center),
                BUTTON_SM,
            )
            .style(if on { style::secondary } else { style::ghost })
            .on_press(Message::Plugin(key.to_string())),
        );
    }
    chips.wrap().into()
}

fn plugins_step(state: &State) -> Column<'_, Message> {
    let mut groups: Vec<(String, Vec<&DefaultPlugin>)> = Vec::new();
    for plugin in &state.plugins {
        let group = group_of(plugin);
        match groups.iter_mut().find(|(name, _)| *name == group) {
            Some((_, members)) => members.push(plugin),
            None => groups.push((group, vec![plugin])),
        }
    }

    let mut content = column![].spacing(SPACE_5);
    if !state.plugins_loaded {
        content = content.push(
            text("Loading your default plugins")
                .size(TEXT_SM)
                .style(style::muted),
        );
    } else if groups.is_empty() {
        content = content.push(
            column![
                text("You have no default plugins yet.").size(TEXT_MD),
                text("Add the plugins you want on every new server in Default plugins. They will show up here.")
                    .size(TEXT_SM)
                    .style(style::muted),
                sized(text("Open default plugins").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                    .style(style::secondary)
                    .on_press(Message::OpenDefaults),
            ]
            .spacing(SPACE_3),
        );
    }
    for (name, members) in groups {
        let active = members
            .iter()
            .filter(|p| !state.disabled.contains(plugin_key(p)))
            .count();
        let summary = format!("{active} of {} plugins", members.len());
        let chips = plugin_chips(state, &members);
        content = content.push(
            column![
                row![
                    column![
                        text(name.clone()).size(TEXT_MD).font(UI_MEDIUM),
                        text(summary).size(TEXT_SM).style(style::muted)
                    ]
                    .width(Length::Fill),
                    toggler(active > 0)
                        .style(style::switch)
                        .on_toggle(move |on| Message::Group(name.clone(), on))
                ]
                .align_y(Alignment::Center),
                chips
            ]
            .spacing(SPACE_3),
        );
    }

    content = content.push(
        toggler(state.analytics)
            .label("Add Monolith Analytics for TPS, memory and player charts")
            .text_size(TEXT_MD)
            .style(style::switch)
            .on_toggle(Message::Analytics),
    );

    step_layout(
        heading(
            "Pick starter plugins".into(),
            "Plugins add features. Switch off a whole group, or single plugins you do not want on this server.",
        ),
        content.into(),
        footer(primary("Continue", Some(Message::Next)), true),
    )
}

fn review_row<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(TEXT_MD).style(style::muted).width(140),
        text(value).size(TEXT_MD)
    ]
    .into()
}

fn review_step(state: &State) -> Column<'_, Message> {
    let (framework, version) = match &state.modpack {
        Some((_, info)) => (info.framework.as_str(), info.game_version.clone()),
        None => (
            state.platform.name(),
            state.version.clone().unwrap_or_default(),
        ),
    };
    let java = get_required_java_version(&version);
    let mut rows = column![
        review_row("Name", state.name.trim().to_string()),
        review_row("Platform", format!("{framework} {version}")),
        review_row("Memory", format_gb(state.ram)),
    ]
    .spacing(SPACE_3);
    if let Some((_, info)) = &state.modpack {
        rows = rows.push(review_row(
            "Modpack",
            format!("{} {}", info.name, info.version),
        ));
    }
    if state.plugin_step() {
        let plugins = match state.enabled_keys().len() {
            0 => "None".to_string(),
            1 => "1 default plugin".to_string(),
            n => format!("{n} default plugins"),
        };
        rows = rows.push(review_row("Plugins", plugins));
        rows = rows.push(review_row(
            "Analytics",
            if state.analytics { "On" } else { "Off" }.into(),
        ));
    }
    let content = column![
        rows,
        text(format!(
            "Monolith downloads the server and sets up Java {java} for you if it is missing. This can take a minute."
        ))
        .size(TEXT_SM)
        .style(style::muted)
    ]
    .spacing(SPACE_5)
    .into();
    step_layout(
        heading(
            "Ready to create".into(),
            "Check the details, then create your server.",
        ),
        content,
        footer(primary("Create server", Some(Message::Create)), true),
    )
}

fn creating_step(state: &State) -> Column<'_, Message> {
    if let Some(error) = &state.error {
        let content = column![
            text(error).size(TEXT_MD).style(style::colored(CORAL)),
            text("Nothing was kept, so it is safe to try again.")
                .size(TEXT_SM)
                .style(style::muted),
        ]
        .spacing(SPACE_3)
        .into();
        return step_layout(
            heading(
                "Could not create the server".into(),
                "Something went wrong along the way.",
            ),
            content,
            footer(primary("Try again", Some(Message::Create)), true),
        );
    }

    let (label, percent, detail) = match &state.progress {
        Some(progress) => {
            let label = match progress.stage {
                ServerCreationStage::CreatingFolder => "Preparing the server folder",
                ServerCreationStage::Downloading => "Downloading the server",
                ServerCreationStage::SetupJava => "Setting up Java",
                ServerCreationStage::WritingFiles => "Writing files and plugins",
                ServerCreationStage::Done => "Finishing up",
                ServerCreationStage::Error => "Something went wrong",
            };
            let detail = match (progress.bytes_downloaded, progress.bytes_total) {
                (Some(done), Some(total)) if total > 0 => {
                    format!("{} of {}", format_mb(done), format_mb(total))
                }
                _ => progress.message.clone(),
            };
            (label, progress.percent as f32, detail)
        }
        None => ("Starting", 0.0, String::new()),
    };
    let content = column![
        progress_bar(0.0..=100.0, percent)
            .girth(6)
            .style(style::progress),
        row![
            text(label).size(TEXT_MD),
            Space::new().width(Length::Fill),
            text(format!("{percent:.0}%"))
                .size(TEXT_MD)
                .style(style::muted),
        ],
        text(detail).size(TEXT_SM).style(style::muted),
    ]
    .spacing(SPACE_2);
    column![
        heading(
            format!("Creating {}", state.name.trim()),
            "Keep this window open."
        ),
        content
    ]
    .spacing(SPACE_6)
}

fn importing_step<'a>() -> Column<'a, Message> {
    column![heading(
        "Importing your server".into(),
        "Unpacking the files. Large worlds can take a minute."
    ),]
    .spacing(SPACE_6)
}

fn done_step(state: &State) -> Column<'_, Message> {
    let name = state
        .created
        .as_ref()
        .map(|server| server.name.clone())
        .unwrap_or_default();
    let title = if state.imported {
        format!("{name} is imported")
    } else {
        format!("{name} is ready")
    };
    let joined = join_address(state);
    let address: Row<Message> = row![
        text(
            joined
                .clone()
                .unwrap_or_else(|| String::from("Finding your address"))
        )
        .font(MONO)
        .size(TEXT_LG)
        .width(Length::Fill),
        sized(
            row![
                icon(Icon::Copy, 16.0),
                text("Copy").size(TEXT_MD).font(UI_MEDIUM)
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
            BUTTON_MD
        )
        .style(style::secondary)
        .on_press_maybe(joined.map(|_| Message::CopyAddress))
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    let mut content = column![
        column![
            text("Friends on your network join with this address.")
                .size(TEXT_SM)
                .style(style::muted),
            address
        ]
        .spacing(SPACE_2),
        text("To play from outside your home network, turn on sharing in the server's settings.")
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_4);
    if let Some(error) = &state.start_error {
        content = content.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    }

    let actions = row![
        sized(
            text("Create another").size(TEXT_MD).font(UI_MEDIUM),
            BUTTON_MD
        )
        .style(style::ghost)
        .on_press(Message::Another),
        sized(text("Open server").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
            .style(style::secondary)
            .on_press(Message::Open),
        Space::new().width(Length::Fill),
        busy_button(
            "Start server",
            "Starting",
            state.starting,
            Some(Message::Start),
            style::primary,
            BUTTON_MD
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    step_layout(
        heading(title, "Start it now, or open it to look around first."),
        content.into(),
        actions.into(),
    )
}
