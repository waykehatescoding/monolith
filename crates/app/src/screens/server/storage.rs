use std::collections::HashMap;

use iced::widget::{column, row, text, Space};
use iced::{Alignment, Element, Length, Task};
use monolith_core::disk_usage::{
    all_servers_size, drive_space, server_disk_usage, DiskUsage, DriveSpace,
};
use monolith_core::types::ServerRecord;
use monolith_theme::style;
use monolith_theme::token::{BUTTON_MD, SPACE_2, SPACE_3, TEXT_MD, TEXT_SM};
use monolith_theme::{MONO, UI_SEMIBOLD};

use super::files::format_size;
use super::settings::section;
use crate::ui::busy_button;

#[derive(Debug, Clone)]
pub struct Measured {
    usage: DiskUsage,
    drive: Option<DriveSpace>,
    all: u64,
}

#[derive(Debug, Default)]
pub struct State {
    measured: HashMap<String, Measured>,
    loading: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Measured(String, Measured),
}

fn measure(state: &mut State, server: &ServerRecord) -> Task<Message> {
    state.loading = Some(server.id.clone());
    let id = server.id.clone();
    let path = server.server_path.clone();
    Task::perform(
        async move {
            let (usage, drive, all) = tokio::join!(
                server_disk_usage(path.clone()),
                drive_space(path),
                all_servers_size()
            );
            Measured { usage, drive, all }
        },
        move |measured| Message::Measured(id.clone(), measured),
    )
}

pub fn on_show(state: &mut State, server: &ServerRecord) -> Task<Message> {
    if state.measured.contains_key(&server.id) || state.loading.as_ref() == Some(&server.id) {
        return Task::none();
    }
    measure(state, server)
}

pub fn update(state: &mut State, message: Message, server: &ServerRecord) -> Task<Message> {
    match message {
        Message::Refresh => measure(state, server),
        Message::Measured(id, measured) => {
            if state.loading.as_ref() == Some(&id) {
                state.loading = None;
            }
            state.measured.insert(id, measured);
            Task::none()
        }
    }
}

fn line<'a>(label: impl text::IntoFragment<'a>, bytes: u64) -> Element<'a, Message> {
    row![
        text(label).size(TEXT_MD).width(Length::Fill),
        text(format_size(bytes)).size(TEXT_MD).font(MONO),
    ]
    .align_y(Alignment::Center)
    .into()
}

pub fn view<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let loading = state.loading.as_deref() == Some(server.id.as_str());
    let refresh = busy_button(
        "Refresh",
        "Measuring",
        loading,
        Some(Message::Refresh),
        style::secondary,
        BUTTON_MD,
    );
    let Some(measured) = state.measured.get(&server.id) else {
        return section(
            "Storage",
            vec![row![
                text("Measuring how much space this server uses.")
                    .size(TEXT_SM)
                    .style(style::muted)
                    .width(Length::Fill),
                refresh
            ]
            .align_y(Alignment::Center)
            .into()],
        );
    };
    let usage = &measured.usage;
    let breakdown = column![
        line("Worlds", usage.worlds),
        line("Backups", usage.backups),
        line("Plugins and mods", usage.content),
        line("Logs", usage.logs),
        line("Other files", usage.other),
        row![
            text("This server")
                .size(TEXT_MD)
                .font(UI_SEMIBOLD)
                .width(Length::Fill),
            text(format_size(usage.total())).size(TEXT_MD).font(MONO)
        ],
    ]
    .spacing(SPACE_2)
    .max_width(480);
    let mut totals = column![line("All servers", measured.all)]
        .spacing(SPACE_2)
        .max_width(480);
    if let Some(drive) = &measured.drive {
        totals = totals.push(line(format!("Free on {}", drive.mount), drive.free));
    }
    section(
        "Storage",
        vec![
            breakdown.into(),
            totals.into(),
            row![Space::new().width(Length::Fill), refresh]
                .spacing(SPACE_3)
                .into(),
        ],
    )
}
