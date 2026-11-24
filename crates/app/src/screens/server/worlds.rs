use iced::widget::{column, container, row, rule, scrollable, text};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::types::ServerRuntimeStatus;
use monolith_core::worlds::{self, World};
use monolith_theme::token::*;
use monolith_theme::{icon_colored, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use super::backups::{format_age, format_size};
use crate::ctx::Ctx;
use crate::ui::busy_button;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    worlds: Vec<World>,
    loaded: bool,
    error: Option<String>,
    switching: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Result<Vec<World>, String>),
    Activate(String),
    Activated(String, Result<(), String>),
}

fn load(id: String) -> Task<Message> {
    Task::perform(worlds::list_worlds(id.clone()), move |result| {
        Message::Loaded(id.clone(), result)
    })
}

fn running(ctx: &Ctx, id: &str) -> bool {
    !matches!(
        ctx.status(id),
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match message {
        Message::Loaded(from, result) => {
            if from == id {
                state.loaded = true;
                match result {
                    Ok(list) => {
                        state.worlds = list;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            Task::none()
        }
        Message::Activate(name) => {
            state.switching = Some(name.clone());
            Task::perform(worlds::set_active_world(id, name.clone()), move |result| {
                Message::Activated(name.clone(), result)
            })
        }
        Message::Activated(name, result) => {
            state.switching = None;
            match result {
                Ok(()) if running(ctx, &id) => {
                    ctx.success(format!(
                        "{name} is now the active world. Restart the server to load it."
                    ));
                    ctx.changed_while_running(&id);
                }
                Ok(()) => ctx.success(format!("{name} is now the active world.")),
                Err(error) => ctx.error(format!("Could not switch worlds: {error}")),
            }
            load(id)
        }
    }
}

fn world_row<'a>(state: &'a State, world: &'a World) -> Element<'a, Message> {
    let mut meta = Vec::new();
    if world.active {
        meta.push("Active".to_string());
    }
    if world.generated {
        meta.push(format_size(world.size));
        if let Some(age) = world.modified.as_deref().and_then(format_age) {
            meta.push(format!("saved {age}"));
        }
    } else {
        meta.push("Generated the next time the server starts".into());
    }
    let mut line = row![
        icon_colored(
            Icon::Globe,
            18.0,
            if world.active { MOSS } else { INK_SECONDARY }
        ),
        column![
            text(&world.name).size(TEXT_MD).font(UI_MEDIUM),
            text(meta.join(" · ")).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
    ]
    .spacing(SPACE_4)
    .align_y(Alignment::Center);
    if !world.active {
        line = line.push(busy_button(
            "Make active",
            "Switching",
            state.switching.as_deref() == Some(world.name.as_str()),
            state
                .switching
                .is_none()
                .then(|| Message::Activate(world.name.clone())),
            style::secondary,
            BUTTON_MD,
        ));
    }
    line.into()
}

fn list_view<'a>(state: &'a State, ctx: &'a Ctx, id: &'a str) -> Element<'a, Message> {
    let summary = if running(ctx, id) {
        "Switching worlds takes effect after a restart."
    } else {
        "The active world is the one the server loads when it starts."
    };
    let header = column![
        text("Worlds").size(TEXT_XL).font(UI_SEMIBOLD),
        text(summary).size(TEXT_SM).style(style::muted),
    ]
    .spacing(SPACE_1);
    let body: Element<'a, Message> = if let Some(error) = &state.error {
        text(format!("Could not read the worlds: {error}"))
            .size(TEXT_MD)
            .color(CORAL)
            .into()
    } else if !state.loaded {
        text("Looking for worlds")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    } else {
        let mut rows = column![];
        for (index, world) in state.worlds.iter().enumerate() {
            if index > 0 {
                rows = rows.push(rule::horizontal(1).style(style::hairline));
            }
            rows = rows.push(container(world_row(state, world)).padding([SPACE_3, SPACE_4]));
        }
        container(rows)
            .width(Length::Fill)
            .style(style::panel)
            .into()
    };
    column![header, body].spacing(SPACE_4).into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(id) = state.server_id.as_deref() else {
        return text("Loading worlds").style(style::muted).into();
    };
    let page = column![list_view(state, ctx, id)]
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
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match event {
        Event::WorldsChanged { server_id } if *server_id == id => load(id),
        Event::ServerStatus(update)
            if update.server_id == id
                && matches!(
                    update.status,
                    ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed
                ) =>
        {
            load(id)
        }
        _ => Task::none(),
    }
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(id) = ctx.selected.clone() else {
        return Task::none();
    };
    if state.server_id.as_deref() != Some(id.as_str()) {
        *state = State {
            server_id: Some(id.clone()),
            ..State::default()
        };
    }
    load(id)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
