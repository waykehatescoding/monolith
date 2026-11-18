use iced::widget::{button, column, container, progress_bar, row, scrollable, text, toggler, Space};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::types::{
    AppPreferencesUpdate, DownloadUpdateResult, SuccessResult, UpdateCheckResult,
};
use monolith_core::{app_preferences, paths, update_checker};
use monolith_theme::{icon, Icon, UI_MEDIUM, UI_SEMIBOLD};
use monolith_theme::style;
use monolith_theme::token::{
    CORAL, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, TEXT_LG, TEXT_MD, TEXT_SM,
    TEXT_XL,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::ctx::{Ctx, Route, SettingsPage};

const REPO_URL: &str = "https://github.com/waykehatescoding/monolith";
const ISSUES_URL: &str = "https://github.com/waykehatescoding/monolith/issues";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Monolith";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Local {
    close_to_tray: bool,
    notify_crash: bool,
    notify_online: bool,
}

impl Default for Local {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            notify_crash: true,
            notify_online: true,
        }
    }
}

fn local_path() -> std::path::PathBuf {
    paths::user_data_dir().join("shell.json")
}

async fn load_local() -> Local {
    match tokio::fs::read_to_string(local_path()).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => Local::default(),
    }
}

async fn save_local(local: Local) -> Result<(), String> {
    let path = local_path();
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&local).map_err(|e| e.to_string())?;
    tokio::fs::write(path, json)
        .await
        .map_err(|e| e.to_string())
}

fn reg() -> Command {
    let mut command = Command::new("reg");
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command
}

async fn read_autostart() -> bool {
    reg()
        .args(["query", RUN_KEY, "/v", RUN_VALUE])
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

async fn write_autostart(enabled: bool) -> Result<bool, String> {
    let output = if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = format!("\"{}\"", exe.display());
        reg()
            .args(["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &value, "/f"])
            .output()
            .await
    } else {
        reg()
            .args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
            .output()
            .await
    };
    match output {
        Ok(out) if out.status.success() => Ok(enabled),
        Ok(_) => Err("Windows refused the change to your startup list.".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Ready,
    Installing,
}

#[derive(Debug, Default)]
pub struct State {
    local: Local,
    autostart: Option<bool>,
    autostart_busy: bool,
    phase: Phase,
    result: Option<UpdateCheckResult>,
    percent: f32,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    LocalLoaded(Local),
    AutostartRead(bool),
    AskBeforeClosing(bool),
    PreferencesSaved(Result<(), String>),
    Autostart(bool),
    AutostartDone(Result<bool, String>),
    CloseToTray(bool),
    NotifyCrash(bool),
    NotifyOnline(bool),
    LocalSaved(Result<(), String>),
    Check,
    Checked(UpdateCheckResult),
    Download,
    Downloaded(DownloadUpdateResult),
    Install,
    Installed(SuccessResult),
    OpenFolder,
    OpenLink(&'static str),
}

fn persist(state: &State) -> Task<Message> {
    Task::perform(save_local(state.local.clone()), Message::LocalSaved)
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::LocalLoaded(local) => {
            ctx.set_close_to_tray(local.close_to_tray);
            state.local = local;
            Task::none()
        }
        Message::AutostartRead(enabled) => {
            state.autostart = Some(enabled);
            Task::none()
        }
        Message::AskBeforeClosing(enabled) => Task::perform(
            app_preferences::update_app_preferences(AppPreferencesUpdate {
                ask_before_close: Some(enabled),
            }),
            |result| Message::PreferencesSaved(result.map(|_| ())),
        ),
        Message::PreferencesSaved(result) => {
            if let Err(e) = result {
                ctx.error(format!("Could not save this setting: {e}"));
            }
            ctx.refresh_preferences();
            Task::none()
        }
        Message::Autostart(enabled) => {
            state.autostart_busy = true;
            Task::perform(write_autostart(enabled), Message::AutostartDone)
        }
        Message::AutostartDone(result) => {
            state.autostart_busy = false;
            match result {
                Ok(enabled) => state.autostart = Some(enabled),
                Err(e) => ctx.error(format!("Could not change start with Windows: {e}")),
            }
            Task::none()
        }
        Message::CloseToTray(enabled) => {
            state.local.close_to_tray = enabled;
            ctx.set_close_to_tray(enabled);
            persist(state)
        }
        Message::NotifyCrash(enabled) => {
            state.local.notify_crash = enabled;
            persist(state)
        }
        Message::NotifyOnline(enabled) => {
            state.local.notify_online = enabled;
            persist(state)
        }
        Message::LocalSaved(result) => {
            if let Err(e) = result {
                ctx.error(format!("Could not save this setting: {e}"));
            }
            Task::none()
        }
        Message::Check => {
            state.phase = Phase::Checking;
            state.error = None;
            Task::perform(update_checker::check_for_updates(), Message::Checked)
        }
        Message::Checked(result) => {
            state.phase = Phase::Idle;
            state.error = result.error.clone();
            state.result = Some(result);
            Task::none()
        }
        Message::Download => {
            state.phase = Phase::Downloading;
            state.percent = 0.0;
            state.error = None;
            Task::perform(update_checker::download_update(), Message::Downloaded)
        }
        Message::Downloaded(result) => {
            if result.success {
                state.phase = Phase::Ready;
            } else {
                state.phase = Phase::Idle;
                state.error = result.error;
            }
            Task::none()
        }
        Message::Install => {
            state.phase = Phase::Installing;
            Task::perform(update_checker::install_update(), Message::Installed)
        }
        Message::Installed(result) => {
            if !result.success {
                state.phase = Phase::Ready;
                state.error = result.error;
            }
            Task::none()
        }
        Message::OpenFolder => {
            if let Err(e) = open::that_detached(paths::user_data_dir()) {
                ctx.error(format!("Could not open the data folder: {e}"));
            }
            Task::none()
        }
        Message::OpenLink(url) => {
            if let Err(e) = open::that_detached(url) {
                ctx.error(format!("Could not open the link: {e}"));
            }
            Task::none()
        }
    }
}

fn heading(label: &str) -> Element<'_, Message> {
    text(label).size(TEXT_XL).font(UI_SEMIBOLD).into()
}

fn subheading(label: &str) -> Element<'_, Message> {
    text(label).size(TEXT_LG).font(UI_SEMIBOLD).into()
}

fn setting<'a>(
    title: &'a str,
    detail: &'a str,
    value: bool,
    on_toggle: Option<fn(bool) -> Message>,
) -> Element<'a, Message> {
    let mut switch = toggler(value).size(20).style(style::switch);
    if let Some(on_toggle) = on_toggle {
        switch = switch.on_toggle(on_toggle);
    }
    row![
        column![
            text(title).size(TEXT_MD).font(UI_MEDIUM),
            text(detail).size(TEXT_SM).style(style::muted)
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        switch
    ]
    .spacing(SPACE_5)
    .align_y(Alignment::Center)
    .into()
}

fn action<'a>(
    label: &'a str,
    glyph: Icon,
    message: Option<Message>,
    primary: bool,
) -> Element<'a, Message> {
    let content = row![icon(glyph, 16.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center);
    let base = button(content).padding([8, 14]).on_press_maybe(message);
    if primary {
        base.style(style::primary).into()
    } else {
        base.style(style::secondary).into()
    }
}

fn link<'a>(label: &'a str, url: &'static str) -> Element<'a, Message> {
    button(text(label).size(TEXT_MD).style(style::link))
        .padding(0)
        .style(style::ghost)
        .on_press(Message::OpenLink(url))
        .into()
}

fn general<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let startup_toggle: Option<fn(bool) -> Message> =
        (state.autostart.is_some() && !state.autostart_busy).then_some(Message::Autostart);
    column![
        heading("General"),
        column![
            subheading("Window"),
            setting(
                "Ask before closing",
                "Confirms before quitting, since closing Monolith stops every running server.",
                ctx.preferences.ask_before_close,
                Some(Message::AskBeforeClosing),
            ),
            setting(
                "Keep running in the tray",
                "The close button hides Monolith instead of quitting, so your servers stay up.",
                ctx.close_to_tray,
                Some(Message::CloseToTray),
            ),
            setting(
                "Start Monolith with Windows",
                "Opens Monolith when you sign in, so servers set to start with the app come up on their own.",
                state.autostart.unwrap_or(false),
                startup_toggle,
            ),
        ]
        .spacing(SPACE_4),
        column![
            subheading("Notifications"),
            setting(
                "A server crashes",
                "Get a Windows notification when a server stops without being asked to.",
                state.local.notify_crash,
                Some(Message::NotifyCrash),
            ),
            setting(
                "A server comes online",
                "Get a Windows notification when a server is ready for players to join.",
                state.local.notify_online,
                Some(Message::NotifyOnline),
            ),
        ]
        .spacing(SPACE_4),
    ]
    .spacing(SPACE_6)
    .into()
}

fn updates<'a>(state: &'a State) -> Element<'a, Message> {
    let busy = state.phase != Phase::Idle;
    let available = state.result.as_ref().is_some_and(|r| r.update_available);

    let status: Element<Message> = match (&state.phase, &state.result) {
        (Phase::Checking, _) => text("Checking for updates")
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
        (Phase::Downloading, _) => column![
            text(format!("Downloading update, {:.0}%", state.percent)).size(TEXT_MD),
            progress_bar(0.0..=100.0, state.percent)
                .girth(6)
                .style(style::progress)
        ]
        .spacing(SPACE_2)
        .into(),
        (Phase::Ready, Some(r)) => text(format!(
            "Monolith {} is downloaded and verified. Installing closes Monolith and stops your servers.",
            r.latest_version
        ))
        .size(TEXT_MD)
        .into(),
        (Phase::Installing, _) => text("Starting the installer")
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
        (_, Some(r)) if r.update_available => {
            text(format!("Monolith {} is available.", r.latest_version))
                .size(TEXT_MD)
                .into()
        }
        (_, Some(r)) if r.error.is_none() => {
            text("You are on the latest version.").size(TEXT_MD).into()
        }
        _ => text("Check whether a newer version is available.")
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
    };

    let controls = match state.phase {
        Phase::Ready | Phase::Installing => action(
            if state.phase == Phase::Installing {
                "Installing"
            } else {
                "Install and restart"
            },
            Icon::Restart,
            (state.phase == Phase::Ready).then_some(Message::Install),
            true,
        ),
        _ if available => action(
            if state.phase == Phase::Downloading {
                "Downloading"
            } else {
                "Download update"
            },
            Icon::Download,
            (!busy).then_some(Message::Download),
            true,
        ),
        _ => action(
            if state.phase == Phase::Checking {
                "Checking"
            } else {
                "Check for updates"
            },
            Icon::Restart,
            (!busy).then_some(Message::Check),
            false,
        ),
    };

    let mut page = column![
        heading("Updates"),
        text(format!("Installed version {VERSION}"))
            .size(TEXT_MD)
            .style(style::muted),
        status,
    ]
    .spacing(SPACE_3);

    if let Some(error) = &state.error {
        page = page.push(
            text(format!("{error} Check your internet connection and try again."))
                .size(TEXT_SM)
                .style(style::colored(CORAL)),
        );
    }
    page = page.push(controls);

    if let Some(result) = state.result.as_ref().filter(|r| r.update_available) {
        let notes = result
            .changelog
            .as_ref()
            .and_then(|c| c.first())
            .map(|entry| entry.changes.join("\n"))
            .or_else(|| result.release_notes.clone())
            .unwrap_or_default();
        if !notes.is_empty() {
            page = page.push(Space::new().height(SPACE_2)).push(
                column![
                    subheading("What is new"),
                    text(notes).size(TEXT_MD).style(style::muted)
                ]
                .spacing(SPACE_2),
            );
        }
    }

    column![page, about()].spacing(SPACE_6).into()
}

fn about<'a>() -> Element<'a, Message> {
    column![
        heading("About Monolith"),
        text(format!("Version {VERSION}, released under the GPL-3.0 licence."))
            .size(TEXT_MD)
            .style(style::muted),
        row![
            link("Source and releases", REPO_URL),
            link("Report a problem", ISSUES_URL),
        ]
        .spacing(SPACE_5),
        text("Hubot Sans and JetBrains Mono are used under the SIL Open Font Licence 1.1. Lucide icons are used under the ISC licence.")
            .size(TEXT_SM)
            .style(style::muted),
        container(action(
            "Open data folder",
            Icon::Folder,
            Some(Message::OpenFolder),
            false
        ))
    ]
    .spacing(SPACE_3)
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let body = match ctx.route {
        Route::Settings(SettingsPage::Updates) => updates(state),
        _ => general(state, ctx),
    };
    scrollable(container(body).padding([0, 16]))
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    if let Event::UpdateDownloadProgress(progress) = event {
        state.percent = progress.percent as f32;
    }
    Task::none()
}

pub fn on_show(_state: &mut State, ctx: &Ctx) -> Task<Message> {
    ctx.refresh_preferences();
    Task::batch([
        Task::perform(load_local(), Message::LocalLoaded),
        Task::perform(read_autostart(), Message::AutostartRead),
    ])
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
