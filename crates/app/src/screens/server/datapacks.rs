use iced::widget::{column, container, row, rule, text, tooltip};
use iced::{Alignment, Element, Length, Task};
use monolith_core::datapacks::{self, Datapack};
use monolith_core::server_runner::send_command;
use monolith_core::types::ServerRuntimeStatus;
use monolith_theme::token::*;
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, icon_colored, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use super::settings::switch;
use crate::ctx::{Confirm, Ctx};
use crate::ui::busy_button;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    world: Option<String>,
    packs: Vec<Datapack>,
    loaded: bool,
    error: Option<String>,
    adding: bool,
    busy: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Result<Vec<Datapack>, String>),
    Add,
    Added(Result<Option<String>, String>),
    Toggle(String, bool),
    Toggled(Result<(), String>),
    Send(String, bool),
    Remove(String, bool),
}

fn load(id: String, world: String) -> Task<Message> {
    Task::perform(
        datapacks::list_datapacks(id, world.clone()),
        move |result| Message::Loaded(world.clone(), result),
    )
}

pub fn show(state: &mut State, id: &str, world: &str) -> Task<Message> {
    if state.server_id.as_deref() != Some(id) || state.world.as_deref() != Some(world) {
        *state = State {
            server_id: Some(id.to_string()),
            world: Some(world.to_string()),
            ..State::default()
        };
    }
    load(id.to_string(), world.to_string())
}

fn online(ctx: &Ctx, id: &str) -> bool {
    ctx.status(id) == ServerRuntimeStatus::Online
}

fn stopped(ctx: &Ctx, id: &str) -> bool {
    matches!(
        ctx.status(id),
        ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let (Some(id), Some(world)) = (state.server_id.clone(), state.world.clone()) else {
        return Task::none();
    };
    match message {
        Message::Loaded(from, result) => {
            if from == world {
                state.loaded = true;
                match result {
                    Ok(packs) => {
                        state.packs = packs;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            Task::none()
        }
        Message::Add => {
            state.adding = true;
            Task::perform(
                async move {
                    let Some(file) = rfd::AsyncFileDialog::new()
                        .set_title("Add a data pack")
                        .add_filter("Data pack", &["zip"])
                        .pick_file()
                        .await
                    else {
                        return Ok(None);
                    };
                    datapacks::add_datapack(id, world, file.path().to_path_buf())
                        .await
                        .map(Some)
                },
                Message::Added,
            )
        }
        Message::Added(result) => {
            state.adding = false;
            match result {
                Ok(Some(name)) if online(ctx, &id) => {
                    ctx.success(format!("{name} added. Run /reload to load it."))
                }
                Ok(Some(name)) => ctx.success(format!("{name} added.")),
                Ok(None) => {}
                Err(error) => ctx.error(format!("Could not add the data pack: {error}")),
            }
            Task::none()
        }
        Message::Toggle(name, enabled) => {
            state.busy = Some(name.clone());
            Task::perform(
                datapacks::set_datapack_enabled(id, world, name, enabled),
                Message::Toggled,
            )
        }
        Message::Toggled(result) => {
            state.busy = None;
            if let Err(error) = result {
                ctx.error(format!("Could not change the data pack: {error}"));
            }
            Task::none()
        }
        Message::Send(name, enable) => {
            let verb = if enable { "enable" } else { "disable" };
            ctx.info(format!(
                "Asked the server to {verb} {name}. The console shows the result."
            ));
            Task::future(send_command(id, format!("datapack {verb} \"file/{name}\""))).discard()
        }
        Message::Remove(name, enabled) => {
            ctx.confirm(
                Confirm::new(
                    format!("Remove {name}?"),
                    format!("The data pack is deleted from {world}. Anything it adds, like recipes, structures or functions, stops working."),
                    "Remove",
                    true,
                    move || {
                        let (id, world, name) = (id.clone(), world.clone(), name.clone());
                        async move {
                            datapacks::remove_datapack(id, world, name.clone(), enabled).await?;
                            Ok(format!("{name} removed"))
                        }
                    },
                )
                .busy_label("Removing"),
            );
            Task::none()
        }
    }
}

fn pack_row<'a>(state: &'a State, pack: &'a Datapack, ctx: &Ctx, id: &str) -> Element<'a, Message> {
    let mut line = row![
        icon_colored(Icon::Package, 18.0, INK_SECONDARY),
        column![
            text(&pack.name).size(TEXT_MD).font(UI_MEDIUM),
            text(if pack.description.is_empty() {
                "No description"
            } else {
                pack.description.as_str()
            })
            .size(TEXT_SM)
            .style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
    ]
    .spacing(SPACE_4)
    .align_y(Alignment::Center);
    if online(ctx, id) {
        let send = |label, enable| {
            sized(text(label).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::ghost)
                .on_press(Message::Send(pack.name.clone(), enable))
        };
        if pack.enabled {
            line = line.push(send("Enable", true)).push(send("Disable", false));
        } else {
            line = line.push(text("Disabled").size(TEXT_SM).style(style::muted));
        }
    } else if stopped(ctx, id) {
        let name = pack.name.clone();
        line = line.push(if state.busy.as_deref() == Some(pack.name.as_str()) {
            text("Moving").size(TEXT_SM).style(style::muted).into()
        } else {
            switch(pack.enabled, move |on| Message::Toggle(name.clone(), on))
        });
    }
    line.push(tooltip(
        sized_icon(icon(Icon::Trash, 16.0), BUTTON_MD)
            .style(style::danger)
            .on_press_maybe(
                stopped(ctx, id).then(|| Message::Remove(pack.name.clone(), pack.enabled)),
            ),
        container(text("Remove data pack").size(TEXT_XS).color(INK))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Left,
    ))
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let (Some(id), Some(world)) = (state.server_id.as_deref(), state.world.as_deref()) else {
        return column![].into();
    };
    let hint = if online(ctx, id) {
        "While the server runs, Enable and Disable send /datapack to it. Stop the server to remove packs or move them in and out of datapacks_disabled."
    } else {
        "Turning a pack off moves it to datapacks_disabled in the world folder, which Minecraft ignores."
    };
    let header = row![
        column![
            text(format!("Data packs in {world}"))
                .size(TEXT_LG)
                .font(UI_SEMIBOLD),
            text(hint).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        busy_button(
            "Add data pack",
            "Adding",
            state.adding,
            Some(Message::Add),
            style::secondary,
            BUTTON_MD,
        ),
    ]
    .spacing(SPACE_4)
    .align_y(Alignment::Center);
    let body: Element<'a, Message> = if let Some(error) = &state.error {
        text(format!("Could not read the data packs: {error}"))
            .size(TEXT_MD)
            .color(CORAL)
            .into()
    } else if !state.loaded {
        text("Reading data packs")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    } else if state.packs.is_empty() {
        text("No data packs in this world yet.")
            .size(TEXT_MD)
            .style(style::muted)
            .into()
    } else {
        let mut rows = column![];
        for (index, pack) in state.packs.iter().enumerate() {
            if index > 0 {
                rows = rows.push(rule::horizontal(1).style(style::hairline));
            }
            rows = rows.push(container(pack_row(state, pack, ctx, id)).padding([SPACE_3, SPACE_4]));
        }
        container(rows)
            .width(Length::Fill)
            .style(style::panel)
            .into()
    };
    column![header, body].spacing(SPACE_3).into()
}
