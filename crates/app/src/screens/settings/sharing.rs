use iced::widget::{
    button, column, container, progress_bar, row, scrollable, text, text_input, toggler,
};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use monolith_core::events::Event;
use monolith_core::playit::{self, PlayitStatus};
use monolith_core::types::{NgrokAuthtokenValidation, SuccessResult};
use monolith_core::{ngrok_manager, safety};
use monolith_theme::style;
use monolith_theme::token::BUTTON_MD;
use monolith_theme::token::{
    CORAL, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XL,
};
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::Ctx;
use crate::ui::busy_button;

const TOKEN_URL: &str = "https://dashboard.ngrok.com/get-started/your-authtoken";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayitBusy {
    Install,
    Link,
    Toggle,
    Unlink,
}

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
    playit: Option<PlayitStatus>,
    playit_busy: Option<PlayitBusy>,
    playit_error: Option<String>,
    claim_url: Option<String>,
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
    PlayitLoaded(PlayitStatus),
    PlayitInstall,
    PlayitLink,
    PlayitEnabled(bool),
    PlayitUnlink,
    PlayitDone(PlayitBusy, Result<(), String>),
    Open(String),
}

fn load_playit() -> Task<Message> {
    Task::perform(playit::status(), Message::PlayitLoaded)
}

fn open_link(url: &str, ctx: &Ctx) {
    match safety::get_safe_external_url(url) {
        Some(url) => {
            if let Err(e) = open::that_detached(url) {
                ctx.error(format!("Could not open the link: {e}"));
            }
        }
        None => ctx.error("That link is not safe to open."),
    }
}

fn load() -> Task<Message> {
    Task::perform(
        async {
            let installed = ngrok_manager::get_ngrok_status(String::new())
                .await
                .installed;
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
            open_link(TOKEN_URL, ctx);
            Task::none()
        }
        Message::Open(url) => {
            open_link(&url, ctx);
            Task::none()
        }
        Message::PlayitLoaded(status) => {
            state.playit = Some(status);
            Task::none()
        }
        Message::PlayitInstall => {
            state.playit_busy = Some(PlayitBusy::Install);
            state.playit_error = None;
            Task::perform(playit::install(), |r| {
                Message::PlayitDone(PlayitBusy::Install, r)
            })
        }
        Message::PlayitLink => {
            let code = playit::claim_code();
            let url = playit::claim_url(&code);
            open_link(&url, ctx);
            state.claim_url = Some(url);
            state.playit_busy = Some(PlayitBusy::Link);
            state.playit_error = None;
            Task::perform(playit::wait_for_claim(code), |r| {
                Message::PlayitDone(PlayitBusy::Link, r)
            })
        }
        Message::PlayitEnabled(enabled) => {
            state.playit_busy = Some(PlayitBusy::Toggle);
            Task::perform(playit::set_enabled(enabled), |r| {
                Message::PlayitDone(PlayitBusy::Toggle, r)
            })
        }
        Message::PlayitUnlink => {
            state.playit_busy = Some(PlayitBusy::Unlink);
            Task::perform(
                async {
                    playit::set_enabled(false).await?;
                    playit::unlink().await
                },
                |r| Message::PlayitDone(PlayitBusy::Unlink, r),
            )
        }
        Message::PlayitDone(busy, result) => {
            state.playit_busy = None;
            state.claim_url = None;
            match result {
                Ok(()) => match busy {
                    PlayitBusy::Install => ctx.success("playit.gg is installed."),
                    PlayitBusy::Link => ctx.success("Your playit.gg account is linked."),
                    PlayitBusy::Unlink => ctx.success("playit.gg is unlinked."),
                    PlayitBusy::Toggle => {}
                },
                Err(error) => state.playit_error = Some(error),
            }
            load_playit()
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
            .size(TEXT_SM)
            .style(style::muted)
            .into()
    } else if state.installing {
        column![
            text(format!("Downloading ngrok, {:.0}%", state.percent)).size(TEXT_SM),
            progress_bar(0.0..=100.0, state.percent)
                .girth(6)
                .style(style::progress)
        ]
        .spacing(SPACE_2)
        .into()
    } else if state.installed {
        text("ngrok is installed.")
            .size(TEXT_SM)
            .style(style::muted)
            .into()
    } else {
        text("ngrok is not installed yet. Monolith downloads it for you.")
            .size(TEXT_SM)
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
        .padding(iced::Padding::ZERO.top(SPACE_1))
    ]
    .spacing(SPACE_2)
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
    let mut section = column![subheading("Authtoken")].spacing(SPACE_2);

    if let Some(saved) = &state.saved_token {
        section = section.push(
            row![
                text(saved.as_str())
                    .size(TEXT_MD)
                    .font(MONO)
                    .width(Length::Fill),
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
        container(
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
        )
        .padding(iced::Padding::ZERO.top(SPACE_1)),
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

fn link<'a>(label: &'a str, url: String) -> Element<'a, Message> {
    button(text(label).size(TEXT_MD).style(style::link))
        .padding(0)
        .style(style::ghost)
        .on_press(Message::Open(url))
        .into()
}

fn playit_section(state: &State) -> Element<'_, Message> {
    let mut section = column![
        subheading("playit.gg"),
        text("A free alternative to ngrok that also carries Bedrock (UDP) traffic for crossplay. When it is on, shared servers use playit.gg instead of ngrok.")
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_2);
    let Some(status) = &state.playit else {
        return section
            .push(
                text("Checking for playit.gg")
                    .size(TEXT_SM)
                    .style(style::muted),
            )
            .into();
    };
    let busy = state.playit_busy;
    let idle = busy.is_none();
    let mut actions = row![busy_button(
        if status.installed {
            "Reinstall playit.gg"
        } else {
            "Install playit.gg"
        },
        "Installing",
        busy == Some(PlayitBusy::Install),
        idle.then_some(Message::PlayitInstall),
        if status.installed {
            style::secondary
        } else {
            style::primary
        },
        BUTTON_MD,
    )]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);
    if status.installed && !status.claimed {
        actions = actions.push(busy_button(
            "Link your playit account",
            "Waiting for approval",
            busy == Some(PlayitBusy::Link),
            idle.then_some(Message::PlayitLink),
            style::primary,
            BUTTON_MD,
        ));
    }
    if status.claimed {
        actions = actions.push(busy_button(
            "Unlink",
            "Unlinking",
            busy == Some(PlayitBusy::Unlink),
            idle.then_some(Message::PlayitUnlink),
            style::secondary,
            BUTTON_MD,
        ));
    }
    section = section.push(container(actions).padding(iced::Padding::ZERO.top(SPACE_1)));

    if let Some(url) = &state.claim_url {
        section = section.push(
            row![
                text("Approve Monolith in your browser:")
                    .size(TEXT_SM)
                    .style(style::muted),
                link(url, url.clone()),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
        );
    }
    if let Some(error) = &state.playit_error {
        section = section.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }

    if status.claimed {
        let mut switch = toggler(status.enabled).size(20).style(style::switch);
        if idle {
            switch = switch.on_toggle(Message::PlayitEnabled);
        }
        section = section
            .push(
                row![
                    column![
                        text("Use playit.gg for sharing").size(TEXT_MD).font(UI_MEDIUM),
                        text("On playit.gg, add a Minecraft Java tunnel to 127.0.0.1 and your server port. For crossplay, add a Minecraft Bedrock tunnel to UDP port 19132.")
                            .size(TEXT_SM)
                            .style(style::muted)
                    ]
                    .spacing(SPACE_1)
                    .width(Length::Fill),
                    switch
                ]
                .spacing(SPACE_5)
                .align_y(Alignment::Center),
            )
            .push(link("Set up tunnels on playit.gg", playit::TUNNELS_URL.into()));
    }
    if status.running {
        for tunnel in &status.tunnels {
            section = section.push(
                text(format!(
                    "{} ({})",
                    tunnel.address,
                    tunnel.protocol.to_uppercase()
                ))
                .size(TEXT_MD)
                .font(MONO),
            );
        }
        if status.tunnels.is_empty() {
            for line in &status.lines {
                section = section.push(
                    text(line.as_str())
                        .size(TEXT_SM)
                        .font(MONO)
                        .style(style::muted),
                );
            }
        }
    }
    section.into()
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
        container(address).padding(iced::Padding::ZERO.top(SPACE_1))
    ]
    .spacing(SPACE_2)
    .into()
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let body = column![
        column![
            heading("Sharing"),
            text("Sharing lets friends outside your home network join your server through a free ngrok or playit.gg tunnel.")
                .size(TEXT_MD)
                .style(style::muted),
        ]
        .spacing(SPACE_2),
        install_section(state),
        token_section(state),
        enable_section(state),
        playit_section(state),
        lan_section(state),
    ]
    .spacing(SPACE_6 + SPACE_2);
    scrollable(container(body).padding([0, 16]))
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    match event {
        Event::NgrokInstallProgress(progress) => state.percent = progress.percent as f32,
        Event::NetworkChanged if state.playit.is_some() => return load_playit(),
        _ => {}
    }
    Task::none()
}

pub fn on_show(_state: &mut State, _ctx: &Ctx) -> Task<Message> {
    Task::batch([load(), load_playit()])
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
