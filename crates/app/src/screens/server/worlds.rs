use std::path::PathBuf;

use iced::widget::{column, container, row, rule, scrollable, text, toggler};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::server_runner::start_server;
use monolith_core::types::ServerRuntimeStatus;
use monolith_core::worlds::{self, Dimension, World};
use monolith_theme::token::*;
use monolith_theme::widget::sized;
use monolith_theme::{icon_colored, style, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};

use super::backups::{format_age, format_size};
use super::datapacks;

use super::properties::{self, Kind};
use super::settings::{dropdown, field};
use crate::ctx::{Confirm, Ctx};
use crate::ui::busy_button;

#[derive(Debug)]
struct Import {
    source: PathBuf,
    name: String,
    make_active: bool,
}

#[derive(Debug)]
struct Reset {
    world: String,
    dimension: Dimension,
    change_seed: bool,
    seed: String,
}

#[derive(Debug)]
struct NewWorld {
    name: String,
    seed: String,
    level_type: &'static str,
    structures: bool,
}

impl Default for NewWorld {
    fn default() -> Self {
        Self {
            name: String::new(),
            seed: String::new(),
            level_type: level_types().0,
            structures: true,
        }
    }
}

fn level_types() -> (&'static str, &'static [&'static str]) {
    match properties::find("level-type").map(|spec| &spec.kind) {
        Some(Kind::Choice(default, options)) => (default, options),
        _ => ("minecraft:normal", &["minecraft:normal"]),
    }
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    worlds: Vec<World>,
    loaded: bool,
    error: Option<String>,
    switching: Option<String>,
    exporting: Option<String>,
    picking: bool,
    import: Option<Import>,
    importing: bool,
    reset: Option<Reset>,
    new_world: NewWorld,
    creating: bool,
    datapacks: datapacks::State,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Result<Vec<World>, String>),
    Activate(String),
    Activated(String, Result<(), String>),
    Export(String),
    Exported(Result<Option<String>, String>),
    PickImport(bool),
    Picked(Option<PathBuf>),
    ImportName(String),
    ImportActive(bool),
    CancelImport,
    Import,
    Imported(Result<String, String>),
    OpenReset(String),
    ResetDimension(Dimension),
    ResetChangeSeed(bool),
    ResetSeed(String),
    CancelReset,
    ConfirmReset,
    NewName(String),
    NewSeed(String),
    NewType(&'static str),
    NewStructures(bool),
    Create,
    Created(Result<(String, bool), String>),
    Datapacks(datapacks::Message),
}

fn suggested_name(source: &std::path::Path) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
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
            if from != id {
                return Task::none();
            }
            state.loaded = true;
            match result {
                Ok(list) => {
                    state.worlds = list;
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
            match state.worlds.iter().find(|w| w.active) {
                Some(active) => {
                    datapacks::show(&mut state.datapacks, &id, &active.name).map(Message::Datapacks)
                }
                None => Task::none(),
            }
        }
        Message::Datapacks(message) => {
            datapacks::update(&mut state.datapacks, message, ctx).map(Message::Datapacks)
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
        Message::Export(name) => {
            state.exporting = Some(name.clone());
            Task::perform(
                async move {
                    let Some(target) = rfd::AsyncFileDialog::new()
                        .set_file_name(format!("{name}.zip"))
                        .add_filter("Zip archive", &["zip"])
                        .save_file()
                        .await
                    else {
                        return Ok(None);
                    };
                    let path = target.path().to_path_buf();
                    worlds::export_world(id, name, path.clone()).await?;
                    Ok(Some(path.to_string_lossy().into_owned()))
                },
                Message::Exported,
            )
        }
        Message::Exported(result) => {
            state.exporting = None;
            match result {
                Ok(Some(path)) => ctx.success(format!("World exported to {path}")),
                Ok(None) => {}
                Err(error) => ctx.error(format!("Export failed: {error}")),
            }
            Task::none()
        }
        Message::PickImport(folder) => {
            state.picking = true;
            Task::perform(
                async move {
                    let dialog = rfd::AsyncFileDialog::new().set_title("Import a world");
                    let picked = if folder {
                        dialog.pick_folder().await
                    } else {
                        dialog.add_filter("Zip archive", &["zip"]).pick_file().await
                    };
                    picked.map(|file| file.path().to_path_buf())
                },
                Message::Picked,
            )
        }
        Message::Picked(source) => {
            state.picking = false;
            if let Some(source) = source {
                state.import = Some(Import {
                    name: suggested_name(&source),
                    source,
                    make_active: false,
                });
            }
            Task::none()
        }
        Message::ImportName(name) => {
            if let Some(import) = &mut state.import {
                import.name = name;
            }
            Task::none()
        }
        Message::ImportActive(on) => {
            if let Some(import) = &mut state.import {
                import.make_active = on;
            }
            Task::none()
        }
        Message::CancelImport => {
            state.import = None;
            Task::none()
        }
        Message::Import => {
            let Some(import) = &state.import else {
                return Task::none();
            };
            state.importing = true;
            Task::perform(
                worlds::import_world(
                    id,
                    import.source.clone(),
                    import.name.clone(),
                    import.make_active,
                ),
                Message::Imported,
            )
        }
        Message::Imported(result) => {
            state.importing = false;
            match result {
                Ok(name) => {
                    let active = state.import.take().is_some_and(|i| i.make_active);
                    if active && running(ctx, &id) {
                        ctx.success(format!(
                            "{name} imported and set as the active world. Restart the server to load it."
                        ));
                        ctx.changed_while_running(&id);
                    } else {
                        ctx.success(format!("{name} imported."));
                    }
                }
                Err(error) => ctx.error(format!("Import failed: {error}")),
            }
            Task::none()
        }
        Message::OpenReset(world) => {
            state.reset = Some(Reset {
                world,
                dimension: Dimension::All,
                change_seed: false,
                seed: String::new(),
            });
            Task::none()
        }
        Message::ResetDimension(dimension) => {
            if let Some(reset) = &mut state.reset {
                reset.dimension = dimension;
            }
            Task::none()
        }
        Message::ResetChangeSeed(on) => {
            if let Some(reset) = &mut state.reset {
                reset.change_seed = on;
            }
            Task::none()
        }
        Message::ResetSeed(seed) => {
            if let Some(reset) = &mut state.reset {
                reset.seed = seed;
            }
            Task::none()
        }
        Message::CancelReset => {
            state.reset = None;
            Task::none()
        }
        Message::ConfirmReset => {
            let Some(reset) = state.reset.take() else {
                return Task::none();
            };
            let seed = (reset.dimension == Dimension::All && reset.change_seed)
                .then(|| reset.seed.trim().to_string());
            let (title, part) = match reset.dimension {
                Dimension::All => (format!("Reset {}?", reset.world), "The whole world"),
                Dimension::Nether => (
                    format!("Reset the Nether of {}?", reset.world),
                    "The Nether",
                ),
                Dimension::End => (format!("Reset the End of {}?", reset.world), "The End"),
            };
            let body = format!(
                "{part} is deleted and generated again the next time the server starts. Monolith backs up the server first as \"Before world reset\", so you can restore it from Backups."
            );
            let world = reset.world;
            let dimension = reset.dimension;
            ctx.confirm(
                Confirm::new(title, body, "Reset", true, move || {
                    let id = id.clone();
                    let world = world.clone();
                    let seed = seed.clone();
                    async move {
                        worlds::reset_world(id, world.clone(), dimension, seed).await?;
                        Ok(format!(
                            "{world} reset. It is generated again on the next start."
                        ))
                    }
                })
                .busy_label("Backing up and resetting"),
            );
            Task::none()
        }
        Message::NewName(name) => {
            state.new_world.name = name;
            Task::none()
        }
        Message::NewSeed(seed) => {
            state.new_world.seed = seed;
            Task::none()
        }
        Message::NewType(level_type) => {
            state.new_world.level_type = level_type;
            Task::none()
        }
        Message::NewStructures(on) => {
            state.new_world.structures = on;
            Task::none()
        }
        Message::Create => {
            state.creating = true;
            let start = !running(ctx, &id);
            let new = &state.new_world;
            let created = worlds::create_world(
                id.clone(),
                new.name.clone(),
                new.seed.clone(),
                new.level_type.to_string(),
                new.structures,
            );
            Task::perform(
                async move {
                    let name = created.await?;
                    if start {
                        let started = start_server(id).await;
                        if !started.success {
                            return Err(format!(
                                "{name} is set up, but the server did not start: {}",
                                started.error.unwrap_or_default()
                            ));
                        }
                    }
                    Ok((name, start))
                },
                Message::Created,
            )
        }
        Message::Created(result) => {
            state.creating = false;
            match result {
                Ok((name, true)) => {
                    ctx.success(format!("Starting the server to generate {name}."));
                    state.new_world = NewWorld::default();
                }
                Ok((name, false)) => {
                    ctx.success(format!(
                        "{name} is now the active world. Restart the server to generate it."
                    ));
                    ctx.changed_while_running(&id);
                    state.new_world = NewWorld::default();
                }

                Err(error) => ctx.error(error),
            }
            Task::none()
        }
    }
}

fn new_world_view<'a>(state: &'a State, running: bool) -> Element<'a, Message> {
    let new = &state.new_world;
    let checked = worlds::check_name(&new.name);
    let taken = checked.as_ref().is_ok_and(|name| {
        state
            .worlds
            .iter()
            .any(|w| w.generated && w.name.eq_ignore_ascii_case(name))
    });
    let problem = match &checked {
        Err(error) if !new.name.is_empty() => Some(error.clone()),
        _ if taken => Some("A world with this name already exists".to_string()),
        _ => None,
    };
    let label = |value| text(value).size(TEXT_MD).width(140);
    let mut form = column![row![
        label("Name"),
        field("survival", &new.name, problem.is_some(), Message::NewName).width(280),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center),]
    .spacing(SPACE_3);
    if let Some(problem) = problem {
        form = form.push(text(problem).size(TEXT_SM).color(CORAL));
    }
    form = form
        .push(
            row![
                label("Seed"),
                field("Random", &new.seed, false, Message::NewSeed).width(280),
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
        )
        .push(
            row![
                label("World type"),
                dropdown(level_types().1, Some(new.level_type), Message::NewType).width(280),
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
        )
        .push(
            toggler(new.structures)
                .label("Generate villages, temples and other structures")
                .size(18)
                .text_size(TEXT_MD)
                .spacing(SPACE_2)
                .style(style::switch)
                .on_toggle(Message::NewStructures),
        )
        .push(row![busy_button(
            if running {
                "Create world"
            } else {
                "Create and start"
            },
            "Creating",
            state.creating,
            (checked.is_ok() && !taken).then_some(Message::Create),
            style::primary,
            BUTTON_MD,
        )]);
    let hint = if running {
        "Monolith sets level-name, level-seed, level-type and generate-structures in server.properties. Minecraft generates the world when the server restarts."
    } else {
        "Monolith sets level-name, level-seed, level-type and generate-structures in server.properties, then starts the server so Minecraft generates the world."
    };
    column![
        column![
            text("New world").size(TEXT_LG).font(UI_SEMIBOLD),
            text(hint).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1),
        container(form)
            .padding(SPACE_4)
            .width(Length::Fill)
            .style(style::panel),
    ]
    .spacing(SPACE_3)
    .into()
}

fn reset_view<'a>(reset: &'a Reset, stopped: bool) -> Element<'a, Message> {
    let mut form = column![row![
        text("Reset").size(TEXT_MD).width(80),
        dropdown(
            &Dimension::ALL[..],
            Some(reset.dimension),
            Message::ResetDimension
        )
        .width(200),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)]
    .spacing(SPACE_3);
    if reset.dimension == Dimension::All {
        form = form.push(
            toggler(reset.change_seed)
                .label("Use a new seed")
                .size(18)
                .text_size(TEXT_MD)
                .spacing(SPACE_2)
                .style(style::switch)
                .on_toggle(Message::ResetChangeSeed),
        );
        if reset.change_seed {
            form = form.push(
                row![
                    text("Seed").size(TEXT_MD).width(80),
                    field("Random", &reset.seed, false, Message::ResetSeed).width(280),
                ]
                .spacing(SPACE_3)
                .align_y(Alignment::Center),
            );
        }
    }
    if !stopped {
        form = form.push(
            text("Stop the server to reset its world.")
                .size(TEXT_SM)
                .style(style::muted),
        );
    }
    form.push(
        row![
            sized(text("Cancel").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::ghost)
                .on_press(Message::CancelReset),
            sized(text("Reset").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::danger)
                .on_press_maybe(stopped.then_some(Message::ConfirmReset)),
        ]
        .spacing(SPACE_2),
    )
    .into()
}

fn import_view(state: &State) -> Element<'_, Message> {
    let header = column![
        text("Import a world").size(TEXT_LG).font(UI_SEMIBOLD),
        text("Add a world from a .zip or a folder that contains level.dat. Nether and End folders saved next to it, like world_nether, come along.")
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_1);
    let Some(import) = &state.import else {
        let choose = |label, folder| {
            sized(text(label).size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::secondary)
                .on_press_maybe((!state.picking).then_some(Message::PickImport(folder)))
        };
        return column![
            header,
            row![choose("Choose zip", false), choose("Choose folder", true)].spacing(SPACE_2)
        ]
        .spacing(SPACE_3)
        .into();
    };
    let checked = worlds::check_name(&import.name);
    let taken = checked.as_ref().is_ok_and(|name| {
        state
            .worlds
            .iter()
            .any(|w| w.name.eq_ignore_ascii_case(name))
    });
    let problem = match &checked {
        Err(error) if !import.name.is_empty() => Some(error.clone()),
        _ if taken => Some("A world with this name already exists".to_string()),
        _ => None,
    };
    let mut form = column![
        text(import.source.to_string_lossy())
            .size(TEXT_SM)
            .font(MONO),
        row![
            text("Name").size(TEXT_MD).width(80),
            field(
                "world_name",
                &import.name,
                problem.is_some(),
                Message::ImportName
            )
            .width(280),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_3);
    if let Some(problem) = problem {
        form = form.push(text(problem).size(TEXT_SM).color(CORAL));
    }
    form = form.push(
        toggler(import.make_active)
            .label("Make it the active world")
            .size(18)
            .text_size(TEXT_MD)
            .spacing(SPACE_2)
            .style(style::switch)
            .on_toggle(Message::ImportActive),
    );
    form = form.push(
        row![
            sized(text("Cancel").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::ghost)
                .on_press_maybe((!state.importing).then_some(Message::CancelImport)),
            busy_button(
                "Import world",
                "Importing",
                state.importing,
                (checked.is_ok() && !taken).then_some(Message::Import),
                style::primary,
                BUTTON_MD,
            ),
        ]
        .spacing(SPACE_2),
    );
    column![
        header,
        container(form)
            .padding(SPACE_4)
            .width(Length::Fill)
            .style(style::panel)
    ]
    .spacing(SPACE_3)
    .into()
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
    if world.generated {
        line = line.push(
            sized(text("Reset").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::ghost)
                .on_press(Message::OpenReset(world.name.clone())),
        );
        line = line.push(busy_button(
            "Export",
            "Exporting",
            state.exporting.as_deref() == Some(world.name.as_str()),
            state
                .exporting
                .is_none()
                .then(|| Message::Export(world.name.clone())),
            style::ghost,
            BUTTON_MD,
        ));
    }
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
            if let Some(reset) = state.reset.as_ref().filter(|r| r.world == world.name) {
                rows = rows.push(container(reset_view(reset, !running(ctx, id))).padding(
                    iced::Padding {
                        top: 0.0,
                        right: SPACE_4,
                        bottom: SPACE_3,
                        left: 56.0,
                    },
                ));
            }
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
    let page = column![
        list_view(state, ctx, id),
        datapacks::view(&state.datapacks, ctx).map(Message::Datapacks),
        new_world_view(state, running(ctx, id)),
        import_view(state)
    ]
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
