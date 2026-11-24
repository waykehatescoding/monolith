use std::time::Duration;

use iced::widget::{column, container, progress_bar, row, text};
use iced::{Alignment, Element, Length, Task};
use monolith_core::chunky::{self, ChunkyProgress};
use monolith_core::content_sources::loader_for;
use monolith_core::events::Event;
use monolith_core::modrinth_installs::install_modrinth_project;
use monolith_core::server_runner::send_command;
use monolith_core::types::{ContentSource, ModrinthInstallRequest, ServerRuntimeStatus};
use monolith_theme::token::*;
use monolith_theme::widget::sized;
use monolith_theme::{style, UI_MEDIUM, UI_SEMIBOLD};

use super::settings::{dropdown, field};
use crate::ctx::Ctx;
use crate::ui::busy_button;

const MAX_RADIUS: u32 = 30_000_000;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    installed: Option<bool>,
    installing: bool,
    worlds: Vec<String>,
    world: Option<String>,
    x: String,
    z: String,
    radius: String,
    progress: Option<ChunkyProgress>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Checked(String, bool),
    Install,
    Installed(Result<(), String>),
    World(String),
    X(String),
    Z(String),
    Radius(String),
    Start,
    Command(&'static str),
}

fn targets(framework: &str, level: &str) -> Vec<String> {
    if matches!(framework, "Paper" | "Purpur") {
        vec![
            level.to_string(),
            format!("{level}_nether"),
            format!("{level}_the_end"),
        ]
    } else {
        [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ]
        .map(String::from)
        .to_vec()
    }
}

pub fn show(state: &mut State, ctx: &Ctx, level: &str) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        *state = State {
            server_id: Some(server.id.clone()),
            x: "0".into(),
            z: "0".into(),
            radius: "1000".into(),
            ..State::default()
        };
    }
    state.worlds = targets(&server.framework, level);
    if !state
        .world
        .as_ref()
        .is_some_and(|w| state.worlds.contains(w))
    {
        state.world = state.worlds.first().cloned();
    }
    let id = server.id.clone();
    Task::perform(chunky::chunky_installed(id.clone()), move |installed| {
        Message::Checked(id.clone(), installed)
    })
}

fn coordinate(value: &str) -> Option<i64> {
    value
        .trim()
        .parse()
        .ok()
        .filter(|n: &i64| n.abs() <= MAX_RADIUS as i64)
}

fn radius(value: &str) -> Option<u32> {
    value
        .trim()
        .parse()
        .ok()
        .filter(|n| (1..=MAX_RADIUS).contains(n))
}

fn digits(value: String, signed: bool) -> String {
    value
        .chars()
        .enumerate()
        .filter(|(i, c)| c.is_ascii_digit() || (signed && *i == 0 && *c == '-'))
        .map(|(_, c)| c)
        .collect()
}

async fn send_all(id: String, commands: Vec<String>) {
    for command in commands {
        send_command(id.clone(), command).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match message {
        Message::Checked(from, installed) => {
            if from == id {
                state.installed = Some(installed);
            }
            Task::none()
        }
        Message::Install => {
            let Some(server) = ctx.selected_server() else {
                return Task::none();
            };
            let Some((project_type, loader)) = loader_for(&server.framework) else {
                return Task::none();
            };
            state.installing = true;
            let request = ModrinthInstallRequest {
                source: ContentSource::Modrinth,
                project_id: "chunky".into(),
                project_type,
                loader: Some(loader.into()),
                game_version: Some(server.version.clone()),
                version_id: None,
                title: Some("Chunky".into()),
                slug: Some("chunky".into()),
                icon_url: None,
            };
            Task::perform(install_modrinth_project(id, request), |result| {
                Message::Installed(if result.success {
                    Ok(())
                } else {
                    Err(result
                        .error
                        .unwrap_or_else(|| "The download failed.".into()))
                })
            })
        }
        Message::Installed(result) => {
            state.installing = false;
            match result {
                Ok(()) => {
                    state.installed = Some(true);
                    if ctx.status(&id) == ServerRuntimeStatus::Offline {
                        ctx.success("Chunky installed. Start the server to use it.");
                    } else {
                        ctx.success("Chunky installed. Restart the server to load it.");
                        ctx.changed_while_running(&id);
                    }
                }
                Err(error) => ctx.error(format!("Could not install Chunky: {error}")),
            }
            Task::none()
        }
        Message::World(world) => {
            state.world = Some(world);
            Task::none()
        }
        Message::X(value) => {
            state.x = digits(value, true);
            Task::none()
        }
        Message::Z(value) => {
            state.z = digits(value, true);
            Task::none()
        }
        Message::Radius(value) => {
            state.radius = digits(value, false);
            Task::none()
        }
        Message::Start => {
            let (Some(world), Some(x), Some(z), Some(r)) = (
                state.world.clone(),
                coordinate(&state.x),
                coordinate(&state.z),
                radius(&state.radius),
            ) else {
                return Task::none();
            };
            state.progress = None;
            Task::future(send_all(id, chunky::start_commands(&world, x, z, r))).discard()
        }
        Message::Command(command) => {
            Task::future(send_all(id, vec![format!("chunky {command}")])).discard()
        }
    }
}

pub fn on_event(state: &mut State, event: &Event) {
    if let Event::ConsoleOutput { server_id, line } = event {
        if state.server_id.as_ref() == Some(server_id) {
            if let Some(progress) = chunky::parse_progress(&line.text) {
                state.progress = Some(progress);
            }
        }
    }
}

fn progress_view(progress: &ChunkyProgress) -> Element<'_, Message> {
    let mut details = vec![
        format!("{:.2}%", progress.percent),
        format!("{} chunks", progress.chunks),
    ];
    if progress.finished {
        details.push("finished".into());
    } else {
        if let Some(eta) = &progress.eta {
            details.push(format!("{eta} left"));
        }
        if let Some(rate) = progress.rate {
            details.push(format!("{rate:.0} chunks/s"));
        }
    }
    container(
        column![
            row![
                text(&progress.world)
                    .size(TEXT_MD)
                    .font(UI_MEDIUM)
                    .width(Length::Fill),
                text(details.join(" · ")).size(TEXT_SM).style(style::muted),
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
            progress_bar(0.0..=100.0, progress.percent)
                .girth(6)
                .style(style::progress),
        ]
        .spacing(SPACE_3),
    )
    .padding(SPACE_4)
    .width(Length::Fill)
    .style(style::panel)
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let header = |hint: &'a str| {
        column![
            text("Pre-generate chunks").size(TEXT_LG).font(UI_SEMIBOLD),
            text(hint).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1)
    };
    let Some(server) = ctx.selected_server() else {
        return column![].into();
    };
    if loader_for(&server.framework).is_none() {
        return header("Chunky generates terrain ahead of time so players do not wait for new chunks. It needs Paper, Purpur, Fabric, Quilt, Forge or NeoForge.").into();
    }
    match state.installed {
        None => return header("Checking for Chunky").into(),
        Some(false) => {
            return column![
                header("Chunky generates terrain ahead of time so players do not wait for new chunks. It is not installed on this server yet."),
                row![busy_button(
                    "Install Chunky",
                    "Installing",
                    state.installing,
                    Some(Message::Install),
                    style::secondary,
                    BUTTON_MD,
                )],
            ]
            .spacing(SPACE_3)
            .into();
        }
        Some(true) => {}
    }
    let online = ctx.status(&server.id) == ServerRuntimeStatus::Online;
    let valid_x = coordinate(&state.x).is_some();
    let valid_z = coordinate(&state.z).is_some();
    let valid_radius = radius(&state.radius).is_some();
    let label = |value| text(value).size(TEXT_MD).width(140);
    let command = |title, command| {
        sized(text(title).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
            .style(style::ghost)
            .on_press_maybe(online.then_some(Message::Command(command)))
    };
    let form = column![
        row![
            label("World"),
            dropdown(state.worlds.as_slice(), state.world.clone(), Message::World).width(280),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
        row![
            label("Centre X, Z"),
            field("0", &state.x, !valid_x, Message::X).width(134),
            field("0", &state.z, !valid_z, Message::Z).width(134),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
        row![
            label("Radius in blocks"),
            field("1000", &state.radius, !valid_radius, Message::Radius).width(134),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
        row![
            sized(text("Start").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::primary)
                .on_press_maybe(
                    (online && valid_x && valid_z && valid_radius).then_some(Message::Start)
                ),
            command("Pause", "pause"),
            command("Continue", "continue"),
            command("Cancel", "cancel"),
        ]
        .spacing(SPACE_2),
    ]
    .spacing(SPACE_3);
    let hint = if online {
        "Chunky generates terrain ahead of time so players do not wait for new chunks. Spawn is usually around 0, 0. Generating uses a lot of CPU, so run it when few players are online."
    } else {
        "Start the server to pre-generate chunks with Chunky."
    };
    let mut body = column![
        header(hint),
        container(form)
            .padding(SPACE_4)
            .width(Length::Fill)
            .style(style::panel)
    ]
    .spacing(SPACE_3);
    if let Some(progress) = &state.progress {
        body = body.push(progress_view(progress));
    }
    body.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_form_values() {
        assert_eq!(digits("-12a3".into(), true), "-123");
        assert_eq!(digits("1-2".into(), true), "12");
        assert_eq!(digits("-5".into(), false), "5");
        assert_eq!(coordinate("-250"), Some(-250));
        assert_eq!(coordinate(""), None);
        assert_eq!(radius("0"), None);
        assert_eq!(radius("5000"), Some(5000));
        assert_eq!(targets("Paper", "hub")[1], "hub_nether");
        assert_eq!(targets("Fabric", "hub")[0], "minecraft:overworld");
    }
}
