use iced::widget::{
    column, container, pick_list, progress_bar, row, rule, scrollable, slider, text,
    text_editor, text_input, toggler, Space,
};
use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::{sized};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use monolith_core::events::Event;
use monolith_core::java_manager::discover_installed_javas;
use monolith_core::schedule::parse_times;
use monolith_core::server_create::get_minecraft_versions;
use monolith_core::server_framework::change_server_framework;
use monolith_core::server_props::{get_server_properties, save_server_properties};
use monolith_core::server_stats::get_system_info;
use monolith_core::server_store::{delete_server, update_server_settings};
use monolith_core::server_transfer::export_server;
use monolith_core::types::{
    MinecraftVersion, ServerProperty, ServerRecord, ServerRuntimeStatus, ServerSettingsUpdate,
    SuccessResult, SystemInfo,
};
use monolith_theme::token::{
    CORAL, INK, INK_TERTIARY, LAYER_2, MOSS, RADIUS_CONTROL, SPACE_1, SPACE_2, SPACE_3, SPACE_4,
    SPACE_5, SPACE_6, SPACE_7, TEXT_MD, TEXT_SM, TEXT_XL,
};
use monolith_theme::{style, MONO, UI_SEMIBOLD};

use crate::ctx::{Confirm, Ctx};
use crate::ui::busy_button;

const FRAMEWORKS: [&str; 4] = ["Paper", "Purpur", "Fabric", "Vanilla"];
const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];
const GAMEMODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];
const CONTROL_WIDTH: f32 = 260.0;

pub struct State {
    server_id: Option<String>,
    name: String,
    ram_mb: u32,
    java_custom: bool,
    java_path: String,
    javas: Vec<JavaChoice>,
    system: SystemInfo,
    java_problem: Option<&'static str>,
    saving_general: bool,
    loaded: bool,
    props: Vec<ServerProperty>,
    saved_props: Vec<ServerProperty>,
    raw: text_editor::Content,
    raw_open: bool,
    raw_error: Option<String>,
    saving_props: bool,
    saving_reliability: bool,
    new_time: String,
    time_error: Option<String>,
    framework: String,
    version: Option<String>,
    snapshots: bool,
    versions: Vec<MinecraftVersion>,
    versions_loading: bool,
    versions_error: Option<String>,
    changing: Option<(f64, String)>,
    platform_armed: bool,
    exporting: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            server_id: None,
            name: String::new(),
            ram_mb: 2048,
            java_custom: false,
            java_path: String::new(),
            javas: Vec::new(),
            system: SystemInfo {
                total_memory_mb: 8192,
                max_ram_mb: 6553,
            },
            java_problem: None,
            saving_general: false,
            loaded: false,
            props: Vec::new(),
            saved_props: Vec::new(),
            raw: text_editor::Content::new(),
            raw_open: false,
            raw_error: None,
            saving_props: false,
            saving_reliability: false,
            new_time: String::new(),
            time_error: None,
            framework: String::new(),
            version: None,
            snapshots: false,
            versions: Vec::new(),
            versions_loading: false,
            versions_error: None,
            changing: None,
            platform_armed: false,
            exporting: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JavaChoice {
    path: String,
    version: u32,
}

impl std::fmt::Display for JavaChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Java {} - {}", self.version, self.path)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Vec<ServerProperty>, Vec<JavaChoice>),
    Name(String),
    Ram(u32),
    JavaCustom(bool),
    JavaPath(String),
    BrowseJava,
    JavaBrowsed(Option<String>),
    SaveGeneral,
    GeneralSaved(Result<(), String>),
    Prop(&'static str, String),
    PropToggle(&'static str, bool),
    ToggleRaw,
    RawAction(text_editor::Action),
    SaveProps,
    PropsSaved(Result<(), String>),
    AutoRestart(bool),
    StartOnLaunch(bool),
    ReliabilitySaved(Result<(), String>),
    NewTime(String),
    AddTime,
    RemoveTime(usize),
    Framework(&'static str),
    Version(String),
    Snapshots(bool),
    RetryVersions,
    Versions(String, String, Result<Vec<MinecraftVersion>, String>),
    ApplyPlatform,
    CancelPlatform,
    PlatformChanged(Result<String, String>),
    Export,
    Exported(Result<Option<String>, String>),
    Delete,
}

fn outcome(result: SuccessResult, fallback: &str) -> Result<(), String> {
    if result.success {
        Ok(())
    } else {
        Err(result.error.unwrap_or_else(|| fallback.to_string()))
    }
}

fn prop<'a>(props: &'a [ServerProperty], key: &str, default: &'a str) -> &'a str {
    props
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.as_str())
        .unwrap_or(default)
}

fn set_prop(props: &mut Vec<ServerProperty>, key: &str, value: String) {
    match props.iter_mut().find(|p| p.key == key) {
        Some(existing) => existing.value = value,
        None => props.push(ServerProperty {
            key: key.to_string(),
            value,
            comment: None,
        }),
    }
}

fn raw_text(props: &[ServerProperty]) -> String {
    props
        .iter()
        .map(|p| format!("{}={}", p.key, p.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_raw(text: &str, previous: &[ServerProperty]) -> Result<Vec<ServerProperty>, String> {
    let mut props: Vec<ServerProperty> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("Line {} needs a key and a value, like pvp=true.", index + 1));
        };
        let key = key.trim().to_string();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return Err(format!("Line {} has an invalid key.", index + 1));
        }
        if props.iter().any(|p| p.key == key) {
            return Err(format!("Line {} repeats the key {key}.", index + 1));
        }
        let comment = previous
            .iter()
            .find(|p| p.key == key)
            .and_then(|p| p.comment.clone());
        props.push(ServerProperty {
            key,
            value: value.trim().to_string(),
            comment,
        });
    }
    Ok(props)
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.');
    if cleaned.is_empty() {
        "server".into()
    } else {
        cleaned.into()
    }
}

fn digits(value: String) -> String {
    value.chars().filter(char::is_ascii_digit).take(6).collect()
}

impl State {
    fn reset(&mut self, server: &ServerRecord) {
        let javas = std::mem::take(&mut self.javas);
        *self = State::default();
        self.javas = javas;
        self.system = get_system_info();
        self.server_id = Some(server.id.clone());
        self.name = server.name.clone();
        self.ram_mb = server.ram_mb.max(512);
        self.java_path = server.java_path.clone().unwrap_or_default();
        self.java_custom = !self.java_path.is_empty();
        self.check_java();
        self.framework = server.framework.clone();
        self.version = Some(server.version.clone());
    }

    fn effective_java(&self) -> &str {
        if self.java_custom {
            self.java_path.trim()
        } else {
            ""
        }
    }

    fn general_dirty(&self, server: &ServerRecord) -> bool {
        self.name.trim() != server.name
            || self.ram_mb != server.ram_mb.max(512)
            || self.effective_java() != server.java_path.as_deref().unwrap_or("")
    }

    fn number_ok(&self, key: &str, default: &str, min: u32, max: u32) -> bool {
        prop(&self.props, key, default)
            .parse::<u32>()
            .is_ok_and(|n| (min..=max).contains(&n))
    }

    fn props_error(&self) -> Option<&'static str> {
        if !self.number_ok("server-port", "25565", 1, 65535) {
            return Some("Port must be a number from 1 to 65535");
        }
        if !self.number_ok("max-players", "20", 1, 100_000) {
            return Some("Max players must be at least 1");
        }
        if !self.number_ok("view-distance", "10", 2, 32) {
            return Some("View distance must be between 2 and 32");
        }
        if !self.number_ok("spawn-protection", "16", 0, 100_000) {
            return Some("Spawn protection must be a whole number");
        }
        None
    }

    fn check_java(&mut self) {
        let path = self.java_path.trim();
        self.java_problem = if !self.java_custom {
            None
        } else if path.is_empty() {
            Some("Enter the path to java.exe, or turn custom Java off.")
        } else if !std::path::Path::new(path).is_file() {
            Some("That file does not exist. Browse to your java.exe.")
        } else {
            None
        };
    }

    fn commit_raw(&mut self) -> bool {
        match parse_raw(&self.raw.text(), &self.props) {
            Ok(props) => {
                self.props = props;
                self.raw_error = None;
                true
            }
            Err(error) => {
                self.raw_error = Some(error);
                false
            }
        }
    }

    fn fetch_versions(&mut self, framework: &str) -> Task<Message> {
        self.versions_loading = true;
        self.versions_error = None;
        let framework = framework.to_string();
        let id = self.server_id.clone().unwrap_or_default();
        Task::perform(
            {
                let framework = framework.clone();
                async move { get_minecraft_versions(framework).await }
            },
            move |result| Message::Versions(id.clone(), framework.clone(), result),
        )
    }

    fn version_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .versions
            .iter()
            .filter(|v| self.snapshots || v.stable)
            .map(|v| v.id.clone())
            .collect();
        if let Some(current) = &self.version {
            if !ids.contains(current) {
                ids.insert(0, current.clone());
            }
        }
        ids
    }
}

async fn save_settings(id: String, update: ServerSettingsUpdate) -> Result<(), String> { outcome(update_server_settings(id, update).await, "Could not save") }

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match message {
        Message::Loaded(loaded_id, props, javas) => {
            if loaded_id != id {
                return Task::none();
            }
            state.javas = javas;
            if !state.loaded || state.props == state.saved_props {
                state.saved_props = props.clone();
                state.props = props;
                state.raw_error = None;
            }
            state.loaded = true;
            Task::none()
        }
        Message::Name(name) => {
            state.name = name;
            Task::none()
        }
        Message::Ram(ram) => {
            state.ram_mb = ram;
            Task::none()
        }
        Message::JavaCustom(on) => {
            state.java_custom = on;
            state.check_java();
            Task::none()
        }
        Message::JavaPath(path) => {
            state.java_path = path;
            state.check_java();
            Task::none()
        }
        Message::BrowseJava => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("Java", &["exe"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_string_lossy().into_owned())
            },
            Message::JavaBrowsed,
        ),
        Message::JavaBrowsed(path) => {
            if let Some(path) = path {
                state.java_path = path;
                state.check_java();
            }
            Task::none()
        }
        Message::SaveGeneral => {
            let name = state.name.trim().to_string();
            if name.is_empty() {
                ctx.error("Give the server a name before saving.");
                return Task::none();
            }
            if let Some(problem) = state.java_problem {
                ctx.error(problem);
                return Task::none();
            }
            state.saving_general = true;
            let update = ServerSettingsUpdate {
                name: Some(name),
                ram_mb: Some(state.ram_mb),
                java_path: Some(state.effective_java().to_string()),
                ..Default::default()
            };
            Task::perform(save_settings(id, update), Message::GeneralSaved)
        }
        Message::GeneralSaved(result) => {
            state.saving_general = false;
            match result {
                Ok(()) => {
                    ctx.success("Saved. Memory and Java changes apply the next time the server starts.");
                    ctx.refresh_servers();
                }
                Err(error) => ctx.error(format!("Could not save settings: {error}")),
            }
            Task::none()
        }
        Message::Prop(key, value) => {
            let value = if matches!(key, "motd" | "difficulty" | "gamemode") {
                value
            } else {
                digits(value)
            };
            set_prop(&mut state.props, key, value);
            Task::none()
        }
        Message::PropToggle(key, on) => {
            set_prop(&mut state.props, key, on.to_string());
            Task::none()
        }
        Message::ToggleRaw => {
            if state.raw_open {
                if state.commit_raw() {
                    state.raw_open = false;
                }
            } else {
                state.raw = text_editor::Content::with_text(&raw_text(&state.props));
                state.raw_open = true;
            }
            Task::none()
        }
        Message::RawAction(action) => {
            state.raw.perform(action);
            Task::none()
        }
        Message::SaveProps => {
            if !state.loaded || (state.raw_open && !state.commit_raw()) {
                return Task::none();
            }
            if let Some(error) = state.props_error() {
                ctx.error(error);
                return Task::none();
            }
            state.saving_props = true;
            let props = state.props.clone();
            Task::perform(
                async move {
                    outcome(
                        save_server_properties(id, props).await,
                        "Could not write server.properties",
                    )
                },
                Message::PropsSaved,
            )
        }
        Message::PropsSaved(result) => {
            state.saving_props = false;
            match result {
                Ok(()) => {
                    state.saved_props = state.props.clone();
                    state.raw_open = false;
                    ctx.success("Gameplay settings saved. Restart the server to apply them.");
                }
                Err(error) => ctx.error(format!("Could not save gameplay settings: {error}")),
            }
            Task::none()
        }
        Message::AutoRestart(on) => save_reliability(
            state,
            id,
            ServerSettingsUpdate {
                auto_restart: Some(on),
                ..Default::default()
            },
        ),
        Message::StartOnLaunch(on) => save_reliability(
            state,
            id,
            ServerSettingsUpdate {
                start_on_launch: Some(on),
                ..Default::default()
            },
        ),
        Message::ReliabilitySaved(result) => {
            state.saving_reliability = false;
            match result {
                Ok(()) => {
                    ctx.success("Saved.");
                    ctx.refresh_servers();
                }
                Err(error) => ctx.error(format!("Could not save: {error}")),
            }
            Task::none()
        }
        Message::NewTime(time) => {
            state.new_time = time;
            state.time_error = None;
            Task::none()
        }
        Message::AddTime => {
            if state.saving_reliability {
                return Task::none();
            }
            let Some(server) = ctx.server(&id) else {
                return Task::none();
            };
            let mut times = server.restart_times.clone();
            times.push(state.new_time.clone());
            match parse_times(&times) {
                Ok(times) => {
                    state.new_time.clear();
                    save_reliability(
                        state,
                        id,
                        ServerSettingsUpdate {
                            restart_times: Some(times),
                            ..Default::default()
                        },
                    )
                }
                Err(_) => {
                    state.time_error = Some("Use 24-hour time like 04:30".into());
                    Task::none()
                }
            }
        }
        Message::RemoveTime(index) => {
            if state.saving_reliability {
                return Task::none();
            }
            let Some(server) = ctx.server(&id) else {
                return Task::none();
            };
            let mut times = server.restart_times.clone();
            if index < times.len() {
                times.remove(index);
            }
            save_reliability(
                state,
                id,
                ServerSettingsUpdate {
                    restart_times: Some(times),
                    ..Default::default()
                },
            )
        }
        Message::Framework(framework) => {
            state.framework = framework.to_string();
            state.version = None;
            state.versions.clear();
            state.platform_armed = false;
            state.fetch_versions(framework)
        }
        Message::RetryVersions => {
            let framework = state.framework.clone();
            state.fetch_versions(&framework)
        }
        Message::Version(version) => {
            state.version = Some(version);
            state.platform_armed = false;
            Task::none()
        }
        Message::Snapshots(on) => {
            state.snapshots = on;
            Task::none()
        }
        Message::Versions(versions_id, framework, result) => {
            if versions_id != id || framework != state.framework {
                return Task::none();
            }
            state.versions_loading = false;
            match result {
                Ok(versions) => {
                    if state.version.is_none() {
                        state.version = versions.iter().find(|v| v.stable).map(|v| v.id.clone());
                    }
                    state.versions = versions;
                }
                Err(error) => state.versions_error = Some(error),
            }
            Task::none()
        }
        Message::CancelPlatform => {
            state.platform_armed = false;
            Task::none()
        }
        Message::ApplyPlatform => {
            let idle = matches!(
                ctx.status(&id),
                ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle
            );
            let Some(version) = state.version.clone() else {
                return Task::none();
            };
            if !idle || state.changing.is_some() {
                return Task::none();
            }
            if !state.platform_armed {
                state.platform_armed = true;
                return Task::none();
            }
            state.platform_armed = false;
            let framework = state.framework.clone();
            state.changing = Some((0.0, "Starting".into()));
            Task::perform(
                async move {
                    let label = format!("{framework} {version}");
                    let result = change_server_framework(id, framework, version).await;
                    if result.success {
                        Ok(label)
                    } else {
                        Err(result.error.unwrap_or_else(|| "The change failed".into()))
                    }
                },
                Message::PlatformChanged,
            )
        }
        Message::PlatformChanged(result) => {
            state.changing = None;
            match result {
                Ok(label) => {
                    ctx.success(format!("Server now runs {label}."));
                    ctx.refresh_servers();
                }
                Err(error) => ctx.error(format!("Could not change the server software: {error}")),
            }
            Task::none()
        }
        Message::Export => {
            state.exporting = true;
            let file = ctx
                .server(&id)
                .map(|s| format!("{}.zip", safe_file_name(&s.name)))
                .unwrap_or_else(|| "server.zip".into());
            Task::perform(
                async move {
                    let Some(target) = rfd::AsyncFileDialog::new()
                        .set_file_name(file)
                        .add_filter("Zip archive", &["zip"])
                        .save_file()
                        .await
                    else {
                        return Ok(None);
                    };
                    let result = export_server(id, target.path().to_path_buf()).await;
                    if result.success {
                        Ok(result.path)
                    } else {
                        Err(result.error.unwrap_or_else(|| "Export failed".into()))
                    }
                },
                Message::Exported,
            )
        }
        Message::Exported(result) => {
            state.exporting = false;
            match result {
                Ok(Some(path)) => ctx.success(format!("Exported to {path}")),
                Ok(None) => {}
                Err(error) => ctx.error(format!("Could not export the server: {error}")),
            }
            Task::none()
        }
        Message::Delete => {
            let name = ctx.server(&id).map(|s| s.name.clone()).unwrap_or_default();
            ctx.confirm(
                Confirm::new(
                    format!("Delete {name}?"),
                    "This removes the server, its worlds and all its files from this PC. Export it first if you may want it back.",
                    "Delete server",
                    true,
                    move || {
                        let id = id.clone();
                        async move {
                            outcome(delete_server(id).await, "Delete failed")
                                .map(|()| "Server deleted.".to_string())
                        }
                    },
                )
                .busy_label("Deleting"),
            );
            Task::none()
        }
    }
}

fn save_reliability(state: &mut State, id: String, update: ServerSettingsUpdate) -> Task<Message> {
    state.saving_reliability = true;
    Task::perform(save_settings(id, update), Message::ReliabilitySaved)
}

fn setting<'a>(
    title: &'a str,
    hint: impl text::IntoFragment<'a>,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(TEXT_MD),
            text(hint).size(TEXT_SM).style(style::muted)
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        container(control)
            .width(CONTROL_WIDTH)
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(SPACE_6)
    .align_y(Alignment::Center)
    .into()
}

fn section<'a>(title: &'a str, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    column![
        text(title).size(TEXT_XL).font(UI_SEMIBOLD),
        column(rows).spacing(SPACE_5)
    ]
    .spacing(SPACE_5)
    .into()
}

fn switch<'a>(on: bool, on_toggle: impl Fn(bool) -> Message + 'a) -> Element<'a, Message> {
    toggler(on)
        .size(18)
        .style(style::switch)
        .on_toggle(on_toggle)
        .into()
}

fn switch_maybe<'a>(
    on: bool,
    enabled: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    toggler(on)
        .size(18)
        .style(style::switch)
        .on_toggle_maybe(enabled.then_some(on_toggle))
        .into()
}

fn field<'a>(
    placeholder: &str,
    value: &'a str,
    invalid: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(TEXT_MD)
        .padding([8, 12])
        .on_input(on_input)
        .style(move |theme: &Theme, status| {
            if invalid {
                style::input_error(theme, status)
            } else {
                style::input(theme, status)
            }
        })
}

fn dropdown<'a, T: Clone + PartialEq + ToString + 'a>(
    options: impl std::borrow::Borrow<[T]> + 'a,
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
) -> pick_list::PickList<'a, T, impl std::borrow::Borrow<[T]> + 'a, T, Message> {
    pick_list(options, selected, on_select)
        .width(Length::Fill)
        .padding([6, 12])
        .text_size(TEXT_MD)
        .style(style::pick)
        .menu_style(style::pick_menu)
}

fn gb(mb: u32) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

fn save_row<'a>(
    label: &'a str,
    busy_label: &'a str,
    busy: bool,
    message: Option<Message>,
    note: Option<&'static str>,
) -> Element<'a, Message> {
    let mut footer = row![].spacing(SPACE_3).align_y(Alignment::Center);
    if let Some(note) = note {
        footer = footer.push(text(note).size(TEXT_SM).color(CORAL));
    }
    footer
        .push(Space::new().width(Length::Fill))
        .push(busy_button(label, busy_label, busy, message, style::primary, BUTTON_MD))
        .padding(iced::Padding { top: SPACE_2, ..iced::Padding::ZERO })
        .into()
}

fn general<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let max = (state.system.max_ram_mb as u32).max(1024);
    let ram = column![
        slider(512..=max, state.ram_mb.min(max), Message::Ram)
            .step(256u32)
            .style(style::range),
        text(format!("{} of {} available", gb(state.ram_mb), gb(max)))
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_2);
    let name_empty = state.name.trim().is_empty();

    let mut java = column![switch(state.java_custom, Message::JavaCustom)]
        .spacing(SPACE_2)
        .align_x(Alignment::End);
    if state.java_custom {
        java = java.push(
            row![
                field(
                    "Path to java.exe",
                    &state.java_path,
                    state.java_problem.is_some(),
                    Message::JavaPath
                )
                .width(Length::Fill),
                sized(text("Browse").size(TEXT_MD), BUTTON_MD)
                    .style(style::secondary)
                    .on_press(Message::BrowseJava)
            ]
            .spacing(SPACE_2),
        );
        if let Some(problem) = state.java_problem {
            java = java.push(text(problem).size(TEXT_SM).color(CORAL));
        }
        if state.javas.is_empty() {
            java = java.push(
                text("No Java found on this PC. Browse to pick one.")
                    .size(TEXT_SM)
                    .style(style::muted),
            );
        } else {
            java = java.push(
                dropdown(state.javas.clone(), None, |choice: JavaChoice| {
                    Message::JavaPath(choice.path)
                })
                .placeholder("Detected on this PC"),
            );
        }
    }
    let java_hint = if state.java_custom {
        "Monolith runs the Java you pick here."
    } else {
        "Off: Monolith picks and installs the right Java for this version."
    };

    section(
        "General",
        vec![
            setting(
                "Name",
                "Shown in the sidebar and in exports.",
                field("Server name", &state.name, name_empty, Message::Name),
            ),
            setting("Memory", "More RAM helps with many players or plugins.", ram),
            setting("Custom Java", java_hint, java),
            save_row(
                "Save changes",
                "Saving",
                state.saving_general,
                (state.general_dirty(server) && !name_empty && state.java_problem.is_none())
                    .then_some(Message::SaveGeneral),
                name_empty.then_some("Give the server a name"),
            ),
        ],
    )
}

fn raw_editor(state: &State) -> Element<'_, Message> {
    let mut raw = column![
        text("One key=value per line, exactly as in server.properties. Lines starting with # are ignored.")
            .size(TEXT_SM)
            .style(style::muted),
        text_editor(&state.raw)
            .on_action(Message::RawAction)
            .font(MONO)
            .size(TEXT_SM)
            .padding(SPACE_3)
            .height(320)
            .style(|_: &Theme, status| text_editor::Style {
                background: LAYER_2.into(),
                border: iced::Border {
                    color: MOSS.scale_alpha(0.7),
                    width: if matches!(status, text_editor::Status::Focused { .. }) {
                        2.0
                    } else {
                        0.0
                    },
                    radius: RADIUS_CONTROL.into(),
                },
                placeholder: INK_TERTIARY,
                value: INK,
                selection: MOSS.scale_alpha(0.28),
            }),
    ]
    .spacing(SPACE_2);
    if let Some(error) = &state.raw_error {
        raw = raw.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }
    raw.into()
}

fn gameplay(state: &State) -> Element<'_, Message> {
    if !state.loaded {
        return section(
            "Gameplay",
            vec![text("Reading server.properties...")
                .size(TEXT_MD)
                .style(style::muted)
                .into()],
        );
    }
    let flag = |key: &'static str, default: &str| prop(&state.props, key, default) == "true";
    let number = |key: &'static str, default: &'static str, min: u32, max: u32| {
        field(
            default,
            prop(&state.props, key, default),
            !state.number_ok(key, default, min, max),
            move |v| Message::Prop(key, v),
        )
        .width(120)
    };
    let choice = |options: [&'static str; 4], key: &'static str, default: &'static str| {
        let current = prop(&state.props, key, default);
        dropdown(
            options,
            options.iter().copied().find(|o| *o == current),
            move |v: &'static str| Message::Prop(key, v.to_string()),
        )
    };
    let error = state.props_error();
    let toggle_raw = sized(
        text(if state.raw_open {
            "Back to simple settings"
        } else {
            "Edit as text"
        })
        .size(TEXT_MD), BUTTON_MD)
    .style(style::ghost)
    .on_press(Message::ToggleRaw);
    let dirty = state.raw_open || state.props != state.saved_props;
    let save = save_row(
        "Save gameplay",
        "Saving",
        state.saving_props,
        (dirty && (state.raw_open || error.is_none())).then_some(Message::SaveProps),
        if state.raw_open { None } else { error },
    );
    if state.raw_open {
        return section("Gameplay", vec![raw_editor(state), toggle_raw.into(), save]);
    }

    section(
        "Gameplay",
        vec![
            setting(
                "Message of the day",
                "Shown under the server name in the multiplayer list.",
                field("A Minecraft Server", prop(&state.props, "motd", "A Minecraft Server"), false, |v| {
                    Message::Prop("motd", v)
                }),
            ),
            setting(
                "Max players",
                "How many people can be online at once.",
                number("max-players", "20", 1, 100_000),
            ),
            setting(
                "Difficulty",
                "How hard mobs and hunger are.",
                choice(DIFFICULTIES, "difficulty", "easy"),
            ),
            setting(
                "Game mode",
                "What new players start as.",
                choice(GAMEMODES, "gamemode", "survival"),
            ),
            setting(
                "Player fighting",
                "Let players damage each other.",
                switch(flag("pvp", "true"), |on| Message::PropToggle("pvp", on)),
            ),
            setting(
                "Whitelist",
                "Only players on the whitelist can join.",
                switch(flag("white-list", "false"), |on| {
                    Message::PropToggle("white-list", on)
                }),
            ),
            setting(
                "Verify accounts",
                "Turn off only for offline setups. Players without a paid account can then join.",
                switch(flag("online-mode", "true"), |on| {
                    Message::PropToggle("online-mode", on)
                }),
            ),
            setting(
                "View distance",
                "Chunks sent to each player, from 2 to 32. Lower is faster.",
                number("view-distance", "10", 2, 32),
            ),
            setting(
                "Spawn protection",
                "Radius around spawn that only operators can change.",
                number("spawn-protection", "16", 0, 100_000),
            ),
            setting(
                "Port",
                "Friends connect through this port. The default is 25565.",
                number("server-port", "25565", 1, 65535),
            ),
            toggle_raw.into(),
            save,
        ],
    )
}

fn reliability<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let saving = state.saving_reliability;
    let mut times = column![].spacing(SPACE_2);
    for (index, time) in server.restart_times.iter().enumerate() {
        times = times.push(
            row![
                text(time.as_str()).font(MONO).size(TEXT_MD).width(Length::Fill),
                sized(text("Remove").size(TEXT_SM), BUTTON_SM)
                    .style(style::ghost)
                    .on_press_maybe((!saving).then_some(Message::RemoveTime(index)))
            ]
            .align_y(Alignment::Center),
        );
    }
    if server.restart_times.is_empty() {
        times = times.push(
            text("No scheduled restarts yet.")
                .size(TEXT_SM)
                .style(style::muted),
        );
    }
    let mut adder = column![row![
        field("04:30", &state.new_time, state.time_error.is_some(), Message::NewTime)
            .on_submit(Message::AddTime)
            .width(Length::Fill),
        busy_button(
            "Add time",
            "Saving",
            saving,
            (!state.new_time.trim().is_empty()).then_some(Message::AddTime),
            style::secondary, BUTTON_MD)
    ]
    .spacing(SPACE_2)]
    .spacing(SPACE_2);
    if let Some(error) = &state.time_error {
        adder = adder.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }

    section(
        "Reliability",
        vec![
            setting(
                "Restart after a crash",
                "Bring the server back up automatically if it stops unexpectedly.",
                switch_maybe(server.auto_restart_enabled(), !saving, Message::AutoRestart),
            ),
            setting(
                "Start with Monolith",
                "Start this server whenever Monolith opens.",
                switch_maybe(server.start_on_launch, !saving, Message::StartOnLaunch),
            ),
            setting(
                "Daily restarts",
                "Players get a warning 5 minutes, 1 minute and 10 seconds before. Times are 24-hour.",
                column![times, adder].spacing(SPACE_3),
            ),
        ],
    )
}

fn platform<'a>(state: &'a State, server: &'a ServerRecord, ctx: &'a Ctx) -> Element<'a, Message> {
    let idle = matches!(
        ctx.status(&server.id),
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle
    );
    let framework = dropdown(
        FRAMEWORKS,
        FRAMEWORKS.iter().copied().find(|f| *f == state.framework),
        Message::Framework,
    );
    let version = dropdown(state.version_ids(), state.version.clone(), Message::Version).placeholder(
        if state.versions_loading {
            "Loading versions"
        } else {
            "Choose a version"
        },
    );
    let unchanged = state.framework == server.framework
        && state.version.as_deref() == Some(server.version.as_str());
    let ready = idle && !unchanged && state.version.is_some() && state.changing.is_none();

    let mut rows = vec![
        setting(
            "Server software",
            format!("Currently {} {}.", server.framework, server.version),
            column![framework, version].spacing(SPACE_2),
        ),
        setting(
            "Snapshots",
            "Include snapshots, pre-releases and release candidates in the list.",
            switch(state.snapshots, Message::Snapshots),
        ),
    ];
    if let Some(error) = &state.versions_error {
        rows.push(
            row![
                text(format!("Could not load versions: {error}. Check your connection and retry."))
                    .size(TEXT_SM)
                    .color(CORAL)
                    .width(Length::Fill),
                sized(text("Retry").size(TEXT_MD), BUTTON_MD)
                    .style(style::secondary)
                    .on_press(Message::RetryVersions)
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center)
            .into(),
        );
    }
    if !idle {
        rows.push(
            text("Stop the server to change its software.")
                .size(TEXT_SM)
                .style(style::muted)
                .into(),
        );
    }
    if let Some((percent, message)) = &state.changing {
        rows.push(
            column![
                progress_bar(0.0..=100.0, *percent as f32).style(style::progress),
                text(message.as_str()).size(TEXT_SM).style(style::muted)
            ]
            .spacing(SPACE_2)
            .into(),
        );
    }
    if state.platform_armed {
        rows.push(
            text("Changing software can break worlds and plugins, and going to an older version can corrupt a world. Export or back up the server first, then confirm.")
                .size(TEXT_SM)
                .color(CORAL)
                .into(),
        );
        rows.push(
            row![
                Space::new().width(Length::Fill),
                sized(text("Cancel").size(TEXT_MD), BUTTON_MD)
                    .style(style::ghost)
                    .on_press(Message::CancelPlatform),
                busy_button(
                    "Confirm change",
                    "Changing",
                    false,
                    ready.then_some(Message::ApplyPlatform),
                    style::danger, BUTTON_MD)
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center)
            .into(),
        );
    } else {
        rows.push(save_row(
            "Change software",
            "Changing",
            state.changing.is_some(),
            ready.then_some(Message::ApplyPlatform),
            None,
        ));
    }
    section("Platform", rows)
}

fn danger<'a>(state: &'a State, ctx: &'a Ctx, server: &'a ServerRecord) -> Element<'a, Message> {
    let idle = matches!(
        ctx.status(&server.id),
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle
    );
    let export_hint = if idle {
        "Save the whole server as a zip you can share or import later."
    } else {
        "The server is running, so the export may miss recent changes. Stop it first for a clean copy."
    };
    section(
        "Export and delete",
        vec![
            setting(
                "Export a copy",
                export_hint,
                busy_button(
                    "Export server",
                    "Exporting",
                    state.exporting,
                    Some(Message::Export),
                    style::secondary, BUTTON_MD),
            ),
            setting(
                "Remove from this PC",
                if idle {
                    "Deletes the server and all its files. This cannot be undone."
                } else {
                    "Stop the server before deleting it."
                },
                busy_button(
                    "Delete server",
                    "Deleting",
                    false,
                    idle.then_some(Message::Delete),
                    style::danger, BUTTON_MD),
            ),
        ],
    )
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return Space::new().into();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        return text("Loading settings").style(style::muted).into();
    }
    let divider = || rule::horizontal(1).style(style::hairline);
    let page = column![
        general(state, server),
        divider(),
        gameplay(state),
        divider(),
        reliability(state, server),
        divider(),
        platform(state, server, ctx),
        divider(),
        danger(state, ctx, server),
    ]
    .spacing(SPACE_7)
    .padding(iced::Padding {
        right: SPACE_4,
        bottom: SPACE_7,
        ..iced::Padding::ZERO
    });
    scrollable(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::scroll)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    if let Event::FrameworkChangeProgress(progress) = event {
        if state.changing.is_some() {
            state.changing = Some((progress.percent.clamp(0.0, 100.0), progress.message.clone()));
        }
    }
    Task::none()
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    let mut tasks = Vec::new();
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        state.reset(server);
        let framework = server.framework.clone();
        tasks.push(state.fetch_versions(&framework));
    }
    let id = server.id.clone();
    tasks.push(Task::perform(
        async move {
            let props = get_server_properties(id.clone()).await;
            let javas = discover_installed_javas()
                .await
                .into_iter()
                .map(|java| JavaChoice {
                    path: java.path.to_string_lossy().into_owned(),
                    version: java.version,
                })
                .collect();
            (id, props, javas)
        },
        |(id, props, javas)| Message::Loaded(id, props, javas),
    ));
    Task::batch(tasks)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
