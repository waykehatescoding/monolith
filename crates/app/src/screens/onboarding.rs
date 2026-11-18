use std::path::PathBuf;

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::java_manager::{discover_installed_javas, InstalledJava};
use monolith_core::server_transfer::import_server;
use monolith_core::types::CreateServerResult;
use monolith_theme::token::{CORAL, GROUND, SPACE_2, SPACE_3, SPACE_5, TEXT_2XL, TEXT_MD, TEXT_SM};
use monolith_theme::{app_mark, icon_colored, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{Ctx, Route};
use crate::ui::busy_button;

#[derive(Debug, Default)]
pub struct State {
    java: Option<Option<u32>>,
    importing: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Create,
    Import,
    Picked(Option<PathBuf>),
    Imported(CreateServerResult),
    JavaFound(Vec<InstalledJava>),
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Create => {
            ctx.navigate(Route::Create);
            Task::none()
        }
        Message::Import => {
            state.error = None;
            Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose a server zip")
                        .add_filter("Server zip", &["zip"])
                        .pick_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                Message::Picked,
            )
        }
        Message::Picked(None) => Task::none(),
        Message::Picked(Some(path)) => {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            state.importing = true;
            Task::perform(
                import_server(path.to_string_lossy().into_owned(), name),
                Message::Imported,
            )
        }
        Message::Imported(result) => {
            state.importing = false;
            match result.server {
                Some(server) if result.success => {
                    ctx.success(format!("Imported {}", server.name));
                    ctx.select_server(server.id);
                }
                _ => {
                    state.error = Some(result.error.unwrap_or_else(|| {
                        "The import failed. Use a zip made by Monolith or a zipped server folder."
                            .into()
                    }));
                }
            }
            Task::none()
        }
        Message::JavaFound(javas) => {
            state.java = Some(javas.iter().map(|java| java.version).max());
            Task::none()
        }
    }
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let create = button(
        row![
            icon_colored(Icon::Plus, 16.0, GROUND),
            text("Create your first server")
                .size(TEXT_MD)
                .font(UI_SEMIBOLD)
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
    )
    .padding([12, 24])
    .style(style::primary)
    .on_press(Message::Create);

    let import = busy_button(
        "Import a server",
        "Importing",
        state.importing,
        Some(Message::Import),
        style::secondary,
    );

    let mut content = column![
        app_mark(40.0),
        column![
            text("Welcome to Monolith").size(TEXT_2XL).font(UI_SEMIBOLD),
            text("Create a Minecraft server in about a minute. Monolith downloads the server files and the right Java for you.")
                .size(TEXT_MD)
                .style(style::muted),
        ]
        .spacing(SPACE_2),
        row![create, import]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_5)
    .max_width(460);

    if let Some(error) = &state.error {
        content = content.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    }
    if let Some(hint) = java_hint(state) {
        content = content.push(
            text(hint)
                .size(TEXT_SM)
                .font(UI_MEDIUM)
                .style(style::faint),
        );
    }

    container(content).center(Length::Fill).into()
}

fn java_hint(state: &State) -> Option<String> {
    Some(match state.java? {
        Some(version) => format!(
            "Java {version} is installed on this PC. Monolith still picks the right version for each server."
        ),
        None => "Java is not installed on this PC. Monolith downloads it when you create a server."
            .into(),
    })
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(state: &mut State, _ctx: &Ctx) -> Task<Message> {
    if state.java.is_some() {
        return Task::none();
    }
    Task::perform(discover_installed_javas(), Message::JavaFound)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
