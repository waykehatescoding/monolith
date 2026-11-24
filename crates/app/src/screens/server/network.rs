use iced::widget::{column, row, rule, scrollable, text, Space};
use iced::{clipboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::crossplay::{self, Crossplay};
use monolith_core::events::Event;
use monolith_core::proxy::{self, Backend};
use monolith_core::reachability::{self, Reachability};
use monolith_core::types::{ServerRecord, ServerRuntimeStatus};
use monolith_core::upnp::{self, UpnpState};
use monolith_core::{ngrok_manager, port_check};
use monolith_theme::token::{
    AMBER, BUTTON_MD, BUTTON_SM, CORAL, MOSS, SPACE_2, SPACE_3, SPACE_4, SPACE_7, TEXT_MD, TEXT_SM,
};
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, style, Icon, MONO, UI_MEDIUM};

use super::settings::{dropdown, section, setting, switch};
use crate::ctx::{Ctx, Route, SettingsPage};
use crate::ui::busy_button;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Crossplay,
    Upnp,
    Check,
    Firewall,
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    id: String,
    name: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    local_ip: String,
    port: u16,
    crossplay: Option<Crossplay>,
    upnp: Option<UpnpState>,
    reach: Option<Reachability>,
    backends: Vec<Backend>,
    pick: Option<Choice>,
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
    backends: Vec<Backend>,
}

#[derive(Debug, Clone)]
pub enum Message {
    CreateProxy,
    ProxyCreated(Result<String, String>),
    Pick(Choice),
    Attach,
    Detach(String),
    StartNetwork,
    Loaded(Loaded),
    Upnp(bool),
    Check,
    Checked(Result<Reachability, String>),
    AddFirewallRule,
    OpenSharing,
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
                backends: proxy::backends(&server).await,
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
                state.backends = loaded.backends;
            }
            Task::none()
        }
        Message::CreateProxy => {
            state.busy = Some(Busy::Proxy);
            state.error = None;
            Task::perform(
                proxy::create_proxy(format!("{} Network", server.name), 1024),
                |result| Message::ProxyCreated(result.map(|created| created.id)),
            )
        }
        Message::ProxyCreated(result) => {
            state.busy = None;
            match result {
                Ok(id) => {
                    ctx.success("Velocity proxy created. Add servers to it here.");
                    ctx.refresh_servers();
                    ctx.select_server(id);
                }
                Err(error) => state.error = Some(error),
            }
            Task::none()
        }
        Message::Pick(choice) => {
            state.pick = Some(choice);
            Task::none()
        }
        Message::Attach => {
            let Some(choice) = state.pick.take() else {
                return Task::none();
            };
            state.busy = Some(Busy::Proxy);
            state.error = None;
            Task::perform(proxy::attach(server.id, choice.id), |result| {
                Message::Done(Busy::Proxy, result)
            })
        }
        Message::Detach(backend) => {
            state.busy = Some(Busy::Proxy);
            state.error = None;
            Task::perform(proxy::detach(server.id, backend), |result| {
                Message::Done(Busy::Proxy, result)
            })
        }
        Message::StartNetwork => {
            state.busy = Some(Busy::Proxy);
            state.error = None;
            Task::perform(proxy::start_network(server.id), |result| {
                Message::Done(Busy::Proxy, result)
            })
        }
        Message::Check => {
            state.busy = Some(Busy::Check);
            state.error = None;
            state.reach = None;
            Task::perform(reachability::check(server), Message::Checked)
        }
        Message::Checked(result) => {
            state.busy = None;
            match result {
                Ok(reach) => state.reach = Some(reach),
                Err(error) => state.error = Some(error),
            }
            Task::none()
        }
        Message::AddFirewallRule => {
            state.busy = Some(Busy::Firewall);
            state.error = None;
            Task::perform(
                reachability::add_firewall_rule(state.port, bedrock_port(state)),
                |result| Message::Done(Busy::Firewall, result),
            )
        }
        Message::OpenSharing => {
            ctx.navigate(Route::Settings(SettingsPage::Sharing));
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
                        Busy::Firewall => ctx.success("Windows Firewall now allows the server."),
                        Busy::Proxy => {
                            ctx.refresh_servers();
                            ctx.changed_while_running(&server.id);
                        }
                        Busy::Check => {}
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

fn bedrock_port(state: &State) -> Option<u16> {
    state
        .crossplay
        .as_ref()
        .filter(|c| c.installed)
        .map(|c| c.bedrock_port)
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
    let bedrock = bedrock_port(state)
        .map(|port| format!(" and UDP {port}"))
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

fn reach_section<'a>(
    state: &'a State,
    ctx: &'a Ctx,
    server: &'a ServerRecord,
) -> Element<'a, Message> {
    let online = ctx.status(&server.id) == ServerRuntimeStatus::Online;
    let idle = state.busy.is_none();
    let mut rows = vec![
        note("Checks whether players outside your network can reach this server. It sends your public IP address and server port to ipify.org and mcsrvstat.us, which test the connection from the internet. Results can be up to 5 minutes old."),
        busy_button(
            "Check if friends can join",
            "Checking",
            state.busy == Some(Busy::Check),
            (idle && online).then_some(Message::Check),
            style::primary,
            BUTTON_MD,
        ),
    ];
    if !online {
        rows.push(note("Start the server first."));
    }
    let Some(reach) = &state.reach else {
        return section("Can friends join?", rows);
    };
    if reach.reachable {
        rows.push(
            text(format!("Friends can join at {}.", reach.address))
                .size(TEXT_MD)
                .style(style::colored(MOSS))
                .into(),
        );
        return section("Can friends join?", rows);
    }
    rows.push(
        text(format!(
            "Players outside your network cannot reach {}.",
            reach.address
        ))
        .size(TEXT_MD)
        .style(style::colored(CORAL))
        .into(),
    );
    let sharing = sized(
        text("Open Sharing settings").size(TEXT_MD).font(UI_MEDIUM),
        BUTTON_MD,
    )
    .style(style::secondary)
    .on_press(Message::OpenSharing);
    if reach.cgnat {
        rows.push(note("Your router's internet address differs from your public address, so your provider most likely shares one address between customers (CGNAT) or there is a second router in front of yours. Port forwarding cannot work here. Use ngrok or playit.gg instead."));
        rows.push(sharing.into());
        return section("Can friends join?", rows);
    }
    rows.push(note(format!(
        "The port may not be forwarded. Turn on UPnP above, or forward TCP {} to {} in your router's settings.",
        state.port, state.local_ip
    )));
    rows.push(note("Windows Firewall may be blocking the server. Adding an inbound rule needs administrator permission, so Windows will ask first. Monolith runs:"));
    rows.push(
        text(reachability::firewall_command(
            state.port,
            bedrock_port(state),
        ))
        .size(TEXT_SM)
        .font(MONO)
        .into(),
    );
    rows.push(
        row![
            busy_button(
                "Add firewall rule",
                "Waiting for Windows",
                state.busy == Some(Busy::Firewall),
                idle.then_some(Message::AddFirewallRule),
                style::secondary,
                BUTTON_MD,
            ),
            sharing,
        ]
        .spacing(SPACE_3)
        .into(),
    );
    rows.push(note(
        "You can also skip port forwarding with ngrok or playit.gg.",
    ));
    section("Can friends join?", rows)
}

fn status_word(status: ServerRuntimeStatus) -> &'static str {
    match status {
        ServerRuntimeStatus::Online => "Online",
        ServerRuntimeStatus::Starting => "Starting",
        ServerRuntimeStatus::Stopping => "Stopping",
        ServerRuntimeStatus::Idle => "Idle",
        ServerRuntimeStatus::Offline => "Offline",
        ServerRuntimeStatus::Crashed => "Crashed",
    }
}

fn proxy_section<'a>(
    state: &'a State,
    ctx: &'a Ctx,
    server: &'a ServerRecord,
) -> Element<'a, Message> {
    let idle = state.busy.is_none();
    let busy = state.busy == Some(Busy::Proxy);
    let mut rows = vec![note("A Velocity proxy lets players move between several servers through one address. The servers behind it only listen on this PC, and players join the proxy.")];
    if server.framework != "Velocity" {
        let parent = ctx
            .servers
            .iter()
            .find(|s| proxy::backend_ids(s).contains(&server.id));
        match parent {
            Some(parent) => rows.push(note(format!(
                "This server is part of the {} network. Players join through the proxy.",
                parent.name
            ))),
            None => rows.push(busy_button(
                "Create Velocity proxy",
                "Downloading Velocity",
                busy,
                idle.then_some(Message::CreateProxy),
                style::secondary,
                BUTTON_MD,
            )),
        }
        return section("Proxy network", rows);
    }

    for backend in &state.backends {
        let status = ctx.status(&backend.id);
        rows.push(
            row![
                text(backend.name.as_str())
                    .size(TEXT_MD)
                    .width(Length::Fill),
                text(format!("127.0.0.1:{}", backend.port))
                    .size(TEXT_SM)
                    .font(MONO)
                    .style(style::muted),
                text(status_word(status))
                    .size(TEXT_SM)
                    .style(style::colored(crate::ctx::status_color(status))),
                busy_button(
                    "Remove",
                    "Removing",
                    false,
                    idle.then_some(Message::Detach(backend.id.clone())),
                    style::secondary,
                    BUTTON_SM,
                ),
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center)
            .into(),
        );
    }
    if state.backends.is_empty() {
        rows.push(note("No servers in this network yet."));
    }

    let attached: Vec<String> = ctx.servers.iter().flat_map(proxy::backend_ids).collect();
    let choices: Vec<Choice> = ctx
        .servers
        .iter()
        .filter(|s| s.framework != "Velocity" && !attached.contains(&s.id))
        .map(|s| Choice {
            id: s.id.clone(),
            name: s.name.clone(),
        })
        .collect();
    rows.push(
        row![
            dropdown(choices, state.pick.clone(), Message::Pick).placeholder("Choose a server"),
            busy_button(
                "Add server",
                "Working",
                busy,
                (idle && state.pick.is_some()).then_some(Message::Attach),
                style::secondary,
                BUTTON_MD,
            ),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center)
        .into(),
    );
    rows.push(note("Servers must be stopped to be added. Paper and Purpur need to have started once so config/paper-global.yml exists. Fabric servers get FabricProxy-Lite from Modrinth. Added servers switch to online-mode=false, listen only on 127.0.0.1 and trust the proxy's forwarding secret."));
    rows.push(busy_button(
        "Start network",
        "Starting network",
        busy,
        (idle && !state.backends.is_empty()).then_some(Message::StartNetwork),
        style::primary,
        BUTTON_MD,
    ));
    rows.push(note(
        "Starts each server, waits for it to come online, then starts the proxy.",
    ));
    section("Proxy network", rows)
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
        reach_section(state, ctx, server),
        proxy_section(state, ctx, server),
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
