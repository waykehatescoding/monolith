use std::path::Path;

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
use monolith_theme::token::BUTTON_MD;
use monolith_theme::token::*;
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, icon_colored, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{Confirm, Ctx};
use crate::ui::busy_button;

const MAX_INTERVAL_HOURS: u32 = 720;
const MAX_KEEP: u32 = 9999;

#[derive(Debug, Default, Clone, PartialEq)]
struct Form {
    auto_enabled: bool,
    interval: String,
    keep_last: String,
    keep_days: String,
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    backups: Vec<BackupEntry>,
    loaded: bool,
    creating: bool,
    cancelling: bool,
    percent: f64,
    stage: Option<BackupStage>,
    form: Form,
    saved: Form,
    saving: bool,
    locking: Option<String>,
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
    Lock(String, bool),
    Locked(String, Result<(), String>),
    OpenFolder,
    Tick,
    ToggleAuto(bool),
    Interval(String),
    KeepLast(String),
    KeepDays(String),
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

fn parse_limit(value: &str) -> Result<Option<u32>, ()> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    match value.trim().parse::<u32>() {
        Ok(n) if (1..=MAX_KEEP).contains(&n) => Ok(Some(n)),
        _ => Err(()),
    }
}

fn digits(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
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
    let limit = |value: Option<u32>| value.map(|n| n.to_string()).unwrap_or_default();
    state.form = Form {
        auto_enabled: config.enabled,
        interval: hours.to_string(),
        keep_last: limit(config.keep_last),
        keep_days: limit(config.keep_days),
    };
    state.saved = state.form.clone();
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
            Task::perform(
                backup_manager::create_backup(id.clone(), None),
                move |result| {
                    let outcome = if result.success {
                        Ok(())
                    } else {
                        Err(result.error.unwrap_or_else(|| "The backup failed.".into()))
                    };
                    Message::Created(id.clone(), outcome)
                },
            )
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
            let locked = state
                .backups
                .iter()
                .any(|backup| backup.filename == filename && backup.locked);
            let (title, body, label) = if locked {
                (
                    "Delete a locked backup?",
                    "You locked this backup so it would be kept. Deleting it removes the file from this computer for good. This cannot be undone.",
                    "Delete locked backup",
                )
            } else {
                (
                    "Delete this backup?",
                    "The backup file is removed from this computer. This cannot be undone.",
                    "Delete backup",
                )
            };
            ctx.confirm(
                Confirm::new(title, body, label, true, move || {
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
                })
                .busy_label("Deleting"),
            );
            Task::none()
        }
        Message::Lock(filename, locked) => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            state.locking = Some(filename.clone());
            Task::perform(
                backup_manager::set_backup_locked(id.clone(), filename, locked),
                move |result| {
                    Message::Locked(
                        id.clone(),
                        if result.success {
                            Ok(())
                        } else {
                            Err(result.error.unwrap_or_else(|| "Unknown error".into()))
                        },
                    )
                },
            )
        }
        Message::Locked(id, outcome) => {
            state.locking = None;
            if let Err(error) = outcome {
                ctx.error(format!("Could not change the lock: {error}"));
            }
            load(id)
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
            state.form.auto_enabled = value;
            Task::none()
        }
        Message::Interval(value) => {
            if value.chars().all(|c| c.is_ascii_digit()) {
                state.form.interval = value;
            }
            Task::none()
        }
        Message::KeepLast(value) => {
            state.form.keep_last = digits(&value);
            Task::none()
        }
        Message::KeepDays(value) => {
            state.form.keep_days = digits(&value);
            Task::none()
        }
        Message::SaveSchedule => {
            let (Some(id), Some(hours), Ok(keep_last), Ok(keep_days)) = (
                state.server_id.clone(),
                parse_interval(&state.form.interval),
                parse_limit(&state.form.keep_last),
                parse_limit(&state.form.keep_days),
            ) else {
                return Task::none();
            };
            state.saving = true;
            let current = ctx
                .selected_server()
                .and_then(|server| server.backup_config.clone())
                .unwrap_or_default();
            let config = BackupConfig {
                enabled: state.form.auto_enabled,
                interval_hours: f64::from(hours),
                keep_last,
                keep_days,
                ..current
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
                    state.saved = state.form.clone();
                    ctx.success("Backup settings saved");
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

fn entry_row<'a>(
    backup: &'a BackupEntry,
    can_restore: bool,
    busy: bool,
    locking: bool,
) -> Element<'a, Message> {
    let kind = match backup.kind {
        BackupType::Auto => "Automatic",
        BackupType::Manual => "Manual",
    };
    let mut meta = format!("{} · {}", format_date(&backup.created_at), kind);
    if backup.locked {
        meta.push_str(" · Locked");
    }
    let details = column![
        text(&backup.name).size(TEXT_MD).font(UI_MEDIUM),
        text(meta).size(TEXT_SM).style(style::muted),
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
            text(if backup.locked { "Unlock" } else { "Lock" })
                .size(TEXT_MD)
                .font(UI_MEDIUM)
                .into(),
            style::ghost,
            (!locking).then(|| Message::Lock(backup.filename.clone(), !backup.locked)),
        ),
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
                text(stage)
                    .size(TEXT_MD)
                    .font(UI_MEDIUM)
                    .width(Length::Fill),
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
        rows = rows.push(
            container(entry_row(
                backup,
                can_restore,
                state.creating,
                state.locking.as_deref() == Some(backup.filename.as_str()),
            ))
            .padding([SPACE_3, SPACE_4]),
        );
    }
    container(rows)
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

fn number_input<'a>(
    placeholder: &str,
    value: &'a str,
    valid: bool,
    submit: Option<Message>,
    on_input: fn(String) -> Message,
) -> Element<'a, Message> {
    text_input(placeholder, value)
        .size(TEXT_MD)
        .padding([8, 12])
        .width(72)
        .on_input(on_input)
        .on_submit_maybe(submit)
        .style(move |theme: &Theme, status| {
            if valid {
                style::input(theme, status)
            } else {
                style::input_error(theme, status)
            }
        })
        .into()
}

fn schedule_view(state: &State) -> Element<'_, Message> {
    let form = &state.form;
    let hours_ok = parse_interval(&form.interval).is_some();
    let keep_last_ok = parse_limit(&form.keep_last).is_ok();
    let keep_days_ok = parse_limit(&form.keep_days).is_ok();
    let valid = hours_ok && keep_last_ok && keep_days_ok;
    let can_save = valid && *form != state.saved && !state.saving;
    let submit = can_save.then_some(Message::SaveSchedule);

    let hint = if !hours_ok {
        text(format!(
            "Enter a whole number from 1 to {MAX_INTERVAL_HOURS}."
        ))
        .size(TEXT_SM)
        .color(CORAL)
    } else {
        text("Backups run while Monolith is open.")
            .size(TEXT_SM)
            .style(style::muted)
    };
    let retention_hint = if keep_last_ok && keep_days_ok {
        text("Leave empty for no limit. Applies to automatic backups only; manual and locked backups are never removed.")
            .size(TEXT_SM)
            .style(style::muted)
    } else {
        text(format!(
            "Enter a whole number from 1 to {MAX_KEEP}, or leave it empty."
        ))
        .size(TEXT_SM)
        .color(CORAL)
    };
    let save = busy_button(
        "Save settings",
        "Saving",
        state.saving,
        submit.clone(),
        style::primary,
        BUTTON_MD,
    );

    column![
        text("Automatic backups").size(TEXT_LG).font(UI_SEMIBOLD),
        toggler(form.auto_enabled)
            .label("Back up this server on a schedule")
            .size(18)
            .text_size(TEXT_MD)
            .spacing(SPACE_2)
            .style(style::switch)
            .on_toggle(Message::ToggleAuto),
        row![
            text("Every").size(TEXT_MD),
            number_input(
                "24",
                &form.interval,
                hours_ok,
                submit.clone(),
                Message::Interval
            ),
            text("hours").size(TEXT_MD),
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
        hint,
        Space::new().height(SPACE_2),
        text("Retention").size(TEXT_MD).font(UI_MEDIUM),
        row![
            text("Keep the last").size(TEXT_MD),
            number_input(
                "All",
                &form.keep_last,
                keep_last_ok,
                submit.clone(),
                Message::KeepLast
            ),
            text("automatic backups").size(TEXT_MD),
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
        row![
            text("Remove automatic backups older than").size(TEXT_MD),
            number_input(
                "Never",
                &form.keep_days,
                keep_days_ok,
                submit,
                Message::KeepDays
            ),
            text("days").size(TEXT_MD),
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
        retention_hint,
        Space::new().height(SPACE_2),
        row![save],
    ]
    .spacing(SPACE_3)
    .into()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let offline = state.server_id.as_deref().is_none_or(|id| {
        matches!(
            ctx.status(id),
            ServerRuntimeStatus::Offline | ServerRuntimeStatus::Crashed
        )
    });
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
            if state.creating {
                "Backing up"
            } else {
                "Create backup"
            },
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
            if progress.error.is_some() {
                state.creating = false;
                state.stage = None;
            } else if progress.percent >= 0.0 && progress.stage.as_deref() != Some("complete") {
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
    } else if state.form == state.saved {
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
