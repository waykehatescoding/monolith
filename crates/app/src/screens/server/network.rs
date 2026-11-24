use iced::widget::{column, row, rule, scrollable, text, Space};
use iced::{clipboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::crossplay::{self, Crossplay};
use monolith_core::events::Event;
use monolith_core::ngrok_manager;
use monolith_core::types::ServerRecord;
use monolith_theme::token::{
    AMBER, BUTTON_MD, BUTTON_SM, CORAL, SPACE_2, SPACE_3, SPACE_4, SPACE_7, TEXT_MD, TEXT_SM,
};
use monolith_theme::widget::sized_icon;
use monolith_theme::{icon, style, Icon, MONO};

use super::settings::section;
use crate::ctx::Ctx;
use crate::ui::busy_button;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Crossplay,
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    local_ip: String,
    crossplay: Option<Crossplay>,
    busy: Option<Busy>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, String, Crossplay),
    EnableCrossplay,
    DisableCrossplay,
    Done(Busy, Result<(), String>),
    Copy(String),
}

fn load(server: ServerRecord) -> Task<Message> {
    Task::perform(
        async move {
            let ip = ngrok_manager::get_local_ip().await;
            let crossplay = crossplay::status(&server).await;
            (server.id, ip, crossplay)
        },
        |(id, ip, crossplay)| Message::Loaded(id, ip, crossplay),
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server().cloned() else {
        return Task::none();
    };
    match message {
        Message::Loaded(id, ip, crossplay) => {
            if state.server_id.as_deref() == Some(id.as_str()) {
                state.local_ip = ip;
                state.crossplay = Some(crossplay);
            }
            Task::none()
        }
        Message::EnableCrossplay => {
            state.busy = Some(Busy::Crossplay);
            state.error = None;
            Task::perform(crossplay::enable(server), |result| {
                Message::Done(Busy::Crossplay, result)
            })
        }
        Message::DisableCrossplay => {
            state.busy = Some(Busy::Crossplay);
            state.error = None;
            Task::perform(crossplay::disable(server), |result| {
                Message::Done(Busy::Crossplay, result)
            })
        }
        Message::Done(busy, result) => {
            state.busy = None;
            match result {
                Ok(()) => {
                    if busy == Busy::Crossplay {
                        ctx.changed_while_running(&server.id);
                    }
                    load(server)
                }
                Err(error) => {
                    state.error = Some(error);
                    Task::none()
                }
            }
        }
        Message::Copy(value) => {
            ctx.success("Address copied");
            clipboard::write(value)
        }
    }
}

fn note<'a>(value: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    text(value).size(TEXT_SM).style(style::muted).into()
}

fn address<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(TEXT_SM).style(style::muted).width(72),
        text(value.clone()).size(TEXT_MD).font(MONO),
        sized_icon(icon(Icon::Copy, 14.0), BUTTON_SM)
            .style(style::icon_button)
            .on_press(Message::Copy(value)),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

fn crossplay_section<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let mut rows = vec![note(
        "Bedrock players on phones, consoles and Windows can join this Java server through Geyser and Floodgate.",
    )];
    let busy = state.busy == Some(Busy::Crossplay);
    let idle = state.busy.is_none();
    match &state.crossplay {
        None => rows.push(note("Checking crossplay")),
        Some(status) if !status.supported => rows.push(note(format!(
            "Crossplay needs Paper, Purpur, Fabric or Velocity. This server runs {}.",
            server.framework
        ))),
        Some(status) if status.installed => {
            if !state.local_ip.is_empty() {
                rows.push(address(
                    "Bedrock",
                    format!("{}:{}", state.local_ip, status.bedrock_port),
                ));
            }
            rows.push(note(format!(
                "Bedrock uses UDP port {}. ngrok only carries TCP, so Bedrock friends outside your network need playit.gg or this UDP port forwarded on your router.",
                status.bedrock_port
            )));
            if status.needs_restart {
                rows.push(
                    text("Restart the server so Geyser finishes its setup.")
                        .size(TEXT_SM)
                        .style(style::colored(AMBER))
                        .into(),
                );
            }
            rows.push(busy_button(
                "Remove crossplay",
                "Removing",
                busy,
                Some(Message::DisableCrossplay).filter(|_| idle),
                style::secondary,
                BUTTON_MD,
            ));
        }
        Some(_) => {
            rows.push(note(if server.framework == "Fabric" {
                "Installs Geyser, Floodgate and Fabric API from Modrinth."
            } else {
                "Downloads Geyser and Floodgate from GeyserMC and checks their checksums."
            }));
            rows.push(busy_button(
                "Enable crossplay",
                "Installing",
                busy,
                Some(Message::EnableCrossplay).filter(|_| idle),
                style::primary,
                BUTTON_MD,
            ));
        }
    }
    section("Bedrock crossplay", rows)
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return Space::new().into();
    };
    let mut page = column![].spacing(SPACE_7).padding(iced::Padding {
        right: SPACE_4,
        bottom: SPACE_7,
        ..iced::Padding::ZERO
    });
    if let Some(error) = &state.error {
        page = page.push(
            row![
                icon(Icon::Warning, 16.0),
                text(error.as_str()).size(TEXT_MD).color(CORAL)
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
        );
    }
    let sections = [crossplay_section(state, server)];
    for (index, part) in sections.into_iter().enumerate() {
        if index > 0 {
            page = page.push(rule::horizontal(1).style(style::hairline));
        }
        page = page.push(part);
    }
    scrollable(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::scroll)
        .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        *state = State {
            server_id: Some(server.id.clone()),
            ..State::default()
        };
    }
    load(server.clone())
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
