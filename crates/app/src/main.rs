#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ctx;
mod demo;
mod screens;
mod ui;

use std::path::PathBuf;

use iced::{window, Size};
use monolith_theme::{theme, FONT_BYTES, UI};

use crate::ctx::{Route, Tab};

#[derive(Debug, Clone)]
pub struct Launch {
    pub route: Route,
    pub palette: bool,
    pub screenshot: Option<PathBuf>,
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn launch_from(args: &[String]) -> Launch {
    let route_name = option_value(args, "--route");
    Launch {
        route: route_name
            .as_deref()
            .and_then(Route::parse)
            .unwrap_or(Route::Server(Tab::Overview)),
        palette: route_name.as_deref() == Some("palette"),
        screenshot: option_value(args, "--screenshot").map(PathBuf::from),
    }
}

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--demo") {
        demo::prepare();
    }
    let launch = launch_from(&args);

    let settings = window::Settings {
        size: Size::new(1280.0, 800.0),
        min_size: Some(Size::new(960.0, 640.0)),
        decorations: false,
        exit_on_close_request: false,
        position: window::Position::Centered,
        icon: window::icon::from_file_data(include_bytes!("../../../assets/brand/icon.png"), None)
            .ok(),
        ..window::Settings::default()
    };

    let mut application = iced::application(
        move || app::App::boot(launch.clone()),
        app::App::update,
        app::App::view,
    )
    .title("Monolith")
    .theme(|_: &app::App| theme())
    .subscription(app::App::subscription)
    .default_font(UI)
    .window(settings);
    for bytes in FONT_BYTES {
        application = application.font(bytes);
    }
    application.run()
}
