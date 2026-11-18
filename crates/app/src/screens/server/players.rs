use std::time::Duration;

use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::sized;
use iced::widget::{button, column, container, row, scrollable, text, text_input, toggler, Space};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use monolith_core::events::Event;
use monolith_core::types::{ServerProperty, ServerRuntimeStatus, SuccessResult};
use monolith_core::{server_props, server_runner};
use monolith_theme::token::{
    SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, TEXT_LG, TEXT_MD, TEXT_SM,
};
use monolith_theme::{style, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{Confirm, Ctx};

const WHITELIST_KEY: &str = "white-list";

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    loaded: bool,
    whitelist_on: bool,
    whitelist: Vec<String>,
    banned: Vec<String>,
    whitelist_input: String,
    ban_input: String,
    ban_reason: String,
    busy: bool,
    refresh_ticks: u8,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, bool, Vec<String>, Vec<String>),
    WhitelistInput(String),
    BanInput(String),
    BanReason(String),
    ToggleWhitelist(bool),
    AddWhitelist,
    RemoveWhitelist(String),
    AddBan,
    ConfirmBan(String, String),
    Unban(String),
    Command(String, String),
    ConfirmKick(String),
    Done(Result<String, String>),
    Tick,
}

fn outcome(result: SuccessResult, done: String) -> Result<String, String> {
    if result.success {
        Ok(done)
    } else {
        Err(result.error.unwrap_or_else(|| {
            "The change could not be saved. Check that the server folder is writable.".into()
        }))
    }
}

fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_online(ctx: &Ctx, id: &str) -> bool {
    ctx.status(id) == ServerRuntimeStatus::Online
}

async fn load(id: String) -> Message {
    let properties = server_props::get_server_properties(id.clone()).await;
    let on = properties
        .iter()
        .any(|p| p.key == WHITELIST_KEY && p.value == "true");
    let whitelist = server_props::get_whitelist(id.clone()).await;
    let banned = server_props::get_banlist(id.clone()).await;
    Message::Loaded(id, on, whitelist, banned)
}

fn reload(id: String) -> Task<Message> {
    Task::perform(load(id), std::convert::identity)
}

async fn command(id: String, command: String, done: String) -> Result<String, String> {
    outcome(server_runner::send_command(id, command).await, done)
}

async fn save_whitelist(
    id: String,
    list: Vec<String>,
    live: bool,
    done: String,
) -> Result<String, String> {
    let result = server_props::save_whitelist(id.clone(), list).await;
    if result.success && live {
        server_runner::send_command(id, "whitelist reload".into()).await;
    }
    outcome(result, done)
}

async fn set_whitelist_enabled(id: String, on: bool, live: bool) -> Result<String, String> {
    let mut properties = server_props::get_server_properties(id.clone()).await;
    let value = on.to_string();
    match properties.iter_mut().find(|p| p.key == WHITELIST_KEY) {
        Some(property) => property.value = value,
        None => properties.push(ServerProperty {
            key: WHITELIST_KEY.into(),
            value,
            comment: None,
        }),
    }
    let result = server_props::save_server_properties(id.clone(), properties).await;
    if result.success && live {
        let word = if on { "on" } else { "off" };
        server_runner::send_command(id, format!("whitelist {word}")).await;
    }
    let done = if on {
        "Whitelist is on. Only listed players can join."
    } else {
        "Whitelist is off. Anyone with the address can join."
    };
    outcome(result, done.into())
}

async fn ban(id: String, live: bool, name: String, reason: String) -> Result<String, String> {
    let done = format!("{name} is banned.");
    if live {
        let line = if reason.is_empty() {
            format!("ban {name}")
        } else {
            format!("ban {name} {reason}")
        };
        return outcome(server_runner::send_command(id, line).await, done);
    }
    let mut list = server_props::get_banlist(id.clone()).await;
    if !list.contains(&name) {
        list.push(name);
    }
    outcome(server_props::save_banlist(id, list).await, done)
}

async fn unban(id: String, live: bool, name: String) -> Result<String, String> {
    let done = format!("{name} is no longer banned.");
    if live {
        return outcome(
            server_runner::send_command(id, format!("pardon {name}")).await,
            done,
        );
    }
    let mut list = server_props::get_banlist(id.clone()).await;
    list.retain(|n| n != &name);
    outcome(server_props::save_banlist(id, list).await, done)
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = ctx.selected.clone() else {
        return Task::none();
    };
    let live = is_online(ctx, &id);
    let blocked = state.busy
        && matches!(
            message,
            Message::AddWhitelist
                | Message::RemoveWhitelist(_)
                | Message::AddBan
                | Message::Unban(_)
                | Message::ToggleWhitelist(_)
                | Message::Command(..)
        );
    if blocked {
        return Task::none();
    }
    match message {
        Message::Loaded(loaded_id, on, whitelist, banned) => {
            if state.server_id.as_deref() == Some(loaded_id.as_str()) {
                state.whitelist_on = on;
                state.whitelist = whitelist;
                let pending = state.ban_input.trim().to_lowercase();
                if !pending.is_empty() && banned.iter().any(|n| n.to_lowercase() == pending) {
                    state.ban_input.clear();
                    state.ban_reason.clear();
                }
                state.banned = banned;
                state.loaded = true;
            }
            Task::none()
        }
        Message::WhitelistInput(value) => {
            state.whitelist_input = value;
            Task::none()
        }
        Message::BanInput(value) => {
            state.ban_input = value;
            Task::none()
        }
        Message::BanReason(value) => {
            state.ban_reason = value;
            Task::none()
        }
        Message::ToggleWhitelist(on) => {
            state.whitelist_on = on;
            state.busy = true;
            Task::perform(set_whitelist_enabled(id, on, live), Message::Done)
        }
        Message::AddWhitelist => {
            let name = state.whitelist_input.trim().to_string();
            if !valid_name(&name) {
                ctx.error("Player names are 3 to 16 letters, numbers or underscores.");
                return Task::none();
            }
            if state.whitelist.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                ctx.info(format!("{name} is already on the whitelist."));
                return Task::none();
            }
            let mut list = state.whitelist.clone();
            list.push(name.clone());
            state.whitelist_input.clear();
            state.busy = true;
            let done = format!("{name} can now join.");
            Task::perform(save_whitelist(id, list, live, done), Message::Done)
        }
        Message::RemoveWhitelist(name) => {
            let list: Vec<String> = state
                .whitelist
                .iter()
                .filter(|n| **n != name)
                .cloned()
                .collect();
            let title = format!("Remove {name} from the whitelist?");
            let body = if state.whitelist_on {
                format!("{name} will not be able to join until you add them again.")
            } else {
                format!("{name} will no longer be listed.")
            };
            let done = format!("{name} removed from the whitelist.");
            ctx.confirm(
                Confirm::new(title, body, "Remove player", true, move || {
                    save_whitelist(id.clone(), list.clone(), live, done.clone())
                })
                .busy_label("Removing"),
            );
            state.refresh_ticks = 20;
            Task::none()
        }
        Message::AddBan => {
            let name = state.ban_input.trim().to_string();
            if !valid_name(&name) {
                ctx.error("Player names are 3 to 16 letters, numbers or underscores.");
                return Task::none();
            }
            let reason = state.ban_reason.trim().to_string();
            update(state, Message::ConfirmBan(name, reason), ctx)
        }
        Message::ConfirmBan(name, reason) => {
            let title = format!("Ban {name}?");
            let body = format!("{name} will be kicked and unable to rejoin until you unban them.");
            ctx.confirm(
                Confirm::new(title, body, "Ban player", true, move || {
                    ban(id.clone(), live, name.clone(), reason.clone())
                })
                .busy_label("Banning"),
            );
            state.refresh_ticks = 20;
            Task::none()
        }
        Message::Unban(name) => {
            state.busy = true;
            Task::perform(unban(id, live, name), Message::Done)
        }
        Message::Command(line, done) => {
            state.busy = true;
            Task::perform(command(id, line, done), Message::Done)
        }
        Message::ConfirmKick(name) => {
            let title = format!("Kick {name}?");
            let body = format!("{name} is disconnected now and can rejoin straight away.");
            let line = format!("kick {name}");
            let done = format!("{name} was kicked.");
            ctx.confirm(
                Confirm::new(title, body, "Kick player", true, move || {
                    command(id.clone(), line.clone(), done.clone())
                })
                .busy_label("Kicking"),
            );
            Task::none()
        }
        Message::Done(result) => {
            state.busy = false;
            match result {
                Ok(message) => {
                    state.refresh_ticks = 6;
                    ctx.success(message)
                }
                Err(message) => ctx.error(message),
            }
            reload(id)
        }
        Message::Tick => {
            state.refresh_ticks = state.refresh_ticks.saturating_sub(1);
            reload(id)
        }
    }
}

fn section<'a>(
    title: &'a str,
    hint: impl text::IntoFragment<'a>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        column![
            column![
                text(title).size(TEXT_LG).font(UI_SEMIBOLD),
                text(hint).size(TEXT_SM).style(style::muted),
            ]
            .spacing(SPACE_1),
            body,
        ]
        .spacing(SPACE_4),
    )
    .padding(SPACE_5)
    .width(Length::Fill)
    .style(style::panel)
    .into()
}

fn action<'a>(
    label: &'a str,
    message: Message,
    look: fn(&Theme, button::Status) -> button::Style,
    enabled: bool,
) -> Element<'a, Message> {
    sized(text(label).size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
        .style(look)
        .on_press_maybe(enabled.then_some(message))
        .into()
}

fn empty<'a>(title: &'a str, hint: &'a str) -> Element<'a, Message> {
    column![
        text(title).size(TEXT_MD).font(UI_MEDIUM),
        text(hint).size(TEXT_SM).style(style::muted),
    ]
    .spacing(SPACE_1)
    .padding([SPACE_3, 0.0])
    .into()
}

fn name_row<'a>(name: &'a str, actions: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(name).size(TEXT_MD).width(Length::Fill),
        row(actions).spacing(SPACE_1),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .padding([SPACE_1, 0.0])
    .into()
}

fn online_section<'a>(ctx: &'a Ctx, id: &str, busy: bool) -> Element<'a, Message> {
    let names: &[String] = ctx
        .stats(id)
        .map(|s| s.online_players.as_slice())
        .unwrap_or(&[]);
    let body: Element<'a, Message> = if !is_online(ctx, id) {
        empty(
            "The server is offline",
            "Start the server from Overview to see who is playing.",
        )
    } else if names.is_empty() {
        empty(
            "Nobody is playing right now",
            "Share the join address from Overview and players will show up here.",
        )
    } else {
        names
            .iter()
            .fold(column![].spacing(SPACE_1), |list, name| {
                let op = Message::Command(format!("op {name}"), format!("{name} is now an operator."));
                let deop = Message::Command(
                    format!("deop {name}"),
                    format!("{name} is no longer an operator."),
                );
                list.push(name_row(
                    name,
                    vec![
                        action("Make operator", op, style::ghost, !busy),
                        action("Remove operator", deop, style::ghost, !busy),
                        action("Kick", Message::ConfirmKick(name.clone()), style::ghost, true),
                        action(
                            "Ban",
                            Message::ConfirmBan(name.clone(), String::new()),
                            style::danger,
                            true,
                        ),
                    ],
                ))
            })
            .into()
    };
    let hint = match names.len() {
        _ if !is_online(ctx, id) => "Offline".to_string(),
        1 => "1 player online".to_string(),
        count => format!("{count} players online"),
    };
    section("Online now", hint, body)
}

fn whitelist_section(state: &State) -> Element<'_, Message> {
    let hint = if state.whitelist_on {
        "Everyone not listed below is turned away."
    } else {
        "Anyone with the join address can play. Turn this on to limit it to the list."
    };
    let switch = row![
        column![
            text("Only whitelisted players can join")
                .size(TEXT_MD)
                .font(UI_MEDIUM),
            text(hint).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        toggler(state.whitelist_on)
            .on_toggle_maybe((!state.busy).then_some(Message::ToggleWhitelist))
            .style(style::switch),
    ]
    .spacing(SPACE_4)
    .align_y(Alignment::Center);

    let add = row![
        text_input("Player name", &state.whitelist_input)
            .on_input_maybe((!state.busy).then_some(Message::WhitelistInput))
            .on_submit_maybe((!state.busy).then_some(Message::AddWhitelist))
            .padding([6, 12])
            .width(Length::Fill)
            .style(style::input),
        sized(text("Add player").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
            .style(style::primary)
            .on_press_maybe((!state.busy).then_some(Message::AddWhitelist)),
    ]
    .spacing(SPACE_2);

    let list: Element<'_, Message> = if state.whitelist.is_empty() {
        empty(
            "The whitelist is empty",
            "Add the names of friends you want to let in.",
        )
    } else {
        state
            .whitelist
            .iter()
            .fold(column![].spacing(SPACE_1), |list, name| {
                list.push(name_row(
                    name,
                    vec![action(
                        "Remove",
                        Message::RemoveWhitelist(name.clone()),
                        style::ghost,
                        !state.busy,
                    )],
                ))
            })
            .into()
    };

    section(
        "Whitelist",
        "Players you approve to join.",
        column![switch, add, list].spacing(SPACE_4).into(),
    )
}

fn ban_section(state: &State) -> Element<'_, Message> {
    let add = row![
        text_input("Player name", &state.ban_input)
            .on_input_maybe((!state.busy).then_some(Message::BanInput))
            .on_submit_maybe((!state.busy).then_some(Message::AddBan))
            .padding([6, 12])
            .width(Length::Fill)
            .style(style::input),
        text_input("Reason (optional)", &state.ban_reason)
            .on_input_maybe((!state.busy).then_some(Message::BanReason))
            .on_submit_maybe((!state.busy).then_some(Message::AddBan))
            .padding([6, 12])
            .width(Length::Fill)
            .style(style::input),
        sized(text("Ban player").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
            .style(style::danger)
            .on_press_maybe((!state.busy).then_some(Message::AddBan)),
    ]
    .spacing(SPACE_2);

    let list: Element<'_, Message> = if state.banned.is_empty() {
        empty(
            "Nobody is banned",
            "Banned players cannot join until you unban them.",
        )
    } else {
        state
            .banned
            .iter()
            .fold(column![].spacing(SPACE_1), |list, name| {
                list.push(name_row(
                    name,
                    vec![action(
                        "Unban",
                        Message::Unban(name.clone()),
                        style::ghost,
                        !state.busy,
                    )],
                ))
            })
            .into()
    };

    section(
        "Banned players",
        "A reason is recorded only while the server is running.",
        column![add, list].spacing(SPACE_4).into(),
    )
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(id) = ctx.selected.as_deref() else {
        return Space::new().into();
    };
    if !state.loaded {
        return text("Loading players").size(TEXT_MD).style(style::muted).into();
    }
    scrollable(
        column![
            online_section(ctx, id, state.busy),
            whitelist_section(state),
            ban_section(state),
            Space::new().height(SPACE_6),
        ]
        .spacing(SPACE_5)
        .padding(iced::Padding::ZERO.right(SPACE_4)),
    )
    .style(style::scroll)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    match (event, ctx.selected.clone()) {
        (Event::ServerStatus(update), Some(id)) if update.server_id == id && state.loaded => {
            reload(id)
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
    reload(id)
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.refresh_ticks > 0 {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    }
}
