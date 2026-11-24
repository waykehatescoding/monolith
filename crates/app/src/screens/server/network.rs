use iced::widget::{column, row, rule, scrollable, text, Space};
use iced::{clipboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::crossplay::{self, Crossplay};
use monolith_core::events::Event;
use monolith_core::types::ServerRecord;
use monolith_core::upnp::{self, UpnpState};
use monolith_core::{ngrok_manager, port_check};
use monolith_theme::token::{
    AMBER, BUTTON_MD, BUTTON_SM, CORAL, SPACE_2, SPACE_3, SPACE_4, SPACE_7, TEXT_MD, TEXT_SM,
};
use monolith_theme::widget::sized_icon;
use monolith_theme::{icon, style, Icon, MONO};

use super::settings::{section, setting, switch};
use crate::ctx::Ctx;
use crate::ui::busy_button;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Crossplay,
    Upnp,
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    local_ip: String,
    port: u16,
    crossplay: Option<Crossplay>,
    upnp: Option<UpnpState>,
    busy: Option<Busy>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Loaded {
    id: String,
    local_ip: String,
    port: u16,
    crossplay: Crossplay,
    upnp: Option<UpnpState>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Loaded),
    Upnp(bool),
    EnableCrossplay,
    DisableCrossplay,
    Done(Busy, Result<(), String>),
    Copy(String),
}

fn load(server: ServerRecord) -> Task<Message> {
    Task::perform(
        async move {
            Loaded {
                local_ip: ngrok_manager::get_local_ip().await,
                port: port_check::configured_port(std::path::Path::new(&server.server_path)).await,
                crossplay: crossplay::status(&server).await,
                upnp: upnp::state(&server.id),
                id: server.id,
            }
        },
        Message::Loaded,
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server().cloned() else {
        return Task::none();
    };
    match message {
        Message::Loaded(loaded) => {
            if state.server_id.as_deref() == Some(loaded.id.as_str()) {
                state.local_ip = loaded.local_ip;
                state.port = loaded.port;
                state.crossplay = Some(loaded.crossplay);
                state.upnp = loaded.upnp;
            }
            Task::none()
        }
        Message::Upnp(enabled) => {
            state.busy = Some(Busy::Upnp);
            state.error = None;
            Task::perform(upnp::set_enabled(server.id, enabled), |result| {
                Message::Done(Busy::Upnp, result)
            })
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
                    match busy {
                        Busy::Crossplay => ctx.changed_while_running(&server.id),
                        Busy::Upnp => ctx.refresh_servers(),
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
                idle.then_some(Message::DisableCrossplay),
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
                idle.then_some(Message::EnableCrossplay),
                style::primary,
                BUTTON_MD,
            ));
        }
    }
    section("Bedrock crossplay", rows)
}

fn router_section<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let enabled = upnp::is_enabled_for(&server.extra);
    let bedrock = state
        .crossplay
        .as_ref()
        .filter(|c| c.installed)
        .map(|c| format!(" and UDP {}", c.bedrock_port))
        .unwrap_or_default();
    let hint = format!(
        "When the server starts, Monolith asks your router to forward TCP {}{bedrock} to this PC. The rule lasts an hour, renews while the server runs and is removed when it stops.",
        state.port
    );
    let toggle: Element<'a, Message> = if state.busy.is_none() {
        switch(enabled, Message::Upnp)
    } else {
        iced::widget::toggler(enabled)
            .size(18)
            .style(style::switch)
            .into()
    };
    let mut rows = vec![setting(
        "Open the port on my router automatically (UPnP)",
        hint,
        toggle,
    )];
    match state.upnp.as_ref().filter(|_| enabled) {
        Some(UpnpState::Opening) => rows.push(note("Asking your router to open the port")),
        Some(UpnpState::Open { external_ip, ports }) => {
            let opened: Vec<String> = ports.iter().map(|(p, n)| format!("{p} {n}")).collect();
            rows.push(note(format!("Forwarded {}.", opened.join(" and "))));
            if !external_ip.is_empty() {
                rows.push(address("Internet", format!("{external_ip}:{}", state.port)));
            }
        }
        Some(UpnpState::Failed(error)) => rows.push(
            text(error.as_str())
                .size(TEXT_SM)
                .style(style::colored(CORAL))
                .into(),
        ),
        None if enabled => rows.push(note("The port opens the next time the server starts.")),
        None => {}
    }
    section("Router", rows)
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
    let sections = [
        crossplay_section(state, server),
        router_section(state, server),
    ];
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

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    match (event, ctx.selected_server()) {
        (Event::NetworkChanged, Some(server))
            if state.server_id.as_deref() == Some(server.id.as_str()) =>
        {
            state.upnp = upnp::state(&server.id);
        }
        _ => {}
    }
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
