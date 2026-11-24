use std::path::Path;

use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{button, column, container, row, rule, scrollable, text, text_input, Space};
use iced::{clipboard, event, keyboard, Alignment, Color, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::types::{ServerRuntimeStatus, SuccessResult};
use monolith_core::{ngrok_manager, server_props, server_runner};
use monolith_theme::token::*;
use monolith_theme::{icon, icon_colored, style, Icon, UI_MEDIUM};

use crate::ctx::{status_color, Ctx, Route, SettingsPage, Tab};

const INPUT_ID: &str = "palette-input";
const LIST_ID: &str = "palette-list";
const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Default)]
pub struct State {
    query: String,
    selected: usize,
}

#[derive(Debug, Clone)]
pub enum Message {
    Query(String),
    Move(isize),
    Submit,
    Run(usize),
    Done(String, SuccessResult),
    Copy(String),
}

#[derive(Clone)]
enum Command {
    Server(String),
    Start,
    Stop,
    Restart,
    NewServer,
    Tab(Tab),
    Settings(SettingsPage),
    OpenFolder,
    CopyAddress,
    Remote,
}

struct Item {
    group: &'static str,
    icon: Icon,
    tint: Option<Color>,
    label: String,
    detail: Option<String>,
    hint: Option<String>,
    command: Command,
}

fn score(query: &str, label: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    if let Some(position) = label.find(query) {
        return Some(position);
    }
    let mut chars = label.chars().enumerate();
    let mut total = 100;
    let mut last = 0;
    for wanted in query.chars() {
        let (index, _) = chars.find(|(_, c)| *c == wanted)?;
        total += index - last;
        last = index;
    }
    Some(total)
}

fn action(icon: Icon, label: String, hint: Option<&str>, command: Command) -> Item {
    Item {
        group: "Actions",
        icon,
        tint: None,
        label,
        detail: None,
        hint: hint.map(str::to_string),
        command,
    }
}

fn tab_icon(tab: Tab) -> Icon {
    match tab {
        Tab::Overview => Icon::Info,
        Tab::Files => Icon::File,
        Tab::Players => Icon::Users,
        Tab::Content => Icon::Package,
        Tab::Worlds => Icon::Globe,
        Tab::Backups => Icon::Archive,
        Tab::Analytics => Icon::Chart,
        Tab::Automation => Icon::Clock,
        Tab::Network => Icon::Globe,
        Tab::Settings => Icon::Settings,
    }
}

fn items(query: &str, ctx: &Ctx) -> Vec<Item> {
    let mut all = Vec::new();
    for (index, server) in ctx.servers.iter().enumerate() {
        all.push(Item {
            group: "Servers",
            icon: Icon::Server,
            tint: Some(status_color(ctx.status(&server.id))),
            label: server.name.clone(),
            detail: Some(ctx.status_text(&server.id)),
            hint: (index < 9).then(|| format!("Ctrl+{}", index + 1)),
            command: Command::Server(server.id.clone()),
        });
    }

    if let Some(server) = ctx.selected_server() {
        let name = &server.name;
        match ctx.status(&server.id) {
            ServerRuntimeStatus::Offline
            | ServerRuntimeStatus::Idle
            | ServerRuntimeStatus::Crashed => {
                all.push(action(
                    Icon::Play,
                    format!("Start {name}"),
                    None,
                    Command::Start,
                ));
            }
            ServerRuntimeStatus::Online => {
                all.push(action(
                    Icon::Stop,
                    format!("Stop {name}"),
                    None,
                    Command::Stop,
                ));
                all.push(action(
                    Icon::Restart,
                    format!("Restart {name}"),
                    None,
                    Command::Restart,
                ));
            }
            ServerRuntimeStatus::Starting => {
                all.push(action(
                    Icon::Stop,
                    format!("Stop {name}"),
                    None,
                    Command::Stop,
                ));
            }
            ServerRuntimeStatus::Stopping => {}
        }
        all.push(action(
            Icon::Copy,
            format!("Copy join address for {name}"),
            None,
            Command::CopyAddress,
        ));
        all.push(action(
            Icon::Folder,
            format!("Open folder of {name}"),
            None,
            Command::OpenFolder,
        ));
        for tab in Tab::ALL {
            let label = match tab {
                Tab::Settings => "Open server settings".to_string(),
                tab => format!("Open {}", tab.label().to_lowercase()),
            };
            all.push(action(tab_icon(tab), label, None, Command::Tab(tab)));
        }
    }
    all.push(action(
        Icon::Plus,
        "New server".into(),
        Some("Ctrl+N"),
        Command::NewServer,
    ));
    all.push(action(
        Icon::Terminal,
        "Remote consoles".into(),
        None,
        Command::Remote,
    ));

    for page in SettingsPage::ALL {
        all.push(Item {
            group: "Settings",
            icon: Icon::Settings,
            tint: None,
            label: page.label().to_string(),
            detail: None,
            hint: (page == SettingsPage::General).then(|| "Ctrl+,".to_string()),
            command: Command::Settings(page),
        });
    }

    let query = query.trim().to_lowercase();
    let mut ranked: Vec<(usize, usize, usize, Item)> = all
        .into_iter()
        .enumerate()
        .filter_map(|(order, item)| {
            let group = match item.group {
                "Servers" => 0,
                "Actions" => 1,
                _ => 2,
            };
            score(&query, &item.label).map(|score| (group, score, order, item))
        })
        .collect();
    ranked.sort_by_key(|(group, score, order, _)| (*group, *score, *order));
    ranked.into_iter().map(|(_, _, _, item)| item).collect()
}

async fn join_address(server_path: String, ngrok_url: Option<String>) -> String {
    if let Some(url) = ngrok_url.filter(|url| !url.is_empty()) {
        return url.trim_start_matches("tcp://").to_string();
    }
    let properties = server_props::read_properties(Path::new(&server_path)).await;
    let port = properties
        .iter()
        .find(|property| property.key == "server-port")
        .and_then(|property| property.value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    format!("{}:{}", ngrok_manager::get_local_ip().await, port)
}

fn run(command: Command, ctx: &Ctx) -> Task<Message> {
    ctx.close_palette();
    let server = ctx.selected_server().cloned();
    match (command, server) {
        (Command::Server(id), _) => {
            ctx.select_server(id);
            Task::none()
        }
        (Command::NewServer, _) => {
            ctx.navigate(Route::Create);
            Task::none()
        }
        (Command::Remote, _) => {
            ctx.navigate(Route::Remote);
            Task::none()
        }
        (Command::Settings(page), _) => {
            ctx.navigate(Route::Settings(page));
            Task::none()
        }
        (Command::Tab(tab), _) => {
            ctx.navigate(Route::Server(tab));
            Task::none()
        }
        (Command::Start, Some(server)) => {
            ctx.info(format!("Starting {}", server.name));
            let done = format!("{} is starting", server.name);
            Task::perform(server_runner::start_server(server.id), move |result| {
                Message::Done(done.clone(), result)
            })
        }
        (Command::Stop, Some(server)) => {
            ctx.info(format!("Stopping {}", server.name));
            let done = format!("{} is stopping", server.name);
            Task::perform(server_runner::stop_server(server.id), move |result| {
                Message::Done(done.clone(), result)
            })
        }
        (Command::Restart, Some(server)) => {
            ctx.info(format!("Restarting {}", server.name));
            let done = format!("{} is restarting", server.name);
            Task::perform(server_runner::restart_server(server.id), move |result| {
                Message::Done(done.clone(), result)
            })
        }
        (Command::CopyAddress, Some(server)) => Task::perform(
            join_address(server.server_path, server.ngrok_url),
            Message::Copy,
        ),
        (Command::OpenFolder, Some(server)) => {
            if !Path::new(&server.server_path).exists()
                || open::that_detached(&server.server_path).is_err()
            {
                ctx.error("The server folder could not be opened. Check that it still exists.");
            }
            Task::none()
        }
        _ => Task::none(),
    }
}

fn snap(selected: usize, found: &[Item]) -> Task<Message> {
    let mut slots = 0;
    let mut position = 0;
    let mut group = "";
    for (index, item) in found.iter().enumerate() {
        if item.group != group {
            group = item.group;
            slots += 2;
        }
        slots += 1;
        if index == selected {
            position = slots;
        }
    }
    let y = if slots > 1 {
        (position - 1) as f32 / (slots - 1) as f32
    } else {
        0.0
    };
    operation::snap_to(
        LIST_ID,
        RelativeOffset {
            x: None,
            y: Some(y),
        },
    )
}

fn run_at(index: usize, state: &State, ctx: &Ctx) -> Task<Message> {
    let mut found = items(&state.query, ctx);
    if found.is_empty() {
        return Task::none();
    }
    let index = index.min(found.len() - 1);
    run(found.swap_remove(index).command, ctx)
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Query(query) => {
            state.query = query;
            state.selected = 0;
            operation::snap_to(LIST_ID, RelativeOffset::START)
        }
        Message::Move(delta) => {
            if !ctx.palette_open {
                return Task::none();
            }
            let found = items(&state.query, ctx);
            if found.is_empty() {
                return Task::none();
            }
            let count = found.len() as isize;
            state.selected = (state.selected as isize + delta).rem_euclid(count) as usize;
            snap(state.selected, &found)
        }
        Message::Submit => run_at(state.selected, state, ctx),
        Message::Run(index) => run_at(index, state, ctx),
        Message::Done(done, result) => {
            if result.success {
                ctx.success(done);
            } else {
                ctx.error(
                    result
                        .error
                        .unwrap_or_else(|| "The server did not respond. Try again.".into()),
                );
            }
            ctx.refresh_servers();
            Task::none()
        }
        Message::Copy(address) => {
            if address.starts_with("localhost") {
                ctx.info("Copied a local address. Other players cannot reach it until you share a public one.");
            } else {
                ctx.success("Join address copied");
            }
            clipboard::write(address)
        }
    }
}

fn row_view<'a>(item: Item, index: usize, selected: bool) -> Element<'a, Message> {
    let mut label = row![text(item.label).size(TEXT_MD)]
        .spacing(SPACE_3)
        .align_y(Alignment::Center);
    if let Some(detail) = item.detail {
        label = label.push(text(detail).size(TEXT_SM).style(style::muted));
    }
    let glyph: Element<'a, Message> = match item.tint {
        Some(color) => icon_colored(item.icon, 16.0, color).into(),
        None => icon(item.icon, 16.0).into(),
    };
    let mut content = row![glyph, label.width(Length::Fill)]
        .spacing(SPACE_3)
        .align_y(Alignment::Center);
    if let Some(hint) = item.hint {
        content = content.push(text(hint).size(TEXT_XS).style(style::faint));
    }
    button(content)
        .width(Length::Fill)
        .padding([SPACE_2, SPACE_3])
        .style(style::sidebar_item(selected))
        .on_press(Message::Run(index))
        .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let input = text_input("Search servers, actions and settings", &state.query)
        .id(INPUT_ID)
        .on_input(Message::Query)
        .on_submit(Message::Submit)
        .size(TEXT_LG)
        .padding([SPACE_4, SPACE_5])
        .style(style::input);

    let found = items(&state.query, ctx);
    let body: Element<'a, Message> = if found.is_empty() {
        container(
            text(format!(
                "Nothing matches \"{}\". Try a server name or a word like start, files or backup.",
                state.query.trim()
            ))
            .size(TEXT_SM)
            .style(style::muted),
        )
        .padding([SPACE_5, SPACE_5])
        .into()
    } else {
        let mut list = column![].spacing(SPACE_1);
        let mut group = "";
        for (index, item) in found.into_iter().enumerate() {
            if item.group != group {
                group = item.group;
                if index > 0 {
                    list = list.push(Space::new().height(SPACE_2));
                }
                list = list.push(
                    container(
                        text(group)
                            .size(TEXT_XS)
                            .font(UI_MEDIUM)
                            .style(style::muted),
                    )
                    .padding([SPACE_1, SPACE_3]),
                );
            }
            list = list.push(row_view(item, index, index == state.selected));
        }
        scrollable(container(list).padding([SPACE_2, SPACE_3]))
            .id(LIST_ID)
            .height(Length::Fill)
            .style(style::scroll)
            .into()
    };

    let footer = container(
        text("Up and down to move, Enter to run, Esc to close")
            .size(TEXT_XS)
            .style(style::faint),
    )
    .padding([SPACE_2, SPACE_5]);

    container(column![
        input,
        rule::horizontal(1).style(style::hairline),
        body,
        rule::horizontal(1).style(style::hairline),
        footer
    ])
    .width(560)
    .height(440)
    .style(style::overlay)
    .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(state: &mut State, _ctx: &Ctx) -> Task<Message> {
    *state = State::default();
    operation::focus(INPUT_ID)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    event::listen_with(|event, status, _| {
        if status == event::Status::Captured {
            return None;
        }
        match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
                ..
            }) => Some(Message::Move(-1)),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
                ..
            }) => Some(Message::Move(1)),
            _ => None,
        }
    })
}
