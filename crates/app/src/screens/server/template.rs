use iced::widget::{checkbox, column, row, text};
use iced::{Alignment, Element, Length, Task};
use monolith_core::templates::save_template;
use monolith_core::types::ServerRecord;
use monolith_theme::style;
use monolith_theme::token::{BUTTON_MD, SPACE_2, SPACE_3, TEXT_MD, TEXT_SM};
use monolith_theme::widget::sized;

use super::settings::{field, section, setting};
use crate::ctx::Ctx;
use crate::ui::busy_button;

#[derive(Debug, Default)]
pub struct State {
    name: Option<String>,
    configs: bool,
    players: bool,
    saving: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Start(String),
    Name(String),
    Configs(bool),
    Players(bool),
    Cancel,
    Save(String),
    Saved(Result<String, String>),
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Start(name) => {
            *state = State {
                name: Some(name),
                ..State::default()
            };
            Task::none()
        }
        Message::Name(name) => {
            state.name = Some(name);
            Task::none()
        }
        Message::Configs(on) => {
            state.configs = on;
            Task::none()
        }
        Message::Players(on) => {
            state.players = on;
            Task::none()
        }
        Message::Cancel => {
            *state = State::default();
            Task::none()
        }
        Message::Save(server_id) => {
            let Some(name) = state.name.clone().filter(|n| !n.trim().is_empty()) else {
                return Task::none();
            };
            state.saving = true;
            Task::perform(
                save_template(server_id, name, state.configs, state.players),
                |result| Message::Saved(result.map(|template| template.name)),
            )
        }
        Message::Saved(result) => {
            state.saving = false;
            match result {
                Ok(name) => {
                    *state = State::default();
                    ctx.success(format!("Saved the template {name}"));
                }
                Err(error) => ctx.error(format!("Could not save the template: {error}")),
            }
            Task::none()
        }
    }
}

pub fn view<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let mut rows = vec![setting(
        "Save as template",
        "Reuse this server's platform, memory, properties and content when you create a new server. Worlds are not included.",
        busy_button(
            "Save as template",
            "Saving",
            state.saving,
            state
                .name
                .is_none()
                .then(|| Message::Start(server.name.clone())),
            style::secondary,
            BUTTON_MD,
        ),
    )];
    if let Some(name) = &state.name {
        let ready = !name.trim().is_empty() && !state.saving;
        let save = ready.then(|| Message::Save(server.id.clone()));
        rows.push(
            column![
                row![
                    field("Template name", name, name.trim().is_empty(), Message::Name)
                        .on_submit_maybe(save.clone())
                        .width(Length::Fill),
                    sized(text("Cancel").size(TEXT_MD), BUTTON_MD)
                        .style(style::ghost)
                        .on_press(Message::Cancel),
                    busy_button(
                        "Save template",
                        "Saving",
                        state.saving,
                        save,
                        style::primary,
                        BUTTON_MD,
                    ),
                ]
                .spacing(SPACE_2)
                .align_y(Alignment::Center),
                checkbox(state.configs)
                    .label(
                        "Include bukkit.yml, spigot.yml, Paper configs and plugin config.yml files"
                    )
                    .size(18)
                    .text_size(TEXT_SM)
                    .style(style::check)
                    .on_toggle(Message::Configs),
                checkbox(state.players)
                    .label("Include the whitelist and operators")
                    .size(18)
                    .text_size(TEXT_SM)
                    .style(style::check)
                    .on_toggle(Message::Players),
            ]
            .spacing(SPACE_3)
            .into(),
        );
    }
    section("Template", rows)
}
