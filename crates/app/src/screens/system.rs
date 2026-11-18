use std::collections::HashMap;
use std::ptr::null;
use std::time::Duration;

use iced::{time, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::server_runner::{start_server, stop_server};
use monolith_core::types::{ServerRuntimeStatus, SuccessResult};
use tauri_winrt_notification::Toast;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
};

use crate::ctx::{Ctx, WindowAction};

const TRAY_ICON: &[u8] = include_bytes!("../../../../assets/brand/icon.png");

type MenuEntry = (String, String, ServerRuntimeStatus);

pub struct State {
    tray: Option<TrayIcon>,
    tray_failed: bool,
    shown: Vec<MenuEntry>,
    last: HashMap<String, ServerRuntimeStatus>,
}

impl Default for State {
    fn default() -> Self {
        guard_single_instance();
        Self {
            tray: None,
            tray_failed: false,
            shown: Vec::new(),
            last: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Toggled(SuccessResult),
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Tick => tick(state, ctx),
        Message::Toggled(result) => {
            if !result.success {
                ctx.error(result.error.unwrap_or_else(|| {
                    "The server did not respond. Open it in Monolith to see what happened.".into()
                }));
            }
            Task::none()
        }
    }
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    match event {
        Event::ServerStatus(update) => {
            let previous = state.last.insert(update.server_id.clone(), update.status);
            let name = server_name(ctx, &update.server_id);
            match (previous, update.status) {
                (Some(ServerRuntimeStatus::Starting), ServerRuntimeStatus::Online) => {
                    notify("Server online", format!("{name} is ready for players."));
                }
                (
                    Some(ServerRuntimeStatus::Starting),
                    ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed,
                ) => {
                    notify(
                        "Server failed to start",
                        format!("{name} stopped while starting. Open its console to see why."),
                    );
                }
                (Some(ServerRuntimeStatus::Online), ServerRuntimeStatus::Crashed) => {
                    notify(
                        "Server stopped unexpectedly",
                        format!("{name} went offline. Open its console to see why."),
                    );
                }
                _ => {}
            }
        }
        Event::BackupCompleted(done) => {
            notify(
                "Backup finished",
                format!("{} was backed up.", server_name(ctx, &done.server_id)),
            );
        }
        _ => {}
    }
    Task::none()
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    time::every(Duration::from_millis(300)).map(|_| Message::Tick)
}

fn tick(state: &mut State, ctx: &Ctx) -> Task<Message> {
    for server in &ctx.servers {
        state
            .last
            .entry(server.id.clone())
            .or_insert_with(|| ctx.status(&server.id));
    }

    if state.tray.is_none() && !state.tray_failed {
        match build_tray() {
            Ok(tray) => state.tray = Some(tray),
            Err(error) => {
                state.tray_failed = true;
                ctx.error(format!(
                    "Monolith could not add its tray icon ({error}). It keeps working without it."
                ));
            }
        }
    }

    refresh_menu(state, ctx);

    let mut tasks = Vec::new();
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        match event.id.0.as_str() {
            "open" => ctx.window(WindowAction::Show),
            "quit" => ctx.window(WindowAction::Quit),
            id => {
                if let Some(server_id) = id.strip_prefix("toggle:") {
                    tasks.push(toggle(ctx, server_id));
                }
            }
        }
    }
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            ctx.window(WindowAction::Show);
        }
    }
    Task::batch(tasks)
}

fn toggle(ctx: &Ctx, server_id: &str) -> Task<Message> {
    let id = server_id.to_string();
    match ctx.status(server_id) {
        ServerRuntimeStatus::Online | ServerRuntimeStatus::Starting => {
            Task::perform(stop_server(id), Message::Toggled)
        }
        ServerRuntimeStatus::Offline
            | ServerRuntimeStatus::Idle
            | ServerRuntimeStatus::Crashed => {
            Task::perform(start_server(id), Message::Toggled)
        }
        ServerRuntimeStatus::Stopping => Task::none(),
    }
}

fn build_tray() -> Result<TrayIcon, String> {
    let image = image::load_from_memory(TRAY_ICON)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let icon =
        tray_icon::Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("Monolith")
        .with_menu(Box::new(Menu::new()))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())
}

fn refresh_menu(state: &mut State, ctx: &Ctx) {
    let Some(tray) = &state.tray else {
        return;
    };
    let current: Vec<MenuEntry> = ctx
        .servers
        .iter()
        .map(|server| (server.id.clone(), server.name.clone(), ctx.status(&server.id)))
        .collect();
    if current == state.shown {
        return;
    }
    if let Ok(menu) = build_menu(&current) {
        tray.set_menu(Some(Box::new(menu)));
        let running = ctx.running_count();
        let tooltip = if running == 0 {
            "Monolith".to_string()
        } else {
            format!("Monolith · {running} running")
        };
        let _ = tray.set_tooltip(Some(tooltip));
        state.shown = current;
    }
}

fn build_menu(servers: &[MenuEntry]) -> Result<Menu, tray_icon::menu::Error> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("open", "Open Monolith", true, None))?;
    if !servers.is_empty() {
        menu.append(&PredefinedMenuItem::separator())?;
    }
    for (id, name, status) in servers {
        let (label, enabled) = match status {
            ServerRuntimeStatus::Online | ServerRuntimeStatus::Starting => {
                (format!("Stop {name}"), true)
            }
            ServerRuntimeStatus::Stopping => (format!("Stopping {name}"), false),
            ServerRuntimeStatus::Offline
            | ServerRuntimeStatus::Idle
            | ServerRuntimeStatus::Crashed => {
                (format!("Start {name}"), true)
            }
        };
        menu.append(&MenuItem::with_id(
            format!("toggle:{id}"),
            label,
            enabled,
            None,
        ))?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))?;
    Ok(menu)
}

fn server_name(ctx: &Ctx, id: &str) -> String {
    ctx.server(id)
        .map(|server| server.name.clone())
        .unwrap_or_else(|| "A server".into())
}

fn notify(title: &'static str, body: String) {
    std::thread::spawn(move || {
        let _ = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(&body)
            .show();
    });
}

fn guard_single_instance() {
    if std::env::args().any(|arg| arg == "--demo" || arg == "--screenshot") {
        return;
    }
    let mutex: Vec<u16> = "Local\\MonolithSingleInstance\0".encode_utf16().collect();
    let title: Vec<u16> = "Monolith\0".encode_utf16().collect();
    unsafe {
        CreateMutexW(null(), 0, mutex.as_ptr());
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return;
        }
        let window = FindWindowW(null(), title.as_ptr());
        if !window.is_null() {
            ShowWindow(window, SW_SHOW);
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window);
        }
    }
    std::process::exit(0);
}
