use std::path::Path;

use monolith_theme::token::{BUTTON_MD};
use monolith_theme::widget::{sized, sized_icon};
use chrono::{DateTime, Local};
use iced::widget::{
    button, column, container, progress_bar, row, rule, scrollable, text, text_input, toggler,
    tooltip, Space,
};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use monolith_core::backup_manager;
use monolith_core::events::Event;
use monolith_core::server_store;
use monolith_core::types::{
    BackupConfig, BackupEntry, BackupStage, BackupStatus, BackupType, ServerRuntimeStatus,
    ServerSettingsUpdate,
};
use monolith_theme::token::*;
use monolith_theme::{icon, icon_colored, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{Confirm, Ctx};

const MAX_INTERVAL_HOURS: u32 = 720;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    backups: Vec<BackupEntry>,
    loaded: bool,
    creating: bool,
    cancelling: bool,
    percent: f64,
    stage: Option<BackupStage>,
    auto_enabled: bool,
    interval: String,
    saved: (bool, String),
    saving: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Vec<BackupEntry>),
    Status(String, Option<BackupStatus>),
    Create,
    Created(String, Result<(), String>),
    Cancel,
    CancelResult(bool),
    Restore(String),
    Delete(String),
    OpenFolder,
    Tick,
    ToggleAuto(bool),
    Interval(String),
    SaveSchedule,
    ScheduleSaved(Result<(), String>),
}

fn parse_interval(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|hours| (1..=MAX_INTERVAL_HOURS).contains(hours))
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else {
        format!("{:.0} KB", (value / KB).max(1.0))
    }
}

fn parse_date(created_at: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(created_at)
        .ok()
        .map(|date| date.with_timezone(&Local))
}

fn format_date(created_at: &str) -> String {
    match parse_date(created_at) {
        Some(date) => date.format("%d %b %Y, %H:%M").to_string(),
        None => created_at.to_string(),
    }
}

fn format_age(created_at: &str) -> Option<String> {
    let minutes = Local::now()
        .signed_duration_since(parse_date(created_at)?)
        .num_minutes()
        .max(0);
    Some(match minutes {
        0..=1 => "just now".to_string(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=1439 => format!("{} hours ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    })
}

fn load(id: String) -> Task<Message> {
    let status_id = id.clone();
    Task::batch([
        Task::perform(backup_manager::get_backups(id.clone()), move |backups| {
            Message::Loaded(id.clone(), backups)
        }),
        Task::perform(
            backup_manager::get_backup_status(status_id.clone()),
            move |status| Message::Status(status_id.clone(), status),
        ),
    ])
}

fn sync_config(state: &mut State, ctx: &Ctx) {
    let config = ctx
        .selected_server()
        .and_then(|server| server.backup_config.clone())
        .unwrap_or_default();
    let hours = if config.interval_hours >= 1.0 {
        config.interval_hours as u32
    } else {
        24
    };
    state.auto_enabled = config.enabled;
    state.interval = hours.to_string();
    state.saved = (config.enabled, state.interval.clone());
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(id, backups) => {
            if state.server_id.as_deref() == Some(id.as_str()) {
                state.backups = backups;
                state.loaded = true;
            }
            Task::none()
        }
        Message::Status(id, status) => {
            if state.server_id.as_deref() == Some(id.as_str()) {
                if let Some(status) = status.filter(|status| status.in_progress) {
                    state.creating = true;
                    state.percent = status.percent;
                    state.stage = Some(status.stage);
                }
            }
            Task::none()
        }
        Message::Create => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            state.creating = true;
            state.cancelling = false;
            state.percent = 0.0;
            state.stage = Some(BackupStage::Calculating);
            Task::perform(backup_manager::create_backup(id.clone(), None), move |result| {
                let outcome = if result.success {
                    Ok(())
                } else {
                    Err(result.error.unwrap_or_else(|| "The backup failed.".into()))
                };
                Message::Created(id.clone(), outcome)
            })
        }
        Message::Created(id, outcome) => {
            if state.server_id.as_deref() != Some(id.as_str()) {
                return Task::none();
            }
            let cancelled = state.cancelling;
            state.creating = false;
            state.cancelling = false;
            state.stage = None;
            match outcome {
                Ok(()) => ctx.success("Backup created"),
                Err(_) if cancelled => ctx.info("Backup cancelled"),
                Err(error) => ctx.error(format!("Backup failed: {error}")),
            }
            load(id)
        }
        Message::Cancel => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            state.cancelling = true;
            Task::perform(backup_manager::cancel_backup(id), |result| {
                Message::CancelResult(result.success)
            })
        }
        Message::CancelResult(success) => {
            if !success {
                state.cancelling = false;
                ctx.error("The backup could not be cancelled. Wait for it to finish.");
            }
            Task::none()
        }
        Message::Restore(filename) => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            ctx.confirm(
                Confirm::new(
                    "Restore this backup?",
                    "The current server files will be replaced with this backup. Monolith saves a safety backup of the current state first, so you can undo the restore.",
                    "Restore backup",
                    true,
                    move || {
                        let id = id.clone();
                        let filename = filename.clone();
                        async move {
                            let result = backup_manager::restore_backup(id, filename).await;
                            if result.success {
                                Ok("Backup restored".to_string())
                            } else {
                                Err(result.error.unwrap_or_else(|| "The restore failed.".into()))
                            }
                        }
                    },
                )
                .busy_label("Restoring"),
            );
            Task::none()
        }
        Message::Delete(filename) => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            if state.creating {
                return Task::none();
            }
            ctx.confirm(
                Confirm::new(
                    "Delete this backup?",
                    "The backup file is removed from this computer. This cannot be undone.",
                    "Delete backup",
                    true,
                    move || {
                        let id = id.clone();
                        let filename = filename.clone();
                        async move {
                            let result = backup_manager::delete_backup(id, filename).await;
                            if result.success {
                                Ok("Backup deleted".to_string())
                            } else {
                                Err(result.error.unwrap_or_else(|| "The delete failed.".into()))
                            }
                        }
                    },
                )
                .busy_label("Deleting"),
            );
            Task::none()
        }
        Message::OpenFolder => {
            let Some(server) = ctx.selected_server() else {
                return Task::none();
            };
            let folder = Path::new(&server.server_path).join("backups");
            let opened = std::fs::create_dir_all(&folder)
                .map_err(|error| error.to_string())
                .and_then(|_| open::that_detached(&folder).map_err(|error| error.to_string()));
            if let Err(error) = opened {
                ctx.error(format!("Could not open the backups folder: {error}"));
            }
            Task::none()
        }
        Message::Tick => match state.server_id.clone() {
            Some(id) => load(id),
            _ => Task::none(),
        },
        Message::ToggleAuto(value) => {
            state.auto_enabled = value;
            Task::none()
        }
        Message::Interval(value) => {
            if value.chars().all(|c| c.is_ascii_digit()) {
                state.interval = value;
            }
            Task::none()
        }
        Message::SaveSchedule => {
            let (Some(id), Some(hours)) = (state.server_id.clone(), parse_interval(&state.interval))
            else {
                return Task::none();
            };
            state.saving = true;
            let last_backup_at = ctx
                .selected_server()
                .and_then(|server| server.backup_config.as_ref())
                .and_then(|config| config.last_backup_at.clone());
            let config = BackupConfig {
                enabled: state.auto_enabled,
                interval_hours: f64::from(hours),
                last_backup_at,
            };
            Task::perform(
                server_store::update_server_settings(
                    id,
                    ServerSettingsUpdate {
                        backup_config: Some(config),
                        ..Default::default()
                    },
                ),
                |result| {
                    Message::ScheduleSaved(if result.success {
                        Ok(())
                    } else {
                        Err(result
                            .error
                            .unwrap_or_else(|| "The schedule could not be saved.".into()))
                    })
                },
            )
        }
        Message::ScheduleSaved(outcome) => {
            state.saving = false;
            match outcome {
                Ok(()) => {
                    state.saved = (state.auto_enabled, state.interval.clone());
                    ctx.success("Backup schedule saved");
                    ctx.refresh_servers();
                }
                Err(error) => ctx.error(format!("Could not save the schedule: {error}")),
            }
            Task::none()
        }
    }
}

fn label(value: &'static str, glyph: Icon) -> Element<'static, Message> {
    row![icon(glyph, 16.0), text(value).size(TEXT_MD).font(UI_MEDIUM)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
}

fn action<'a>(
    content: Element<'a, Message>,
    look: fn(&Theme, button::Status) -> button::Style,
    message: Option<Message>,
) -> Element<'a, Message> {
    sized(content, BUTTON_MD)
        .style(look)
        .on_press_maybe(message)
        .into()
}

fn entry_row<'a>(backup: &'a BackupEntry, can_restore: bool, busy: bool) -> Element<'a, Message> {
    let kind = match backup.kind {
        BackupType::Auto => "Automatic",
        BackupType::Manual => "Manual",
    };
    let details = column![
        text(&backup.name).size(TEXT_MD).font(UI_MEDIUM),
        text(format!("{} · {}", format_date(&backup.created_at), kind))
            .size(TEXT_SM)
            .style(style::muted),
    ]
    .spacing(SPACE_1)
    .width(Length::Fill);

    row![
        icon_colored(Icon::Archive, 18.0, INK_SECONDARY),
        details,
        text(format_size(backup.size))
            .size(TEXT_SM)
            .style(style::muted),
        action(
            text("Restore").size(TEXT_MD).font(UI_MEDIUM).into(),
            style::secondary,
            can_restore.then(|| Message::Restore(backup.filename.clone())),
        ),
        tooltip(
            sized_icon(icon(Icon::Trash, 16.0), BUTTON_MD)
                .style(style::danger)
                .on_press_maybe((!busy).then(|| Message::Delete(backup.filename.clone()))),
            container(text("Delete backup").size(TEXT_XS).color(INK))
                .padding([4, 8])
                .style(style::tooltip),
            tooltip::Position::Left,
        ),
    ]
    .spacing(SPACE_4)
    .align_y(Alignment::Center)
    .into()
}

fn progress_view(state: &State) -> Element<'_, Message> {
    let stage = if state.cancelling {
        "Cancelling"
    } else {
        match state.stage {
            Some(BackupStage::Archiving) => "Compressing files",
            Some(BackupStage::Complete) => "Finishing up",
            _ => "Counting files",
        }
    };
    let cancel = action(
        text("Cancel").size(TEXT_MD).font(UI_MEDIUM).into(),
        style::ghost,
        (!state.cancelling).then_some(Message::Cancel),
    );
    container(
        column![
            row![
                text(stage).size(TEXT_MD).font(UI_MEDIUM).width(Length::Fill),
                text(format!("{:.0}%", state.percent.max(0.0)))
                    .size(TEXT_SM)
                    .style(style::muted),
                cancel,
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center),
            progress_bar(0.0..=100.0, state.percent.max(0.0) as f32)
                .girth(6)
                .style(style::progress),
        ]
        .spacing(SPACE_3),
    )
    .padding(SPACE_4)
    .width(Length::Fill)
    .style(style::panel)
    .into()
}

fn list_view<'a>(state: &'a State, can_restore: bool) -> Element<'a, Message> {
    if !state.loaded {
        return container(text("Loading backups").size(TEXT_MD).style(style::muted))
            .padding([SPACE_5, 0.0])
            .into();
    }
    if state.backups.is_empty() {
        return column![
            text("No backups yet").size(TEXT_LG).font(UI_SEMIBOLD),
            text("A backup is a zip of this server's world and settings. Make one before you change plugins or update the server.")
                .size(TEXT_MD)
                .style(style::muted),
        ]
        .spacing(SPACE_1)
        .max_width(480)
        .padding([SPACE_4, 0.0])
        .into();
    }

    let mut rows = column![];
    for (index, backup) in state.backups.iter().enumerate() {
        if index > 0 {
            rows = rows.push(rule::horizontal(1).style(style::hairline));
        }
        rows = rows.push(container(entry_row(backup, can_restore, state.creating)).padding([SPACE_3, SPACE_4]));
    }
    container(rows)
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

fn schedule_view(state: &State) -> Element<'_, Message> {
    let hours = parse_interval(&state.interval);
    let dirty = (state.auto_enabled, state.interval.clone()) != state.saved;
    let can_save = hours.is_some() && dirty && !state.saving;

    let input = text_input("24", &state.interval)
        .size(TEXT_MD)
        .padding([8, 12])
        .width(72)
        .on_input(Message::Interval)
        .on_submit_maybe(can_save.then_some(Message::SaveSchedule))
        .style(move |theme: &Theme, status| {
            if hours.is_none() {
                style::input_error(theme, status)
            } else {
                style::input(theme, status)
            }
        });
    let hint = if hours.is_none() {
        text(format!("Enter a whole number from 1 to {MAX_INTERVAL_HOURS}."))
            .size(TEXT_SM)
            .color(CORAL)
    } else {
        text("Backups run while Monolith is open.")
            .size(TEXT_SM)
            .style(style::muted)
    };
    let save = action(
        text(if state.saving { "Saving" } else { "Save schedule" })
            .size(TEXT_MD)
            .font(UI_MEDIUM)
            .into(),
        style::secondary,
        can_save.then_some(Message::SaveSchedule),
    );

    column![
        text("Automatic backups").size(TEXT_LG).font(UI_SEMIBOLD),
        toggler(state.auto_enabled)
            .label("Back up this server on a schedule")
            .size(18)
            .text_size(TEXT_MD)
            .spacing(SPACE_2)
            .style(style::switch)
            .on_toggle(Message::ToggleAuto),
        row![
            text("Every").size(TEXT_MD),
            input,
            text("hours").size(TEXT_MD),
            Space::new().width(SPACE_3),
            save,
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
        hint,
    ]
    .spacing(SPACE_3)
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let offline = state
        .server_id
        .as_deref()
        .is_none_or(|id| ctx.status(id) == ServerRuntimeStatus::Offline);
    let total: u64 = state.backups.iter().map(|backup| backup.size).sum();
    let summary = if state.backups.is_empty() {
        "Nothing saved yet".to_string()
    } else {
        let latest = state
            .backups
            .iter()
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
            .and_then(|backup| format_age(&backup.created_at));
        let base = format!("{} saved · {}", state.backups.len(), format_size(total));
        match latest {
            Some(age) => format!("{base} · latest {age}"),
            None => base,
        }
    };

    let mut header = row![
        column![
            text("Backups").size(TEXT_XL).font(UI_SEMIBOLD),
            text(summary).size(TEXT_SM).style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        action(
            label("Open folder", Icon::Folder),
            style::ghost,
            Some(Message::OpenFolder)
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    header = header.push(action(
        label(
            if state.creating { "Backing up" } else { "Create backup" },
            Icon::Plus,
        ),
        style::primary,
        (!state.creating).then_some(Message::Create),
    ));

    let mut body = column![header].spacing(SPACE_4);
    if state.creating {
        body = body.push(progress_view(state));
    }
    if !offline && !state.backups.is_empty() {
        body = body.push(
            text("Stop the server to restore a backup.")
                .size(TEXT_SM)
                .style(style::muted),
        );
    }
    body = body
        .push(list_view(state, offline))
        .push(Space::new().height(SPACE_4))
        .push(schedule_view(state));

    scrollable(body.padding([0, 0]).width(Length::Fill))
        .style(style::scroll)
        .height(Length::Fill)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match event {
        Event::BackupProgress(progress) if progress.server_id == id => {
            if progress.percent >= 0.0 && progress.stage.as_deref() != Some("complete") {
                state.creating = true;
                state.percent = progress.percent;
                state.stage = match progress.stage.as_deref() {
                    Some("archiving") => Some(BackupStage::Archiving),
                    Some("complete") => Some(BackupStage::Complete),
                    Some("calculating") => Some(BackupStage::Calculating),
                    _ => state.stage,
                };
            }
            Task::none()
        }
        Event::BackupCompleted(done) if done.server_id == id => {
            state.creating = false;
            state.cancelling = false;
            state.stage = None;
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
        sync_config(state, ctx);
    } else if (state.auto_enabled, state.interval.clone()) == state.saved {
        sync_config(state, ctx);
    }
    load(id)
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.server_id.is_some() {
        iced::time::every(std::time::Duration::from_secs(4)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    }
}
