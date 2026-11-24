use iced::widget::{column, row, text};
use iced::{Alignment, Element, Length, Task};
use monolith_core::types::{ServerRecord, SuccessResult};
use monolith_core::webhooks::{
    is_webhook_url, save_webhook_config, send_test, webhook_config, WebhookConfig,
};
use monolith_theme::style;
use monolith_theme::token::{BUTTON_MD, CORAL, SPACE_2, SPACE_3, TEXT_SM};

use super::settings::{field, save_row, section, setting, switch};
use crate::ctx::Ctx;
use crate::ui::busy_button;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    saved: WebhookConfig,
    draft: WebhookConfig,
    saving: bool,
    testing: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Toggle {
    Started,
    Stopped,
    Crashed,
    Joined,
    Left,
    BackupFinished,
    BackupFailed,
}

#[derive(Debug, Clone)]
pub enum Message {
    Url(String),
    Toggle(Toggle, bool),
    Save,
    Saved(WebhookConfig, SuccessResult),
    Test,
    Tested(SuccessResult),
}

fn flag(config: &mut WebhookConfig, toggle: Toggle) -> &mut bool {
    match toggle {
        Toggle::Started => &mut config.started,
        Toggle::Stopped => &mut config.stopped,
        Toggle::Crashed => &mut config.crashed,
        Toggle::Joined => &mut config.joined,
        Toggle::Left => &mut config.left,
        Toggle::BackupFinished => &mut config.backup_finished,
        Toggle::BackupFailed => &mut config.backup_failed,
    }
}

impl State {
    fn url_invalid(&self) -> bool {
        let url = self.draft.url.trim();
        !url.is_empty() && !is_webhook_url(url)
    }
}

pub fn on_show(state: &mut State, server: &ServerRecord) {
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        let config = webhook_config(server);
        *state = State {
            server_id: Some(server.id.clone()),
            saved: config.clone(),
            draft: config,
            ..State::default()
        };
    }
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    match message {
        Message::Url(url) => {
            state.draft.url = url;
            state.error = None;
        }
        Message::Toggle(toggle, on) => *flag(&mut state.draft, toggle) = on,
        Message::Save => {
            state.saving = true;
            state.error = None;
            let config = state.draft.clone();
            return Task::perform(
                save_webhook_config(server.id.clone(), config.clone()),
                move |result| Message::Saved(config.clone(), result),
            );
        }
        Message::Saved(config, result) => {
            state.saving = false;
            if result.success {
                state.saved = config;
                ctx.success("Discord alerts saved");
                ctx.refresh_servers();
            } else {
                state.error = result.error;
            }
        }
        Message::Test => {
            state.testing = true;
            state.error = None;
            return Task::perform(
                send_test(state.draft.url.clone(), server.name.clone()),
                Message::Tested,
            );
        }
        Message::Tested(result) => {
            state.testing = false;
            if result.success {
                ctx.success("Test message sent to Discord");
            } else {
                state.error = result.error;
            }
        }
    }
    Task::none()
}

pub fn view(state: &State) -> Element<'_, Message> {
    let has_url = is_webhook_url(&state.draft.url);
    let mut url = column![row![
        field(
            "https://discord.com/api/webhooks/...",
            &state.draft.url,
            state.url_invalid(),
            Message::Url,
        )
        .secure(true)
        .width(Length::Fill),
        busy_button(
            "Send test",
            "Sending",
            state.testing,
            has_url.then_some(Message::Test),
            style::secondary,
            BUTTON_MD,
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)]
    .spacing(SPACE_2);
    if let Some(error) = &state.error {
        url = url.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }

    let toggles = [
        ("Server online", Toggle::Started, state.draft.started),
        ("Server stopped", Toggle::Stopped, state.draft.stopped),
        ("Server crashed", Toggle::Crashed, state.draft.crashed),
        ("Player joined", Toggle::Joined, state.draft.joined),
        ("Player left", Toggle::Left, state.draft.left),
        (
            "Backup finished",
            Toggle::BackupFinished,
            state.draft.backup_finished,
        ),
        (
            "Backup failed",
            Toggle::BackupFailed,
            state.draft.backup_failed,
        ),
    ];
    let events =
        toggles
            .into_iter()
            .fold(column![].spacing(SPACE_3), |list, (label, toggle, on)| {
                list.push(
                    row![
                        text(label).size(TEXT_SM).width(Length::Fill),
                        switch(on, move |on| Message::Toggle(toggle, on)),
                    ]
                    .align_y(Alignment::Center),
                )
            });

    let dirty = state.draft != state.saved;
    section(
        "Discord alerts",
        vec![
            column![
                text("Post to a Discord channel when something happens on this server. In Discord, open the channel settings, then Integrations, Webhooks, and copy the webhook URL.")
                    .size(TEXT_SM)
                    .style(style::muted),
                url,
            ]
            .spacing(SPACE_3)
            .into(),
            setting("Send a message when", "Crash alerts include the last errors from the console.", events),
            save_row(
                "Save",
                "Saving",
                state.saving,
                (dirty && !state.url_invalid()).then_some(Message::Save),
                None,
            ),
        ],
    )
}
