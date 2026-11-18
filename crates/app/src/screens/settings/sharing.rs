use iced::widget::{button, column, container, progress_bar, row, scrollable, text, text_input, toggler};
use monolith_theme::token::{BUTTON_MD};
use monolith_theme::widget::{sized, sized_icon};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use monolith_core::events::Event;
use monolith_core::types::{NgrokAuthtokenValidation, SuccessResult};
use monolith_core::{ngrok_manager, safety};
use monolith_theme::style;
use monolith_theme::token::{
    CORAL, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, TEXT_LG, TEXT_MD, TEXT_SM,
    TEXT_XL,
};
use monolith_theme::{icon, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::Ctx;

const TOKEN_URL: &str = "https://dashboard.ngrok.com/get-started/your-authtoken";

#[derive(Debug, Default)]
pub struct State {
    loaded: bool,
    installed: bool,
    installing: bool,
    percent: f32,
    enabled: bool,
    saved_token: Option<String>,
    token: String,
    saving: bool,
    toggling: bool,
    confirm_remove: bool,
    error: Option<String>,
    local_ip: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(bool, bool, Option<String>, String),
    Install,
    Installed(SuccessResult),
    Enabled(bool),
    EnabledSaved(bool, Result<(), String>),
    Token(String),
    SaveToken,
    Validated(String, NgrokAuthtokenValidation),
    TokenSaved(SuccessResult),
    AskRemove,
    CancelRemove,
    RemoveToken,
    TokenRemoved(SuccessResult),
    OpenTokenPage,
    CopyIp,
}

fn load() -> Task<Message> {
    Task::perform(
        async {
            let installed = ngrok_manager::get_ngrok_status(String::new()).await.installed;
            let enabled = ngrok_manager::is_ngrok_enabled().await;
            let token = ngrok_manager::get_ngrok_authtoken_censored().await;
            let ip = ngrok_manager::get_local_ip().await;
            (installed, enabled, token, ip)
        },
        |(installed, enabled, token, ip)| Message::Loaded(installed, enabled, token, ip),
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(installed, enabled, token, ip) => {
            state.loaded = true;
            state.installed = installed;
            state.enabled = enabled;
            state.saved_token = token;
            state.local_ip = Some(ip).filter(|ip| !ip.trim().is_empty());
            Task::none()
        }
        Message::Install => {
            if state.installing || !state.loaded {
                return Task::none();
            }
            state.installing = true;
            state.percent = 0.0;
            Task::perform(ngrok_manager::install_ngrok(), Message::Installed)
        }
        Message::Installed(result) => {
            state.installing = false;
            if result.success {
                state.installed = true;
                ctx.success("ngrok is installed.");
            } else {
                ctx.error(format!(
                    "Could not install ngrok: {} Check your internet connection and try again.",
                    result.error.unwrap_or_default()
                ));
            }
            Task::none()
        }
        Message::Enabled(enabled) => {
            if state.toggling {
                return Task::none();
            }
            state.toggling = true;
            state.enabled = enabled;
            Task::perform(ngrok_manager::set_ngrok_enabled(enabled), move |result| {
                Message::EnabledSaved(enabled, result)
            })
        }
        Message::EnabledSaved(enabled, result) => {
            state.toggling = false;
            if let Err(e) = result {
                state.enabled = !enabled;
                ctx.error(format!("Could not save this setting: {e} Try again."));
            }
            Task::none()
        }
        Message::Token(value) => {
            state.token = value;
            state.error = None;
            Task::none()
        }
        Message::SaveToken => {
            let token = state.token.trim().to_string();
            if token.is_empty() || state.saving {
                return Task::none();
            }
            state.saving = true;
            state.error = None;
            Task::perform(
                async move {
                    let validation = ngrok_manager::validate_ngrok_authtoken(token.clone()).await;
                    (token, validation)
                },
                |(token, validation)| Message::Validated(token, validation),
            )
        }
        Message::Validated(token, validation) => {
            if validation.valid {
                Task::perform(
                    ngrok_manager::configure_ngrok_authtoken(token),
                    Message::TokenSaved,
                )
            } else {
                state.saving = false;
                state.error = validation.error;
                Task::none()
            }
        }
        Message::TokenSaved(result) => {
            state.saving = false;
            if result.success {
                state.token.clear();
                ctx.success("Authtoken saved.");
                load()
            } else {
                state.error = result.error;
                Task::none()
            }
        }
        Message::AskRemove => {
            state.confirm_remove = true;
            Task::none()
        }
        Message::CancelRemove => {
            state.confirm_remove = false;
            Task::none()
        }
        Message::RemoveToken => {
            state.confirm_remove = false;
            state.saving = true;
            Task::perform(
                async {
                    let removed = ngrok_manager::remove_ngrok_authtoken().await;
                    if removed.success {
                        let _ = ngrok_manager::set_ngrok_enabled(false).await;
                    }
                    removed
                },
                Message::TokenRemoved,
            )
        }
        Message::TokenRemoved(result) => {
            state.saving = false;
            if result.success {
                state.saved_token = None;
                state.enabled = false;
                ctx.success("Authtoken removed.");
            } else {
                ctx.error(format!(
                    "Could not remove the authtoken: {}",
                    result.error.unwrap_or_default()
                ));
            }
            Task::none()
        }
        Message::OpenTokenPage => {
            match safety::get_safe_external_url(TOKEN_URL) {
                Some(url) => {
                    if let Err(e) = open::that_detached(url) {
                        ctx.error(format!("Could not open the link: {e}"));
                    }
                }
                None => ctx.error("That link is not safe to open."),
            }
            Task::none()
        }
        Message::CopyIp => match &state.local_ip {
            Some(ip) => {
                ctx.info("Address copied.");
                iced::clipboard::write(ip.clone())
            }
            None => Task::none(),
        },
    }
}

fn heading(label: &str) -> Element<'_, Message> {
    text(label).size(TEXT_XL).font(UI_SEMIBOLD).into()
}

fn subheading(label: &str) -> Element<'_, Message> {
    text(label).size(TEXT_LG).font(UI_SEMIBOLD).into()
}

fn action<'a>(
    label: &'a str,
    glyph: Icon,
    message: Option<Message>,
    primary: bool,
) -> Element<'a, Message> {
    let content = row![icon(glyph, 16.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center);
    let base = sized(content, BUTTON_MD).on_press_maybe(message);
    if primary {
        base.style(style::primary).into()
    } else {
        base.style(style::secondary).into()
    }
}

fn install_section(state: &State) -> Element<'_, Message> {
    let status: Element<Message> = if !state.loaded {
        text("Checking for ngrok")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    } else if state.installing {
        column![
            text(format!("Downloading ngrok, {:.0}%", state.percent)).size(TEXT_MD),
            progress_bar(0.0..=100.0, state.percent)
                .girth(6)
                .style(style::progress)
        ]
        .spacing(SPACE_2)
        .into()
    } else if state.installed {
        text("ngrok is installed.")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    } else {
        text("ngrok is not installed yet. Monolith downloads it for you.")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    };
    let label = if state.installing {
        "Installing"
    } else if state.installed {
        "Reinstall ngrok"
    } else {
        "Install ngrok"
    };
    column![
        subheading("ngrok"),
        status,
        container(action(
            label,
            Icon::Download,
            (state.loaded && !state.installing).then_some(Message::Install),
            !state.installed,
        ))
    ]
    .spacing(SPACE_3)
    .into()
}

fn remove_controls(state: &State) -> Element<'_, Message> {
    if state.confirm_remove {
        row![
            text("Remove this authtoken?")
                .size(TEXT_SM)
                .style(style::muted),
            action("Cancel", Icon::Close, Some(Message::CancelRemove), false),
            action("Remove", Icon::Trash, Some(Message::RemoveToken), false)
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center)
        .into()
    } else {
        action(
            "Remove",
            Icon::Trash,
            (!state.saving).then_some(Message::AskRemove),
            false,
        )
    }
}

fn token_section(state: &State) -> Element<'_, Message> {
    let mut section = column![subheading("Authtoken")].spacing(SPACE_3);

    if let Some(saved) = &state.saved_token {
        section = section.push(
            row![
                text(saved.as_str()).size(TEXT_MD).width(Length::Fill),
                remove_controls(state)
            ]
            .spacing(SPACE_4)
            .align_y(Alignment::Center),
        );
    } else {
        section = section.push(
            text("Paste the authtoken from your ngrok account. Monolith checks it before saving.")
                .size(TEXT_SM)
                .style(style::muted),
        );
    }

    let has_error = state.error.is_some();
    let can_save = !state.token.trim().is_empty() && !state.saving;
    let input = text_input("Paste authtoken", &state.token)
        .secure(true)
        .size(TEXT_MD)
        .padding([8, 12])
        .width(Length::Fill)
        .on_input_maybe((!state.saving).then_some(Message::Token))
        .on_submit_maybe(can_save.then_some(Message::SaveToken))
        .style(move |theme: &Theme, status| {
            if has_error {
                style::input_error(theme, status)
            } else {
                style::input(theme, status)
            }
        });
    let save_label = if state.saving {
        "Checking"
    } else if state.saved_token.is_some() {
        "Replace"
    } else {
        "Save"
    };
    section = section.push(
        row![
            input,
            action(
                save_label,
                Icon::Check,
                can_save.then_some(Message::SaveToken),
                true
            )
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
    );

    if let Some(error) = &state.error {
        section = section.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }

    section
        .push(
            button(
                text("Get a free authtoken from ngrok")
                    .size(TEXT_MD)
                    .style(style::link),
            )
            .padding(0)
            .style(style::ghost)
            .on_press(Message::OpenTokenPage),
        )
        .into()
}

fn enable_section(state: &State) -> Element<'_, Message> {
    let ready = state.saved_token.is_some() && state.installed;
    let mut switch = toggler(state.enabled && ready)
        .size(20)
        .style(style::switch);
    if ready && !state.toggling {
        switch = switch.on_toggle(Message::Enabled);
    }
    let detail = if ready {
        "Servers get a public address friends can join from anywhere."
    } else if state.saved_token.is_none() {
        "Add an authtoken first to turn this on."
    } else {
        "Install ngrok first to turn this on."
    };
    row![
        column![
            text("Share servers online").size(TEXT_MD).font(UI_MEDIUM),
            text(detail).size(TEXT_SM).style(style::muted)
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        switch
    ]
    .spacing(SPACE_5)
    .align_y(Alignment::Center)
    .into()
}

fn lan_section(state: &State) -> Element<'_, Message> {
    let address: Element<Message> = match &state.local_ip {
        Some(ip) => row![
            text(ip.as_str()).size(TEXT_MD).font(MONO),
            sized_icon(icon(Icon::Copy, 16.0), BUTTON_MD)
                .style(style::icon_button)
                .on_press(Message::CopyIp)
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center)
        .into(),
        None => text("Looking up your address")
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
    };
    column![
        subheading("Same network"),
        text("Players on your Wi-Fi or router can join with this address and the server port. No account needed.")
            .size(TEXT_SM)
            .style(style::muted),
        address
    ]
    .spacing(SPACE_3)
    .into()
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let body = column![
        heading("Sharing"),
        text("Sharing lets friends outside your home network join your server through a free ngrok tunnel.")
            .size(TEXT_MD)
            .style(style::muted),
        install_section(state),
        token_section(state),
        enable_section(state),
        lan_section(state),
    ]
    .spacing(SPACE_6);
    scrollable(container(body).padding([0, 16]))
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    if let Event::NgrokInstallProgress(progress) = event {
        state.percent = progress.percent as f32;
    }
    Task::none()
}

pub fn on_show(_state: &mut State, _ctx: &Ctx) -> Task<Message> {
    load()
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
