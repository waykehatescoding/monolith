use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::{sized, sized_icon};
use std::path::Path;
use std::time::{Duration, Instant};

use iced::widget::{column, container, row, text, Space};
use iced::{clipboard, Alignment, Color, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::types::{
    ServerProperty, ServerRecord, ServerRuntimeStatus, ServerSettingsUpdate, SuccessResult,
};
use monolith_core::{ngrok_manager, server_props, server_runner, server_store};
use monolith_theme::token::{
    AMBER, CORAL, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, TEXT_2XL, TEXT_MD, TEXT_SM, TEXT_XL,
};
use monolith_theme::{icon, icon_colored, style, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};

use super::listing;
use crate::ctx::{status_color, Ctx, Route, SettingsPage};
use crate::demo;
use crate::ui::busy_button;

const DEFAULT_PORT: u16 = 25565;
const NO_TOKEN: &str = "Add your ngrok authtoken in Settings, Sharing, then try again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Start,
    Stop,
    Restart,
    Share,
    Eula,
    Port,
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    address: String,
    port: u16,
    eula_accepted: bool,
    tunnel_url: Option<String>,
    tunnel_error: Option<String>,
    busy: Option<Busy>,
    error: Option<String>,
    port_fix: Option<u16>,
    started_at: Option<Instant>,
    listing: listing::State,
}

#[derive(Debug, Clone)]
pub struct Loaded {
    server_id: String,
    address: String,
    port: u16,
    eula_accepted: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Loaded),
    Start,
    Stop,
    Restart,
    Share,
    AcceptEula,
    UsePort(u16),
    OpenSharing,
    Dismiss,
    Done(Busy, SuccessResult),
    Copy(String),
    Tick,
    Listing(listing::Message),
}

fn port_of(properties: &[ServerProperty]) -> u16 {
    properties
        .iter()
        .find(|property| property.key == "server-port")
        .and_then(|property| property.value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

async fn load(server: ServerRecord) -> Loaded {
    let path = Path::new(&server.server_path);
    let properties = server_props::read_properties(path).await;
    let eula = tokio::fs::read_to_string(path.join("eula.txt"))
        .await
        .map(|content| content.lines().any(|line| line.trim() == "eula=true"))
        .unwrap_or(false);
    let address = ngrok_manager::get_local_ip().await;
    Loaded {
        server_id: server.id,
        address,
        port: port_of(&properties),
        eula_accepted: eula || server.eula_accepted == Some(true),
    }
}

async fn use_port(server: ServerRecord, port: u16) -> SuccessResult {
    let path = Path::new(&server.server_path);
    let mut properties = server_props::read_properties(path).await;
    match properties
        .iter_mut()
        .find(|property| property.key == "server-port")
    {
        Some(property) => property.value = port.to_string(),
        None => properties.push(ServerProperty {
            key: "server-port".into(),
            value: port.to_string(),
            comment: None,
        }),
    }
    let saved = server_props::write_properties(path, &properties).await;
    if !saved.success {
        return saved;
    }
    server_runner::start_server(server.id).await
}

async fn update_sharing(id: &str, enabled: bool) -> SuccessResult {
    server_store::update_server_settings(
        id.to_string(),
        ServerSettingsUpdate {
            use_ngrok: Some(enabled),
            ..ServerSettingsUpdate::default()
        },
    )
    .await
}

async fn toggle_share(server: ServerRecord, port: u16, enable: bool) -> SuccessResult {
    if !enable {
        let stopped = ngrok_manager::stop_ngrok(server.id.clone()).await;
        if !stopped.success {
            return stopped;
        }
        return update_sharing(&server.id, false).await;
    }
    if !ngrok_manager::is_ngrok_authtoken_configured().await {
        return SuccessResult::err(NO_TOKEN);
    }
    let saved = update_sharing(&server.id, true).await;
    if !saved.success {
        return saved;
    }
    if server_runner::is_running(&server.id) {
        let started = ngrok_manager::start_ngrok(server.id, port).await;
        if !started.success {
            return SuccessResult::err(
                started
                    .error
                    .unwrap_or_else(|| "The public address could not be created.".into()),
            );
        }
    }
    SuccessResult::ok()
}

fn port_from_error(error: &str) -> Option<u16> {
    let rest = error.split("Try port ").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn tunnel(state: &State, server: &ServerRecord) -> Option<String> {
    if server.use_ngrok != Some(true) {
        return None;
    }
    state
        .tunnel_url
        .clone()
        .or_else(|| server.ngrok_url.clone())
        .filter(|url| !url.is_empty())
        .map(|url| url.trim_start_matches("tcp://").to_string())
}

fn run(
    state: &mut State,
    busy: Busy,
    work: impl std::future::Future<Output = SuccessResult> + Send + 'static,
) -> Task<Message> {
    state.busy = Some(busy);
    state.error = None;
    state.port_fix = None;
    Task::perform(work, move |result| Message::Done(busy, result))
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server().cloned() else {
        return Task::none();
    };
    match message {
        Message::Loaded(loaded) => {
            if state.server_id.as_deref() == Some(loaded.server_id.as_str()) {
                state.address = loaded.address;
                state.port = loaded.port;
                state.eula_accepted = loaded.eula_accepted;
            }
            Task::none()
        }
        Message::Start => run(state, Busy::Start, server_runner::start_server(server.id)),
        Message::Stop => run(state, Busy::Stop, server_runner::stop_server(server.id)),
        Message::Restart => run(
            state,
            Busy::Restart,
            server_runner::restart_server(server.id),
        ),
        Message::Share => {
            let enable = server.use_ngrok != Some(true);
            let port = state.port;
            state.tunnel_url = None;
            state.tunnel_error = None;
            run(state, Busy::Share, toggle_share(server, port, enable))
        }
        Message::AcceptEula => run(state, Busy::Eula, server_props::accept_eula(server.id)),
        Message::UsePort(port) => {
            state.port = port;
            run(state, Busy::Port, use_port(server, port))
        }
        Message::OpenSharing => {
            ctx.navigate(Route::Settings(SettingsPage::Sharing));
            Task::none()
        }
        Message::Dismiss => {
            state.error = None;
            state.port_fix = None;
            Task::none()
        }
        Message::Done(busy, result) => {
            state.busy = None;
            if result.success {
                if busy == Busy::Eula {
                    state.eula_accepted = true;
                }
                ctx.refresh_servers();
                return Task::none();
            }
            let error = result
                .error
                .unwrap_or_else(|| "Something went wrong. Try again.".into());
            state.port_fix = port_from_error(&error);
            state.error = Some(error);
            Task::none()
        }
        Message::Copy(value) => {
            ctx.success("Address copied");
            clipboard::write(value)
        }
        Message::Tick => Task::none(),
        Message::Listing(message) => {
            let online = ctx.status(&server.id) == ServerRuntimeStatus::Online;
            listing::update(&mut state.listing, message, &server, online).map(Message::Listing)
        }
    }
}

fn state_word(status: ServerRuntimeStatus) -> &'static str {
    match status {
        ServerRuntimeStatus::Online => "Online",
        ServerRuntimeStatus::Starting => "Starting",
        ServerRuntimeStatus::Stopping => "Stopping",
        ServerRuntimeStatus::Idle => "Idle",
        ServerRuntimeStatus::Offline => "Offline",
        ServerRuntimeStatus::Crashed => "Crashed",
    }
}

fn address_row<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(TEXT_SM).style(style::muted).width(52),
        text(value.clone()).size(TEXT_MD).font(MONO),
        sized_icon(icon(Icon::Copy, 14.0), BUTTON_SM)
            .style(style::icon_button)
            .on_press(Message::Copy(value)),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

pub fn header<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return Space::new().into();
    };
    let status = ctx.status(&server.id);
    let online = status == ServerRuntimeStatus::Online;
    let idle = state.busy.is_none();
    let public = tunnel(state, server);
    let sharing = server.use_ngrok == Some(true);
    let stale = online && ctx.needs_restart(&server.id);

    let mut line = row![
        text(format!("{} {}", server.framework, server.version))
            .size(TEXT_MD)
            .style(style::muted),
        text("·").size(TEXT_MD).style(style::faint),
        text(state_word(status))
            .size(TEXT_MD)
            .font(UI_MEDIUM)
            .style(style::colored(status_color(status))),
    ]
    .spacing(SPACE_2);
    if stale {
        line = line.push(text("·").size(TEXT_MD).style(style::faint)).push(
            text("Restart to apply changes")
                .size(TEXT_MD)
                .font(UI_MEDIUM)
                .style(style::colored(AMBER)),
        );
    }
    let title = column![
        text(server.name.as_str()).size(TEXT_2XL).font(UI_SEMIBOLD),
        line,
    ]
    .spacing(SPACE_1);

    let primary = match status {
        ServerRuntimeStatus::Online => busy_button(
            "Stop",
            "Stopping",
            state.busy == Some(Busy::Stop),
            Some(Message::Stop).filter(|_| idle),
            style::secondary,
            BUTTON_MD,
        ),
        ServerRuntimeStatus::Stopping => {
            busy_button("Stop", "Stopping", true, None, style::secondary, BUTTON_MD)
        }
        ServerRuntimeStatus::Starting => {
            busy_button("Start", "Starting", true, None, style::primary, BUTTON_MD)
        }
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle | ServerRuntimeStatus::Crashed
            if !state.eula_accepted =>
        {
            busy_button(
                "Accept EULA",
                "Accepting",
                state.busy == Some(Busy::Eula),
                Some(Message::AcceptEula).filter(|_| idle),
                style::primary,
                BUTTON_MD,
            )
        }
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Idle | ServerRuntimeStatus::Crashed => {
            busy_button(
                "Start",
                "Starting",
                state.busy == Some(Busy::Start),
                Some(Message::Start).filter(|_| idle && state.eula_accepted),
                style::primary,
                BUTTON_MD,
            )
        }
    };

    let actions = row![
        primary,
        busy_button(
            "Restart",
            "Restarting",
            state.busy == Some(Busy::Restart),
            Some(Message::Restart).filter(|_| idle && online),
            if stale {
                style::primary
            } else {
                style::secondary
            },
            BUTTON_MD
        ),
        busy_button(
            if sharing {
                "Stop sharing"
            } else {
                "Share online"
            },
            "Working",
            state.busy == Some(Busy::Share),
            Some(Message::Share).filter(|_| idle),
            style::secondary,
            BUTTON_MD
        ),
    ]
    .spacing(SPACE_2);

    let mut addresses = column![].spacing(SPACE_1);
    if !state.address.is_empty() {
        addresses = addresses.push(address_row(
            "Local",
            format!("{}:{}", state.address, state.port),
        ));
    }
    match public {
        Some(url) => addresses = addresses.push(address_row("Public", url)),
        None if sharing && online => {
            addresses = addresses.push(
                row![
                    text("Public").size(TEXT_SM).style(style::muted).width(52),
                    text(
                        state
                            .tunnel_error
                            .as_deref()
                            .unwrap_or("Creating public address")
                    )
                    .size(TEXT_MD)
                    .style(|theme| match state.tunnel_error {
                        Some(_) => style::colored(CORAL)(theme),
                        None => style::muted(theme),
                    }),
                ]
                .spacing(SPACE_2),
            );
        }
        None => {}
    }

    column![title, addresses, actions.wrap()]
        .spacing(SPACE_4)
        .into()
}

fn readout<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    column![
        text(label).size(TEXT_SM).style(style::muted),
        text(value).size(TEXT_XL).font(UI_SEMIBOLD),
    ]
    .spacing(SPACE_1)
    .width(Length::FillPortion(1))
    .into()
}

fn memory_label(used: f64, max: f64) -> String {
    if max >= 1024.0 {
        format!("{:.1} / {:.1} GB", used / 1024.0, max / 1024.0)
    } else {
        format!("{used:.0} / {max:.0} MB")
    }
}

fn uptime_label(started: Instant) -> String {
    let seconds = started.elapsed().as_secs();
    let (hours, minutes) = (seconds / 3600, seconds / 60 % 60);
    match (hours, minutes) {
        (0, 0) => format!("{seconds}s"),
        (0, m) => format!("{m}m {:02}s", seconds % 60),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

fn notice<'a>(
    color: Color,
    message: impl Into<String>,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut line = row![
        icon_colored(Icon::Warning, 16.0, color),
        text(message.into())
            .size(TEXT_MD)
            .style(style::colored(color))
            .width(Length::Fill),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);
    if let Some(action) = action {
        line = line.push(action);
    }
    container(line)
        .padding([SPACE_3, SPACE_4])
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return Space::new().into();
    };
    let status = ctx.status(&server.id);
    let online = status == ServerRuntimeStatus::Online;
    let stats = ctx.stats(&server.id).filter(|_| online);
    let idle = state.busy.is_none();

    let mut page = column![].spacing(SPACE_5);

    if !state.eula_accepted {
        page = page.push(notice(
            AMBER,
            "Minecraft needs you to accept its EULA (minecraft.net/eula) before this server can start.",
            Some(busy_button(
                "Accept EULA",
                "Accepting",
                state.busy == Some(Busy::Eula),
                Some(Message::AcceptEula).filter(|_| idle),
                style::primary, BUTTON_MD)),
        ));
    }

    if let Some(error) = &state.error {
        let dismiss = || -> Element<'a, Message> {
            sized(text("Dismiss").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::ghost)
                .on_press(Message::Dismiss)
                .into()
        };
        let fix = state.port_fix.map(|port| {
            let busy = state.busy == Some(Busy::Port);
            let label = if busy {
                "Starting".to_string()
            } else {
                format!("Use port {port}")
            };
            let fix =
                sized(text(label).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD).style(style::secondary);
            let fix: Element<'a, Message> = match idle {
                true => fix.on_press(Message::UsePort(port)).into(),
                false => fix.into(),
            };
            fix
        });
        let action = match fix {
            Some(fix) => Some(fix),
            None if error == NO_TOKEN => Some(
                sized(
                    text("Open Sharing settings").size(TEXT_MD).font(UI_MEDIUM),
                    BUTTON_MD,
                )
                .style(style::secondary)
                .on_press(Message::OpenSharing)
                .into(),
            ),
            None => Some(dismiss()),
        };
        page = page.push(notice(CORAL, error.as_str(), action));
    }

    if status == ServerRuntimeStatus::Crashed {
        page = page.push(notice(
            AMBER,
            if server.auto_restart_enabled() {
                "The server stopped unexpectedly. Monolith restarts it automatically and gives up after 3 crashes in 10 minutes."
            } else {
                "The server stopped unexpectedly. Auto-restart is off for this server, so start it again or turn auto-restart on in its settings."
            },
            None,
        ));
    }

    let dash = || "—".to_string();
    let players = stats.map_or_else(dash, |s| format!("{} / {}", s.player_count, s.max_players));
    let tps = stats
        .and_then(|s| s.tps)
        .map_or_else(dash, |tps| format!("{tps:.1}"));
    let memory = stats
        .and_then(|s| s.memory_used_mb.zip(s.memory_max_mb))
        .map_or_else(dash, |(used, max)| memory_label(used, max));
    let uptime = state
        .started_at
        .filter(|_| online)
        .map_or_else(dash, uptime_label);
    if online {
        page = page.push(
            row![
                readout("Players", players),
                readout("TPS", tps),
                readout("Memory", memory),
                readout("Uptime", uptime),
            ]
            .spacing(SPACE_4),
        );
    } else if state.eula_accepted
        && !matches!(
            status,
            ServerRuntimeStatus::Starting | ServerRuntimeStatus::Stopping
        )
    {
        page = page.push(
            text("This server is offline. Start it to see players, TPS, memory and uptime.")
                .size(TEXT_MD)
                .style(style::muted),
        );
    }

    page.push(listing::view(&state.listing, server).map(Message::Listing))
        .into()
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    let Some(id) = ctx.selected.as_deref() else {
        return Task::none();
    };
    let mut task = Task::none();
    match event {
        Event::ServerStatus(update) if update.server_id == id => match update.status {
            ServerRuntimeStatus::Online => {
                if let Some(server) = ctx.selected_server() {
                    task = listing::refresh(&mut state.listing, server, true).map(Message::Listing);
                }
            }
            ServerRuntimeStatus::Starting => {
                state.error = None;
                state.port_fix = None;
                state.started_at = Some(Instant::now());
                state.tunnel_error = None;
            }
            ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed => {
                state.started_at = None;
                state.tunnel_url = None;
                if let Some(server) = ctx.selected_server() {
                    task =
                        listing::refresh(&mut state.listing, server, false).map(Message::Listing);
                }
            }
            _ => {}
        },
        Event::NgrokUrlChanged(info) if info.server_id == id => {
            state.tunnel_url = Some(info.public_url.clone());
            state.tunnel_error = None;
            ctx.refresh_servers();
        }
        Event::NgrokFailed { server_id, error } if *server_id == id => {
            state.tunnel_error = Some(error.clone());
        }
        _ => {}
    }
    task
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        let status = ctx.status(&server.id);
        *state = State {
            server_id: Some(server.id.clone()),
            eula_accepted: true,
            port: DEFAULT_PORT,
            started_at: matches!(
                status,
                ServerRuntimeStatus::Online | ServerRuntimeStatus::Starting
            )
            .then(|| {
                let now = Instant::now();
                now.checked_sub(demo::uptime(&server.id)).unwrap_or(now)
            }),
            ..State::default()
        };
    }
    let online = ctx.status(&server.id) == ServerRuntimeStatus::Online;
    Task::batch([
        Task::perform(load(server.clone()), Message::Loaded),
        listing::refresh(&mut state.listing, server, online).map(Message::Listing),
    ])
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.started_at.is_some() {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    }
}
